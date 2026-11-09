//! Live run: the real `affidavit_program` on a **running LEZ sequencer**, driven through
//! LEZ's own wallet library (deployment via `program_loader`, public transactions with fee
//! declarations, privacy-preserving transactions proven client-side and submitted over RPC).
//!
//! Steps, mirroring the state-machine test in `m0/`:
//!   0. deploy the program (segments + header through `program_loader`)
//!   1. `Initialize` (public): instance PDA + bootstrap header record
//!   2. every mainnet update as a privately-proven `Verify` whose `Apply` (+ `Record`) lands on
//!      the public instance, until the on-chain instance equals the native follower
//!   3. `AttestRecord` (public consumer path) against the head record, plus one refusal
//!   4. `AttestPrivate` -> `CheckRoot` (private consumer path), disclosing only the root
//!
//! Usage: affidavit_live <affidavit_program ELF> <payer Public account id> <verifier Private account id>
//! Env:   LEE_WALLET_HOME_DIR (wallet home, network set to the sequencer), AFFIDAVIT_LIVE_OUT
//!        (where result.json and the deployed .bin are written), LIVE_UPDATES (how many of the
//!        seven updates to run; default all), RISC0_DEV_MODE=1 for dev-mode receipts (the
//!        sequencer must run in the same mode).

use std::borrow::Borrow;
use std::time::Instant;

use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::execution::TransactionProof;
use affidavit_core::lc::InstanceConfig;
use affidavit_core::ssz;
use affidavit_core::statement::{attest, Clause, Evidence, Fact, Field, Op, Predicate};
use affidavit_core::types::LightClientUpdate;
use affidavit_io::beacon;
use affidavit_io::follower::Follower;
use affidavit_lez::{header_actor, header_seed, instance_actor, instance_seed, HeaderAccount, InstanceAccount, Message};
use alloy_primitives::{b256, Bytes, B256, U256};
use anyhow::{anyhow, ensure, Context};
use lee::privacy_preserving_transaction::circuit::ProgramCatalog;
use lee::program::Program;
use lee::AccountId;
use lee_core::account::Actor;
use lee_core::program::PdaSeed;
use program_deployment::deploy_program;
use serde_json::json;
use wallet::{AccountIdentity, AccountMention, WalletCore, WalletSequencerClient};

const CHECKPOINT: B256 = b256!("5afc212a7924789b2bc86acad3ab3a6ffb1f6e97253ea50bee7f4f51422c9275");
const NOW_SLOT: u64 = 7_109_500;

