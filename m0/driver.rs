//! M0 validation: drive the real `affidavit_program` through LEZ v0.4's actual execution
//! state machine (`lee::V03State`), the same code path the sequencer applies. This settles
//! whether the one-transaction design works at the state-machine level, not just in the raw
//! RISC Zero executor, and then runs both consumer paths against the state it produced.
//!
//! Six steps, all against `lee_core` v0.4.0-rc1 and the mainnet light-client fixtures:
//!   1. `Initialize` (public): creates the instance PDA and writes the tracked state.
//!   2. `Verify` -> `Apply` (public): the whole advance logic, proving the cross-actor
//!      `from_own_program()` authorization the one-transaction design relies on.
//!   3. `Verify` -> `Apply` (PRIVATE): the production placement, where the submitter proves
//!      `Verify` off-chain and only the result crosses to the public instance.
//!   4. The rest of the fixture chain, privately: five more committee handoffs and the
//!      finality update, each a private `Verify` -> public `Apply` (+ `Record` of the newly
//!      finalised header), until the on-chain instance equals the native follower.
//!   5. `AttestRecord` (public consumer path) against a recorded header, and two refusals.
//!   6. `AttestPrivate` -> `CheckRoot` (private consumer path), and one refusal.
//!
//! RISC0_DEV_MODE=1 skips proof generation (the mechanism, not the proof, is under test).

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::execution::TransactionProof;
use affidavit_core::lc::InstanceConfig;
use affidavit_core::mmr::Mmr;
use affidavit_core::ssz;
use affidavit_core::statement::{attest, Clause, Evidence, Fact, Field, Op, Predicate};
use affidavit_core::types::LightClientUpdate;
use affidavit_io::beacon;
use affidavit_io::follower::Follower;
use affidavit_lez::{
    header_actor, header_seed, instance_actor, instance_seed, HeaderAccount, InstanceAccount, Message,
};
use alloy_primitives::{b256, Bytes, B256, U256};
use lee::privacy_preserving_transaction::circuit::ProgramCatalog;
use lee::privacy_preserving_transaction::message::Message as PpMessage;
use lee::privacy_preserving_transaction::witness_set::WitnessSet as PpWitnessSet;
use lee::privacy_preserving_transaction::PrivacyPreservingTransaction;
use lee::program::Program;
use lee::public_transaction::{Message as PubMessage, WitnessSet};
use lee::test_utils::{init_witness, no_seal, TestPrivateKeys};
use lee::{
    execute_and_prove, AccountId, PublicAccountEvidence, PublicExecutionContext, PublicTransaction, SenderPresentation,
    Simulation, V03State,
};
use lee_core::account::Actor;
use lee_core::execution_state::TransactionEntry;
use lee_core::program::PdaSeed;
use lee_core::{AuthorizationSecretKey, RootCall};

const CHECKPOINT: B256 = b256!("5afc212a7924789b2bc86acad3ab3a6ffb1f6e97253ea50bee7f4f51422c9275");
const NOW_SLOT: u64 = 7_109_500;
const PROGRAM_ID: [u8; 32] = [0xaf; 32];

fn testdata(p: &str) -> String {
    std::fs::read_to_string(format!("/home/user/affidavit/testdata/{p}")).expect("fixture")
}

fn load_program() -> Program {
    let elf = std::fs::read(
        "/home/user/affidavit/lez/program/target-zkvm/riscv32im-risc0-zkvm-elf/release/affidavit_program",
    )
    .expect("program binary (run lez/build-program.sh)");
    let wrapped = risc0_binfmt::ProgramBinary::new(&elf, risc0_zkos_v1compat::V1COMPAT_ELF).encode();
    Program::new(wrapped.into()).expect("valid program binary")
}

fn config() -> InstanceConfig {
    InstanceConfig {
        chain: ChainSpec::mainnet(),
        checkpoint_root: CHECKPOINT,
        ws_period_epochs: weak_subjectivity_period_epochs(24_000_000 * 1_000_000_000),
    }
}

/// The six period updates (862..867) and the finality update, in order.
fn all_updates() -> anyhow::Result<Vec<LightClientUpdate>> {
    let mut updates = beacon::parse_updates(&testdata("mainnet/light-client/updates.json"))?;
    updates.push(beacon::parse_finality_update(&testdata("mainnet/light-client/finality.json"))?);
    Ok(updates)
}

/// The bytes of a public actor's state, as settled.
fn state_bytes(state: &V03State, actor: Actor) -> Vec<u8> {
    let account = state.get_account_by_id(actor.account_id);
    let bytes: &[u8] = account.data.actor_state(actor.program_account_id).borrow();
    bytes.to_vec()
}

