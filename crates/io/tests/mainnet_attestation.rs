//! Inclusion and predicates on real mainnet execution data.
//!
//! - Block 17,923,026 is the header the light client verified (see `mainnet_light_client.rs`).
//!   Its transactions come from the beacon block, so a native ETH transfer is attested end
//!   to end with no JSON-RPC root in the trust path.
//! - Blocks 21,925,176 and 26,050,794 carry receipts and an `eth_getProof`, but no
//!   light-client fixtures reach them; their roots come from the block JSON, which is
//!   exactly the input the module never trusts in production. These tests exercise the
//!   inclusion code on real tries, not the binding (that is the test above).

mod common;

use affidavit_core::config::ChainSpec;
use affidavit_core::error::Error;
use affidavit_core::execution::{verify_receipt, TransactionProof};
use affidavit_core::mmr;
use affidavit_core::ssz;
use affidavit_core::statement::{
    attest, attest_with_membership, statement_id, Clause, Evidence, Fact, Field, Op, Predicate,
};
use affidavit_core::types::HeaderRecord;
use affidavit_io::{beacon, rpc};
use alloy_consensus::{Transaction as _, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{address, b256, Address, Bytes, B256, U256};
use common::*;

const TRANSFER_TOPIC: B256 = b256!("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
const WETH: Address = address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
const USDC: Address = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
const DEPOSIT_CONTRACT: Address = address!("00000000219ab540356cbb839cbe05303d7705fa");

fn rpc_header(dir: &str) -> HeaderRecord {
    let block = rpc::parse_block(&read(&format!("mainnet/{dir}/block.json"))).unwrap();
    HeaderRecord {
        slot: 0,
        beacon_root: B256::ZERO,
        block_number: block.number,
        block_hash: block.hash,
        timestamp: block.timestamp,
        state_root: block.state_root,
        receipts_root: block.receipts_root,
        transactions_root: B256::ZERO,
    }
}

fn erc20_transfer(token: Address, to: Address, min_amount: U256) -> Predicate {
    Predicate(vec![
        Clause { field: Field::LogAddress, op: Op::Eq, value: U256::from_be_slice(token.as_slice()) },
        Clause { field: Field::LogTopic(0), op: Op::Eq, value: U256::from_be_bytes(TRANSFER_TOPIC.0) },
        Clause { field: Field::LogTopic(2), op: Op::Eq, value: U256::from_be_slice(to.as_slice()) },
        Clause { field: Field::LogDataWord(0), op: Op::Ge, value: min_amount },
    ])
}

#[test]
fn receipts_tries_rebuild_to_the_header_roots() {
    for dir in ["block-21925176", "block-26050794"] {
        let header = rpc_header(dir);
        let receipts = rpc::parse_receipts(&read(&format!("mainnet/{dir}/receipts.json"))).unwrap();
        // Building any proof checks the rebuilt root against the header's.
        for i in [0, receipts.len() as u64 / 2, receipts.len() as u64 - 1] {
            let proof = rpc::receipt_proof(&receipts, &header.receipts_root, i).unwrap();
            verify_receipt(&header.receipts_root, &proof).unwrap();
        }
    }
}

#[test]
fn erc20_transfer_logs_attest() {
    let mainnet = ChainSpec::mainnet();
    let cases = [
        // (block, tx, log, token, recipient, amount)
        (
            "block-21925176",
            0,
            1,
            WETH,
            address!("999b8fc9161fb1f399a0a2d280ae6a75c697f432"),
            32_913_110_348_906_795u128,
        ),
        ("block-26050794", 0, 0, USDC, address!("0000000aa232009084bd71a5797d089aa4edfad4"), 3_300_795u128),
    ];
    for (dir, tx, log, token, to, amount) in cases {
        let header = rpc_header(dir);
        let receipts = rpc::parse_receipts(&read(&format!("mainnet/{dir}/receipts.json"))).unwrap();
        let receipt = rpc::receipt_proof(&receipts, &header.receipts_root, tx).unwrap();
        let evidence = Evidence::Log { receipt, log_index: log };

        let ok =
            attest(&mainnet, B256::ZERO, &header, &evidence, &erc20_transfer(token, to, U256::from(amount))).unwrap();
        match &ok.fact {
            Fact::Log { address, topics, .. } => {
                assert_eq!(*address, token);
                assert_eq!(topics[0], TRANSFER_TOPIC);
            }
            other => panic!("unexpected fact {other:?}"),
        }

        // One wei more than was sent: the predicate, not the proof, fails.
        let too_much = erc20_transfer(token, to, U256::from(amount + 1));
        assert_eq!(
            attest(&mainnet, B256::ZERO, &header, &evidence, &too_much).unwrap_err(),
            Error::PredicateNotSatisfied { clause: 3 }
        );
        // A different token at the same position is a different fact.
        let wrong_token = erc20_transfer(DEPOSIT_CONTRACT, to, U256::from(amount));
        assert_eq!(
            attest(&mainnet, B256::ZERO, &header, &evidence, &wrong_token).unwrap_err(),
            Error::PredicateNotSatisfied { clause: 0 }
        );
    }
}

#[test]
fn tampered_receipt_is_rejected() {
    let header = rpc_header("block-21925176");
    let receipts = rpc::parse_receipts(&read("mainnet/block-21925176/receipts.json")).unwrap();
    let mut proof = rpc::receipt_proof(&receipts, &header.receipts_root, 0).unwrap();
    let mut bytes = proof.receipt.to_vec();
    let last = bytes.len() - 1;
    bytes[last] ^= 1; // change one bit of the log data
    proof.receipt = Bytes::from(bytes);
    let evidence = Evidence::Log { receipt: proof, log_index: 1 };
    assert_eq!(
        attest(&ChainSpec::mainnet(), B256::ZERO, &header, &evidence, &Predicate::default()).unwrap_err(),
        Error::InclusionProofInvalid
    );
}

#[test]
fn account_and_storage_proofs_attest() {
    let header = rpc_header("block-21925176");
    let (account, storage) = rpc::parse_get_proof(&read("mainnet/block-21925176/proof.json")).unwrap();
    assert_eq!(account.address, DEPOSIT_CONTRACT);
    let mainnet = ChainSpec::mainnet();

    // "The deposit contract holds at least 57 million ETH."
    let min_balance = U256::from(57_000_000u64) * U256::from(10u64).pow(U256::from(18u64));
    let holds = Predicate(vec![Clause { field: Field::Balance, op: Op::Ge, value: min_balance }]);
    attest(&mainnet, B256::ZERO, &header, &Evidence::Account(account.clone()), &holds).unwrap();

    let slot = storage[0].clone();
    let equals = Predicate(vec![Clause { field: Field::StorageValue, op: Op::Eq, value: slot.value }]);
    let statement = attest(
        &mainnet,
        B256::ZERO,
        &header,
        &Evidence::Storage { account: account.clone(), storage: slot.clone() },
        &equals,
    )
    .unwrap();
    assert!(matches!(statement.fact, Fact::Storage { .. }));

    // Claiming a different balance or slot value fails the proof itself.
    let mut forged = account.clone();
    forged.balance += U256::from(1u8);
    assert_eq!(
        attest(&mainnet, B256::ZERO, &header, &Evidence::Account(forged), &Predicate::default()).unwrap_err(),
        Error::InclusionProofInvalid
    );
    let mut forged_slot = slot;
    forged_slot.value += U256::from(1u8);
    assert_eq!(
        attest(
            &mainnet,
            B256::ZERO,
            &header,
            &Evidence::Storage { account, storage: forged_slot },
            &Predicate::default()
        )
        .unwrap_err(),
        Error::InclusionProofInvalid
    );
}

/// First transaction in the verified block that sends ETH with no calldata.
fn plain_transfer(txs: &[Vec<u8>]) -> (usize, TxEnvelope) {
    txs.iter()
        .enumerate()
        .map(|(i, raw)| (i, TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap()))
        .find(|(_, tx)| !tx.value().is_zero() && tx.input().is_empty() && tx.to().is_some())
        .expect("block has a plain ETH transfer")
}

#[test]
fn native_eth_transfer_under_the_light_client_header() {
    let follower = synced();
    let header = follower.instance.state.finalized;
    let (_, txs) = beacon::parse_block_transactions(&read("mainnet/light-client/beacon-block-7109344.json")).unwrap();
    let (index, tx) = plain_transfer(&txs);
    let proof = TransactionProof {
        tx_index: index as u64,
        tx: Bytes::from(txs[index].clone()),
        branch: ssz::transaction_branch(&txs, index),
    };
    let to = tx.to().unwrap();
    let predicate = Predicate(vec![
        Clause { field: Field::TxTo, op: Op::Eq, value: U256::from_be_slice(to.as_slice()) },
        Clause { field: Field::TxValue, op: Op::Ge, value: tx.value() },
    ]);
    let evidence = Evidence::Transfer { tx: proof.clone(), receipt: None };

    // The header is checked against the instance's accumulator, as a private consumer would.
    let membership = follower.headers.prove(follower.records.len() as u64 - 1).unwrap();
    let root = follower.instance.state.headers.root();
    let statement = attest_with_membership(
        &ChainSpec::mainnet(),
        follower.instance.config.id(),
        &root,
        &header,
        &membership,
        &evidence,
        &predicate,
    )
    .unwrap();
    match statement.fact {
        Fact::Transaction { value, to: Some(dest), success, .. } => {
            assert_eq!(value, tx.value());
            assert_eq!(dest, to);
            // No receipt in this fixture, so success is unproven and a predicate that needs it is refused.
            assert_eq!(success, None);
        }
        ref other => panic!("unexpected fact {other:?}"),
    }
    let needs_status = Predicate(vec![Clause { field: Field::TxSuccess, op: Op::Eq, value: U256::from(1u8) }]);
    assert_eq!(
        attest(&ChainSpec::mainnet(), B256::ZERO, &header, &evidence, &needs_status).unwrap_err(),
        Error::StateNotCovered
    );

    // The same transaction under a Sepolia instance is refused: its signature binds chain id 1.
    assert_eq!(
        attest(&ChainSpec::sepolia(), B256::ZERO, &header, &evidence, &Predicate::default()).unwrap_err(),
        Error::TransactionChainMismatch
    );

    // A transaction that is not in the block does not verify.
    let mut other = proof;
    other.tx_index += 1;
    assert_eq!(
        attest(
            &ChainSpec::mainnet(),
            B256::ZERO,
            &header,
            &Evidence::Transfer { tx: other, receipt: None },
            &predicate
        )
        .unwrap_err(),
        Error::InclusionProofInvalid
    );
}

#[test]
fn header_outside_the_accumulator_is_unknown() {
    let follower = synced();
    let mut forged = follower.instance.state.finalized;
    forged.state_root = B256::repeat_byte(0x42);
    let membership = follower.headers.prove(follower.records.len() as u64 - 1).unwrap();
    let root = follower.instance.state.headers.root();
    assert!(mmr::verify(&root, &follower.instance.state.finalized.leaf(), &membership));
    let evidence = Evidence::Account(rpc::parse_get_proof(&read("mainnet/block-21925176/proof.json")).unwrap().0);
    assert_eq!(
        attest_with_membership(
            &ChainSpec::mainnet(),
            B256::ZERO,
            &root,
            &forged,
            &membership,
            &evidence,
            &Predicate::default()
        )
        .unwrap_err(),
        Error::HeaderUnknown
    );
}

#[test]
fn statement_identifiers() {
    let mainnet = ChainSpec::mainnet();
    let log = |log_index| Fact::Log {
        block_hash: B256::repeat_byte(1),
        tx_index: 3,
        log_index,
        address: WETH,
        topics: vec![TRANSFER_TOPIC],
        data: Bytes::new(),
    };
    // Identical state, identical identifier; the instance it was read under does not matter.
    assert_eq!(statement_id(&mainnet, &log(0)), statement_id(&mainnet, &log(0)));
    // Distinct state, distinct identifier.
    assert_ne!(statement_id(&mainnet, &log(0)), statement_id(&mainnet, &log(1)));
    // Same contract address and slot on two networks: never the same identifier.
    let slot = Fact::Storage { address: WETH, slot: B256::ZERO, value: U256::from(7u8) };
    assert_ne!(statement_id(&mainnet, &slot), statement_id(&ChainSpec::hoodi(), &slot));
    assert_ne!(statement_id(&mainnet, &slot), statement_id(&ChainSpec::sepolia(), &slot));
}
