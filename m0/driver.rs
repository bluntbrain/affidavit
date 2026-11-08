//! M0 validation: drive the real `affidavit_program` through LEZ v0.4's actual execution
//! state machine (`lee::V03State`), the same code path the sequencer applies. This settles
//! whether the one-transaction design works at the state-machine level, not just in the raw
//! RISC Zero executor.
//!
//! Three steps, all against `lee_core` v0.4.0-rc1:
//!   1. `Initialize` (public): creates the instance PDA and writes the tracked state.
//!   2. `Verify` -> `Apply` (public): the whole advance logic, proving the cross-actor
//!      `from_own_program()` authorization the one-transaction design relies on.
//!   3. `Verify` -> `Apply` (PRIVATE): the production placement, where the submitter proves
//!      `Verify` off-chain and only the result crosses to the public instance.
//!
//! RISC0_DEV_MODE=1 skips proof generation (the mechanism, not the proof, is under test).

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};

use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::lc::InstanceConfig;
use affidavit_io::beacon;
use affidavit_io::follower::Follower;
use affidavit_lez::{header_actor, header_seed, instance_actor, instance_seed, InstanceAccount, Message};
use alloy_primitives::{b256, B256};
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
    let base = PubMessage::try_new(instance, vec![instance, header_rec], BTreeMap::new(), init)?;
    let msg = PubMessage {
        admission_evidence: vec![
            PublicAccountEvidence::Pda { program: program_account_id, seed: instance_seed(&cfg.id()) },
            PublicAccountEvidence::Pda { program: program_account_id, seed: header_seed(&cfg.id(), slot) },
        ],
        ..base
    };
    let tx = PublicTransaction::new(msg.clone(), WitnessSet::for_message(&msg, &[]));
    state.transition_from_public_transaction(&tx, 1, 0)?;
    let decoded: InstanceAccount =
        borsh::from_slice(state.get_account_by_id(instance.account_id).data.actor_state(program_account_id).borrow())?;
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
    let verify = verify_message(&decoded)?;
    let base = PubMessage::try_new(verifier, vec![verifier, instance], BTreeMap::new(), verify)?;
    let vmsg = PubMessage {
        admission_evidence: vec![
            PublicAccountEvidence::Pda { program: program_account_id, seed: verifier_seed },
            PublicAccountEvidence::Pda { program: program_account_id, seed: instance_seed(&cfg.id()) },
        ],
        ..base
    };
    let vtx = PublicTransaction::new(vmsg.clone(), WitnessSet::for_message(&vmsg, &[]));
    state.transition_from_public_transaction(&vtx, 2, 0)?;
    let after: InstanceAccount =
        borsh::from_slice(state.get_account_by_id(instance.account_id).data.actor_state(program_account_id).borrow())?;
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
    let keys = TestPrivateKeys { ask: AuthorizationSecretKey([13; 32]), d: [31; 32], z: [32; 32] };
    let priv_account = AccountId::for_regular_private_account(&keys.npk(), &keys.vpk());
    let priv_actor = Actor::new(priv_account, program_account_id);
    let verify = verify_message(&pdecoded)?;

    let root: TransactionEntry<lee_core::MessageWitness> =
        TransactionEntry::Sync(RootCall { to: priv_actor, message: borsh::to_vec(&verify)? });
    let input = lee::ProvingInput {
        claims: Vec::new(),
        root,
        public_context: PublicExecutionContext::new(vec![instance], []),
        private_witnesses: vec![init_witness(&keys)],
        dummy_inputs: Vec::new(),
        ciphertext_padding: None,
        recoveries: Vec::new(),
        private_promotions: BTreeSet::new(),
        fee: None,
    };
    let catalog = ProgramCatalog::from([(program_account_id, load_program())]);
    // The submitter supplies the public instance's current state, so the private execution can
    // simulate the Apply against it (exactly the snapshot the design hands the verifier).
    let mut sim = Simulation::default();
    sim.admitted_accounts = None;
    let instance_state = pstate.get_account_by_id(instance.account_id).data.actor_state(program_account_id).clone();
    sim.public_actor_states.insert(instance, instance_state);
    println!("[3] Verify -> Apply (PRIVATE): proving the private transition...");
    match execute_and_prove(input, &sim, &catalog, |_| SenderPresentation::Canonical, |_, _| false, no_seal) {
        Ok((output, proof)) => {
            let msg = PpMessage::from_circuit_output(BTreeMap::new(), output);
            let tx = PrivacyPreservingTransaction::new(msg.clone(), PpWitnessSet::for_message(&msg, proof, &[]));
            match pstate.transition_from_privacy_preserving_transaction(&tx, 2, 0) {
                Ok(_) => {
                    let a: InstanceAccount = borsh::from_slice(
                        pstate.get_account_by_id(instance.account_id).data.actor_state(program_account_id).borrow(),
                    )?;
                    println!(
                        "    Private Apply landed: updates_applied {}, next committee learned {}",
                        a.instance.state.updates_applied,
                        a.instance.state.next_committee_root.is_some()
                    );
                    if a.instance.state.next_committee_root.is_some() {
                        println!("    PASS: a privately-proven Verify advanced the PUBLIC instance.");
                    } else {
                        println!("    PARTIAL: private tx applied but instance did not advance.");
                    }
                }
                Err(e) => println!("    DIAGNOSIS (apply stage): {e:?}"),
            }
        }
        Err(e) => println!("    DIAGNOSIS (prove stage): {e:?}"),
    }
    Ok(())
}
