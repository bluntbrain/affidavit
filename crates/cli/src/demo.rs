//! `affidavit demo`: the whole pipeline and the attacks it refuses, on real mainnet data,
//! in one command.

use std::path::Path;
use std::time::Instant;

use affidavit_core::bls::{Blst, DST};
use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::execution::{AccountProof, TransactionProof};
use affidavit_core::lc::{self, compute_domain, InstanceConfig, DOMAIN_SYNC_COMMITTEE};
use affidavit_core::ssz::{self, hash_pair};
use affidavit_core::statement::{attest, attest_with_membership, Clause, Evidence, Fact, Field, Op, Predicate};
use affidavit_core::types::{HeaderRecord, SyncCommittee};
use affidavit_io::follower::Follower;
use affidavit_io::{beacon, rpc};
use alloy_consensus::{Transaction as _, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{address, b256, keccak256, Bytes, B256, U256};
use alloy_trie::{proof::ProofRetainer, HashBuilder, Nibbles, TrieAccount, EMPTY_ROOT_HASH, KECCAK_EMPTY};
use anyhow::{Context, Result};
use blst::min_pk::{AggregateSignature, SecretKey};

use crate::predicate::describe;
use crate::render::{eth, short};

const CHECKPOINT: B256 = b256!("5afc212a7924789b2bc86acad3ab3a6ffb1f6e97253ea50bee7f4f51422c9275");
const NOW_SLOT: u64 = 7_109_500;

fn read(dir: &Path, path: &str) -> Result<String> {
    std::fs::read_to_string(dir.join(path)).with_context(|| format!("reading {path}"))
}

fn refused<T>(label: &str, result: affidavit_core::Result<T>) {
    match result {
        Err(e) => println!("  {label:<52} AFF-{} {}", e.code(), e.name()),
        Ok(_) => println!("  {label:<52} ACCEPTED (this is a bug)"),
    }
}

pub fn run(dir: &Path) -> Result<()> {
    println!("affidavit demo: trustless Ethereum state attestation, on real mainnet data\n");
    let spec = ChainSpec::mainnet();
    let config = InstanceConfig {
        chain: spec.clone(),
        checkpoint_root: CHECKPOINT,
        ws_period_epochs: weak_subjectivity_period_epochs(24_000_000 * 1_000_000_000),
    };

    println!("[1/5] Checkpoint (the one trusted input, checkable against any beacon explorer)");
    let bootstrap = beacon::parse_bootstrap(&read(dir, "mainnet/light-client/bootstrap.json")?)?;
    println!("  configured root   {CHECKPOINT}");
    println!(
        "  bootstrap header  slot {} hashes to it, sync-committee period {}",
        bootstrap.header.beacon.slot,
        spec.period_at_slot(bootstrap.header.beacon.slot)
    );
    let mut follower = Follower::new(config, &bootstrap, NOW_SLOT)?;
    println!("  instance id       {}", follower.instance.config.id());
    println!("  weak-subjectivity limit {} epochs\n", follower.instance.config.ws_period_epochs);

    println!("[2/5] Following the sync committee (BLS aggregate checked on every step)");
    let updates = beacon::parse_updates(&read(dir, "mainnet/light-client/updates.json")?)?;
    let finality = beacon::parse_finality_update(&read(dir, "mainnet/light-client/finality.json")?)?;
    let mut all = updates.clone();
    all.push(finality.clone());
    for (i, update) in all.iter().enumerate() {
        let started = Instant::now();
        let advance = follower.process(update, NOW_SLOT)?;
        let elapsed = started.elapsed().as_millis();
        let what = match advance.new_finalized {
            Some(r) => format!("finalised slot {} block {}", r.slot, r.block_number),
            None => "learned the next committee".to_string(),
        };
        let label = if i == all.len() - 1 { "finality".to_string() } else { format!("update {}", i + 1) };
        println!(
            "  {label:<9} period {}  {}/512 signed  {what}  ({elapsed} ms)",
            spec.period_at_slot(update.signature_slot),
            advance.participants
        );
    }
    let head = follower.instance.state.finalized;
    println!("  head state root   {}", head.state_root);
    println!(
        "  headers recorded  {}, accumulator root {}\n",
        follower.instance.state.headers.leaf_count,
        short(follower.instance.state.headers.root())
    );

    println!("[3/5] Native ETH transfer in block {} (the header verified above)", head.block_number);
    let (_, txs) = beacon::parse_block_transactions(&read(dir, "mainnet/light-client/beacon-block-7109344.json")?)?;
    anyhow::ensure!(ssz::transactions_root(&txs) == head.transactions_root, "block does not match header");
    let (index, tx) = txs
        .iter()
        .enumerate()
        .map(|(i, raw)| (i, TxEnvelope::decode_2718(&mut raw.as_slice()).expect("payload transaction")))
        .find(|(_, tx)| !tx.value().is_zero() && tx.input().is_empty() && tx.to().is_some())
        .context("no plain transfer")?;
    let to = tx.to().expect("checked");
    let predicate = Predicate(vec![
        Clause { field: Field::TxTo, op: Op::Eq, value: U256::from_be_slice(to.as_slice()) },
        Clause { field: Field::TxValue, op: Op::Ge, value: tx.value() },
    ]);
    let evidence = Evidence::Transfer {
        tx: TransactionProof {
            tx_index: index as u64,
            tx: Bytes::from(txs[index].clone()),
            branch: ssz::transaction_branch(&txs, index),
        },
        receipt: None,
    };
    let (record, membership) = follower.prove_block(head.block_number).context("header recorded")?;
    let root = follower.instance.state.headers.root();
    let statement = attest_with_membership(
        &spec,
        follower.instance.config.id(),
        &root,
        &record,
        &membership,
        &evidence,
        &predicate,
    )?;
    if let Fact::Transaction { from, value, tx_hash, .. } = &statement.fact {
        println!("  tx #{index} {}: {} -> {}, {} ETH", short(tx_hash), short(from), short(to), eth(*value));
    }
    println!("  predicate         {}", describe(&predicate));
    println!("  statement id      {}", statement.id);
    println!("  (this fixture has no receipts, so tx.success is unproven and a predicate on it is refused)\n");

    println!("[4/5] ERC-20 Transfer log, block 26,050,794 (Sept 2026; inclusion only, root taken from JSON-RPC)");
    let block = rpc::parse_block(&read(dir, "mainnet/block-26050794/block.json")?)?;
    let receipts = rpc::parse_receipts(&read(dir, "mainnet/block-26050794/receipts.json")?)?;
    let receipt = rpc::receipt_proof(&receipts, &block.receipts_root, 0)?;
    let usdc = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
    let transfer = b256!("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
    let log_predicate = Predicate(vec![
        Clause { field: Field::LogAddress, op: Op::Eq, value: U256::from_be_slice(usdc.as_slice()) },
        Clause { field: Field::LogTopic(0), op: Op::Eq, value: U256::from_be_bytes(transfer.0) },
        Clause { field: Field::LogDataWord(0), op: Op::Ge, value: U256::from(3_000_000u64) },
    ]);
    let rpc_record = HeaderRecord {
        block_number: block.number,
        block_hash: block.hash,
        timestamp: block.timestamp,
        state_root: block.state_root,
        receipts_root: block.receipts_root,
        ..HeaderRecord::default()
    };
    let log_statement =
        attest(&spec, B256::ZERO, &rpc_record, &Evidence::Log { receipt, log_index: 0 }, &log_predicate)?;
    if let Fact::Log { topics, data, .. } = &log_statement.fact {
        let amount = U256::from_be_slice(&data[..32]);
        println!(
            "  USDC {} -> {}, {}.{:06} USDC",
            short(B256::from(topics[1])),
            short(B256::from(topics[2])),
            amount / U256::from(1_000_000u64),
            amount % U256::from(1_000_000u64)
        );
    }
    println!("  statement id      {}\n", log_statement.id);

    println!("[5/5] Attacks, each refused with its typed error");
    let mut synced_before_finality = Follower::new(follower.instance.config.clone(), &bootstrap, NOW_SLOT)?;
    for update in &updates {
        synced_before_finality.process(update, NOW_SLOT)?;
    }
    let committee = synced_before_finality.signing_committee(&finality);
    let instance = &synced_before_finality.instance;

    // A self-consistent forged header around a fabricated state root.
    let mut forged = finality.clone();
    let fin = forged.finalized_header.as_mut().expect("finality update");
    let (fake_root, fake_account) = fabricated_state();
    fin.execution.state_root = fake_root;
    let fork = spec.fork_at_slot(fin.beacon.slot);
    fin.beacon.body_root = root_from_branch(&fin.execution.hash_tree_root(fork), &fin.execution_branch, 9);
    let forged_finalized = fin.beacon.hash_tree_root();
    forged.attested_header.beacon.state_root = root_from_branch(&forged_finalized, &forged.finality_branch, 41);
    let fake_record = HeaderRecord::from_header(forged.finalized_header.as_ref().expect("set"));
    let inclusion_alone =
        attest(&spec, B256::ZERO, &fake_record, &Evidence::Account(fake_account.clone()), &Predicate::default());
    println!(
        "  {:<52} {}",
        "fabricated state: inclusion proof alone",
        if inclusion_alone.is_ok() { "verifies (why inclusion is not enough)" } else { "fails" }
    );
    refused(
        "fabricated chain: forged header, every path repaired",
        lc::verify_update(instance, &forged, &committee, NOW_SLOT, &Blst),
    );
    refused(
        "attesting against that header",
        attest_with_membership(
            &spec,
            B256::ZERO,
            &root,
            &fake_record,
            &membership,
            &Evidence::Account(fake_account),
            &Predicate::default(),
        ),
    );

    let mut thin = finality.clone();
    clear_bits_beyond(&mut thin.sync_aggregate.sync_committee_bits, 341);
    refused("341 of 512 signatures", lc::verify_update(instance, &thin, &committee, NOW_SLOT, &Blst));

    let attacker = attacker_keys();
    let mut foreign = finality.clone();
    let version = spec.fork_version_at_epoch(spec.epoch_at_slot(foreign.signature_slot - 1));
    let message = hash_pair(
        &foreign.attested_header.beacon.hash_tree_root(),
        &compute_domain(DOMAIN_SYNC_COMMITTEE, version, &spec.genesis_validators_root),
    );
    foreign.sync_aggregate.sync_committee_signature = aggregate(&attacker, &message);
    refused(
        "512 attacker keys sign the real header",
        lc::verify_update(instance, &foreign, &committee_of(&attacker), NOW_SLOT, &Blst),
    );

    let mut tampered = finality.clone();
    tampered.finalized_header.as_mut().expect("set").execution.receipts_root = B256::repeat_byte(0xbb);
    refused(
        "finalised header with a swapped receipts root",
        lc::verify_update(instance, &tampered, &committee, NOW_SLOT, &Blst),
    );

    let mut no_finality = finality.clone();
    no_finality.finalized_header = None;
    refused("update without a finality proof", lc::verify_update(instance, &no_finality, &committee, NOW_SLOT, &Blst));

    refused(
        "the same update submitted again",
        lc::verify_update(&follower.instance, &finality, &committee, NOW_SLOT, &Blst),
    );
    refused("skipping three committee periods", {
        let fresh = Follower::new(follower.instance.config.clone(), &bootstrap, NOW_SLOT)?;
        lc::verify_update(&fresh.instance, &updates[3], &fresh.signing_committee(&updates[3]), NOW_SLOT, &Blst)
    });
    let stale_at = head.slot + (follower.instance.config.ws_period_epochs + 1) * 32;
    refused(
        "advancing after the weak-subjectivity period",
        lc::verify_update(instance, &finality, &committee, stale_at, &Blst),
    );
    refused(
        "mainnet statement to a Sepolia-bound consumer",
        statement.check_binding(&ChainSpec::sepolia(), &statement.instance_id),
    );
    refused(
        "same transfer attested under a Sepolia instance",
        attest(&ChainSpec::sepolia(), B256::ZERO, &record, &evidence, &predicate),
    );
    let attested_record = HeaderRecord::from_header(&finality.attested_header);
    refused(
        "signed but not finalised header",
        attest_with_membership(&spec, B256::ZERO, &root, &attested_record, &membership, &evidence, &predicate),
    );
    let greedy = Predicate(vec![Clause { field: Field::TxValue, op: Op::Gt, value: tx.value() }]);
    refused("predicate the state does not satisfy", attest(&spec, B256::ZERO, &record, &evidence, &greedy));
    let needs_receipt = Predicate(vec![Clause { field: Field::TxSuccess, op: Op::Eq, value: U256::from(1u8) }]);
    refused(
        "predicate over state the proof does not cover",
        attest(&spec, B256::ZERO, &record, &evidence, &needs_receipt),
    );
    println!("\nEvery accepted step above is reproducible with `cargo test --release`.");
    Ok(())
}

fn root_from_branch(leaf: &B256, branch: &[B256], index: u64) -> B256 {
    let mut value = *leaf;
    for (i, node) in branch.iter().enumerate() {
        value = if (index >> i) & 1 == 1 { hash_pair(node, &value) } else { hash_pair(&value, node) };
    }
    value
}

fn fabricated_state() -> (B256, AccountProof) {
    let victim = address!("00000000219ab540356cbb839cbe05303d7705fa");
    let account = TrieAccount {
        nonce: 1,
        balance: U256::from(1_000_000u64) * U256::from(10u64).pow(U256::from(18u64)),
        storage_root: EMPTY_ROOT_HASH,
        code_hash: KECCAK_EMPTY,
    };
    let key = Nibbles::unpack(keccak256(victim));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![key]));
    hb.add_leaf(key, &alloy_rlp::encode(account));
    let root = hb.root();
    let proof = hb.take_proof_nodes().matching_nodes_sorted(&key).into_iter().map(|(_, n)| n).collect();
    let proof = AccountProof {
        address: victim,
        nonce: account.nonce,
        balance: account.balance,
        storage_root: account.storage_root,
        code_hash: account.code_hash,
        proof,
    };
    (root, proof)
}

fn clear_bits_beyond(bits: &mut [u8], keep: usize) {
    let mut kept = 0;
    for byte in bits.iter_mut() {
        for bit in 0..8 {
            if *byte >> bit & 1 == 1 {
                if kept >= keep {
                    *byte &= !(1 << bit);
                } else {
                    kept += 1;
                }
            }
        }
    }
}

fn attacker_keys() -> Vec<SecretKey> {
    (0..512u32)
        .map(|i| SecretKey::key_gen(&[i.to_le_bytes(), [0xa7; 4]].concat().repeat(4), &[]).expect("ikm"))
        .collect()
}

fn committee_of(keys: &[SecretKey]) -> SyncCommittee {
    let pubkeys: Vec<[u8; 48]> = keys.iter().map(|k| k.sk_to_pk().compress()).collect();
    SyncCommittee { aggregate_pubkey: pubkeys[0], pubkeys }
}

fn aggregate(keys: &[SecretKey], message: &B256) -> [u8; 96] {
    let sigs: Vec<_> = keys.iter().map(|k| k.sign(message.as_slice(), DST, &[])).collect();
    let refs: Vec<_> = sigs.iter().collect();
    AggregateSignature::aggregate(&refs, true).expect("non-empty").to_signature().compress()
}
