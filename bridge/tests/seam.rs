//! The seam, end to end on real receipts: Affidavit's receipt-proof path attests the
//! `Deposit` log that the **compiled `CausewayVault` emitted on a real EVM**
//! (`testdata/vault/deposit-receipts.json`, produced by `contracts/scripts/make-fixture.mjs`),
//! and the depositor's seed-derived secret turns the statement into a wrapped-mint
//! authorisation. Someone else's deposit, a replay, a wrong vault, a wrong denomination, a
//! wrong instance and a plain ERC-20 transfer are all refused.
//!
//! The statements are produced by `affidavit_core::statement::attest` over Merkle proofs
//! against the receipts root rebuilt from the fixture, the same code path the RFP-021 suite
//! runs on mainnet data, so this exercises the actual RFP-021 → RFP-022 seam, not a mock.

use std::path::PathBuf;

use affidavit_core::config::ChainSpec;
use affidavit_core::statement::{attest, Clause, Evidence, Field, Op, Predicate, Statement};
use affidavit_core::types::HeaderRecord;
use affidavit_io::rpc;
use alloy_primitives::{address, keccak256, Address, B256, U256};
use causeway_mint_demo::recovery::{deposit_secret, Coordinates};
use causeway_mint_demo::{authorize_mint, BridgeConfig, BridgeError, NullifierSet, RegisteredToken, DEPOSIT_TOPIC};

const ETH: Address = Address::ZERO;
/// 0.1 ETH, the denomination the fixture deposits use.
const DENOMINATION: u128 = 100_000_000_000_000_000;
const INSTANCE: B256 = B256::repeat_byte(0x42);
/// The seeds the fixture's depositors derived their commitments from (never on chain).
const ALICE_SEED: B256 = B256::repeat_byte(0xa1);
const BOB_SEED: B256 = B256::repeat_byte(0xb0);

