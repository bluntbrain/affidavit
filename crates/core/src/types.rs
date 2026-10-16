//! Light-client containers from the Altair sync protocol with the Capella/Deneb/Electra
//! execution extensions, and their `hash_tree_root`.

use alloc::vec::Vec;
use alloy_primitives::{Address, B256, U256};

use crate::config::ForkName;
use crate::ssz;

pub type BlsPublicKey = [u8; 48];
pub type BlsSignature = [u8; 96];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct BeaconBlockHeader {
    pub slot: u64,
    pub proposer_index: u64,
    pub parent_root: B256,
    pub state_root: B256,
    pub body_root: B256,
}

impl BeaconBlockHeader {
    pub fn hash_tree_root(&self) -> B256 {
        ssz::container_root(&[
            ssz::u64_chunk(self.slot),
            ssz::u64_chunk(self.proposer_index),
            self.parent_root,
            self.state_root,
            self.body_root,
        ])
    }
}

/// Execution payload header. Holds the superset of fields; the fork decides which are hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct ExecutionPayloadHeader {
    pub parent_hash: B256,
    pub fee_recipient: Address,
    pub state_root: B256,
    pub receipts_root: B256,
    pub logs_bloom: Vec<u8>,
    pub prev_randao: B256,
    pub block_number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub extra_data: Vec<u8>,
    pub base_fee_per_gas: U256,
    pub block_hash: B256,
    pub transactions_root: B256,
    pub withdrawals_root: B256,
    pub blob_gas_used: u64,
    pub excess_blob_gas: u64,
}

impl ExecutionPayloadHeader {
    pub fn hash_tree_root(&self, fork: ForkName) -> B256 {
        let mut fee_recipient = [0u8; 32];
        fee_recipient[..20].copy_from_slice(self.fee_recipient.as_slice());
        let mut fields = Vec::with_capacity(17);
        fields.extend_from_slice(&[
            self.parent_hash,
            B256::from(fee_recipient),
            self.state_root,
            self.receipts_root,
            ssz::byte_vector_root(&self.logs_bloom),
            self.prev_randao,
            ssz::u64_chunk(self.block_number),
            ssz::u64_chunk(self.gas_limit),
            ssz::u64_chunk(self.gas_used),
            ssz::u64_chunk(self.timestamp),
            ssz::byte_list_root(&self.extra_data, 32),
            B256::from(self.base_fee_per_gas.to_le_bytes::<32>()),
            self.block_hash,
            self.transactions_root,
            self.withdrawals_root,
        ]);
        if fork >= ForkName::Deneb {
            fields.push(ssz::u64_chunk(self.blob_gas_used));
            fields.push(ssz::u64_chunk(self.excess_blob_gas));
        }
        ssz::container_root(&fields)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct LightClientHeader {
    pub beacon: BeaconBlockHeader,
    pub execution: ExecutionPayloadHeader,
    pub execution_branch: Vec<B256>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct SyncCommittee {
    pub pubkeys: Vec<BlsPublicKey>,
    pub aggregate_pubkey: BlsPublicKey,
}

impl SyncCommittee {
    pub fn hash_tree_root(&self) -> B256 {
        let roots: Vec<B256> = self.pubkeys.iter().map(|pk| ssz::byte_vector_root(pk)).collect();
        let depth = roots.len().next_power_of_two().trailing_zeros() as usize;
        let pubkeys_root = ssz::merkleize(&roots, depth);
        ssz::hash_pair(&pubkeys_root, &ssz::byte_vector_root(&self.aggregate_pubkey))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct SyncAggregate {
    /// Bitvector, one bit per committee member, little-endian within each byte.
    pub sync_committee_bits: Vec<u8>,
    pub sync_committee_signature: BlsSignature,
}

impl SyncAggregate {
    pub fn participants(&self) -> u32 {
        self.sync_committee_bits.iter().map(|b| b.count_ones()).sum()
    }

    pub fn is_set(&self, i: usize) -> bool {
        self.sync_committee_bits.get(i / 8).is_some_and(|b| (b >> (i % 8)) & 1 == 1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct LightClientBootstrap {
    pub header: LightClientHeader,
    pub current_sync_committee: SyncCommittee,
    pub current_sync_committee_branch: Vec<B256>,
}

/// A `LightClientUpdate`, or a `LightClientFinalityUpdate` when `next_sync_committee` is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct LightClientUpdate {
    pub attested_header: LightClientHeader,
    pub next_sync_committee: Option<SyncCommittee>,
    pub next_sync_committee_branch: Vec<B256>,
    pub finalized_header: Option<LightClientHeader>,
    pub finality_branch: Vec<B256>,
    pub sync_aggregate: SyncAggregate,
    pub signature_slot: u64,
}

/// What an instance keeps about a finalised header: everything a consumer needs to check
/// inclusion against it, and nothing else. Written once, never modified (Reliability #3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct HeaderRecord {
    pub slot: u64,
    pub beacon_root: B256,
    pub block_number: u64,
    pub block_hash: B256,
    pub timestamp: u64,
    pub state_root: B256,
    pub receipts_root: B256,
    pub transactions_root: B256,
}

impl HeaderRecord {
    pub fn from_header(header: &LightClientHeader) -> Self {
        Self {
            slot: header.beacon.slot,
            beacon_root: header.beacon.hash_tree_root(),
            block_number: header.execution.block_number,
            block_hash: header.execution.block_hash,
            timestamp: header.execution.timestamp,
            state_root: header.execution.state_root,
            receipts_root: header.execution.receipts_root,
            transactions_root: header.execution.transactions_root,
        }
    }

    /// Canonical 184-byte encoding, the leaf of the instance's header accumulator.
    pub fn encode(&self) -> [u8; 184] {
        let mut out = [0u8; 184];
        out[0..8].copy_from_slice(&self.slot.to_le_bytes());
        out[8..40].copy_from_slice(self.beacon_root.as_slice());
        out[40..48].copy_from_slice(&self.block_number.to_le_bytes());
        out[48..80].copy_from_slice(self.block_hash.as_slice());
        out[80..88].copy_from_slice(&self.timestamp.to_le_bytes());
        out[88..120].copy_from_slice(self.state_root.as_slice());
        out[120..152].copy_from_slice(self.receipts_root.as_slice());
        out[152..184].copy_from_slice(self.transactions_root.as_slice());
        out
    }

    pub fn leaf(&self) -> B256 {
        let mut buf = Vec::with_capacity(208);
        buf.extend_from_slice(b"affidavit/header/v1\0\0\0\0\0");
        buf.extend_from_slice(&self.encode());
        ssz::sha256(&buf)
    }
}
