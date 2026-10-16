//! Cycles per program action of the LEZ program binary itself (`lez/build-program.sh`),
//! executed in the RISC Zero executor with LEZ's own input framing (`to_borsh_frame` of a
//! `ReceiveInput`), on the mainnet fixtures. This is the per-action breakdown of
//! Performance #2: on LEZ, public execution gas is these cycles.
//!
//! With `--features prove`, the private actions are also proven the way their submitter
//! would prove them, into a succinct receipt that is then verified. Proving needs RISC
//! Zero's C++ kernels, so that build runs without `RISC0_SKIP_BUILD_KERNELS`, in its own
//! target directory.

use std::path::PathBuf;

use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::execution::TransactionProof;
use affidavit_core::lc::InstanceConfig;
use affidavit_core::ssz;
use affidavit_core::statement::{Clause, Evidence, Field, Op, Predicate};
use affidavit_io::beacon;
use affidavit_io::follower::Follower;
use affidavit_lez::{handle_message, header_actor, instance_actor, Message};
use alloy_primitives::{b256, Bytes, B256, U256};
use anyhow::{bail, Context, Result};
use lee_core::account::{AccountId, Actor, ActorState};
use lee_core::program::{ReceiveInput, Response};
use risc0_zkvm::{default_executor, ExecutorEnv, ExitCode};

const CHECKPOINT: B256 = b256!("5afc212a7924789b2bc86acad3ab3a6ffb1f6e97253ea50bee7f4f51422c9275");
const NOW_SLOT: u64 = 7_109_500;

fn testdata(path: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(path))
        .expect("fixture")
}

fn binary() -> Result<Vec<u8>> {
    let elf = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../lez/program/target-zkvm/riscv32im-risc0-zkvm-elf/release/affidavit_program"),
    )
    .context("program ELF missing: run lez/build-program.sh first")?;
    Ok(risc0_binfmt::ProgramBinary::new(&elf, risc0_zkos_v1compat::V1COMPAT_ELF).encode())
}

fn input(receiver: Actor, from: Option<Actor>, pre_state: &[u8], message: &Message) -> ReceiveInput {
    ReceiveInput {
        receiver,
        from,
        is_authorized: false,
        pre_state: ActorState::from(pre_state.to_vec()),
        message: borsh::to_vec(message).expect("encodes"),
    }
}

/// Executes one delivery in the zkVM and returns its user cycles; also runs it natively
/// so the caller can continue the flow with the real post-state.
fn measure(binary: &[u8], label: &str, path: &str, input: &ReceiveInput) -> Result<Response> {
    let env = ExecutorEnv::builder().write_slice(&lee_core::to_borsh_frame(input)).build()?;
    let session = default_executor().execute(env, binary)?;
    if session.exit_code != ExitCode::Halted(0) {
        bail!("{label}: exited with {:?}", session.exit_code);
    }
    let cycles: u64 = session.segments.iter().map(|s| s.cycles as u64).sum();
    let padded: u64 = session.segments.iter().map(|s| 1u64 << s.po2).sum();
    println!("| {label} | {path} | {cycles} | {padded} |");
    // `PROVE_ONLY=AttestPrivate` (any part of a label) proves only the matching actions.
    #[cfg(feature = "prove")]
    if path.starts_with("private") && std::env::var("PROVE_ONLY").map_or(true, |only| label.contains(&only)) {
        prove(binary, label, input)?;
    }
    Ok(handle_message(input))
}

/// Proves one delivery into a succinct receipt (the kind LEZ verifies for a private
/// transaction, before its privacy-preserving composition) and verifies it.
#[cfg(feature = "prove")]
fn prove(binary: &[u8], label: &str, input: &ReceiveInput) -> Result<()> {
    use risc0_zkvm::{compute_image_id, default_prover, ProverOpts};
    let env = ExecutorEnv::builder().write_slice(&lee_core::to_borsh_frame(input)).build()?;
    let started = std::time::Instant::now();
    let info = default_prover().prove_with_opts(env, binary, &ProverOpts::succinct())?;
    let seconds = started.elapsed().as_secs_f64();
    info.receipt.verify(compute_image_id(binary)?)?;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    eprintln!(
        "PROOF | {label} | {seconds:.0} s | {} segments | succinct seal {} bytes | {threads} threads |",
        info.stats.segments,
        info.receipt.seal_size()
    );
    Ok(())
}

