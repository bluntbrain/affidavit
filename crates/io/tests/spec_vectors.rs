//! Official consensus-spec-tests `light_client/sync` vectors (minimal preset), Deneb and
//! Electra. Electra moved the finality and committee generalised indices, and Electra
//! containers are what mainnet and Hoodi use today (Fulu kept them unchanged).
//!
//! The instance is stricter than the spec's reference store: it only applies supermajority
//! finality updates and never force-updates. So for every step the check is one-sided:
//! whenever the instance accepts an update, its finalised header must be exactly the one
//! the reference store holds after that step.

mod common;

use std::path::PathBuf;

use affidavit_core::bls::{Blst, Zkcrypto};
use affidavit_core::config::ForkName;
use affidavit_core::lc::{self, InstanceConfig};
use affidavit_io::spec_tests::load_sync_test;
use common::Follower;

fn run(fork_dir: &str, case: &str, fork: ForkName) -> (usize, usize) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/spec-tests").join(fork_dir).join(case);
    let test = load_sync_test(&dir, fork).unwrap();
    let config = InstanceConfig {
        chain: test.spec.clone(),
        checkpoint_root: test.trusted_block_root,
        ws_period_epochs: u64::MAX / 64,
    };
    let mut follower = Follower::new(config, &test.bootstrap, test.steps[0].current_slot).unwrap();
    let (mut accepted, mut refused) = (0, 0);
    for step in &test.steps {
        if step.after_force_update {
            // From here the reference store holds a header it forced, not one it saw
            // finalised. The instance deliberately never does that, so stop comparing.
            break;
        }
        let committee = follower.signing_committee(&step.update);
        let verdict = lc::verify_update(&follower.instance, &step.update, &committee, step.current_slot, &Blst);
        // The program's path (aggregate minus witnessed non-signers) reaches the same verdict.
        let witness = Follower::committee_witness(&committee, &step.update.sync_aggregate);
        for (backend, program_path) in [
            (
                "blst",
                lc::verify_update_with_witness(
                    &follower.instance,
                    &step.update,
                    &committee,
                    Some(&witness),
                    step.current_slot,
                    &Blst,
                ),
            ),
            (
                "zkcrypto",
                lc::verify_update_with_witness(
                    &follower.instance,
                    &step.update,
                    &committee,
                    Some(&witness),
                    step.current_slot,
                    &Zkcrypto { check_subgroups: false },
                ),
            ),
        ] {
            assert_eq!(program_path, verdict, "{fork_dir}/{case}: committee witness path ({backend})");
        }
        match verdict {
            Ok(advance) => {
                lc::apply_advance(&mut follower.instance, &advance).unwrap();
                if let Some(next) = &step.update.next_sync_committee {
                    follower.committees.insert(next.hash_tree_root(), next.clone());
                }
                let finalized = follower.instance.state.finalized;
                assert_eq!(
                    finalized.slot, step.expected_finalized_slot,
                    "{fork_dir}/{case}: slot after accepted update"
                );
                assert_eq!(
                    finalized.beacon_root, step.expected_finalized_root,
                    "{fork_dir}/{case}: root after accepted update"
                );
                accepted += 1;
            }
            Err(e) => {
                // Refusals must be the documented strictness, never a verification failure
                // on material the reference store accepted as valid.
                assert!(
                    matches!(
                        e,
                        affidavit_core::Error::HeaderNotFinalized
                            | affidavit_core::Error::UpdateNotNewer
                            | affidavit_core::Error::ParticipationBelowThreshold { .. }
                    ),
                    "{fork_dir}/{case}: unexpected refusal {e}"
                );
                refused += 1;
            }
        }
    }
    (accepted, refused)
}

const CASES: [&str; 3] =
    ["light_client_sync", "advance_finality_without_sync_committee", "supply_sync_committee_from_past_update"];

#[test]
fn deneb_vectors() {
    for case in CASES {
        let (accepted, refused) = run("deneb", case, ForkName::Deneb);
        println!("deneb/{case}: {accepted} accepted, {refused} refused by policy");
        assert!(accepted >= 1, "deneb/{case}");
    }
}

#[test]
fn electra_vectors() {
    for case in CASES {
        let (accepted, refused) = run("electra", case, ForkName::Electra);
        println!("electra/{case}: {accepted} accepted, {refused} refused by policy");
        assert!(accepted >= 1, "electra/{case}");
    }
}
