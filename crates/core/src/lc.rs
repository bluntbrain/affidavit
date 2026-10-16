//! The light client an instance runs: bootstrap from a weak-subjectivity checkpoint, then
//! follow finalised headers and committee handoffs forward (Functionality #1, #2).
//!
//! The rules are the Altair `sync-protocol.md` rules with the Capella execution extension
//! and the Electra generalised indices, restricted to what an on-chain instance should do:
//! it only accepts finality updates signed by at least two thirds of the committee (the
//! spec's own threshold for applying an update), keeps no optimistic header, and never
//! force-updates on a timeout. Verification is split from application: `verify_update` is
//! pure and read-only and returns an [`Advance`]; `apply_advance` is a compare-and-swap on
//! the tracked state. That split is what lets a private actor do the expensive checks and
//! hand a public actor only the result (see `docs/DESIGN.md`).

use alloc::vec::Vec;
use alloy_primitives::B256;
use sha2::{Digest, Sha256};

use crate::bls::BlsVerifier;
use crate::config::{
    current_sync_committee_gindex, finalized_root_gindex, next_sync_committee_gindex, ChainSpec, ForkName,
    EXECUTION_PAYLOAD_GINDEX,
};
use crate::error::{Error, Result};
use crate::mmr::MmrAccumulator;
use crate::ssz::{hash_pair, is_valid_merkle_branch};
use crate::types::{
    BlsPublicKey, HeaderRecord, LightClientBootstrap, LightClientHeader, LightClientUpdate, SyncAggregate,
    SyncCommittee,
};

pub const DOMAIN_SYNC_COMMITTEE: [u8; 4] = [0x07, 0x00, 0x00, 0x00];

/// Everything that is fixed when an instance is initialised and never changes (#9, #13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct InstanceConfig {
    pub chain: ChainSpec,
    /// Weak-subjectivity checkpoint: the beacon block root the instance bootstraps from.
    pub checkpoint_root: B256,
    /// Beyond this many epochs without a finalised header the instance stops advancing (#10).
    pub ws_period_epochs: u64,
}

impl InstanceConfig {
    /// Instance identifier a consumer binds to (#15). Distinct configurations never collide.
    pub fn id(&self) -> B256 {
        let mut h = Sha256::new();
        h.update(b"affidavit/instance/v1");
        h.update(self.chain.digest());
        h.update(self.checkpoint_root);
        h.update(self.ws_period_epochs.to_le_bytes());
        B256::from_slice(&h.finalize())
    }
}

/// The consensus state an update advances. Committees are kept as roots: the submitter
/// supplies the 512 keys with each update and the program checks them against the root,
/// so the instance account stays a few hundred bytes (Performance #7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct TrackedState {
    pub finalized: HeaderRecord,
    pub current_committee_root: B256,
    pub next_committee_root: Option<B256>,
    pub headers: MmrAccumulator,
    pub updates_applied: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct Instance {
    pub config: InstanceConfig,
    pub state: TrackedState,
}

/// The verified result of an update: what the public instance applies without repeating
/// any signature check. It names the state it was computed against, so applying it is a
/// compare-and-swap and two operators racing the same update cannot double-apply (#R8).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct Advance {
    pub instance_id: B256,
    pub prior_period: u64,
    pub prior_current_committee_root: B256,
    pub prior_next_committee_root: Option<B256>,
    pub new_finalized: Option<HeaderRecord>,
    pub new_current_committee_root: B256,
    pub new_next_committee_root: Option<B256>,
    pub participants: u32,
}

pub fn compute_domain(domain_type: [u8; 4], fork_version: [u8; 4], genesis_validators_root: &B256) -> B256 {
    let mut version = [0u8; 32];
    version[..4].copy_from_slice(&fork_version);
    let fork_data_root = hash_pair(&B256::from(version), genesis_validators_root);
    let mut domain = [0u8; 32];
    domain[..4].copy_from_slice(&domain_type);
    domain[4..].copy_from_slice(&fork_data_root[..28]);
    B256::from(domain)
}

/// `is_valid_light_client_header`: the execution payload header is bound to the beacon
/// block body by an SSZ branch, so a caller cannot substitute its own state or receipts root.
pub fn check_execution_binding(spec: &ChainSpec, header: &LightClientHeader) -> Result<ForkName> {
    let epoch = spec.epoch_at_slot(header.beacon.slot);
    let fork = spec.fork_at_epoch(epoch);
    if !(ForkName::Capella..=ForkName::Fulu).contains(&fork) {
        return Err(Error::UnsupportedFork { epoch });
    }
    if fork < ForkName::Deneb && (header.execution.blob_gas_used != 0 || header.execution.excess_blob_gas != 0) {
        return Err(Error::ExecutionNotBound);
    }
    let leaf = header.execution.hash_tree_root(fork);
    let g = EXECUTION_PAYLOAD_GINDEX;
    if !is_valid_merkle_branch(&leaf, &header.execution_branch, g.depth, g.index, &header.beacon.body_root) {
        return Err(Error::ExecutionNotBound);
    }
    Ok(fork)
}

