//! The slice of SSZ merkleization the light client needs: `hash_tree_root` of a few fixed
//! containers, Merkle branches, and list roots with a length mix-in. Hashing is SHA-256,
//! which RISC Zero accelerates.

use alloc::vec::Vec;
use alloy_primitives::B256;
use sha2::{Digest, Sha256};

pub fn hash_pair(left: &B256, right: &B256) -> B256 {
    let mut h = Sha256::new();
    h.update(left);
    h.update(right);
    B256::from_slice(&h.finalize())
}

pub fn sha256(data: &[u8]) -> B256 {
    B256::from_slice(&Sha256::digest(data))
}

pub fn u64_chunk(value: u64) -> B256 {
    let mut chunk = [0u8; 32];
    chunk[..8].copy_from_slice(&value.to_le_bytes());
    B256::from(chunk)
}

/// Root of an all-zero subtree of the given depth.
pub fn zero_hash(depth: usize) -> B256 {
    let mut node = B256::ZERO;
    for _ in 0..depth {
        node = hash_pair(&node, &node);
    }
    node
}

/// Merkleizes `chunks` as the leaves of a tree of `2^depth` leaves, padding with zero
/// subtrees without materialising them (so a list limit of 2^20 costs nothing extra).
pub fn merkleize(chunks: &[B256], depth: usize) -> B256 {
    assert!(depth >= 64 || chunks.len() as u128 <= 1u128 << depth, "too many chunks for depth");
    if chunks.is_empty() {
        return zero_hash(depth);
    }
    let mut layer: Vec<B256> = chunks.to_vec();
    let mut zero = B256::ZERO;
    for _ in 0..depth {
        if layer.len() % 2 == 1 {
            layer.push(zero);
        }
        layer = layer.chunks(2).map(|pair| hash_pair(&pair[0], &pair[1])).collect();
        zero = hash_pair(&zero, &zero);
    }
    layer[0]
}

/// Sibling path for leaf `index` in the same tree `merkleize(chunks, depth)` builds,
/// ordered leaf-to-root as `is_valid_merkle_branch` expects.
pub fn merkle_branch(chunks: &[B256], depth: usize, index: usize) -> Vec<B256> {
    let mut branch = Vec::with_capacity(depth);
    let mut layer: Vec<B256> = chunks.to_vec();
    let mut zero = B256::ZERO;
    let mut idx = index;
    for _ in 0..depth {
        if layer.len() % 2 == 1 {
            layer.push(zero);
        }
        let sibling = idx ^ 1;
        branch.push(if sibling < layer.len() { layer[sibling] } else { zero });
        layer = layer.chunks(2).map(|pair| hash_pair(&pair[0], &pair[1])).collect();
        zero = hash_pair(&zero, &zero);
        idx /= 2;
    }
    branch
}

pub fn mix_in_length(root: &B256, length: u64) -> B256 {
    hash_pair(root, &u64_chunk(length))
}

/// `is_valid_merkle_branch` from the consensus specs.
pub fn is_valid_merkle_branch(leaf: &B256, branch: &[B256], depth: usize, index: u64, root: &B256) -> bool {
    if branch.len() != depth {
        return false;
    }
    let mut value = *leaf;
    for (i, node) in branch.iter().enumerate() {
        value = if (index >> i) & 1 == 1 { hash_pair(node, &value) } else { hash_pair(&value, node) };
    }
    value == *root
}

/// Packs bytes into 32-byte chunks, zero padding the last one.
pub fn pack_bytes(data: &[u8]) -> Vec<B256> {
    data.chunks(32)
        .map(|chunk| {
            let mut c = [0u8; 32];
            c[..chunk.len()].copy_from_slice(chunk);
            B256::from(c)
        })
        .collect()
}

