//! BLS12-381 aggregate verification behind a narrow trait, so the light-client logic is
//! identical whichever audited library runs underneath: supranational `blst` natively and
//! in the zkVM (through RISC Zero's accelerated fork), or zkcrypto `bls12_381`.

use crate::types::{BlsPublicKey, BlsSignature};
use alloy_primitives::B256;

/// Ciphersuite of Ethereum consensus signatures (proof-of-possession scheme, G2 signatures).
pub const DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// A key as an affine point, `x ‖ y` big-endian (the ZCash serialisation both libraries
/// read). Supplying it saves the verifier a square root.
pub type UncompressedPublicKey = [u8; 96];

/// A committed key and the point a submitter claims it encodes.
pub type WitnessedKey<'a> = (&'a BlsPublicKey, &'a UncompressedPublicKey);

pub trait BlsVerifier {
    /// `FastAggregateVerify(pubkeys, message, signature)` from the IETF BLS draft, as the
    /// consensus specs use it for sync-committee signatures: every key is decompressed and
    /// validated, then the keys are added up.
    fn fast_aggregate_verify(&self, pubkeys: &[&BlsPublicKey], message: &B256, signature: &BlsSignature) -> bool;

    /// The same check for a committee that commits to its own aggregate key, as a sync
    /// committee does: the signers' key is `aggregate − Σ non_signers`. Every point is an
    /// uncompressed witness that must be on the curve, not the identity, and compress to
    /// exactly its committed bytes, so it is the committed key. What is skipped is
    /// decompression, the subgroup check and adding up the signers (`docs/DECISIONS.md` D3).
    fn verify_aggregate_minus(
        &self,
        aggregate: WitnessedKey<'_>,
        non_signers: &[WitnessedKey<'_>],
        message: &B256,
        signature: &BlsSignature,
    ) -> bool;
}

#[cfg(feature = "blst")]
pub use self::blst_backend::{uncompress, Blst};

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
    /// after verifying its proof of possession (see `docs/DECISIONS.md` D3).
    #[derive(Debug, Clone, Copy)]
    pub struct Zkcrypto {
        pub check_subgroups: bool,
    }

    impl Default for Zkcrypto {
        fn default() -> Self {
            Self { check_subgroups: true }
        }
    }

    fn pairing_check(aggregate: G1Projective, message: &B256, signature: &BlsSignature) -> bool {
        let sig: Option<G2Affine> = G2Affine::from_compressed(signature).into();
        let Some(sig) = sig else {
            return false;
        };
        let hashed =
            <G2Projective as HashToCurve<ExpandMsgXmd<sha2_09::Sha256>>>::hash_to_curve(message.as_slice(), DST);
        let lhs = G1Affine::from(aggregate);
        let neg_g1 = -G1Affine::generator();
        let result =
            multi_miller_loop(&[(&lhs, &G2Prepared::from(G2Affine::from(hashed))), (&neg_g1, &G2Prepared::from(sig))])
                .final_exponentiation();
        result == Gt::identity()
    }

    /// The committed key, if the witness is exactly the point it encodes.
    fn witnessed((compressed, uncompressed): WitnessedKey<'_>, check_subgroups: bool) -> Option<G1Affine> {
        let point: Option<G1Affine> = G1Affine::from_uncompressed_unchecked(uncompressed).into();
        let p = point?;
        if !bool::from(p.is_on_curve()) || bool::from(p.is_identity()) || p.to_compressed() != *compressed {
            return None;
        }
        if check_subgroups && !bool::from(p.is_torsion_free()) {
            return None;
        }
        Some(p)
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
            pairing_check(aggregate, message, signature)
        }

        fn verify_aggregate_minus(
            &self,
            aggregate: WitnessedKey<'_>,
            non_signers: &[WitnessedKey<'_>],
            message: &B256,
            signature: &BlsSignature,
        ) -> bool {
            let Some(total) = witnessed(aggregate, self.check_subgroups) else {
                return false;
            };
            let mut signers = G1Projective::from(total);
            for key in non_signers {
                let Some(p) = witnessed(*key, self.check_subgroups) else {
                    return false;
                };
                signers -= p;
            }
            // Nobody left would verify only a signature of the identity.
            if bool::from(signers.is_identity()) {
                return false;
            }
            pairing_check(signers, message, signature)
        }
    }
}

