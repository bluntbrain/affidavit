//! Runs affidavit-core's verification paths inside the RISC Zero zkVM and reports cycle
//! counts per phase. The code under measurement is the same crate the native tests run.

#![no_main]

use std::io::Read;

use affidavit_core::bls::{BlsVerifier, Zkcrypto, DST};
use affidavit_core::lc::{self, compute_domain, DOMAIN_SYNC_COMMITTEE};
use affidavit_core::ssz::hash_pair;
use affidavit_core::statement::attest_with_membership;
use affidavit_core::types::BlsPublicKey;
use affidavit_zkvm_jobs::{Job, Report};
use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{multi_miller_loop, G1Affine, G1Projective, G2Affine, G2Prepared, G2Projective, Gt};
use risc0_zkvm::guest::env;

risc0_zkvm::guest::entry!(main);

struct Timer {
    last: u64,
    phases: Vec<(String, u64)>,
}

impl Timer {
    fn new() -> Self {
        Self { last: env::cycle_count(), phases: Vec::new() }
    }
    fn mark(&mut self, name: &str) {
        let now = env::cycle_count();
        self.phases.push((name.to_string(), now - self.last));
        self.last = env::cycle_count();
    }
}

fn main() {
    let mut input = Vec::new();
    env::stdin().read_to_end(&mut input).expect("stdin");
    let mut t = Timer::new();
    let job: Job = borsh::from_slice(&input).expect("job decodes");
    t.mark("decode input");

    let verdict = match job {
        Job::VerifyUpdate { instance, update, committee, now_slot, check_subgroups } => {
            // End to end, exactly as the program would run it.
            #[cfg(not(feature = "blst"))]
            let result = lc::verify_update(&instance, &update, &committee, now_slot, &Zkcrypto { check_subgroups });
            #[cfg(feature = "blst")]
            let result = lc::verify_update(&instance, &update, &committee, now_slot, &affidavit_core::bls::Blst);
            t.mark(if cfg!(feature = "blst") { "verify_update (total, blst)" } else { "verify_update (total, zkcrypto)" });

            // The same work again, broken into its parts.
            let spec = &instance.config.chain;
            let _ = committee.hash_tree_root();
            t.mark("  committee root (512 keys, SSZ)");
            let _ = lc::check_execution_binding(spec, &update.attested_header);
            if let Some(fin) = &update.finalized_header {
                let _ = lc::check_execution_binding(spec, fin);
                let _ = fin.beacon.hash_tree_root();
            }
            if let Some(next) = &update.next_sync_committee {
                let _ = next.hash_tree_root();
            }
            t.mark("  execution, finality and handoff branches");
            let attested_root = update.attested_header.beacon.hash_tree_root();
            let version = spec.fork_version_at_epoch(spec.epoch_at_slot(update.signature_slot.max(1) - 1));
            let message = hash_pair(
                &attested_root,
                &compute_domain(DOMAIN_SYNC_COMMITTEE, version, &spec.genesis_validators_root),
            );
            let signers: Vec<&BlsPublicKey> = committee
                .pubkeys
                .iter()
                .enumerate()
                .filter(|(i, _)| update.sync_aggregate.is_set(*i))
                .map(|(_, pk)| pk)
                .collect();
            t.mark("  signing root and signer set");
            let mut aggregate = G1Projective::identity();
            for pk in &signers {
                let p: Option<G1Affine> = if check_subgroups {
                    G1Affine::from_compressed(pk).into()
                } else {
                    G1Affine::from_compressed_unchecked(pk).into()
                };
                aggregate += p.expect("valid key");
            }
            let aggregate = G1Affine::from(aggregate);
            t.mark(if check_subgroups {
                "  keys: decompress + subgroup check + add"
            } else {
                "  keys: decompress + add"
            });
            let hashed =
                <G2Projective as HashToCurve<ExpandMsgXmd<sha2_09::Sha256>>>::hash_to_curve(message.as_slice(), DST);
            let hashed = G2Affine::from(hashed);
            t.mark("  hash to G2");
            let sig: G2Affine =
                Option::from(G2Affine::from_compressed(&update.sync_aggregate.sync_committee_signature)).expect("sig");
            t.mark("  signature decode + subgroup check");
            let neg_g1 = -G1Affine::generator();
            let miller =
                multi_miller_loop(&[(&aggregate, &G2Prepared::from(hashed)), (&neg_g1, &G2Prepared::from(sig))]);
            t.mark("  Miller loops (2)");
            let ok = miller.final_exponentiation() == Gt::identity();
            t.mark("  final exponentiation");
            assert_eq!(
                ok,
                Zkcrypto { check_subgroups }.fast_aggregate_verify(
                    &signers,
                    &message,
                    &update.sync_aggregate.sync_committee_signature
                )
            );
            result.err().map_or(0, |e| e.code())
        }
        Job::ApplyAdvance { mut instance, advance } => {
            let result = lc::apply_advance(&mut instance, &advance);
            t.mark("apply_advance (public instance actor)");
            result.err().map_or(0, |e| e.code())
        }
        Job::Attest { chain, instance_id, headers_root, header, membership, evidence, predicate } => {
            let result =
                attest_with_membership(&chain, instance_id, &headers_root, &header, &membership, &evidence, &predicate);
            t.mark("attest_with_membership (total)");
            result.err().map_or(0, |e| e.code())
        }
    };
    env::commit_slice(&borsh::to_vec(&Report { verdict, phases: t.phases }).expect("report encodes"));
}