/// Root of a `ByteList[limit]`.
pub fn byte_list_root(data: &[u8], limit_bytes: u64) -> B256 {
    let chunk_limit = limit_bytes.div_ceil(32).max(1);
    let depth = chunk_limit.next_power_of_two().trailing_zeros() as usize;
    mix_in_length(&merkleize(&pack_bytes(data), depth), data.len() as u64)
}

/// Root of a `ByteVector[N]` (N > 32 bytes, e.g. a BLS public key or the logs bloom).
pub fn byte_vector_root(data: &[u8]) -> B256 {
    let chunks = pack_bytes(data);
    let depth = chunks.len().next_power_of_two().trailing_zeros() as usize;
    merkleize(&chunks, depth)
}

/// `hash_tree_root` of a container from its field roots.
pub fn container_root(fields: &[B256]) -> B256 {
    let depth = fields.len().next_power_of_two().trailing_zeros() as usize;
    merkleize(fields, depth)
}

/// `MAX_BYTES_PER_TRANSACTION` and `MAX_TRANSACTIONS_PER_PAYLOAD` (Bellatrix onwards).
pub const MAX_BYTES_PER_TRANSACTION: u64 = 1 << 30;
pub const TRANSACTIONS_DEPTH: usize = 20;

/// `hash_tree_root(Transaction)`: a `ByteList[MAX_BYTES_PER_TRANSACTION]`.
pub fn transaction_root(tx: &[u8]) -> B256 {
    byte_list_root(tx, MAX_BYTES_PER_TRANSACTION)
}

/// `transactions_root` of an execution payload.
pub fn transactions_root(txs: &[Vec<u8>]) -> B256 {
    let leaves: Vec<B256> = txs.iter().map(|tx| transaction_root(tx)).collect();
    mix_in_length(&merkleize(&leaves, TRANSACTIONS_DEPTH), txs.len() as u64)
}

/// Branch proving transaction `index` under `transactions_root`: 20 siblings in the data
/// tree, then the length chunk. Verify with depth 21 and the same index.
pub fn transaction_branch(txs: &[Vec<u8>], index: usize) -> Vec<B256> {
    let leaves: Vec<B256> = txs.iter().map(|tx| transaction_root(tx)).collect();
    let mut branch = merkle_branch(&leaves, TRANSACTIONS_DEPTH, index);
    branch.push(u64_chunk(txs.len() as u64));
    branch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkleize_matches_naive_padding() {
        let leaves: Vec<B256> = (0u8..5).map(|i| B256::repeat_byte(i + 1)).collect();
        let mut padded = leaves.clone();
        padded.resize(8, B256::ZERO);
        let naive = {
            let l1: Vec<B256> = padded.chunks(2).map(|p| hash_pair(&p[0], &p[1])).collect();
            let l2: Vec<B256> = l1.chunks(2).map(|p| hash_pair(&p[0], &p[1])).collect();
            hash_pair(&l2[0], &l2[1])
        };
        assert_eq!(merkleize(&leaves, 3), naive);
    }

    #[test]
    fn branches_verify_for_every_leaf() {
        let leaves: Vec<B256> = (0u8..11).map(B256::repeat_byte).collect();
        let root = merkleize(&leaves, 6);
        for (i, leaf) in leaves.iter().enumerate() {
            let branch = merkle_branch(&leaves, 6, i);
            assert!(is_valid_merkle_branch(leaf, &branch, 6, i as u64, &root));
            assert!(!is_valid_merkle_branch(leaf, &branch, 6, (i as u64) ^ 1, &root));
        }
    }

    #[test]
    fn transaction_branch_verifies() {
        let txs: Vec<Vec<u8>> = (0..7).map(|i| alloc::vec![i as u8; 40 + i * 13]).collect();
        let root = transactions_root(&txs);
        for i in 0..txs.len() {
            let branch = transaction_branch(&txs, i);
            assert!(is_valid_merkle_branch(&transaction_root(&txs[i]), &branch, 21, i as u64, &root));
        }
    }
}