#[cfg(feature = "blst")]
mod blst_backend {
    use super::*;
    use alloc::vec::Vec;
    use blst::min_pk::{AggregatePublicKey, PublicKey, Signature};
    use blst::BLST_ERROR;

    /// supranational `blst`: audited by NCC Group, the library Ethereum consensus clients sign with.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct Blst;

    /// Decompresses a committee key into the witness form the program consumes. Off-chain only.
    pub fn uncompress(pk: &BlsPublicKey) -> Option<UncompressedPublicKey> {
        PublicKey::uncompress(pk).ok().map(|key| key.serialize())
    }

    /// The base-field modulus of BLS12-381, big-endian.
    const P: [u8; 48] = [
        0x1a, 0x01, 0x11, 0xea, 0x39, 0x7f, 0xe6, 0x9a, 0x4b, 0x1b, 0xa7, 0xb6, 0x43, 0x4b, 0xac, 0xd7, 0x64, 0x77,
        0x4b, 0x84, 0xf3, 0x85, 0x12, 0xbf, 0x67, 0x30, 0xd2, 0xa0, 0xf6, 0xb0, 0xf6, 0x24, 0x1e, 0xab, 0xff, 0xfe,
        0xb1, 0x53, 0xff, 0xff, 0xb9, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xaa, 0xab,
    ];

    /// `−(x, y) = (x, p − y)` on the encoding of a witnessed point: `y` is below `p`
    /// (`deserialize` refuses anything else) and never zero (no point of the curve has
    /// order two), so `p − y` is again a canonical coordinate.
    fn negate(point: &UncompressedPublicKey) -> UncompressedPublicKey {
        let mut out = *point;
        let mut borrow = 0u16;
        for i in (0..48).rev() {
            let difference = 256 + u16::from(P[i]) - u16::from(point[48 + i]) - borrow;
            out[48 + i] = difference as u8;
            borrow = u16::from(difference < 256);
        }
        out
    }

    /// The committed key, if the witness is exactly the point it encodes. `deserialize`
    /// refuses non-canonical coordinates and points off the curve; the infinity flag (0x40)
    /// must be clear, which no committed key has set.
    fn witnessed((compressed, uncompressed): WitnessedKey<'_>) -> Option<PublicKey> {
        let pk = PublicKey::deserialize(uncompressed.as_slice()).ok()?;
        let recompressed = pk.compress();
        (recompressed == *compressed && recompressed[0] & 0x40 == 0).then_some(pk)
    }

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

        fn verify_aggregate_minus(
            &self,
            aggregate: WitnessedKey<'_>,
            non_signers: &[WitnessedKey<'_>],
            message: &B256,
            signature: &BlsSignature,
        ) -> bool {
            let Some(total) = witnessed(aggregate) else {
                return false;
            };
            let mut terms = Vec::with_capacity(non_signers.len() + 1);
            terms.push(total);
            for key in non_signers {
                if witnessed(*key).is_none() {
                    return false;
                }
                let Ok(negated) = PublicKey::deserialize(&negate(key.1)) else {
                    return false;
                };
                terms.push(negated);
            }
            let refs: Vec<&PublicKey> = terms.iter().collect();
            let Ok(signers) = AggregatePublicKey::aggregate(&refs, false) else {
                return false;
            };
            let signers = signers.to_public_key();
            // Nobody left would verify only a signature of the identity.
            if signers.compress()[0] & 0x40 != 0 {
                return false;
            }
            let Ok(sig) = Signature::sig_validate(signature.as_slice(), true) else {
                return false;
            };
            sig.verify(true, message.as_slice(), DST, &[], &signers, false) == BLST_ERROR::BLST_SUCCESS
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn negation_cancels() {
            let key = blst::min_pk::SecretKey::key_gen(&[7u8; 32], &[]).expect("key").sk_to_pk();
            let point = key.serialize();
            let negated = negate(&point);
            assert_ne!(negated, point);
            assert_eq!(negate(&negated), point);
            let minus = PublicKey::deserialize(&negated).expect("on the curve");
            let sum = AggregatePublicKey::aggregate(&[&key, &minus], false).expect("adds").to_public_key();
            assert_eq!(sum.compress()[0] & 0x40, 0x40, "P + (−P) is the identity");
        }
    }
}
