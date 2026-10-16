//! Append-only accumulator (Merkle mountain range) over every finalised header an instance
//! records. On chain it is a handful of peaks; a consumer proves its header is a member of
//! the current root. A private consumer reveals only that shared root, not which header it
//! used (Functionality #8), and a header recorded once stays provable forever (Reliability #3).

use alloc::vec::Vec;
use alloy_primitives::B256;
use sha2::{Digest, Sha256};

fn hash_node(left: &B256, right: &B256) -> B256 {
    let mut h = Sha256::new();
    h.update([0x01]);
    h.update(left);
    h.update(right);
    B256::from_slice(&h.finalize())
}

fn bag(leaf_count: u64, peaks: &[B256]) -> B256 {
    let mut h = Sha256::new();
    h.update([0x02]);
    h.update(leaf_count.to_le_bytes());
    for peak in peaks {
        h.update(peak);
    }
    B256::from_slice(&h.finalize())
}

/// What the instance stores: at most 64 peaks, independent of how many headers it has seen
/// beyond a logarithm (Performance #7, #9).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct MmrAccumulator {
    pub leaf_count: u64,
    /// Peaks from the largest (leftmost) mountain to the smallest.
    pub peaks: Vec<B256>,
}

impl MmrAccumulator {
    pub fn append(&mut self, leaf: B256) {
        let mut node = leaf;
        let mut count = self.leaf_count;
        while count & 1 == 1 {
            let left = self.peaks.pop().expect("peak exists for every set bit");
            node = hash_node(&left, &node);
            count >>= 1;
        }
        self.peaks.push(node);
        self.leaf_count += 1;
    }

    pub fn root(&self) -> B256 {
        bag(self.leaf_count, &self.peaks)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct MmrProof {
    pub leaf_index: u64,
    pub leaf_count: u64,
    /// Siblings from the leaf up to its mountain's peak.
    pub siblings: Vec<B256>,
    /// Every peak at `leaf_count`, as the accumulator holds them.
    pub peaks: Vec<B256>,
}

/// Locates the mountain holding `index`: (offset of its first leaf, its height, its peak position).
fn mountain_of(leaf_count: u64, index: u64) -> Option<(u64, u32, usize)> {
    let mut offset = 0u64;
    let mut position = 0usize;
    for height in (0..64u32).rev() {
        if (leaf_count >> height) & 1 == 1 {
            let size = 1u64 << height;
            if index < offset + size {
                return Some((offset, height, position));
            }
            offset += size;
            position += 1;
        }
    }
    None
}

pub fn verify(root: &B256, leaf: &B256, proof: &MmrProof) -> bool {
    let Some((offset, height, position)) = mountain_of(proof.leaf_count, proof.leaf_index) else {
        return false;
    };
    if proof.siblings.len() != height as usize || proof.peaks.len() != proof.leaf_count.count_ones() as usize {
        return false;
    }
    let mut node = *leaf;
    let mut local = proof.leaf_index - offset;
    for sibling in &proof.siblings {
        node = if local & 1 == 1 { hash_node(sibling, &node) } else { hash_node(&node, sibling) };
        local >>= 1;
    }
    proof.peaks[position] == node && bag(proof.leaf_count, &proof.peaks) == *root
}

/// Prover-side copy that keeps the leaves, so it can produce membership proofs.
#[derive(Debug, Clone, Default)]
pub struct Mmr {
    leaves: Vec<B256>,
    accumulator: MmrAccumulator,
}

impl Mmr {
    pub fn append(&mut self, leaf: B256) {
        self.leaves.push(leaf);
        self.accumulator.append(leaf);
    }

    pub fn accumulator(&self) -> &MmrAccumulator {
        &self.accumulator
    }

    pub fn leaves(&self) -> &[B256] {
        &self.leaves
    }

    pub fn prove(&self, index: u64) -> Option<MmrProof> {
        let count = self.leaves.len() as u64;
        let (offset, height, _) = mountain_of(count, index)?;
        let mut layer: Vec<B256> = self.leaves[offset as usize..(offset + (1u64 << height)) as usize].to_vec();
        let mut local = (index - offset) as usize;
        let mut siblings = Vec::with_capacity(height as usize);
        for _ in 0..height {
            siblings.push(layer[local ^ 1]);
            layer = layer.chunks(2).map(|pair| hash_node(&pair[0], &pair[1])).collect();
            local /= 2;
        }
        Some(MmrProof { leaf_index: index, leaf_count: count, siblings, peaks: self.accumulator.peaks.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_leaf_proves_at_every_size() {
        let mut mmr = Mmr::default();
        for n in 0..40u8 {
            mmr.append(B256::repeat_byte(n));
            let root = mmr.accumulator().root();
            for i in 0..=n as u64 {
                let proof = mmr.prove(i).unwrap();
                assert!(verify(&root, &B256::repeat_byte(i as u8), &proof), "leaf {i} of {}", n + 1);
                assert!(!verify(&root, &B256::repeat_byte(0xee), &proof));
            }
        }
    }

    #[test]
    fn stale_proof_fails_against_newer_root() {
        let mut mmr = Mmr::default();
        for n in 0..5u8 {
            mmr.append(B256::repeat_byte(n));
        }
        let proof = mmr.prove(2).unwrap();
        mmr.append(B256::repeat_byte(9));
        assert!(!verify(&mmr.accumulator().root(), &B256::repeat_byte(2), &proof));
        assert!(verify(&mmr.accumulator().root(), &B256::repeat_byte(2), &mmr.prove(2).unwrap()));
    }
}