fn instance_account(state: &V03State, instance: Actor) -> anyhow::Result<InstanceAccount> {
    Ok(borsh::from_slice(&state_bytes(state, instance))?)
}

fn header_account(state: &V03State, record: Actor) -> anyhow::Result<HeaderAccount> {
    Ok(borsh::from_slice(&state_bytes(state, record))?)
}

/// A public transaction whose root call goes to `to` (a PDA of the program, so no signature),
/// settled on `state`. Errors come back as text so refusals can be matched on their code.
fn public_call(
    state: &mut V03State,
    to: Actor,
    actors: Vec<Actor>,
    evidence: Vec<PublicAccountEvidence>,
    message: Message,
    block: u64,
) -> Result<(), String> {
    let base = PubMessage::try_new(to, actors, BTreeMap::new(), message).map_err(|e| e.to_string())?;
    let msg = PubMessage { admission_evidence: evidence, ..base };
    let tx = PublicTransaction::new(msg.clone(), WitnessSet::for_message(&msg, &[]));
    state.transition_from_public_transaction(&tx, block, 0).map(|_| ()).map_err(|e| e.to_string())
}

/// A privacy-preserving transaction: `message` is delivered to a fresh private account of the
/// program (keyed by `key`), executed and proven off-chain against the declared public actors'
/// current states (a PDA that does not exist yet is simply empty), then settled on `state`.
/// `new_pdas` are the program-derived public accounts the execution creates, admitted by seed.
fn private_call(
    state: &mut V03State,
    program: AccountId,
    key: u8,
    message: &Message,
    public_actors: Vec<Actor>,
    new_pdas: Vec<PdaSeed>,
    block: u64,
) -> Result<(), String> {
    let keys = TestPrivateKeys {
        ask: AuthorizationSecretKey([key; 32]),
        d: [key.wrapping_add(1); 32],
        z: [key.wrapping_add(2); 32],
    };
    let private_actor = Actor::new(AccountId::for_regular_private_account(&keys.npk(), &keys.vpk()), program);
    let root: TransactionEntry<lee_core::MessageWitness> = TransactionEntry::Sync(RootCall {
        to: private_actor,
        message: borsh::to_vec(message).map_err(|e| e.to_string())?,
    });
    let input = lee::ProvingInput {
        claims: Vec::new(),
        root,
        public_context: PublicExecutionContext::new(public_actors.clone(), []),
        private_witnesses: vec![init_witness(&keys)],
        dummy_inputs: Vec::new(),
        ciphertext_padding: None,
        recoveries: Vec::new(),
        private_promotions: BTreeSet::new(),
        fee: None,
    };
    let catalog = ProgramCatalog::from([(program, load_program())]);
    // The submitter supplies the current public states the private execution touches: exactly
    // the snapshot the design hands the verifier.
    let mut sim = Simulation::default();
    sim.admitted_accounts = None;
    for actor in &public_actors {
        let current = state.get_account_by_id(actor.account_id).data.actor_state(actor.program_account_id).clone();
        if !current.is_empty() {
            sim.public_actor_states.insert(*actor, current);
        }
    }
    let (output, proof) =
        execute_and_prove(input, &sim, &catalog, |_| SenderPresentation::Canonical, |_, _| false, no_seal)
            .map_err(|e| format!("prove stage: {e}"))?;
    let mut msg = PpMessage::from_circuit_output(BTreeMap::new(), output);
    msg.admission_evidence = new_pdas.into_iter().map(|seed| PublicAccountEvidence::Pda { program, seed }).collect();
    let tx = PrivacyPreservingTransaction::new(msg.clone(), PpWitnessSet::for_message(&msg, proof, &[]));
    state.transition_from_privacy_preserving_transaction(&tx, block, 0).map(|_| ()).map_err(|e| format!("apply stage: {e}"))
}

fn expect_refusal(label: &str, result: Result<(), String>, code: &str) -> anyhow::Result<()> {
    match result {
        Err(e) if e.contains(code) => {
            println!("    refused as expected with {code}: {label}");
            Ok(())
        }
        Err(e) => anyhow::bail!("{label}: refused with an unexpected error: {e}"),
        Ok(()) => anyhow::bail!("{label}: accepted, but should have been refused with {code}"),
    }
}

