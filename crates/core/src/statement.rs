//! Facts, predicates and verified statements (Functionality #4, #5, #6).
//!
//! A consumer names what it wants proven (an account, a storage slot, a log, a transfer)
//! and a predicate over it; the module checks inclusion under a header the instance
//! verified, evaluates the predicate, and returns a [`Statement`]. The module attaches no
//! action to it. Predicates are data, so a consumer expresses new conditions without
//! touching the module.

use alloc::vec::Vec;
use alloy_consensus::{transaction::SignerRecoverable, Transaction as _};
use alloy_primitives::{Address, Bytes, B256, U256};
use sha2::{Digest, Sha256};

use crate::config::ChainSpec;
use crate::error::{Error, Result};
use crate::execution::{
    verify_account, verify_receipt, verify_storage, verify_transaction, AccountProof, ReceiptProof, StorageProof,
    TransactionProof,
};
use crate::mmr::{self, MmrProof};
use crate::types::HeaderRecord;

/// A fact about finalised Ethereum state, as proven.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub enum Fact {
    Account {
        address: Address,
        nonce: u64,
        balance: U256,
        code_hash: B256,
        storage_root: B256,
    },
    Storage {
        address: Address,
        slot: B256,
        value: U256,
    },
    Log {
        block_hash: B256,
        tx_index: u64,
        log_index: u64,
        address: Address,
        topics: Vec<B256>,
        data: Bytes,
    },
    Transaction {
        block_hash: B256,
        tx_index: u64,
        tx_hash: B256,
        from: Address,
        to: Option<Address>,
        value: U256,
        /// `Some(true)` when the receipt was proven too; `None` when only inclusion was.
        success: Option<bool>,
    },
}

/// A value a predicate can compare. Addresses and hashes compare as 256-bit integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub enum Field {
    Balance,
    Nonce,
    CodeHash,
    StorageValue,
    LogAddress,
    LogTopic(u8),
    /// 32-byte word `n` of the log data (word 0 is the amount of an ERC-20 `Transfer`).
    LogDataWord(u32),
    TxFrom,
    TxTo,
    TxValue,
    TxSuccess,
    BlockNumber,
    BlockTimestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct Clause {
    pub field: Field,
    pub op: Op,
    pub value: U256,
}

/// A conjunction of clauses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct Predicate(pub Vec<Clause>);

fn addr(a: &Address) -> U256 {
    U256::from_be_slice(a.as_slice())
}

fn word(b: &B256) -> U256 {
    U256::from_be_bytes(b.0)
}

impl Fact {
    pub fn get(&self, field: Field, header: &HeaderRecord) -> Option<U256> {
        match (field, self) {
            (Field::BlockNumber, _) => Some(U256::from(header.block_number)),
            (Field::BlockTimestamp, _) => Some(U256::from(header.timestamp)),
            (Field::Balance, Fact::Account { balance, .. }) => Some(*balance),
            (Field::Nonce, Fact::Account { nonce, .. }) => Some(U256::from(*nonce)),
            (Field::CodeHash, Fact::Account { code_hash, .. }) => Some(word(code_hash)),
            (Field::StorageValue, Fact::Storage { value, .. }) => Some(*value),
            (Field::LogAddress, Fact::Log { address, .. }) => Some(addr(address)),
            (Field::LogTopic(i), Fact::Log { topics, .. }) => topics.get(i as usize).map(word),
            (Field::LogDataWord(i), Fact::Log { data, .. }) => {
                let start = i as usize * 32;
                data.get(start..start + 32).map(U256::from_be_slice)
            }
            (Field::TxFrom, Fact::Transaction { from, .. }) => Some(addr(from)),
            (Field::TxTo, Fact::Transaction { to: Some(to), .. }) => Some(addr(to)),
            (Field::TxValue, Fact::Transaction { value, .. }) => Some(*value),
            (Field::TxSuccess, Fact::Transaction { success: Some(ok), .. }) => Some(U256::from(*ok as u8)),
            _ => None,
        }
    }
}

impl Predicate {
    pub fn evaluate(&self, fact: &Fact, header: &HeaderRecord) -> Result<()> {
        for (i, clause) in self.0.iter().enumerate() {
            let lhs = fact.get(clause.field, header).ok_or(Error::StateNotCovered)?;
            let holds = match clause.op {
                Op::Eq => lhs == clause.value,
                Op::Ne => lhs != clause.value,
                Op::Lt => lhs < clause.value,
                Op::Le => lhs <= clause.value,
                Op::Gt => lhs > clause.value,
                Op::Ge => lhs >= clause.value,
            };
            if !holds {
                return Err(Error::PredicateNotSatisfied { clause: i as u32 });
            }
        }
        Ok(())
    }
}

/// The verified output. Carries the chain binding and the instance it was produced under.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct Statement {
    pub chain_id: u64,
    pub chain_digest: B256,
    pub instance_id: B256,
    pub header: HeaderRecord,
    pub fact: Fact,
    pub predicate: Predicate,
    /// Deterministic identifier of the attested Ethereum state (#6). It depends on the chain
    /// and the state only, not on the instance or the header it was read under, so a
    /// consumer that migrates to a replacement instance keeps its nullifiers valid.
    pub id: B256,
}

