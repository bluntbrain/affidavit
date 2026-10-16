//! `affidavit`: follow Ethereum's sync committee from a checkpoint, produce attestations,
//! and dry-run them with the same typed errors the on-chain path returns.

mod demo;
mod predicate;
mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::execution::TransactionProof;
use affidavit_core::lc::InstanceConfig;
use affidavit_core::ssz;
use affidavit_core::statement::{attest, attest_with_membership, Evidence};
use affidavit_core::types::HeaderRecord;
use affidavit_io::follower::Follower;
use affidavit_io::{beacon, rpc};
use alloy_primitives::{Bytes, B256};
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "affidavit", version, about = "Trustless Ethereum state attestation for LEZ (prototype)")]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the whole pipeline, and the attacks it refuses, on the bundled mainnet data.
    Demo {
        #[arg(long, default_value = "testdata")]
        testdata: PathBuf,
    },
    /// Initialise an instance from a checkpoint and apply light-client updates.
    Sync(SyncArgs),
    /// Prove a fact under a finalised header and evaluate a predicate (off-chain dry run).
    Attest {
        #[command(subcommand)]
        what: AttestCommand,
    },
    /// Check a checkpoint: does the bootstrap hash to it, and is it inside the weak-subjectivity period?
    Checkpoint {
        #[arg(long, default_value = "mainnet")]
        network: String,
        #[arg(long)]
        root: B256,
        #[arg(long)]
        bootstrap: PathBuf,
        /// Current slot, to judge staleness.
        #[arg(long)]
        now_slot: u64,
        /// Total active stake in ETH, for the weak-subjectivity period.
        #[arg(long, default_value_t = 34_000_000)]
        total_stake_eth: u64,
    },
}

#[derive(Args, Clone)]
struct SyncArgs {
    #[arg(long, default_value = "mainnet")]
    network: String,
    /// Weak-subjectivity checkpoint (beacon block root).
    #[arg(long)]
    checkpoint: B256,
    /// Beacon API `light_client/bootstrap` response.
    #[arg(long)]
    bootstrap: PathBuf,
    /// Beacon API `light_client/updates` responses, in order.
    #[arg(long, num_args = 0..)]
    updates: Vec<PathBuf>,
    /// Beacon API `light_client/finality_update` response.
    #[arg(long)]
    finality: Option<PathBuf>,
    /// Current slot (on chain this comes from the LEZ clock).
    #[arg(long)]
    now_slot: u64,
    /// Total active stake in ETH, for the weak-subjectivity period frozen into the instance.
    #[arg(long, default_value_t = 34_000_000)]
    total_stake_eth: u64,
}

#[derive(Subcommand)]
enum AttestCommand {
    /// A native ETH transfer, from the raw transactions of a beacon block.
    Transfer {
        #[command(flatten)]
        sync: SyncArgs,
        /// Beacon API `blocks/{id}` response for the finalised slot.
        #[arg(long)]
        beacon_block: PathBuf,
        #[arg(long)]
        tx: usize,
        #[arg(long, default_value = "")]
        r#where: String,
    },
    /// A log, from a block's receipts. Header roots come from the given block JSON: a dry
    /// run of the inclusion check only, for blocks no light-client material reaches.
    Log {
        #[arg(long, default_value = "mainnet")]
        network: String,
        /// `eth_getBlockByHash` response (UNVERIFIED header source).
        #[arg(long)]
        block: PathBuf,
        /// `eth_getBlockReceipts` response.
        #[arg(long)]
        receipts: PathBuf,
        #[arg(long)]
        tx: u64,
        #[arg(long)]
        log: u64,
        #[arg(long, default_value = "")]
        r#where: String,
    },
    /// An account and optionally a storage slot, from `eth_getProof`. Same header caveat as `log`.
    State {
        #[arg(long, default_value = "mainnet")]
        network: String,
        #[arg(long)]
        block: PathBuf,
        /// `eth_getProof` response.
        #[arg(long)]
        proof: PathBuf,
        /// Index into the proof's storage proofs; omit to attest the account itself.
        #[arg(long)]
        slot_index: Option<usize>,
        #[arg(long, default_value = "")]
        r#where: String,
    },
}

