//! The off-chain side of an instance: what an operator or a consumer's wallet keeps next to
//! the on-chain state so it can supply witnesses (the committee keys behind each root) and
//! membership proofs (the header accumulator's leaves).

use std::collections::HashMap;

use affidavit_core::bls::{uncompress, Blst};
use affidavit_core::lc::{self, Advance, CommitteeWitness, Instance, InstanceConfig};
use affidavit_core::mmr::{Mmr, MmrProof};
use affidavit_core::types::{
    BlsPublicKey, HeaderRecord, LightClientBootstrap, LightClientUpdate, SyncAggregate, SyncCommittee,
};
use alloy_primitives::B256;

pub struct Follower {
    pub instance: Instance,
    pub committees: HashMap<B256, SyncCommittee>,
    pub headers: Mmr,
    pub records: Vec<HeaderRecord>,
}

impl Follower {
    pub fn new(
        config: InstanceConfig,
        bootstrap: &LightClientBootstrap,
        now_slot: u64,
    ) -> affidavit_core::Result<Self> {
        let instance = lc::initialize(config, bootstrap, now_slot)?;
        let mut committees = HashMap::new();
        committees.insert(instance.state.current_committee_root, bootstrap.current_sync_committee.clone());
        let mut headers = Mmr::default();
        headers.append(instance.state.finalized.leaf());
        Ok(Self { records: vec![instance.state.finalized], instance, committees, headers })
    }

    /// The committee keys behind the root the instance tracks for `update`'s signature
    /// period. When the follower has never seen them it returns an empty committee, and
    /// the verifier reports the mismatch.
    pub fn signing_committee(&self, update: &LightClientUpdate) -> SyncCommittee {
        let spec = &self.instance.config.chain;
        let store_period = spec.period_at_slot(self.instance.state.finalized.slot);
        let root = if spec.period_at_slot(update.signature_slot) == store_period {
            Some(self.instance.state.current_committee_root)
        } else {
            self.instance.state.next_committee_root
        };
        root.and_then(|r| self.committees.get(&r).cloned()).unwrap_or_else(|| SyncCommittee {
            pubkeys: vec![[0u8; 48]; spec.preset.sync_committee_size],
            aggregate_pubkey: [0u8; 48],
        })
    }

    /// What an operator supplies with the committee keys so the program never decompresses
    /// one (`docs/DECISIONS.md` D3): the aggregate key and the non-signers' keys as
    /// uncompressed points. Keys that do not decompress come back zeroed, which the verifier
    /// refuses.
    pub fn committee_witness(committee: &SyncCommittee, aggregate: &SyncAggregate) -> CommitteeWitness {
        let point = |pk: &BlsPublicKey| uncompress(pk).unwrap_or([0u8; 96]);
        CommitteeWitness {
            aggregate: point(&committee.aggregate_pubkey),
            non_signers: committee
                .pubkeys
                .iter()
                .enumerate()
                .filter(|(i, _)| !aggregate.is_set(*i))
                .map(|(_, pk)| point(pk))
                .collect(),
        }
    }

    /// Verifies `update` the way the program does (from a committee witness) and applies it.
    pub fn process(&mut self, update: &LightClientUpdate, now_slot: u64) -> affidavit_core::Result<Advance> {
        let committee = self.signing_committee(update);
        let witness = Self::committee_witness(&committee, &update.sync_aggregate);
        let advance =
            lc::verify_update_with_witness(&self.instance, update, &committee, Some(&witness), now_slot, &Blst)?;
        lc::apply_advance(&mut self.instance, &advance)?;
        if let Some(next) = &update.next_sync_committee {
            self.committees.insert(next.hash_tree_root(), next.clone());
        }
        if let Some(record) = advance.new_finalized {
            self.headers.append(record.leaf());
            self.records.push(record);
        }
        Ok(advance)
    }

    /// Membership proof for the recorded header at `block_number`, if the instance has one.
    pub fn prove_block(&self, block_number: u64) -> Option<(HeaderRecord, MmrProof)> {
        let index = self.records.iter().position(|r| r.block_number == block_number)?;
        Some((self.records[index], self.headers.prove(index as u64)?))
    }
}