fn testdata(p: &str) -> String {
    std::fs::read_to_string(format!("/home/user/affidavit/testdata/{p}")).expect("fixture")
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

fn hex32(h: common::HashType) -> String {
    hex::encode(h.0)
}

/// A program-derived public account, admitted by its seed (no signature).
fn pda(program: AccountId, seed: PdaSeed) -> AccountMention {
    AccountIdentity::PublicPda { program, seed }.select_program_actor_state(program)
}

/// The settled bytes of a public actor's state, from the sequencer.
async fn state_bytes<C: WalletSequencerClient>(wallet: &WalletCore<C>, actor: Actor) -> anyhow::Result<Vec<u8>> {
    let Some(account) = wallet.get_account_public(actor.account_id).await? else {
        return Ok(Vec::new());
    };
    let bytes: &[u8] = account.data.actor_state(actor.program_account_id).borrow();
    Ok(bytes.to_vec())
}

async fn instance_state<C: WalletSequencerClient>(wallet: &WalletCore<C>, instance: Actor) -> anyhow::Result<InstanceAccount> {
    let bytes = state_bytes(wallet, instance).await?;
    ensure!(!bytes.is_empty(), "instance PDA {} holds no state", instance.account_id);
    Ok(borsh::from_slice(&bytes)?)
}

async fn header_state<C: WalletSequencerClient>(wallet: &WalletCore<C>, record: Actor) -> anyhow::Result<HeaderAccount> {
    let bytes = state_bytes(wallet, record).await?;
    ensure!(!bytes.is_empty(), "header record PDA {} holds no state", record.account_id);
    Ok(borsh::from_slice(&bytes)?)
}

struct Landed {
    hash: String,
    block: u64,
    secs: f32,
}

/// A public transaction rooted at `accounts[0]`, paid by `payer`, waited for inclusion.
async fn pub_call<C: WalletSequencerClient>(
    wallet: &WalletCore<C>,
    accounts: Vec<AccountMention>,
    message: &Message,
    payer: AccountId,
) -> anyhow::Result<Landed> {
    let t = Instant::now();
    let hash = wallet
        .send_pub_tx_paid_by(accounts, 0, borsh::to_vec(message)?, Some(payer))
        .await
        .map_err(|e| anyhow!("public send: {e}"))?;
    let (_, block) = wallet.poll_transaction(hash).await.context("waiting for the public transaction")?;
    Ok(Landed { hash: hex32(hash), block, secs: t.elapsed().as_secs_f32() })
}

/// A privacy-preserving transaction rooted at `accounts[0]` (a private account of the program):
/// executed and proven here, submitted, waited for inclusion, then the wallet's private state
/// is synced so the same account can be used again.
async fn priv_call<C: WalletSequencerClient>(
    wallet: &mut WalletCore<C>,
    accounts: Vec<AccountMention>,
    message: &Message,
    programs: &ProgramCatalog,
) -> anyhow::Result<Landed> {
    let t = Instant::now();
    let (hash, _) = wallet
        .send_privacy_preserving_tx_with_pre_check(accounts, 0, borsh::to_vec(message)?, programs, None, |_| Ok(()))
        .await
        .map_err(|e| anyhow!("private send: {e}"))?;
    let (_, block) = wallet.poll_transaction(hash).await.context("waiting for the private transaction")?;
    wallet.sync_to_latest_block().await.context("syncing private state")?;
    Ok(Landed { hash: hex32(hash), block, secs: t.elapsed().as_secs_f32() })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let started = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    ensure!(args.len() >= 4, "usage: affidavit_live <affidavit_program ELF> <payer Public account id> <verifier Private account id>");
    let elf = std::fs::read(&args[1]).context("program ELF")?;
    let payer: AccountId = args[2].parse().map_err(|e| anyhow!("payer account id: {e:?}"))?;
    let verifier: AccountId = args[3].parse().map_err(|e| anyhow!("verifier account id: {e:?}"))?;
    let out = std::env::var("AFFIDAVIT_LIVE_OUT").unwrap_or_else(|_| ".".into());
    let n_updates: usize = std::env::var("LIVE_UPDATES").ok().and_then(|s| s.parse().ok()).unwrap_or(7);
    let dev = std::env::var("RISC0_DEV_MODE").map(|v| v == "1").unwrap_or(false);
    let mut log = Vec::new();

    let mut wallet = WalletCore::from_env()?;
    let tip = wallet.get_last_block_id().await?;
    println!(
        "# Affidavit live run: sequencer {} (tip block {tip}), lee_core v0.4.0-rc1, RISC0_DEV_MODE={dev}\n",
        wallet.client_url()
    );
    wallet.sync_to_latest_block().await.context("initial private sync")?;

    // === 0. Deploy the program through program_loader ===
    let wrapped = risc0_binfmt::ProgramBinary::new(&elf, risc0_zkos_v1compat::V1COMPAT_ELF).encode();
    std::fs::write(format!("{out}/affidavit_program.bin"), &wrapped)?;
    let program = Program::new(wrapped.clone().into()).map_err(|e| anyhow!("program binary: {e}"))?;
    let t = Instant::now();
    let program_id = deploy_program(&mut wallet, wrapped.clone(), payer).await.context("deploying the program")?;
    let deploy_secs = t.elapsed().as_secs_f32();
    println!(
        "[0] deployed affidavit_program ({} bytes ELF, {} bytes wrapped) as program account {program_id} ({deploy_secs:.1} s)\n",
        elf.len(),
        wrapped.len()
    );
    log.push(json!({"step": "deploy", "program_account": program_id.to_string(), "elf_bytes": elf.len(), "wrapped_bytes": wrapped.len(), "secs": deploy_secs}));
    let programs = ProgramCatalog::from([(program_id, program)]);

    // === 1. Initialize (public) ===
    let cfg = config();
    let bootstrap = beacon::parse_bootstrap(&testdata("mainnet/light-client/bootstrap.json"))?;
    let boot_slot = bootstrap.header.beacon.slot;
    let instance = instance_actor(program_id, &cfg.id());
    let init = Message::Initialize { config: cfg.clone(), bootstrap: bootstrap.clone(), now_slot: NOW_SLOT };
    let landed = pub_call(
        &wallet,
        vec![pda(program_id, instance_seed(&cfg.id())), pda(program_id, header_seed(&cfg.id(), boot_slot))],
        &init,
        payer,
    )
    .await
    .context("Initialize")?;
    let decoded = instance_state(&wallet, instance).await?;
    ensure!(decoded.instance.config.id() == cfg.id(), "instance id mismatch");
    println!(
        "[1] Initialize (public): tx {} in block {}: instance PDA {}, finalized slot {}, period {} ({:.1} s)\n    PASS\n",
        landed.hash,
        landed.block,
        instance.account_id,
        decoded.instance.state.finalized.slot,
        cfg.chain.period_at_slot(decoded.instance.state.finalized.slot),
        landed.secs
    );
    log.push(json!({"step": "initialize", "tx": landed.hash, "block": landed.block, "instance_pda": instance.account_id.to_string(), "finalized_slot": decoded.instance.state.finalized.slot, "secs": landed.secs}));

    // === 2. The mainnet updates, each a privately-proven Verify -> public Apply (+ Record) ===
    let updates = all_updates()?;
    let n_updates = n_updates.min(updates.len());
    let mut follower = Follower::new(cfg.clone(), &bootstrap, NOW_SLOT)?;
    let mut records = 1usize;
    println!("[2] {n_updates} update(s) as private Verify -> public Apply (+ Record), proven client-side, settled by the sequencer");
    for (i, update) in updates.iter().enumerate().take(n_updates) {
        let snapshot = instance_state(&wallet, instance).await?;
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
        let mut accounts = vec![
            AccountIdentity::PrivateOwned(verifier).select_program_actor_state(program_id),
            pda(program_id, instance_seed(&cfg.id())),
        ];
        if let Some(record) = expected.new_finalized {
            accounts.push(pda(program_id, header_seed(&cfg.id(), record.slot)));
        }
        let landed = priv_call(&mut wallet, accounts, &verify, &programs).await.with_context(|| format!("update {i}"))?;
        let after = instance_state(&wallet, instance).await?;
        let kind = if update.next_sync_committee.is_some() { "committee handoff" } else { "finality update" };
        match expected.new_finalized {
            Some(record) => {
                let at = header_actor(program_id, &cfg.id(), record.slot);
                let written = header_state(&wallet, at).await?;
                ensure!(written.record == record, "update {i}: header record differs from the follower's");
                records += 1;
                println!(
                    "    update {i} ({kind}): tx {} in block {}: Apply + Record landed, finalized slot {} (block {}), record PDA {}, updates_applied {} ({:.1} s)",
                    landed.hash, landed.block, record.slot, record.block_number, at.account_id, after.instance.state.updates_applied, landed.secs
                );
                log.push(json!({"step": format!("verify_private_{i}"), "kind": kind, "tx": landed.hash, "block": landed.block, "finalized_slot": record.slot, "eth_block": record.block_number, "record_pda": at.account_id.to_string(), "updates_applied": after.instance.state.updates_applied, "secs": landed.secs}));
            }
            None => {
                ensure!(after.instance.state.updates_applied == snapshot.instance.state.updates_applied + 1, "update {i}: instance did not advance");
                println!(
                    "    update {i} ({kind}): tx {} in block {}: Apply landed, committees only, updates_applied {} ({:.1} s)",
                    landed.hash, landed.block, after.instance.state.updates_applied, landed.secs
                );
                log.push(json!({"step": format!("verify_private_{i}"), "kind": kind, "tx": landed.hash, "block": landed.block, "updates_applied": after.instance.state.updates_applied, "secs": landed.secs}));
            }
        }
    }
    let synced = instance_state(&wallet, instance).await?;
    let complete = n_updates == updates.len();
    if complete {
        ensure!(synced.instance == follower.instance, "on-chain instance differs from the native follower");
        ensure!(records == follower.records.len(), "record count differs from the follower's");
    }
    let head = synced.instance.state.finalized;
    println!(
        "    on-chain instance{} the native follower: finalized slot {} (block {}), {} updates applied, {} header records, accumulator root {}\n    PASS\n",
        if complete { " ==" } else { " tracks" },
        head.slot,
        head.block_number,
        synced.instance.state.updates_applied,
        records,
        synced.instance.state.headers.root()
    );

    if !complete {
        println!("(LIVE_UPDATES={n_updates}: attestations need the full chain; skipped)");
    } else {
        // === 3. AttestRecord (public consumer path) ===
        let record_at = header_actor(program_id, &cfg.id(), head.slot);
        let (_, txs) = beacon::parse_block_transactions(&testdata("mainnet/light-client/beacon-block-7109344.json"))?;
        let evidence = Evidence::Transfer {
            tx: TransactionProof { tx_index: 0, tx: Bytes::from(txs[0].clone()), branch: ssz::transaction_branch(&txs, 0) },
            receipt: None,
        };
        let predicate = Predicate(vec![Clause { field: Field::TxValue, op: Op::Gt, value: U256::ZERO }]);
        let statement = attest(&cfg.chain, cfg.id(), &head, &evidence, &predicate)?;
        let landed = pub_call(
            &wallet,
            vec![pda(program_id, header_seed(&cfg.id(), head.slot))],
            &Message::AttestRecord { chain: cfg.chain.clone(), evidence: evidence.clone(), predicate: predicate.clone() },
            payer,
        )
        .await
        .context("AttestRecord")?;
        println!(
            "[3] AttestRecord (public): tx {} in block {}: accepted at header record slot {} (block {}), {} transactions in the block ({:.1} s)",
            landed.hash, landed.block, head.slot, head.block_number, txs.len(), landed.secs
        );
        if let Fact::Transaction { tx_hash, from, to, value, .. } = &statement.fact {
            println!(
                "    statement id {}: tx {tx_hash} from {from} to {} value {value} wei",
                statement.id,
                to.map_or("-".to_string(), |a| a.to_string())
            );
        }
        log.push(json!({"step": "attest_record_public", "tx": landed.hash, "block": landed.block, "record_slot": head.slot, "statement_id": statement.id.to_string(), "secs": landed.secs}));
        // A predicate the evidence cannot satisfy: the sequencer must not settle it.
        let bad = Message::AttestRecord {
            chain: cfg.chain.clone(),
            evidence: evidence.clone(),
            predicate: Predicate(vec![Clause { field: Field::TxValue, op: Op::Gt, value: U256::MAX }]),
        };
        match wallet
            .send_pub_tx_paid_by(vec![pda(program_id, header_seed(&cfg.id(), head.slot))], 0, borsh::to_vec(&bad)?, Some(payer))
            .await
        {
            Err(e) => {
                println!("    refusal (value > 2^256-1): rejected before inclusion: {e}");
                log.push(json!({"step": "attest_record_refusal", "outcome": "rejected", "error": e.to_string()}));
            }
            Ok(h) => {
                let polled = wallet.poll_transaction(h).await.map(|(_, b)| b);
                println!("    refusal (value > 2^256-1): submitted as {} -> {:?} (see the sequencer log for the AFF-301 failure)", hex32(h), polled);
                log.push(json!({"step": "attest_record_refusal", "outcome": "submitted", "tx": hex32(h)}));
            }
        }
        ensure!(header_state(&wallet, record_at).await?.record == head, "the record changed");
        println!("    PASS\n");

        // === 4. AttestPrivate -> CheckRoot (private consumer path) ===
        let index = follower.records.len() as u64 - 1;
        let membership = follower.headers.prove(index).expect("the head is in the follower's accumulator");
        let root = synced.instance.state.headers.root();
        let before = state_bytes(&wallet, instance).await?;
        let landed = priv_call(
            &mut wallet,
            vec![
                AccountIdentity::PrivateOwned(verifier).select_program_actor_state(program_id),
                pda(program_id, instance_seed(&cfg.id())),
            ],
            &Message::AttestPrivate {
                chain: cfg.chain.clone(),
                instance_id: cfg.id(),
                headers_root: root,
                header: head,
                membership,
                evidence,
                predicate,
            },
            &programs,
        )
        .await
        .context("AttestPrivate")?;
        ensure!(state_bytes(&wallet, instance).await? == before, "CheckRoot changed the instance");
        println!(
            "[4] AttestPrivate -> CheckRoot (PRIVATE): tx {} in block {}: accepted; the public crossing disclosed only accumulator root {root}; instance state unchanged ({:.1} s)\n    PASS\n",
            landed.hash, landed.block, landed.secs
        );
        log.push(json!({"step": "attest_private", "tx": landed.hash, "block": landed.block, "root": root.to_string(), "secs": landed.secs}));
    }

    let total = started.elapsed().as_secs_f32();
    println!("total wall-clock {total:.0} s");
    let result = json!({
        "sequencer": wallet.client_url().to_string(),
        "lee_core": "v0.4.0-rc1",
        "risc0_dev_mode": dev,
        "program_account": program_id.to_string(),
        "instance_pda": instance.account_id.to_string(),
        "updates_run": n_updates,
        "total_secs": total,
        "steps": log,
    });
    std::fs::write(format!("{out}/result.json"), serde_json::to_string_pretty(&result)?)?;
    Ok(())
}