fn check_not_stale(config: &InstanceConfig, finalized_slot: u64, now_slot: u64) -> Result<()> {
    let spec = &config.chain;
    let age_epochs = spec.epoch_at_slot(now_slot).saturating_sub(spec.epoch_at_slot(finalized_slot));
    if age_epochs > config.ws_period_epochs {
        return Err(Error::CheckpointStale { age_epochs, limit_epochs: config.ws_period_epochs });
    }
    Ok(())
}

pub fn required_participants(committee_size: usize) -> u32 {
    (2 * committee_size as u32).div_ceil(3)
}

fn check_participation(spec: &ChainSpec, aggregate: &SyncAggregate) -> Result<u32> {
    let size = spec.preset.sync_committee_size;
    if aggregate.sync_committee_bits.len() * 8 != size {
        return Err(Error::MalformedInput("sync_committee_bits length"));
    }
    let participants = aggregate.participants();
    let required = required_participants(size);
    if participants < required {
        return Err(Error::ParticipationBelowThreshold { participants, required });
    }
    Ok(participants)
}

/// Checks the committee's aggregate signature over the attested header.
pub fn verify_sync_aggregate(
    spec: &ChainSpec,
    committee: &SyncCommittee,
    aggregate: &SyncAggregate,
    attested_root: &B256,
    signature_slot: u64,
    bls: &impl BlsVerifier,
) -> Result<u32> {
    let participants = check_participation(spec, aggregate)?;
    if committee.pubkeys.len() != spec.preset.sync_committee_size {
        return Err(Error::MalformedInput("sync committee size"));
    }
    let signers: Vec<&BlsPublicKey> =
        committee.pubkeys.iter().enumerate().filter(|(i, _)| aggregate.is_set(*i)).map(|(_, pk)| pk).collect();
    let fork_version_slot = signature_slot.max(1) - 1;
    let version = spec.fork_version_at_epoch(spec.epoch_at_slot(fork_version_slot));
    let domain = compute_domain(DOMAIN_SYNC_COMMITTEE, version, &spec.genesis_validators_root);
    let signing_root = hash_pair(attested_root, &domain);
    if !bls.fast_aggregate_verify(&signers, &signing_root, &aggregate.sync_committee_signature) {
        return Err(Error::SyncCommitteeSignatureInvalid);
    }
    Ok(participants)
}

/// Initialises an instance from a bootstrap that must hash to the configured checkpoint.
pub fn initialize(config: InstanceConfig, bootstrap: &LightClientBootstrap, now_slot: u64) -> Result<Instance> {
    let spec = &config.chain;
    if bootstrap.header.beacon.hash_tree_root() != config.checkpoint_root {
        return Err(Error::CheckpointMismatch);
    }
    let fork = check_execution_binding(spec, &bootstrap.header)?;
    if bootstrap.current_sync_committee.pubkeys.len() != spec.preset.sync_committee_size {
        return Err(Error::MalformedInput("sync committee size"));
    }
    let committee_root = bootstrap.current_sync_committee.hash_tree_root();
    let g = current_sync_committee_gindex(fork);
    if !is_valid_merkle_branch(
        &committee_root,
        &bootstrap.current_sync_committee_branch,
        g.depth,
        g.index,
        &bootstrap.header.beacon.state_root,
    ) {
        return Err(Error::CheckpointMismatch);
    }
    check_not_stale(&config, bootstrap.header.beacon.slot, now_slot)?;
    let record = HeaderRecord::from_header(&bootstrap.header);
    let mut headers = MmrAccumulator::default();
    headers.append(record.leaf());
    Ok(Instance {
        config,
        state: TrackedState {
            finalized: record,
            current_committee_root: committee_root,
            next_committee_root: None,
            headers,
            updates_applied: 0,
        },
    })
}

