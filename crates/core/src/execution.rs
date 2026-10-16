//! Inclusion of execution-layer state under a finalised header (Functionality #3): accounts
//! and storage slots by Merkle-Patricia proof against the state root, receipts (and so
//! logs) against the receipts root, and transactions by SSZ proof against the payload's
//! `transactions_root`. Proof verification is `alloy-trie`'s, the trie code reth uses.

use alloc::vec::Vec;
use alloy_consensus::{Header, ReceiptEnvelope, TxEnvelope};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_rlp::Decodable;
use alloy_trie::{proof::verify_proof, Nibbles, TrieAccount, EMPTY_ROOT_HASH, KECCAK_EMPTY};

use crate::error::{Error, Result};
use crate::ssz;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct AccountProof {
    pub address: Address,
    pub nonce: u64,
    pub balance: U256,
    pub storage_root: B256,
    pub code_hash: B256,
    pub proof: Vec<Bytes>,
}

impl AccountProof {
    /// EIP-161: an empty account is not in the trie, so its proof is an exclusion proof.
    pub fn is_empty(&self) -> bool {
        self.nonce == 0
            && self.balance.is_zero()
            && self.code_hash == KECCAK_EMPTY
            && self.storage_root == EMPTY_ROOT_HASH
    }
}

pub fn verify_account(state_root: &B256, account: &AccountProof) -> Result<()> {
    let key = Nibbles::unpack(keccak256(account.address));
    let expected = (!account.is_empty()).then(|| {
        alloy_rlp::encode(TrieAccount {
            nonce: account.nonce,
            balance: account.balance,
            storage_root: account.storage_root,
            code_hash: account.code_hash,
        })
    });
    verify_proof(*state_root, key, expected, &account.proof).map_err(|_| Error::InclusionProofInvalid)
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct StorageProof {
    pub slot: B256,
    pub value: U256,
    pub proof: Vec<Bytes>,
}

pub fn verify_storage(storage_root: &B256, storage: &StorageProof) -> Result<()> {
    let key = Nibbles::unpack(keccak256(storage.slot));
    let expected = (!storage.value.is_zero()).then(|| alloy_rlp::encode(storage.value));
    verify_proof(*storage_root, key, expected, &storage.proof).map_err(|_| Error::InclusionProofInvalid)
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct ReceiptProof {
    pub tx_index: u64,
    /// The receipt's EIP-2718 encoding, which is the value stored in the receipts trie.
    pub receipt: Bytes,
    pub proof: Vec<Bytes>,
}

pub fn receipt_key(tx_index: u64) -> Vec<u8> {
    alloy_rlp::encode(tx_index)
}

pub fn verify_receipt(receipts_root: &B256, receipt: &ReceiptProof) -> Result<ReceiptEnvelope> {
    let key = Nibbles::unpack(receipt_key(receipt.tx_index));
    verify_proof(*receipts_root, key, Some(receipt.receipt.to_vec()), &receipt.proof)
        .map_err(|_| Error::InclusionProofInvalid)?;
    ReceiptEnvelope::decode_2718(&mut receipt.receipt.as_ref()).map_err(|_| Error::MalformedInput("receipt"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct TransactionProof {
    pub tx_index: u64,
    /// The transaction's EIP-2718 encoding, as it appears in the execution payload.
    pub tx: Bytes,
    /// 20 siblings in the transactions tree, then the length chunk.
    pub branch: Vec<B256>,
}

pub fn verify_transaction(transactions_root: &B256, tx: &TransactionProof) -> Result<TxEnvelope> {
    if tx.tx_index >= 1 << ssz::TRANSACTIONS_DEPTH {
        return Err(Error::MalformedInput("transaction index"));
    }
    let leaf = ssz::transaction_root(&tx.tx);
    if !ssz::is_valid_merkle_branch(&leaf, &tx.branch, ssz::TRANSACTIONS_DEPTH + 1, tx.tx_index, transactions_root) {
        return Err(Error::InclusionProofInvalid);
    }
    TxEnvelope::decode_2718(&mut tx.tx.as_ref()).map_err(|_| Error::MalformedInput("transaction"))
}

/// From Gloas on, a light-client header commits only to the execution block hash. The
/// roots then come from the RLP execution header, checked against that hash.
pub fn verify_execution_header(block_hash: &B256, rlp_header: &[u8]) -> Result<Header> {
    if keccak256(rlp_header) != *block_hash {
        return Err(Error::ExecutionNotBound);
    }
    Header::decode(&mut &rlp_header[..]).map_err(|_| Error::MalformedInput("execution header"))
}