impl Statement {
    /// The consumer-side binding check (#5, #15): refuse a statement about another network or
    /// produced under an instance this consumer is not bound to.
    pub fn check_binding(&self, chain: &ChainSpec, instance_id: &B256) -> Result<()> {
        if self.chain_id != chain.chain_id || self.chain_digest != chain.digest() {
            return Err(Error::ChainMismatch);
        }
        if self.instance_id != *instance_id {
            return Err(Error::InstanceMismatch);
        }
        Ok(())
    }
}

/// Identifier derivation. Events are named by their position on the chain; state by what
/// it is. Identical state gives an identical identifier, distinct state a distinct one.
pub fn statement_id(chain: &ChainSpec, fact: &Fact) -> B256 {
    let mut h = Sha256::new();
    h.update(b"affidavit/statement/v1");
    h.update(chain.digest());
    match fact {
        Fact::Account { address, nonce, balance, code_hash, storage_root } => {
            h.update([0u8]);
            h.update(address);
            h.update(nonce.to_be_bytes());
            h.update(balance.to_be_bytes::<32>());
            h.update(code_hash);
            h.update(storage_root);
        }
        Fact::Storage { address, slot, value } => {
            h.update([1u8]);
            h.update(address);
            h.update(slot);
            h.update(value.to_be_bytes::<32>());
        }
        Fact::Log { block_hash, tx_index, log_index, .. } => {
            h.update([2u8]);
            h.update(block_hash);
            h.update(tx_index.to_be_bytes());
            h.update(log_index.to_be_bytes());
        }
        Fact::Transaction { tx_hash, .. } => {
            h.update([3u8]);
            h.update(tx_hash);
        }
    }
    B256::from_slice(&h.finalize())
}

/// Proof material for one fact.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub enum Evidence {
    Account(AccountProof),
    Storage { account: AccountProof, storage: StorageProof },
    Log { receipt: ReceiptProof, log_index: u64 },
    Transfer { tx: TransactionProof, receipt: Option<ReceiptProof> },
}

/// Proves `evidence` under `header` and evaluates `predicate`. The header must already be
/// one the instance verified; see [`attest_with_membership`] for that check.
pub fn attest(
    chain: &ChainSpec,
    instance_id: B256,
    header: &HeaderRecord,
    evidence: &Evidence,
    predicate: &Predicate,
) -> Result<Statement> {
    let fact = match evidence {
        Evidence::Account(account) => {
            verify_account(&header.state_root, account)?;
            Fact::Account {
                address: account.address,
                nonce: account.nonce,
                balance: account.balance,
                code_hash: account.code_hash,
                storage_root: account.storage_root,
            }
        }
        Evidence::Storage { account, storage } => {
            verify_account(&header.state_root, account)?;
            verify_storage(&account.storage_root, storage)?;
            Fact::Storage { address: account.address, slot: storage.slot, value: storage.value }
        }
        Evidence::Log { receipt, log_index } => {
            let envelope = verify_receipt(&header.receipts_root, receipt)?;
            let log = envelope.logs().get(*log_index as usize).ok_or(Error::StateNotCovered)?;
            Fact::Log {
                block_hash: header.block_hash,
                tx_index: receipt.tx_index,
                log_index: *log_index,
                address: log.address,
                topics: log.data.topics().to_vec(),
                data: log.data.data.clone(),
            }
        }
        Evidence::Transfer { tx, receipt } => {
            let envelope = verify_transaction(&header.transactions_root, tx)?;
            if envelope.chain_id() != Some(chain.chain_id) {
                return Err(Error::TransactionChainMismatch);
            }
            let from = envelope.recover_signer().map_err(|_| Error::MalformedInput("transaction signature"))?;
            let success = match receipt {
                Some(r) => {
                    if r.tx_index != tx.tx_index {
                        return Err(Error::StateNotCovered);
                    }
                    let ok = verify_receipt(&header.receipts_root, r)?.status();
                    if !ok {
                        return Err(Error::TransactionReverted);
                    }
                    Some(ok)
                }
                None => None,
            };
            Fact::Transaction {
                block_hash: header.block_hash,
                tx_index: tx.tx_index,
                tx_hash: *envelope.tx_hash(),
                from,
                to: envelope.to(),
                value: envelope.value(),
                success,
            }
        }
    };
    predicate.evaluate(&fact, header)?;
    let id = statement_id(chain, &fact);
    Ok(Statement {
        chain_id: chain.chain_id,
        chain_digest: chain.digest(),
        instance_id,
        header: *header,
        fact,
        predicate: predicate.clone(),
        id,
    })
}

/// As [`attest`], after checking the header is a member of the instance's header
/// accumulator. The only instance data this reads is the accumulator root, which every
/// consumer shares, so a private consumer discloses nothing about which header it used.
pub fn attest_with_membership(
    chain: &ChainSpec,
    instance_id: B256,
    headers_root: &B256,
    header: &HeaderRecord,
    membership: &MmrProof,
    evidence: &Evidence,
    predicate: &Predicate,
) -> Result<Statement> {
    if !mmr::verify(headers_root, &header.leaf(), membership) {
        return Err(Error::HeaderUnknown);
    }
    attest(chain, instance_id, header, evidence, predicate)
}