/// A fresh state with the program registered and the instance initialised publicly.
fn fresh_initialised() -> anyhow::Result<(V03State, InstanceAccount)> {
    let program_account_id = AccountId::new(PROGRAM_ID);
    let cfg = config();
    let bootstrap = beacon::parse_bootstrap(&testdata("mainnet/light-client/bootstrap.json"))?;
    let slot = bootstrap.header.beacon.slot;
    let instance = instance_actor(program_account_id, &cfg.id());
    let header_rec = header_actor(program_account_id, &cfg.id(), slot);
    let mut state = V03State::new().with_named_programs([(program_account_id, load_program())]);

    let init = Message::Initialize { config: cfg.clone(), bootstrap, now_slot: NOW_SLOT };
    public_call(
        &mut state,
        instance,
        vec![instance, header_rec],
        vec![
            PublicAccountEvidence::Pda { program: program_account_id, seed: instance_seed(&cfg.id()) },
            PublicAccountEvidence::Pda { program: program_account_id, seed: header_seed(&cfg.id(), slot) },
        ],
        init,
        1,
    )
    .map_err(anyhow::Error::msg)?;
    let decoded = instance_account(&state, instance)?;
    Ok((state, decoded))
}

fn verify_message(snapshot: &InstanceAccount) -> anyhow::Result<Message> {
    let cfg = config();
    let bootstrap = beacon::parse_bootstrap(&testdata("mainnet/light-client/bootstrap.json"))?;
    let updates = beacon::parse_updates(&testdata("mainnet/light-client/updates.json"))?;
    let follower = Follower::new(cfg, &bootstrap, NOW_SLOT)?;
    let update0 = updates[0].clone();
    let committee = follower.signing_committee(&update0);
    let witness = Follower::committee_witness(&committee, &update0.sync_aggregate);
    Ok(Message::Verify { snapshot: snapshot.instance.clone(), update: update0, committee, witness: Some(witness), now_slot: NOW_SLOT })
}

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    println!("# M0: affidavit_program through LEZ v0.4 V03State (lee_core v0.4.0-rc1)\n");
    let program_account_id = AccountId::new(PROGRAM_ID);
    let cfg = config();
    let instance = instance_actor(program_account_id, &cfg.id());

    // === 1. Initialize (public) ===
    let (mut state, decoded) = fresh_initialised()?;
    println!(
        "[1] Initialize (public): instance PDA {} created, finalized slot {}, period {}",
        instance.account_id,
        decoded.instance.state.finalized.slot,
        cfg.chain.period_at_slot(decoded.instance.state.finalized.slot)
    );
    anyhow::ensure!(decoded.instance.config.id() == cfg.id(), "instance id mismatch");
    println!("    PASS\n");

    // === 2. Verify -> Apply (public): the cross-actor authorization ===
    let verifier_seed = PdaSeed::new([0x51; 32]);
    let verifier = Actor::new(AccountId::for_public_pda(&program_account_id, &verifier_seed), program_account_id);
    public_call(
        &mut state,
        verifier,
        vec![verifier, instance],
        vec![
            PublicAccountEvidence::Pda { program: program_account_id, seed: verifier_seed },
            PublicAccountEvidence::Pda { program: program_account_id, seed: instance_seed(&cfg.id()) },
        ],
        verify_message(&decoded)?,
        2,
    )
    .map_err(anyhow::Error::msg)?;
    let after = instance_account(&state, instance)?;
    println!(
        "[2] Verify -> Apply (public): updates_applied {}, next committee learned {}",
        after.instance.state.updates_applied,
        after.instance.state.next_committee_root.is_some()
    );
    anyhow::ensure!(after.instance.state.next_committee_root.is_some(), "did not advance");
    println!("    Apply accepted: the instance's from_own_program() authorization held.\n    PASS\n");

    // === 3. Verify -> Apply (PRIVATE): the production placement ===
    // The submitter proves Verify off-chain in a privacy-preserving transaction whose private
    // actor emits Apply to the public instance. We run it against a fresh initialised instance.
    let (mut pstate, pdecoded) = fresh_initialised()?;
    let mut next_block = 2u64;
    println!("[3] Verify -> Apply (PRIVATE): proving the private transition...");
    let t = Instant::now();
    match private_call(&mut pstate, program_account_id, 13, &verify_message(&pdecoded)?, vec![instance], vec![], next_block)
    {
        Ok(()) => {
            next_block += 1;
            let a = instance_account(&pstate, instance)?;
            println!(
                "    Private Apply landed: updates_applied {}, next committee learned {} ({:.1} s)",
                a.instance.state.updates_applied,
                a.instance.state.next_committee_root.is_some(),
                t.elapsed().as_secs_f32()
            );
            anyhow::ensure!(a.instance.state.next_committee_root.is_some(), "private tx applied but instance did not advance");
            println!("    PASS: a privately-proven Verify advanced the PUBLIC instance.\n");
        }
        Err(e) => {
            println!("    DIAGNOSIS: {e}");
            anyhow::bail!("step 3 failed");
        }
    }

    // === 4. The rest of the mainnet fixture chain, privately, on the same instance ===
    // Five more committee handoffs (periods 863..867) and the finality update. Each one is a
    // private Verify whose Apply lands on the public instance and, when finality advances,
    // writes the new header record (a write-once PDA the transaction admits by seed). A native
    // follower runs alongside as the expected result.
    println!("[4] The whole fixture chain, privately: updates 1..6 as private Verify -> public Apply (+ Record)");
    let bootstrap = beacon::parse_bootstrap(&testdata("mainnet/light-client/bootstrap.json"))?;
    let updates = all_updates()?;
    let mut follower = Follower::new(cfg.clone(), &bootstrap, NOW_SLOT)?;
    follower.process(&updates[0], NOW_SLOT)?; // the instance of step 3 already carries update 0
    let mut records = 1usize; // the bootstrap header, recorded by Initialize
    for (i, update) in updates.iter().enumerate().skip(1) {
        let snapshot = instance_account(&pstate, instance)?;
        let committee = follower.signing_committee(update);
        let witness = Follower::committee_witness(&committee, &update.sync_aggregate);
        let verify = Message::Verify {
            snapshot: snapshot.instance.clone(),
            update: update.clone(),
            committee,
            witness: Some(witness),
            now_slot: NOW_SLOT,
        };
        let expected = follower.process(update, NOW_SLOT)?;
        let mut actors = vec![instance];
        let mut new_pdas = Vec::new();
        if let Some(record) = expected.new_finalized {
            actors.push(header_actor(program_account_id, &cfg.id(), record.slot));
            new_pdas.push(header_seed(&cfg.id(), record.slot));
        }
        let t = Instant::now();
        private_call(&mut pstate, program_account_id, 20 + i as u8, &verify, actors, new_pdas, next_block)
            .map_err(|e| anyhow::anyhow!("update {i}: {e}"))?;
        next_block += 1;
        let after = instance_account(&pstate, instance)?;
        let kind = if update.next_sync_committee.is_some() { "committee handoff" } else { "finality update" };
        match expected.new_finalized {
            Some(record) => {
                let record_at = header_actor(program_account_id, &cfg.id(), record.slot);
                let written = header_account(&pstate, record_at)?;
                anyhow::ensure!(written.record == record, "update {i}: header record differs from the follower's");
                records += 1;
                println!(
                    "    update {i} ({kind}): Apply + Record landed: finalized slot {} (block {}), record PDA {}, updates_applied {} ({:.1} s)",
                    record.slot, record.block_number, record_at.account_id, after.instance.state.updates_applied,
                    t.elapsed().as_secs_f32()
                );
            }
            None => println!(
                "    update {i} ({kind}): Apply landed: committees only, updates_applied {} ({:.1} s)",
                after.instance.state.updates_applied,
                t.elapsed().as_secs_f32()
            ),
        }
    }
    let synced = instance_account(&pstate, instance)?;
    anyhow::ensure!(synced.instance == follower.instance, "on-chain instance differs from the native follower");
    anyhow::ensure!(records == follower.records.len(), "record count differs from the follower's");
    let head = synced.instance.state.finalized;
    println!(
        "    on-chain instance == native follower: finalized slot {} (block {}), {} updates applied, {} header records, accumulator root {}",
        head.slot, head.block_number, synced.instance.state.updates_applied, records, synced.instance.state.headers.root()
    );
    println!("    PASS\n");

    // === 5. AttestRecord (public consumer path) ===
    // A consumer asks the header record of the finalised block for an attestation of the
    // block's first transaction (a native ETH transfer) under the predicate "value > 0". At
    // the top level nobody receives the Reply, so acceptance is the result: the handler
    // refuses (and the transaction fails) unless the evidence verifies under the record and
    // the predicate holds. The same statement is computed offline as the one a consumer
    // program receives.
    let record_at = header_actor(program_account_id, &cfg.id(), head.slot);
    let (_, txs) = beacon::parse_block_transactions(&testdata("mainnet/light-client/beacon-block-7109344.json"))?;
    let evidence = Evidence::Transfer {
        tx: TransactionProof { tx_index: 0, tx: Bytes::from(txs[0].clone()), branch: ssz::transaction_branch(&txs, 0) },
        receipt: None,
    };
    let predicate = Predicate(vec![Clause { field: Field::TxValue, op: Op::Gt, value: U256::ZERO }]);
    let statement = attest(&cfg.chain, cfg.id(), &head, &evidence, &predicate)?;
    let record_evidence = vec![PublicAccountEvidence::Pda { program: program_account_id, seed: header_seed(&cfg.id(), head.slot) }];
    let t = Instant::now();
    public_call(
        &mut pstate,
        record_at,
        vec![record_at],
        record_evidence.clone(),
        Message::AttestRecord { chain: cfg.chain.clone(), evidence: evidence.clone(), predicate: predicate.clone() },
        next_block,
    )
    .map_err(|e| anyhow::anyhow!("AttestRecord: {e}"))?;
    next_block += 1;
    println!(
        "[5] AttestRecord (public): accepted at header record slot {} (block {}), {} transactions in the block ({:.1} s)",
        head.slot,
        head.block_number,
        txs.len(),
        t.elapsed().as_secs_f32()
    );
    if let Fact::Transaction { tx_hash, from, to, value, .. } = &statement.fact {
        println!(
            "    statement id {}: tx {tx_hash} from {from} to {} value {value} wei",
            statement.id,
            to.map_or("-".to_string(), |a| a.to_string())
        );
    }
    expect_refusal(
        "the same evidence under a predicate it does not satisfy (value > 2^256-1)",
        public_call(
            &mut pstate,
            record_at,
            vec![record_at],
            record_evidence.clone(),
            Message::AttestRecord {
                chain: cfg.chain.clone(),
                evidence: evidence.clone(),
                predicate: Predicate(vec![Clause { field: Field::TxValue, op: Op::Gt, value: U256::MAX }]),
            },
            next_block,
        ),
        "AFF-301",
    )?;
    expect_refusal(
        "a Sepolia consumer at a mainnet record",
        public_call(
            &mut pstate,
            record_at,
            vec![record_at],
            record_evidence,
            Message::AttestRecord { chain: ChainSpec::sepolia(), evidence: evidence.clone(), predicate: predicate.clone() },
            next_block,
        ),
        "AFF-101",
    )?;
    anyhow::ensure!(header_account(&pstate, record_at)?.record == head, "a refused transaction changed the record");
    println!("    PASS\n");

    // === 6. AttestPrivate -> CheckRoot (private consumer path) ===
    // A private actor proves the header is in the instance's accumulator and the evidence
    // under it, and discloses only the accumulator root to the public instance, which
    // confirms it is one of its recent roots. The instance's state is untouched.
    let index = follower.records.len() as u64 - 1;
    let membership = follower.headers.prove(index).expect("the head is in the follower's accumulator");
    let root = synced.instance.state.headers.root();
    anyhow::ensure!(root == follower.instance.state.headers.root(), "accumulator roots differ");
    let before = state_bytes(&pstate, instance);
    let t = Instant::now();
    private_call(
        &mut pstate,
        program_account_id,
        0x99,
        &Message::AttestPrivate {
            chain: cfg.chain.clone(),
            instance_id: cfg.id(),
            headers_root: root,
            header: head,
            membership,
            evidence: evidence.clone(),
            predicate: predicate.clone(),
        },
        vec![instance],
        vec![],
        next_block,
    )
    .map_err(|e| anyhow::anyhow!("AttestPrivate: {e}"))?;
    next_block += 1;
    anyhow::ensure!(state_bytes(&pstate, instance) == before, "CheckRoot changed the instance");
    println!(
        "[6] AttestPrivate -> CheckRoot (PRIVATE): accepted; the public crossing disclosed only accumulator root {root}; instance state unchanged ({:.1} s)",
        t.elapsed().as_secs_f32()
    );
    // A root the instance never produced, under which the header's membership nevertheless
    // verifies (an accumulator with one extra, forged leaf): CheckRoot refuses it.
    let mut forged = Mmr::default();
    for record in &follower.records {
        forged.append(record.leaf());
    }
    forged.append(B256::repeat_byte(0xfa));
    let forged_membership = forged.prove(index).expect("member of the forged accumulator");
    expect_refusal(
        "an attestation under an accumulator root the instance never produced",
        private_call(
            &mut pstate,
            program_account_id,
            0x9a,
            &Message::AttestPrivate {
                chain: cfg.chain.clone(),
                instance_id: cfg.id(),
                headers_root: forged.accumulator().root(),
                header: head,
                membership: forged_membership,
                evidence,
                predicate,
            },
            vec![instance],
            vec![],
            next_block,
        ),
        "AFF-210",
    )?;
    println!("    PASS\n");

    println!("total wall-clock {:.0} s", started.elapsed().as_secs_f32());
    Ok(())
}