fn network(name: &str) -> Result<ChainSpec> {
    ChainSpec::by_name(name).ok_or_else(|| anyhow!("unknown network {name} (mainnet, sepolia, hoodi)"))
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn sync(args: &SyncArgs, json: bool) -> Result<Follower> {
    let config = InstanceConfig {
        chain: network(&args.network)?,
        checkpoint_root: args.checkpoint,
        ws_period_epochs: weak_subjectivity_period_epochs(args.total_stake_eth * 1_000_000_000),
    };
    let bootstrap = beacon::parse_bootstrap(&read(&args.bootstrap)?)?;
    let mut follower = Follower::new(config, &bootstrap, args.now_slot)?;
    let mut updates = Vec::new();
    for path in &args.updates {
        updates.extend(beacon::parse_updates(&read(path)?)?);
    }
    if let Some(path) = &args.finality {
        updates.push(beacon::parse_finality_update(&read(path)?)?);
    }
    for update in &updates {
        let advance = follower.process(update, args.now_slot)?;
        if !json {
            match advance.new_finalized {
                Some(r) => eprintln!(
                    "applied: finalised slot {} block {} ({} signers)",
                    r.slot, r.block_number, advance.participants
                ),
                None => eprintln!("applied: learned next committee ({} signers)", advance.participants),
            }
        }
    }
    Ok(follower)
}

fn rpc_header(path: &Path) -> Result<HeaderRecord> {
    let block = rpc::parse_block(&read(path)?)?;
    eprintln!("warning: header roots taken from JSON-RPC; this dry run checks inclusion, not finality");
    Ok(HeaderRecord {
        block_number: block.number,
        block_hash: block.hash,
        timestamp: block.timestamp,
        state_root: block.state_root,
        receipts_root: block.receipts_root,
        ..HeaderRecord::default()
    })
}

fn print_statement(s: &affidavit_core::statement::Statement, json: bool) {
    let rendered = render::statement_json(s);
    if json {
        println!("{rendered}");
    } else {
        println!("{}", serde_json::to_string_pretty(&rendered).expect("json"));
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Demo { testdata } => {
            let dir = if testdata.exists() {
                testdata
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
            };
            demo::run(&dir)
        }
        Command::Sync(args) => {
            let follower = sync(&args, cli.json)?;
            let s = &follower.instance.state;
            let out = serde_json::json!({
                "instance_id": follower.instance.config.id(),
                "finalized": { "slot": s.finalized.slot, "block_number": s.finalized.block_number,
                    "beacon_root": s.finalized.beacon_root, "state_root": s.finalized.state_root },
                "current_committee_root": s.current_committee_root,
                "next_committee_root": s.next_committee_root,
                "headers": { "count": s.headers.leaf_count, "root": s.headers.root() },
                "updates_applied": s.updates_applied,
            });
            println!("{}", if cli.json { out.to_string() } else { serde_json::to_string_pretty(&out)? });
            Ok(())
        }
        Command::Attest { what } => {
            let statement = match what {
                AttestCommand::Transfer { sync: args, beacon_block, tx, r#where } => {
                    let follower = sync(&args, cli.json)?;
                    let (_, txs) = beacon::parse_block_transactions(&read(&beacon_block)?)?;
                    let tip = follower.instance.state.finalized;
                    if ssz::transactions_root(&txs) != tip.transactions_root {
                        bail!("the beacon block is not the finalised head the instance verified");
                    }
                    let raw = txs.get(tx).ok_or_else(|| anyhow!("block has {} transactions", txs.len()))?;
                    let evidence = Evidence::Transfer {
                        tx: TransactionProof {
                            tx_index: tx as u64,
                            tx: Bytes::from(raw.clone()),
                            branch: ssz::transaction_branch(&txs, tx),
                        },
                        receipt: None,
                    };
                    let (record, membership) = follower.prove_block(tip.block_number).expect("head is recorded");
                    attest_with_membership(
                        &follower.instance.config.chain,
                        follower.instance.config.id(),
                        &follower.instance.state.headers.root(),
                        &record,
                        &membership,
                        &evidence,
                        &predicate::parse(&r#where)?,
                    )?
                }
                AttestCommand::Log { network: net, block, receipts, tx, log, r#where } => {
                    let header = rpc_header(&block)?;
                    let receipts = rpc::parse_receipts(&read(&receipts)?)?;
                    let receipt = rpc::receipt_proof(&receipts, &header.receipts_root, tx)?;
                    attest(
                        &network(&net)?,
                        B256::ZERO,
                        &header,
                        &Evidence::Log { receipt, log_index: log },
                        &predicate::parse(&r#where)?,
                    )?
                }
                AttestCommand::State { network: net, block, proof, slot_index, r#where } => {
                    let header = rpc_header(&block)?;
                    let (account, slots) = rpc::parse_get_proof(&read(&proof)?)?;
                    let evidence = match slot_index {
                        Some(i) => Evidence::Storage {
                            account,
                            storage: slots.get(i).cloned().ok_or_else(|| anyhow!("no storage proof {i}"))?,
                        },
                        None => Evidence::Account(account),
                    };
                    attest(&network(&net)?, B256::ZERO, &header, &evidence, &predicate::parse(&r#where)?)?
                }
            };
            print_statement(&statement, cli.json);
            Ok(())
        }
        Command::Checkpoint { network: net, root, bootstrap, now_slot, total_stake_eth } => {
            let spec = network(&net)?;
            let bootstrap = beacon::parse_bootstrap(&read(&bootstrap)?)?;
            let ws = weak_subjectivity_period_epochs(total_stake_eth * 1_000_000_000);
            let config = InstanceConfig { chain: spec.clone(), checkpoint_root: root, ws_period_epochs: ws };
            let follower = Follower::new(config, &bootstrap, now_slot)?;
            let age = spec.epoch_at_slot(now_slot).saturating_sub(spec.epoch_at_slot(bootstrap.header.beacon.slot));
            let out = serde_json::json!({
                "checkpoint": root,
                "slot": bootstrap.header.beacon.slot,
                "block_number": bootstrap.header.execution.block_number,
                "slot_time_unix": spec.slot_time(bootstrap.header.beacon.slot),
                "age_epochs": age,
                "weak_subjectivity_epochs": ws,
                "instance_id": follower.instance.config.id(),
                "verify_independently": format!("compare {root} with the finalised block root at slot {} on two independent beacon explorers or nodes", bootstrap.header.beacon.slot),
            });
            println!("{}", if cli.json { out.to_string() } else { serde_json::to_string_pretty(&out)? });
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            match err.downcast_ref::<affidavit_core::Error>() {
                Some(e) if json => println!("{}", render::error_json(e)),
                Some(e) => eprintln!("{e}"),
                None => eprintln!("error: {err:#}"),
            }
            ExitCode::FAILURE
        }
    }
}