/// Verifies an update against the instance without modifying it. `signing_committee` is
/// the full committee whose root the instance tracks for the update's signature period.
pub fn verify_update(
    instance: &Instance,
    update: &LightClientUpdate,
    signing_committee: &SyncCommittee,
    now_slot: u64,
    bls: &impl BlsVerifier,
) -> Result<Advance> {
    let config = &instance.config;
    let spec = &config.chain;
    let state = &instance.state;
    check_not_stale(config, state.finalized.slot, now_slot)?;

    // Cheap structural checks first, so junk is rejected before any hashing.
    check_participation(spec, &update.sync_aggregate)?;
    let finalized_header = update.finalized_header.as_ref().ok_or(Error::HeaderNotFinalized)?;
    let attested_slot = update.attested_header.beacon.slot;
    let finalized_slot = finalized_header.beacon.slot;
    if !(now_slot >= update.signature_slot && update.signature_slot > attested_slot && attested_slot >= finalized_slot)
    {
        return Err(Error::SlotOrdering);
    }

    // The update must be signed by a committee the instance has verified.
    let store_period = spec.period_at_slot(state.finalized.slot);
    let signature_period = spec.period_at_slot(update.signature_slot);
    let reachable = match state.next_committee_root {
        Some(_) => signature_period == store_period || signature_period == store_period + 1,
        None => signature_period == store_period,
    };
    if !reachable {
        return Err(Error::CommitteeHandoffNotVerified);
    }

    // It must advance something.
    let attested_period = spec.period_at_slot(attested_slot);
    let finalized_period = spec.period_at_slot(finalized_slot);
    let learns_next = state.next_committee_root.is_none()
        && update.next_sync_committee.is_some()
        && attested_period == store_period
        && finalized_period == attested_period;
    if !(finalized_slot > state.finalized.slot || learns_next) {
        return Err(Error::UpdateNotNewer);
    }

    // The attested header, its finalised header and both execution payloads are bound.
    let attested_fork = check_execution_binding(spec, &update.attested_header)?;
    if finalized_slot == 0 {
        return Err(Error::HeaderNotFinalized);
    }
    let g = finalized_root_gindex(attested_fork);
    if !is_valid_merkle_branch(
        &finalized_header.beacon.hash_tree_root(),
        &update.finality_branch,
        g.depth,
        g.index,
        &update.attested_header.beacon.state_root,
    ) {
        return Err(Error::HeaderNotFinalized);
    }
    check_execution_binding(spec, finalized_header)?;

    // A handoff is proven by the attested state.
    let next_root = match &update.next_sync_committee {
        Some(next) => {
            if next.pubkeys.len() != spec.preset.sync_committee_size {
                return Err(Error::MalformedInput("next sync committee size"));
            }
            let root = next.hash_tree_root();
            let g = next_sync_committee_gindex(attested_fork);
            if !is_valid_merkle_branch(
                &root,
                &update.next_sync_committee_branch,
                g.depth,
                g.index,
                &update.attested_header.beacon.state_root,
            ) {
                return Err(Error::CommitteeHandoffNotVerified);
            }
            if attested_period == store_period {
                if let Some(known) = state.next_committee_root {
                    if known != root {
                        return Err(Error::CommitteeMismatch);
                    }
                }
            }
            Some(root)
        }
        None => None,
    };

    // The signature, by the committee the instance tracks for that period.
    let expected_root = if signature_period == store_period {
        state.current_committee_root
    } else {
        state.next_committee_root.ok_or(Error::CommitteeHandoffNotVerified)?
    };
    if signing_committee.hash_tree_root() != expected_root {
        return Err(Error::CommitteeMismatch);
    }
    let participants = verify_sync_aggregate(
        spec,
        signing_committee,
        &update.sync_aggregate,
        &update.attested_header.beacon.hash_tree_root(),
        update.signature_slot,
        bls,
    )?;

    // `apply_light_client_update`.
    let (new_current, new_next) = match state.next_committee_root {
        None => {
            if finalized_period != store_period {
                return Err(Error::CommitteeHandoffNotVerified);
            }
            (state.current_committee_root, next_root)
        }
        Some(known_next) if finalized_period == store_period + 1 => (known_next, next_root),
        Some(known_next) => (state.current_committee_root, Some(known_next)),
    };
    let new_finalized = (finalized_slot > state.finalized.slot).then(|| HeaderRecord::from_header(finalized_header));

    Ok(Advance {
        instance_id: config.id(),
        prior_period: store_period,
        prior_current_committee_root: state.current_committee_root,
        prior_next_committee_root: state.next_committee_root,
        new_finalized,
        new_current_committee_root: new_current,
        new_next_committee_root: new_next,
        participants,
    })
}

/// Applies a verified advance if the instance is still in the state it was computed
/// against. Anything else is a duplicate or a lost race, and changes nothing (#R8).
pub fn apply_advance(instance: &mut Instance, advance: &Advance) -> Result<()> {
    if advance.instance_id != instance.config.id() {
        return Err(Error::ChainMismatch);
    }
    let state = &mut instance.state;
    let current_period = instance.config.chain.period_at_slot(state.finalized.slot);
    if advance.prior_period != current_period
        || advance.prior_current_committee_root != state.current_committee_root
        || advance.prior_next_committee_root != state.next_committee_root
    {
        return Err(Error::UpdateNotNewer);
    }
    let advances_finality = advance.new_finalized.is_some_and(|r| r.slot > state.finalized.slot);
    let changes_committees = advance.new_current_committee_root != state.current_committee_root
        || advance.new_next_committee_root != state.next_committee_root;
    if !advances_finality && !changes_committees {
        return Err(Error::UpdateNotNewer);
    }
    if advances_finality {
        let record = advance.new_finalized.expect("checked above");
        state.finalized = record;
        state.headers.append(record.leaf());
    }
    state.current_committee_root = advance.new_current_committee_root;
    state.next_committee_root = advance.new_next_committee_root;
    state.updates_applied += 1;
    Ok(())
}

/// `verify_update` then `apply_advance`, for callers that hold the instance themselves.
pub fn process_update(
    instance: &mut Instance,
    update: &LightClientUpdate,
    signing_committee: &SyncCommittee,
    now_slot: u64,
    bls: &impl BlsVerifier,
) -> Result<Advance> {
    let advance = verify_update(instance, update, signing_committee, now_slot, bls)?;
    apply_advance(instance, &advance)?;
    Ok(advance)
}
