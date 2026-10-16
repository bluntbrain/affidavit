//! Runs affidavit-core's verification paths inside the RISC Zero zkVM and reports cycle
//! counts per phase. The code under measurement is the same crate the native tests run.

#![no_main]

use std::io::Read;

use affidavit_core::bls::{BlsVerifier, UncompressedPublicKey, Zkcrypto, DST};
use affidavit_core::lc::{self, compute_domain, DOMAIN_SYNC_COMMITTEE};
use affidavit_core::ssz::hash_pair;
use affidavit_core::statement::attest_with_membership;
use affidavit_core::types::BlsPublicKey;
use affidavit_zkvm_jobs::{Job, Report};
use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{multi_miller_loop, G1Affine, G1Projective, G2Affine, G2Prepared, G2Projective, Gt};
use risc0_zkvm::guest::env;

/// The committed key behind a witness, checked the way the program checks it.
fn witnessed(uncompressed: &UncompressedPublicKey, compressed: &BlsPublicKey, check_subgroups: bool) -> G1Affine {
    let p: G1Affine = Option::from(G1Affine::from_uncompressed_unchecked(uncompressed)).expect("point");
    assert!(bool::from(p.is_on_curve()) && !bool::from(p.is_identity()) && p.to_compressed() == *compressed);
    if check_subgroups {
        assert!(bool::from(p.is_torsion_free()));
    }
    p
}

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
        Job::VerifyUpdate { instance, update, committee, witness, now_slot, check_subgroups } => {
            // End to end, exactly as the program runs it.
            #[cfg(not(feature = "blst"))]
            let (bls, backend) = (Zkcrypto { check_subgroups }, "zkcrypto");
            #[cfg(feature = "blst")]
            let (bls, backend) = (affidavit_core::bls::Blst, "blst");
            let result =
                lc::verify_update_with_witness(&instance, &update, &committee, witness.as_ref(), now_slot, &bls);
            t.mark(&format!("verify_update, total ({backend})"));

            // The same work again, in parts, with the zkcrypto primitives.
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
            let signers: Vec<usize> =
                (0..committee.pubkeys.len()).filter(|i| update.sync_aggregate.is_set(*i)).collect();
            let non_signers: Vec<usize> =
                (0..committee.pubkeys.len()).filter(|i| !update.sync_aggregate.is_set(*i)).collect();
            t.mark("  signing root and signer set");
            let aggregate = match &witness {
                Some(witness) => {
                    let total = witnessed(&witness.aggregate, &committee.aggregate_pubkey, check_subgroups);
                    let removed: Vec<G1Affine> = non_signers
                        .iter()
                        .zip(&witness.non_signers)
                        .map(|(&i, point)| witnessed(point, &committee.pubkeys[i], check_subgroups))
                        .collect();
                    t.mark(&format!("  keys: aggregate + {} non-signer witnesses checked", removed.len()));
                    let mut signers_key = G1Projective::from(total);
                    for p in &removed {
                        signers_key -= p;
                    }
                    let signers_key = G1Affine::from(signers_key);
                    t.mark("  signers' key: aggregate minus non-signers");
                    signers_key
                }
                None => {
                    let keys: Vec<G1Affine> = signers
                        .iter()
                        .map(|&i| {
                            let pk = &committee.pubkeys[i];
                            let p: Option<G1Affine> = if check_subgroups {
                                G1Affine::from_compressed(pk).into()
                            } else {
                                G1Affine::from_compressed_unchecked(pk).into()
                            };
                            p.expect("valid key")
                        })
                        .collect();
                    t.mark(if check_subgroups { "  keys: decompress + subgroup check" } else { "  keys: decompress" });
                    let mut signers_key = G1Projective::identity();
                    for k in &keys {
                        signers_key += k;
                    }
                    let signers_key = G1Affine::from(signers_key);
                    t.mark("  signers' key: add up the signers");
                    signers_key
                }
            };
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
            let compressed: Vec<&BlsPublicKey> = signers.iter().map(|&i| &committee.pubkeys[i]).collect();
            assert_eq!(
                ok,
                Zkcrypto { check_subgroups: false }.fast_aggregate_verify(
                    &compressed,
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