fn main() -> Result<()> {
    let binary = binary()?;
    let program = AccountId::new([0xaf; 32]);
    let config = InstanceConfig {
        chain: ChainSpec::mainnet(),
        checkpoint_root: CHECKPOINT,
        ws_period_epochs: weak_subjectivity_period_epochs(24_000_000 * 1_000_000_000),
    };
    let bootstrap = beacon::parse_bootstrap(&testdata("mainnet/light-client/bootstrap.json"))?;
    let mut updates = beacon::parse_updates(&testdata("mainnet/light-client/updates.json"))?;
    updates.push(beacon::parse_finality_update(&testdata("mainnet/light-client/finality.json"))?);
    let instance_at = instance_actor(program, &config.id());
    let verifier = Actor::new(AccountId::new([0x51; 32]), program);
    let consumer = Actor::new(AccountId::new([0xc0; 32]), AccountId::new([0xc1; 32]));

    println!("LEZ program binary (lee_core d7ba18f, actor model), RISC Zero executor 3.0.5, mainnet fixtures\n");
    println!("| action | runs | user cycles | padded cycles |");
    println!("|---|---|---:|---:|");

    let init = input(
        instance_at,
        None,
        &[],
        &Message::Initialize { config: config.clone(), bootstrap: bootstrap.clone(), now_slot: NOW_SLOT },
    );
    let response = measure(&binary, "Initialize (bootstrap check, once per instance)", "public", &init)?;
    let mut instance_state = response.post_state.expect("state").to_vec();
    let mut follower = Follower::new(config.clone(), &bootstrap, NOW_SLOT)?;
    let mut last_record: Option<(Actor, Vec<u8>)> = None;

    for (i, update) in updates.iter().enumerate() {
        let account: affidavit_lez::InstanceAccount = borsh::from_slice(&instance_state)?;
        let committee = follower.signing_committee(update);
        let verify = input(
            verifier,
            None,
            &[],
            &Message::Verify {
                snapshot: account.instance,
                update: update.clone(),
                witness: Some(Follower::committee_witness(&committee, &update.sync_aggregate)),
                committee,
                now_slot: NOW_SLOT,
            },
        );
        let measured = [1usize, updates.len() - 1].contains(&i);
        let verified = if measured {
            let label = if update.next_sync_committee.is_some() {
                "Verify (update with handoff)"
            } else {
                "Verify (finality update)"
            };
            measure(&binary, label, "private: proven by the submitter", &verify)?
        } else {
            handle_message(&verify)
        };
        let apply_msg: Message = borsh::from_slice(&verified.sync_messages[0].message)?;
        let apply = input(instance_at, Some(verifier), &instance_state, &apply_msg);
        let applied = if measured {
            measure(&binary, "Apply (instance compare-and-swap)", "public", &apply)?
        } else {
            handle_message(&apply)
        };
        instance_state = applied.post_state.expect("state").to_vec();
        if let Some(m) = applied.sync_messages.first() {
            let record_msg: Message = borsh::from_slice(&m.message)?;
            let record = input(m.to, Some(instance_at), &[], &record_msg);
            let written = if measured {
                measure(&binary, "Record (write-once header record)", "public", &record)?
            } else {
                handle_message(&record)
            };
            last_record = Some((m.to, written.post_state.expect("state").to_vec()));
        }
        follower.process(update, NOW_SLOT)?;
    }

    // Consumers: the native ETH transfer in the verified block.
    let (_, txs) = beacon::parse_block_transactions(&testdata("mainnet/light-client/beacon-block-7109344.json"))?;
    let evidence = Evidence::Transfer {
        tx: TransactionProof { tx_index: 0, tx: Bytes::from(txs[0].clone()), branch: ssz::transaction_branch(&txs, 0) },
        receipt: None,
    };
    let predicate = Predicate(vec![Clause { field: Field::TxValue, op: Op::Gt, value: U256::ZERO }]);
    let (record_at, record_state) = last_record.expect("records");
    assert_eq!(record_at, header_actor(program, &config.id(), follower.instance.state.finalized.slot));
    let attest = input(
        record_at,
        Some(consumer),
        &record_state,
        &Message::AttestRecord {
            chain: ChainSpec::mainnet(),
            evidence: evidence.clone(),
            predicate: predicate.clone(),
        },
    );
    let _ = measure(&binary, "AttestRecord (native ETH transfer, public consumer)", "public", &attest)?;

    let membership = follower.headers.prove(follower.records.len() as u64 - 1).expect("member");
    let attester = Actor::new(AccountId::new([0x99; 32]), program);
    let private = input(
        attester,
        Some(consumer),
        &[],
        &Message::AttestPrivate {
            chain: ChainSpec::mainnet(),
            instance_id: config.id(),
            headers_root: follower.instance.state.headers.root(),
            header: follower.instance.state.finalized,
            membership,
            evidence,
            predicate,
        },
    );
    let attested = measure(
        &binary,
        "AttestPrivate (native ETH transfer, private consumer)",
        "private: proven by the user",
        &private,
    )?;
    let check_msg: Message = borsh::from_slice(&attested.sync_messages[1].message)?;
    let check = input(instance_at, Some(attester), &instance_state, &check_msg);
    let _ = measure(&binary, "CheckRoot (the private path's public crossing)", "public", &check)?;
    Ok(())
}
