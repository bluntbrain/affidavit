//! What the measurement guest is asked to do, and what it reports back.

#![no_std]
extern crate alloc;

use affidavit_core::config::ChainSpec;
use affidavit_core::lc::{Advance, CommitteeWitness, Instance};
use affidavit_core::mmr::MmrProof;
use affidavit_core::statement::{Evidence, Predicate};
use affidavit_core::types::{HeaderRecord, LightClientUpdate, SyncCommittee};
use alloc::string::String;
use alloc::vec::Vec;
use alloy_primitives::B256;
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshSerialize, BorshDeserialize)]
pub enum Job {
    /// `lc::verify_update` end to end: what a private verifier actor proves.
    VerifyUpdate {
        instance: Instance,
        update: LightClientUpdate,
        committee: SyncCommittee,
        /// The aggregate and non-signer keys uncompressed, as the program receives them;
        /// `None` makes the guest decompress every signer's key instead.
        witness: Option<CommitteeWitness>,
        now_slot: u64,
        check_subgroups: bool,
    },
    /// `lc::apply_advance`: what the public instance actor executes.
    ApplyAdvance { instance: Instance, advance: Advance },
    /// `statement::attest_with_membership`: what a consumer proves or executes.
    Attest {
        chain: ChainSpec,
        instance_id: B256,
        headers_root: B256,
        header: HeaderRecord,
        membership: MmrProof,
        evidence: Evidence,
        predicate: Predicate,
    },
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Default)]
pub struct Report {
    /// Error code, 0 when the job succeeded.
    pub verdict: u16,
    /// Cycle counts of the phases the guest timed, in order.
    pub phases: Vec<(String, u64)>,
}
