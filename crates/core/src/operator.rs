//! Submission timing for the update-submitting component (Reliability #9, #10).
//!
//! Every operator waits a deferral before acting on a new update, then checks whether the
//! instance already moved, and only then spends gas (or proving time). The deferral is a
//! hash of the operator's own key and the update, so it is uniform over the window,
//! different for every update, and the same on every restart of the same operator, which
//! rotates who goes first instead of always rewarding the lowest latency. Nobody
//! coordinates and nobody can be excluded: an operator that wants to go first on every
//! update can simply use a zero deferral, and the protocol does not care (#11).

use alloy_primitives::B256;
use sha2::{Digest, Sha256};

/// Milliseconds to wait before checking and submitting `update_digest`.
pub fn deferral_ms(operator: &B256, update_digest: &B256, window_ms: u64) -> u64 {
    if window_ms == 0 {
        return 0;
    }
    let mut h = Sha256::new();
    h.update(b"affidavit/deferral/v1");
    h.update(operator);
    h.update(update_digest);
    let digest = h.finalize();
    let mut word = [0u8; 8];
    word.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(word) % window_ms
}

/// What an operator does once its deferral has elapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The instance already holds this update (or a newer one): spend nothing.
    Skip,
    /// Still needed: build and submit it.
    Submit,
}

/// `instance_finalized_slot` is read immediately before deciding (#R9).
pub fn decide(instance_finalized_slot: u64, update_finalized_slot: u64, teaches_next_committee: bool) -> Decision {
    if update_finalized_slot > instance_finalized_slot || teaches_next_committee {
        Decision::Submit
    } else {
        Decision::Skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferral_is_spread_and_stable() {
        let update = B256::repeat_byte(3);
        let window = 300_000;
        let values: alloc::vec::Vec<u64> =
            (0u8..64).map(|i| deferral_ms(&B256::repeat_byte(i), &update, window)).collect();
        assert!(values.iter().all(|v| *v < window));
        // Same inputs, same deferral: a restarted operator keeps its place.
        assert_eq!(deferral_ms(&B256::repeat_byte(1), &update, window), values[1]);
        // Spread across the window rather than clustered.
        let early = values.iter().filter(|v| **v < window / 2).count();
        assert!((20..=44).contains(&early), "{early} of 64 in the first half");
    }
}
