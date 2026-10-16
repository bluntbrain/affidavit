//! `cargo run --release` (after `../build-guest.sh`): executes every verification path of
//! affidavit-core in the RISC Zero executor on real mainnet material and prints cycle
//! counts, the number Performance #1, #2 and #8 of RFP-021 ask for.
//!
//! `-- --prove` also proves the consumer paths on this machine and reports time and size.

use std::path::PathBuf;
use std::time::Instant;

use affidavit_core::bls::Blst;
use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::execution::TransactionProof;
use affidavit_core::lc::{self, InstanceConfig};
use affidavit_core::mmr::Mmr;
use affidavit_core::ssz;
use affidavit_core::statement::{Clause, Evidence, Field, Op, Predicate};
use affidavit_core::types::HeaderRecord;
use affidavit_io::follower::Follower;
use affidavit_io::{beacon, rpc};
use affidavit_zkvm_jobs::{Job, Report};
use alloy_consensus::{Transaction as _, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{b256, Bytes, B256, U256};
use anyhow::{Context, Result};
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv, ProverOpts};

const CHECKPOINT: B256 = b256!("5afc212a7924789b2bc86acad3ab3a6ffb1f6e97253ea50bee7f4f51422c9275");
const NOW_SLOT: u64 = 7_109_500;

fn testdata(path: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// `target` is the guest build directory: `target` (zkcrypto) or `target-blst`.
fn program(target: &str) -> Result<Vec<u8>> {
    let elf = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../guest")
            .join(target)
            .join("riscv32im-risc0-zkvm-elf/release/affidavit-guest"),
    )
    .with_context(|| format!("guest ELF missing in {target}: run zkvm/build-guest.sh first"))?;
    Ok(risc0_binfmt::ProgramBinary::new(&elf, risc0_zkos_v1compat::V1COMPAT_ELF).encode())
}

struct Measured {
    label: String,
    report: Report,
    user_cycles: u64,
    padded_cycles: u64,
    segments: usize,
    prove: Option<(f64, usize)>,
}

fn execute(binary: &[u8], label: &str, job: &Job, prove: bool) -> Result<Measured> {
    let input = borsh::to_vec(job)?;
    let env = ExecutorEnv::builder().stdin(&input[..]).build()?;
    let session = default_executor().execute(env, binary)?;
    let report: Report = borsh::from_slice(&session.journal.bytes)?;
    let user_cycles = session.segments.iter().map(|s| s.cycles as u64).sum();
    let padded_cycles = session.segments.iter().map(|s| 1u64 << s.po2).sum();
    let prove = if prove {
        let env = ExecutorEnv::builder().stdin(&input[..]).build()?;
        let started = Instant::now();
        let info = default_prover().prove_with_opts(env, binary, &ProverOpts::succinct())?;
        let seconds = started.elapsed().as_secs_f64();
        let size = borsh::to_vec(&info.receipt.inner).map(|b| b.len()).unwrap_or(0);
        Some((seconds, size))
    } else {
        None
    };
    Ok(Measured { label: label.into(), report, user_cycles, padded_cycles, segments: session.segments.len(), prove })
}

