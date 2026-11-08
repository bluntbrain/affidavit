//! The seam, end to end on real mainnet data: a genuine Affidavit statement about a USDC
//! `Transfer` is turned into a wrapped-mint authorisation, and the replay, wrong-vault,
//! wrong-denomination and wrong-instance cases are all refused.
//!
//! The statement here is produced by `affidavit_core::statement::attest` over the real
//! receipt proof in `testdata/mainnet/block-26050794` — the same fixture the RFP-021 suite
//! attests — so this exercises the actual RFP-021 → RFP-022 seam, not a mock.

use std::path::PathBuf;

use affidavit_core::config::ChainSpec;
use affidavit_core::statement::{attest, Clause, Evidence, Field, Op, Predicate};
use affidavit_core::types::HeaderRecord;
use affidavit_io::rpc;
use alloy_primitives::{address, Address, B256, U256};
use causeway_mint_demo::{authorize_mint, BridgeConfig, BridgeError, NullifierSet, RegisteredToken, TRANSFER_TOPIC};

const USDC: Address = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
// The recipient of the real USDC transfer in the fixture; this demo treats it as the vault.
const VAULT: Address = address!("0000000aa232009084bd71a5797d089aa4edfad4");
const AMOUNT: u128 = 3_300_795;

fn testdata(path: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata").join(path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn fixture_header() -> HeaderRecord {
    let block = rpc::parse_block(&testdata("mainnet/block-26050794/block.json")).unwrap();
    HeaderRecord {
        block_number: block.number,
        block_hash: block.hash,
        timestamp: block.timestamp,
        state_root: block.state_root,
        receipts_root: block.receipts_root,
        ..HeaderRecord::default()
    }
}

fn transfer_predicate(token: Address, to: Address, amount: U256) -> Predicate {
    Predicate(vec![
        Clause { field: Field::LogAddress, op: Op::Eq, value: U256::from_be_slice(token.as_slice()) },
        Clause { field: Field::LogTopic(0), op: Op::Eq, value: U256::from_be_bytes(TRANSFER_TOPIC.0) },
        Clause { field: Field::LogTopic(2), op: Op::Eq, value: U256::from_be_slice(to.as_slice()) },
        Clause { field: Field::LogDataWord(0), op: Op::Ge, value: amount },
    ])
}

/// Produce a real verified statement about the USDC transfer in the fixture.
fn deposit_statement() -> affidavit_core::statement::Statement {
    let header = fixture_header();
    let receipts = rpc::parse_receipts(&testdata("mainnet/block-26050794/receipts.json")).unwrap();
    let receipt = rpc::receipt_proof(&receipts, &header.receipts_root, 0).unwrap();
    let evidence = Evidence::Log { receipt, log_index: 0 };
    attest(
        &ChainSpec::mainnet(),
        B256::repeat_byte(0x42),
        &header,
        &evidence,
        &transfer_predicate(USDC, VAULT, U256::from(AMOUNT)),
    )
    .unwrap()
}

fn config() -> BridgeConfig {
    BridgeConfig {
        chain: ChainSpec::mainnet(),
        instance_id: B256::repeat_byte(0x42),
        vault: VAULT,
        tokens: vec![RegisteredToken {
            erc20: USDC,
            wrapped_mint: [0xcc; 32],
            denominations: vec![U256::from(AMOUNT)],
        }],
    }
}

#[test]
fn real_deposit_authorises_a_mint_once() {
    let statement = deposit_statement();
    let cfg = config();
    let auth = authorize_mint(&cfg, &statement).expect("the USDC deposit authorises a mint");
    assert_eq!(auth.amount, U256::from(AMOUNT));
    assert_eq!(auth.wrapped_mint, [0xcc; 32]);

    // Minting is one-per-deposit: the nullifier spends once, a replay is refused.
    let mut spent = NullifierSet::default();
    spent.spend(&auth).expect("first mint spends the nullifier");
    assert_eq!(spent.spend(&auth).unwrap_err(), BridgeError::AlreadyMinted);

    // The nullifier is deterministic in the statement id and nothing else, so an operator
    // re-deriving it from the same deposit gets the same key.
    let again = authorize_mint(&cfg, &statement).unwrap();
    assert_eq!(again.nullifier, auth.nullifier);
}

#[test]
fn wrong_vault_registry_denomination_and_instance_are_refused() {
    let statement = deposit_statement();

    // A deployment whose vault is some other address does not accept this deposit.
    let mut other_vault = config();
    other_vault.vault = address!("1111111111111111111111111111111111111111");
    assert_eq!(authorize_mint(&other_vault, &statement).unwrap_err(), BridgeError::NotToVault);

    // The token is not registered.
    let mut empty = config();
    empty.tokens.clear();
    assert_eq!(authorize_mint(&empty, &statement).unwrap_err(), BridgeError::TokenNotRegistered);

    // The exact amount is not one of the registered denominations.
    let mut other_denom = config();
    other_denom.tokens[0].denominations = vec![U256::from(1_000_000u64)];
    assert_eq!(authorize_mint(&other_denom, &statement).unwrap_err(), BridgeError::AmountNotADenomination);

    // A bridge bound to a different instance refuses a statement from this one.
    let mut other_instance = config();
    other_instance.instance_id = B256::repeat_byte(0x99);
    assert_eq!(authorize_mint(&other_instance, &statement).unwrap_err(), BridgeError::WrongInstance);

    // A statement from another network is refused even if everything else matches.
    let mut sepolia = config();
    sepolia.chain = ChainSpec::sepolia();
    assert_eq!(authorize_mint(&sepolia, &statement).unwrap_err(), BridgeError::WrongInstance);
}
