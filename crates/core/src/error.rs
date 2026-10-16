//! Typed errors. Every rejection the module can produce has a stable code, so the
//! off-chain dry run and the on-chain path report the same thing (Usability #2, #5).

use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Input does not decode, or a field has the wrong shape.
    MalformedInput(&'static str),
    /// The material belongs to a different network than the instance is bound to.
    ChainMismatch,
    /// The header is from a fork whose light-client containers this version does not support.
    UnsupportedFork { epoch: u64 },
    /// The statement was produced under a different instance than the consumer is bound to.
    InstanceMismatch,
    /// The message is only accepted from this program's own actors.
    Unauthorized,
    /// Fewer than two thirds of the sync committee signed.
    ParticipationBelowThreshold { participants: u32, required: u32 },
    /// The aggregate BLS signature does not verify against the tracked committee.
    SyncCommitteeSignatureInvalid,
    /// The update is signed by, or hands off to, a committee the instance has not verified.
    CommitteeHandoffNotVerified,
    /// The supplied committee does not hash to the committee the instance tracks.
    CommitteeMismatch,
    /// The header is not proven finalised by the attested state (missing or invalid finality branch).
    HeaderNotFinalized,
    /// The execution payload header is not bound to the beacon block body (tampered state or receipts root).
    ExecutionNotBound,
    /// Slots are out of order (signature after attested after finalised) or the update is from the future.
    SlotOrdering,
    /// The update does not advance the tracked state (duplicate or already applied).
    UpdateNotNewer,
    /// The instance's newest finalised header is older than the weak-subjectivity period.
    CheckpointStale { age_epochs: u64, limit_epochs: u64 },
    /// The bootstrap does not match the configured checkpoint root.
    CheckpointMismatch,
    /// The attestation refers to a header this instance never verified.
    HeaderUnknown,
    /// A Merkle-Patricia or SSZ inclusion proof does not verify.
    InclusionProofInvalid,
    /// The predicate does not hold over the attested state.
    PredicateNotSatisfied { clause: u32 },
    /// The predicate refers to a value the proof does not cover.
    StateNotCovered,
    /// The transaction is included but its receipt reports failure.
    TransactionReverted,
    /// The transaction is signed for a different chain id than the instance.
    TransactionChainMismatch,
}

impl Error {
    /// Stable numeric code. 1xx input and configuration, 2xx consensus, 3xx inclusion and predicate.
    pub const fn code(&self) -> u16 {
        match self {
            Error::MalformedInput(_) => 100,
            Error::ChainMismatch => 101,
            Error::UnsupportedFork { .. } => 102,
            Error::InstanceMismatch => 103,
            Error::Unauthorized => 104,
            Error::ParticipationBelowThreshold { .. } => 200,
            Error::SyncCommitteeSignatureInvalid => 201,
            Error::CommitteeHandoffNotVerified => 202,
            Error::CommitteeMismatch => 203,
            Error::HeaderNotFinalized => 204,
            Error::ExecutionNotBound => 205,
            Error::SlotOrdering => 206,
            Error::UpdateNotNewer => 207,
            Error::CheckpointStale { .. } => 208,
            Error::CheckpointMismatch => 209,
            Error::HeaderUnknown => 210,
            Error::InclusionProofInvalid => 300,
            Error::PredicateNotSatisfied { .. } => 301,
            Error::StateNotCovered => 302,
            Error::TransactionReverted => 303,
            Error::TransactionChainMismatch => 304,
        }
    }

    /// Short machine-friendly name, used by the CLI and in logs.
    pub const fn name(&self) -> &'static str {
        match self {
            Error::MalformedInput(_) => "malformed_input",
            Error::ChainMismatch => "chain_mismatch",
            Error::UnsupportedFork { .. } => "unsupported_fork",
            Error::InstanceMismatch => "instance_mismatch",
            Error::Unauthorized => "unauthorized",
            Error::ParticipationBelowThreshold { .. } => "participation_below_threshold",
            Error::SyncCommitteeSignatureInvalid => "sync_committee_signature_invalid",
            Error::CommitteeHandoffNotVerified => "committee_handoff_not_verified",
            Error::CommitteeMismatch => "committee_mismatch",
            Error::HeaderNotFinalized => "header_not_finalized",
            Error::ExecutionNotBound => "execution_not_bound",
            Error::SlotOrdering => "slot_ordering",
            Error::UpdateNotNewer => "update_not_newer",
            Error::CheckpointStale { .. } => "checkpoint_stale",
            Error::CheckpointMismatch => "checkpoint_mismatch",
            Error::HeaderUnknown => "header_unknown",
            Error::InclusionProofInvalid => "inclusion_proof_invalid",
            Error::PredicateNotSatisfied { .. } => "predicate_not_satisfied",
            Error::StateNotCovered => "state_not_covered",
            Error::TransactionReverted => "transaction_reverted",
            Error::TransactionChainMismatch => "transaction_chain_mismatch",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AFF-{} {}: ", self.code(), self.name())?;
        match self {
            Error::MalformedInput(what) => write!(f, "input does not decode ({what})"),
            Error::ChainMismatch => f.write_str(
                "material is for a different network; check the instance's chain id and genesis root",
            ),
            Error::UnsupportedFork { epoch } => write!(
                f,
                "epoch {epoch} is in a fork this program version does not support; use the program version released for that fork"
            ),
            Error::InstanceMismatch => f.write_str(
                "statement comes from a different instance than the one this consumer is bound to",
            ),
            Error::Unauthorized => {
                f.write_str("only this program's own actors may send this message; submit a Verify instead")
            }
            Error::ParticipationBelowThreshold { participants, required } => write!(
                f,
                "{participants} sync-committee signatures, {required} required; wait for a better-signed update"
            ),
            Error::SyncCommitteeSignatureInvalid => {
                f.write_str("aggregate signature does not verify against the tracked committee")
            }
            Error::CommitteeHandoffNotVerified => f.write_str(
                "update is not reachable from the tracked committee; submit the missing period updates first",
            ),
            Error::CommitteeMismatch => {
                f.write_str("supplied committee does not match the committee the instance tracks")
            }
            Error::HeaderNotFinalized => {
                f.write_str("header is not proven finalised; wait for finality or supply the finality branch")
            }
            Error::ExecutionNotBound => f.write_str(
                "execution payload header is not bound to the beacon block (state, receipts or transactions root tampered)",
            ),
            Error::SlotOrdering => f.write_str("slots out of order or update from the future"),
            Error::UpdateNotNewer => {
                f.write_str("update does not advance the instance (already applied by someone else?)")
            }
            Error::CheckpointStale { age_epochs, limit_epochs } => write!(
                f,
                "newest finalised header is {age_epochs} epochs old, weak-subjectivity limit is {limit_epochs}; migrate to a freshly initialised instance"
            ),
            Error::CheckpointMismatch => {
                f.write_str("bootstrap does not match the configured checkpoint block root")
            }
            Error::HeaderUnknown => {
                f.write_str("header is not in this instance's verified set; advance the instance or pick another header")
            }
            Error::InclusionProofInvalid => f.write_str("inclusion proof does not verify against the finalised root"),
            Error::PredicateNotSatisfied { clause } => {
                write!(f, "predicate clause {clause} does not hold over the attested state")
            }
            Error::StateNotCovered => f.write_str("predicate refers to a value the proof does not cover"),
            Error::TransactionReverted => f.write_str("transaction is included but reverted"),
            Error::TransactionChainMismatch => {
                f.write_str("transaction is signed for a different chain id than the instance")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;