fn main() -> Result<()> {
    let prove = std::env::args().any(|a| a == "--prove");
    let zkcrypto = program("target")?;
    // Built with `CARGO_TARGET_DIR=target-blst zkvm/build-guest.sh --features blst`.
    let blst = program("target-blst").ok();
    let config = InstanceConfig {
        chain: ChainSpec::mainnet(),
        checkpoint_root: CHECKPOINT,
        ws_period_epochs: weak_subjectivity_period_epochs(24_000_000 * 1_000_000_000),
    };
    let bootstrap = beacon::parse_bootstrap(&testdata("mainnet/light-client/bootstrap.json"))?;
    let updates = beacon::parse_updates(&testdata("mainnet/light-client/updates.json"))?;
    let finality = beacon::parse_finality_update(&testdata("mainnet/light-client/finality.json"))?;

    let mut follower = Follower::new(config.clone(), &bootstrap, NOW_SLOT)?;
    follower.process(&updates[0], NOW_SLOT)?;
    // updates[1] rotates the committee: a handoff, signed by the next committee.
    let handoff = updates[1].clone();
    let handoff_committee = follower.signing_committee(&handoff);
    let before_handoff = follower.instance.clone();
    for u in &updates[1..] {
        follower.process(u, NOW_SLOT)?;
    }
    let finality_committee = follower.signing_committee(&finality);
    let before_finality = follower.instance.clone();
    let advance = lc::verify_update(&before_finality, &finality, &finality_committee, NOW_SLOT, &Blst)?;
    follower.process(&finality, NOW_SLOT)?;

    let mut jobs: Vec<(String, Job, bool)> = Vec::new();
    for check_subgroups in [true, false] {
        let suffix = if check_subgroups { "subgroup-checked keys" } else { "keys bound by state root" };
        jobs.push((
            format!("update with committee handoff ({suffix})"),
            Job::VerifyUpdate {
                instance: before_handoff.clone(),
                update: handoff.clone(),
                committee: handoff_committee.clone(),
                now_slot: NOW_SLOT,
                check_subgroups,
            },
            false,
        ));
    }
    jobs.push((
        "finality update, no handoff (keys bound by state root)".into(),
        Job::VerifyUpdate {
            instance: before_finality.clone(),
            update: finality.clone(),
            committee: finality_committee,
            now_slot: NOW_SLOT,
            check_subgroups: false,
        },
        false,
    ));
    jobs.push((
        "apply verified advance (public instance)".into(),
        Job::ApplyAdvance { instance: before_finality, advance },
        false,
    ));

    // Consumers prove membership in a header accumulator sized for years of operation.
    let head = follower.instance.state.finalized;
    let (_, txs) = beacon::parse_block_transactions(&testdata("mainnet/light-client/beacon-block-7109344.json"))?;
    let (index, tx) = txs
        .iter()
        .enumerate()
        .map(|(i, raw)| (i, TxEnvelope::decode_2718(&mut raw.as_slice()).expect("tx")))
        .find(|(_, tx)| !tx.value().is_zero() && tx.input().is_empty() && tx.to().is_some())
        .context("plain transfer")?;
    let block = rpc::parse_block(&testdata("mainnet/block-26050794/block.json"))?;
    let receipts = rpc::parse_receipts(&testdata("mainnet/block-26050794/receipts.json"))?;
    let state_block = rpc::parse_block(&testdata("mainnet/block-21925176/block.json"))?;
    let (account, slots) = rpc::parse_get_proof(&testdata("mainnet/block-21925176/proof.json"))?;
    let log_header = HeaderRecord {
        block_number: block.number,
        block_hash: block.hash,
        timestamp: block.timestamp,
        state_root: block.state_root,
        receipts_root: block.receipts_root,
        ..HeaderRecord::default()
    };
    let state_header = HeaderRecord {
        block_number: state_block.number,
        block_hash: state_block.hash,
        timestamp: state_block.timestamp,
        state_root: state_block.state_root,
        receipts_root: state_block.receipts_root,
        ..HeaderRecord::default()
    };
    let leaves: u64 = 1 << 16;
    let mut mmr = Mmr::default();
    for i in 0..leaves - 3 {
        mmr.append(B256::from(U256::from(i)));
    }
    let base = mmr.leaves().len() as u64;
    mmr.append(head.leaf());
    mmr.append(log_header.leaf());
    mmr.append(state_header.leaf());
    let root = mmr.accumulator().root();
    let usdc = U256::from_be_slice(&alloy_primitives::address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")[..]);
    let consumers = [
        (
            "consumer: native ETH transfer (SSZ tx proof + ecrecover)",
            head,
            base,
            Evidence::Transfer {
                tx: TransactionProof {
                    tx_index: index as u64,
                    tx: Bytes::from(txs[index].clone()),
                    branch: ssz::transaction_branch(&txs, index),
                },
                receipt: None,
            },
            Predicate(vec![
                Clause { field: Field::TxTo, op: Op::Eq, value: U256::from_be_slice(tx.to().expect("to").as_slice()) },
                Clause { field: Field::TxValue, op: Op::Ge, value: tx.value() },
            ]),
        ),
        (
            "consumer: ERC-20 Transfer log (receipt MPT proof)",
            log_header,
            base + 1,
            Evidence::Log { receipt: rpc::receipt_proof(&receipts, &block.receipts_root, 0)?, log_index: 0 },
            Predicate(vec![
                Clause { field: Field::LogAddress, op: Op::Eq, value: usdc },
                Clause { field: Field::LogDataWord(0), op: Op::Ge, value: U256::from(3_000_000u64) },
            ]),
        ),
        (
            "consumer: storage slot (account + storage MPT proofs)",
            state_header,
            base + 2,
            Evidence::Storage { account: account.clone(), storage: slots[0].clone() },
            Predicate(vec![Clause { field: Field::StorageValue, op: Op::Eq, value: slots[0].value }]),
        ),
    ];
    for (label, header, leaf, evidence, predicate) in consumers {
        jobs.push((
            label.to_string(),
            Job::Attest {
                chain: ChainSpec::mainnet(),
                instance_id: config.id(),
                headers_root: root,
                header,
                membership: mmr.prove(leaf).expect("member"),
                evidence,
                predicate,
            },
            prove,
        ));
    }

    println!("RISC Zero executor, risc0-zkvm 3.0.5, accelerated forks (bigint2, sha2), keccak in software");
    println!("mainnet fixtures; header accumulator of {leaves} leaves for consumer paths\n");
    for (label, job, prove_it) in &jobs {
        let mut binaries = vec![("zkcrypto", &zkcrypto)];
        if let (Job::VerifyUpdate { .. }, Some(blst)) = (job, blst.as_ref()) {
            binaries.push(("blst", blst));
        }
        for (backend, binary) in binaries {
            let m = execute(binary, label, job, *prove_it)?;
            assert_eq!(m.report.verdict, 0, "{label} refused with AFF-{}", m.report.verdict);
            let tag = if matches!(job, Job::VerifyUpdate { .. }) { format!(" [{backend} guest]") } else { String::new() };
            println!(
                "{}{tag}\n  user cycles {:>12}   padded {:>12}   segments {}",
                m.label, m.user_cycles, m.padded_cycles, m.segments
            );
            for (phase, cycles) in &m.report.phases {
                println!("    {phase:<46} {cycles:>12}");
            }
            if let Some((seconds, size)) = m.prove {
                println!("  proved (succinct) in {seconds:.1} s on this machine, receipt {size} bytes");
            }
            println!();
        }
    }
    Ok(())
}