fn read(p: PathBuf) -> String {
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Causeway's own fixtures (`bridge/testdata`).
fn bridge_testdata(path: &str) -> String {
    read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata").join(path))
}

/// The RFP-021 mainnet fixtures at the repository root.
fn repo_testdata(path: &str) -> String {
    read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata").join(path))
}

/// The vault the fixture's receipts came from.
fn fixture_vault() -> Address {
    let v: serde_json::Value = serde_json::from_str(&bridge_testdata("vault/deposit-receipts.json")).unwrap();
    serde_json::from_value(v["vault"].clone()).unwrap()
}

fn deposit_predicate(vault: Address, token: Address, amount: U256) -> Predicate {
    Predicate(vec![
        Clause { field: Field::LogAddress, op: Op::Eq, value: U256::from_be_slice(vault.as_slice()) },
        Clause { field: Field::LogTopic(0), op: Op::Eq, value: U256::from_be_bytes(DEPOSIT_TOPIC.0) },
        Clause { field: Field::LogTopic(2), op: Op::Eq, value: U256::from_be_slice(token.as_slice()) },
        Clause { field: Field::LogDataWord(0), op: Op::Eq, value: amount },
    ])
}

/// A real verified statement about the `Deposit` log of transaction `tx_index` in the fixture.
fn vault_deposit_statement(tx_index: u64) -> Statement {
    let receipts = rpc::parse_receipts(&bridge_testdata("vault/deposit-receipts.json")).unwrap();
    let header = HeaderRecord {
        block_number: 1,
        block_hash: B256::repeat_byte(0xb1),
        timestamp: 1_762_646_400,
        receipts_root: rpc::receipts_root(&receipts),
        ..HeaderRecord::default()
    };
    let receipt = rpc::receipt_proof(&receipts, &header.receipts_root, tx_index).unwrap();
    attest(
        &ChainSpec::mainnet(),
        INSTANCE,
        &header,
        &Evidence::Log { receipt, log_index: 0 },
        &deposit_predicate(fixture_vault(), ETH, U256::from(DENOMINATION)),
    )
    .unwrap()
}

fn config() -> BridgeConfig {
    BridgeConfig {
        chain: ChainSpec::mainnet(),
        instance_id: INSTANCE,
        vault: fixture_vault(),
        tokens: vec![RegisteredToken {
            erc20: ETH,
            wrapped_mint: [0xcc; 32],
            denominations: vec![U256::from(DENOMINATION)],
        }],
    }
}

/// The secret a depositor re-derives from their seed for deposit `nonce` (RECOVERY.md).
fn secret(seed: &B256, nonce: u32) -> B256 {
    let cfg = config();
    deposit_secret(seed, &Coordinates { chain_id: cfg.chain.chain_id, vault: cfg.vault, token: ETH }, nonce)
}

#[test]
fn a_real_vault_deposit_authorises_a_mint_once_for_its_depositor() {
    // tx 1 of the fixture is Alice's deposit (her nonce 0).
    let statement = vault_deposit_statement(1);
    let cfg = config();
    let alice = secret(&ALICE_SEED, 0);

    let auth = authorize_mint(&cfg, &statement, &alice).expect("the vault deposit authorises a mint");
    assert_eq!(auth.amount, U256::from(DENOMINATION));
    assert_eq!(auth.wrapped_mint, [0xcc; 32]);

    // Minting is one-per-deposit: the nullifier spends once, a replay is refused.
    let mut spent = NullifierSet::default();
    spent.spend(&auth).expect("first mint spends the nullifier");
    assert_eq!(spent.spend(&auth).unwrap_err(), BridgeError::AlreadyMinted);

    // Deterministic: the depositor re-deriving from the same seed gets the same key.
    assert_eq!(authorize_mint(&cfg, &statement, &alice).unwrap().nullifier, auth.nullifier);

    // Only the depositor: Bob's secret, or any other, does not open Alice's commitment.
    assert_eq!(authorize_mint(&cfg, &statement, &secret(&BOB_SEED, 0)).unwrap_err(), BridgeError::WrongSecret);
    assert_eq!(authorize_mint(&cfg, &statement, &B256::repeat_byte(0xee)).unwrap_err(), BridgeError::WrongSecret);
    // And Alice cannot claim Bob's deposits with hers.
    assert_eq!(authorize_mint(&cfg, &vault_deposit_statement(0), &alice).unwrap_err(), BridgeError::WrongSecret);
}

#[test]
fn every_deposit_has_its_own_nullifier() {
    // Bob's two deposits (tx 0 at nonce 0, tx 2 at nonce 1) mint independently.
    let cfg = config();
    let first = authorize_mint(&cfg, &vault_deposit_statement(0), &secret(&BOB_SEED, 0)).unwrap();
    let second = authorize_mint(&cfg, &vault_deposit_statement(2), &secret(&BOB_SEED, 1)).unwrap();
    assert_ne!(first.nullifier, second.nullifier);
    let mut spent = NullifierSet::default();
    spent.spend(&first).unwrap();
    spent.spend(&second).unwrap();
}

#[test]
fn wrong_vault_registry_denomination_instance_and_network_are_refused() {
    let statement = vault_deposit_statement(1);
    let alice = secret(&ALICE_SEED, 0);

    // A deployment bound to some other vault does not honour this vault's deposits.
    let mut other_vault = config();
    other_vault.vault = address!("1111111111111111111111111111111111111111");
    assert_eq!(authorize_mint(&other_vault, &statement, &alice).unwrap_err(), BridgeError::NotThisVault);

    // The token is not registered.
    let mut empty = config();
    empty.tokens.clear();
    assert_eq!(authorize_mint(&empty, &statement, &alice).unwrap_err(), BridgeError::TokenNotRegistered);

    // The exact amount is not one of the registered denominations.
    let mut other_denom = config();
    other_denom.tokens[0].denominations = vec![U256::from(1_000_000_000_000_000_000u128)];
    assert_eq!(authorize_mint(&other_denom, &statement, &alice).unwrap_err(), BridgeError::AmountNotADenomination);

    // A bridge bound to a different instance refuses a statement from this one.
    let mut other_instance = config();
    other_instance.instance_id = B256::repeat_byte(0x99);
    assert_eq!(authorize_mint(&other_instance, &statement, &alice).unwrap_err(), BridgeError::WrongInstance);

    // A statement from another network is refused even if everything else matches.
    let mut sepolia = config();
    sepolia.chain = ChainSpec::sepolia();
    assert_eq!(authorize_mint(&sepolia, &statement, &alice).unwrap_err(), BridgeError::WrongInstance);
}

/// A plain ERC-20 `Transfer` into the vault, even a genuine one on mainnet, is not a
/// claimable deposit: it carries no commitment, so nobody could prove they own it. Only the
/// vault's own `Deposit` log, written by `depositETH`/`depositERC20`, mints.
#[test]
fn a_plain_erc20_transfer_is_not_a_claimable_deposit() {
    const USDC: Address = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
    const RECIPIENT: Address = address!("0000000aa232009084bd71a5797d089aa4edfad4");
    let block = rpc::parse_block(&repo_testdata("mainnet/block-26050794/block.json")).unwrap();
    let header = HeaderRecord {
        block_number: block.number,
        block_hash: block.hash,
        timestamp: block.timestamp,
        state_root: block.state_root,
        receipts_root: block.receipts_root,
        ..HeaderRecord::default()
    };
    let receipts = rpc::parse_receipts(&repo_testdata("mainnet/block-26050794/receipts.json")).unwrap();
    let receipt = rpc::receipt_proof(&receipts, &header.receipts_root, 0).unwrap();
    let transfer_topic = keccak256(b"Transfer(address,address,uint256)");
    let predicate = Predicate(vec![
        Clause { field: Field::LogAddress, op: Op::Eq, value: U256::from_be_slice(USDC.as_slice()) },
        Clause { field: Field::LogTopic(0), op: Op::Eq, value: U256::from_be_bytes(transfer_topic.0) },
        Clause { field: Field::LogTopic(2), op: Op::Eq, value: U256::from_be_slice(RECIPIENT.as_slice()) },
    ]);
    let statement =
        attest(&ChainSpec::mainnet(), INSTANCE, &header, &Evidence::Log { receipt, log_index: 0 }, &predicate).unwrap();

    let mut cfg = config();
    cfg.vault = RECIPIENT;
    cfg.tokens[0].erc20 = USDC;
    assert_eq!(authorize_mint(&cfg, &statement, &B256::repeat_byte(0xee)).unwrap_err(), BridgeError::NotADeposit);
}
