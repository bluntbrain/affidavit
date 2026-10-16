//! BLS12-381 aggregate verification behind a narrow trait, so the light-client logic is
//! identical whichever audited library runs underneath: supranational `blst` natively and
//! in the zkVM (through RISC Zero's accelerated fork), or zkcrypto `bls12_381`.

use crate::types::{BlsPublicKey, BlsSignature};
use alloy_primitives::B256;

/// Ciphersuite of Ethereum consensus signatures (proof-of-possession scheme, G2 signatures).
pub const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

pub trait BlsVerifier {
    /// `FastAggregateVerify(pubkeys, message, signature)` from the IETF BLS draft, as the
    /// consensus specs use it for sync-committee signatures.
    fn fast_aggregate_verify(&self, pubkeys: &[&BlsPublicKey], message: &B256, signature: &BlsSignature) -> bool;
}

#[cfg(feature = "blst")]
pub use self::blst_backend::Blst;

#[cfg(feature = "zkcrypto")]
pub use self::zkcrypto_backend::Zkcrypto;

#[cfg(feature = "zkcrypto")]
mod zkcrypto_backend {
    use super::*;
    use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
    use bls12_381::{multi_miller_loop, G1Affine, G1Projective, G2Affine, G2Prepared, G2Projective, Gt};

    /// zkcrypto `bls12_381`, the library r0vm-helios and SP1 Helios verify sync-committee
    /// signatures with. `check_subgroups: false` skips the per-key subgroup check: every
    /// key is bound through the committee root to a beacon state that admitted it only
    /// after verifying its proof of possession (see `docs/DECISIONS.md`).
    #[derive(Debug, Clone, Copy)]
    pub struct Zkcrypto {
        pub check_subgroups: bool,
    }

    impl Default for Zkcrypto {
        fn default() -> Self {
            Self { check_subgroups: true }
        }
    }

    impl BlsVerifier for Zkcrypto {
        fn fast_aggregate_verify(&self, pubkeys: &[&BlsPublicKey], message: &B256, signature: &BlsSignature) -> bool {
            if pubkeys.is_empty() {
                return false;
            }
            let mut aggregate = G1Projective::identity();
            for pk in pubkeys {
                let point = if self.check_subgroups {
                    G1Affine::from_compressed(pk)
                } else {
                    G1Affine::from_compressed_unchecked(pk)
                };
                let point: Option<G1Affine> = point.into();
                match point {
                    Some(p) if !bool::from(p.is_identity()) => aggregate += p,
                    _ => return false,
                }
            }
            let sig: Option<G2Affine> = G2Affine::from_compressed(signature).into();
            let Some(sig) = sig else {
                return false;
            };
            let hashed =
                <G2Projective as HashToCurve<ExpandMsgXmd<sha2_09::Sha256>>>::hash_to_curve(message.as_slice(), DST);
            let lhs = G1Affine::from(aggregate);
            let neg_g1 = -G1Affine::generator();
            let result = multi_miller_loop(&[
                (&lhs, &G2Prepared::from(G2Affine::from(hashed))),
                (&neg_g1, &G2Prepared::from(sig)),
            ])
            .final_exponentiation();
            result == Gt::identity()
        }
    }
}

#[cfg(feature = "blst")]
mod blst_backend {
    use super::*;
    use alloc::vec::Vec;
    use blst::min_pk::{PublicKey, Signature};
    use blst::BLST_ERROR;

    /// supranational `blst`: audited by NCC Group, the library Ethereum consensus clients sign with.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct Blst;

    impl BlsVerifier for Blst {
        fn fast_aggregate_verify(&self, pubkeys: &[&BlsPublicKey], message: &B256, signature: &BlsSignature) -> bool {
            if pubkeys.is_empty() {
                return false;
            }
            let mut keys = Vec::with_capacity(pubkeys.len());
            for pk in pubkeys {
                // Decompress, reject infinity, check subgroup membership.
                match PublicKey::key_validate(pk.as_slice()) {
                    Ok(key) => keys.push(key),
                    Err(_) => return false,
                }
            }
            let refs: Vec<&PublicKey> = keys.iter().collect();
            let Ok(sig) = Signature::sig_validate(signature.as_slice(), true) else {
                return false;
            };
            sig.fast_aggregate_verify(true, message.as_slice(), DST, &refs) == BLST_ERROR::BLST_SUCCESS
        }
    }
}
