//! Execution JSON-RPC material (`eth_getProof`, `eth_getBlockReceipts`, `eth_getBlockByHash`)
//! into core proofs. Receipt proofs do not exist in JSON-RPC, so the prover rebuilds the
//! block's receipts trie and checks it reproduces the header's receipts root before
//! extracting a proof from it.

use affidavit_core::execution::{receipt_key, AccountProof, ReceiptProof, StorageProof};
use alloy_consensus::{Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, Bloom, Bytes, Log, LogData, B256, U256};
use alloy_trie::{proof::ProofRetainer, HashBuilder, Nibbles};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;

fn quantity(v: &Value) -> Result<u64> {
    let s = v.as_str().ok_or_else(|| anyhow!("quantity is not a string"))?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16).with_context(|| format!("bad quantity {s}"))
}

/// Accepts a bare result or a `{"jsonrpc", "result"}` envelope.
fn result(json: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(json)?;
    Ok(if v.get("jsonrpc").is_some() { v["result"].clone() } else { v })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonStorageProof {
    key: String,
    value: U256,
    proof: Vec<Bytes>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonAccountProof {
    address: Address,
    account_proof: Vec<Bytes>,
    balance: U256,
    code_hash: B256,
    nonce: U256,
    storage_hash: B256,
    storage_proof: Vec<JsonStorageProof>,
}

/// `eth_getProof` result: the account proof and one proof per requested slot.
pub fn parse_get_proof(json: &str) -> Result<(AccountProof, Vec<StorageProof>)> {
    let p: JsonAccountProof = serde_json::from_value(result(json)?).context("eth_getProof")?;
    let account = AccountProof {
        address: p.address,
        nonce: p.nonce.try_into().map_err(|_| anyhow!("nonce overflow"))?,
        balance: p.balance,
        storage_root: p.storage_hash,
        code_hash: p.code_hash,
        proof: p.account_proof,
    };
    let storage = p
        .storage_proof
        .into_iter()
        .map(|s| {
            // Slot keys come back unpadded from some clients; left-pad to 32 bytes.
            let raw = crate::hex_bytes(&s.key)?;
            if raw.len() > 32 {
                bail!("storage key longer than 32 bytes");
            }
            let mut slot = [0u8; 32];
            slot[32 - raw.len()..].copy_from_slice(&raw);
            Ok(StorageProof { slot: B256::from(slot), value: s.value, proof: s.proof })
        })
        .collect::<Result<_>>()?;
    Ok((account, storage))
}

/// The fields of an execution block header the attestation path reads.
#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub number: u64,
    pub hash: B256,
    pub timestamp: u64,
    pub state_root: B256,
    pub receipts_root: B256,
}

pub fn parse_block(json: &str) -> Result<BlockInfo> {
    let b = result(json)?;
    let field = |k: &str| b.get(k).ok_or_else(|| anyhow!("block field {k}"));
    Ok(BlockInfo {
        number: quantity(field("number")?)?,
        hash: serde_json::from_value(field("hash")?.clone())?,
        timestamp: quantity(field("timestamp")?)?,
        state_root: serde_json::from_value(field("stateRoot")?.clone())?,
        receipts_root: serde_json::from_value(field("receiptsRoot")?.clone())?,
    })
}

/// Re-encodes `eth_getBlockReceipts` output into consensus receipts.
pub fn parse_receipts(json: &str) -> Result<Vec<ReceiptEnvelope>> {
    let list = result(json)?;
    let list = list.as_array().ok_or_else(|| anyhow!("receipts are not an array"))?;
    list.iter().map(receipt_from_json).collect()
}

fn receipt_from_json(r: &Value) -> Result<ReceiptEnvelope> {
    let status = quantity(&r["status"])? == 1;
    let logs = r["logs"]
        .as_array()
        .ok_or_else(|| anyhow!("logs"))?
        .iter()
        .map(|l| {
            let address: Address = serde_json::from_value(l["address"].clone())?;
            let topics: Vec<B256> = serde_json::from_value(l["topics"].clone())?;
            let data: Bytes = serde_json::from_value(l["data"].clone())?;
            Ok(Log { address, data: LogData::new_unchecked(topics, data) })
        })
        .collect::<Result<Vec<_>>>()?;
    let receipt =
        Receipt { status: Eip658Value::Eip658(status), cumulative_gas_used: quantity(&r["cumulativeGasUsed"])?, logs };
    let bloom: Bloom = serde_json::from_value(r["logsBloom"].clone())?;
    let rwb = ReceiptWithBloom { receipt, logs_bloom: bloom };
    Ok(match quantity(&r["type"])? {
        0 => ReceiptEnvelope::Legacy(rwb),
        1 => ReceiptEnvelope::Eip2930(rwb),
        2 => ReceiptEnvelope::Eip1559(rwb),
        3 => ReceiptEnvelope::Eip4844(rwb),
        4 => ReceiptEnvelope::Eip7702(rwb),
        t => bail!("unknown receipt type {t}"),
    })
}

/// Builds the ordered trie of `values` (keyed by `rlp(index)`), returning its root and,
/// for `target`, the proof nodes from the root down.
pub fn ordered_trie_proof(values: &[Vec<u8>], target: Option<u64>) -> (B256, Vec<Bytes>) {
    let mut entries: Vec<(Nibbles, &Vec<u8>)> =
        values.iter().enumerate().map(|(i, v)| (Nibbles::unpack(receipt_key(i as u64)), v)).collect();
    entries.sort_by_key(|entry| entry.0);
    let targets: Vec<Nibbles> = target.map(|t| Nibbles::unpack(receipt_key(t))).into_iter().collect();
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(targets.clone()));
    for (key, value) in &entries {
        hb.add_leaf(*key, value);
    }
    let root = hb.root();
    let proof = match targets.first() {
        Some(t) => hb.take_proof_nodes().matching_nodes_sorted(t).into_iter().map(|(_, node)| node).collect(),
        None => Vec::new(),
    };
    (root, proof)
}

/// Receipt proof for transaction `tx_index`, checked against `receipts_root`.
/// The receipts root a header carries for exactly these receipts, in order.
pub fn receipts_root(receipts: &[ReceiptEnvelope]) -> B256 {
    let encoded: Vec<Vec<u8>> = receipts.iter().map(|r| r.encoded_2718()).collect();
    ordered_trie_proof(&encoded, None).0
}

pub fn receipt_proof(receipts: &[ReceiptEnvelope], receipts_root: &B256, tx_index: u64) -> Result<ReceiptProof> {
    let encoded: Vec<Vec<u8>> = receipts.iter().map(|r| r.encoded_2718()).collect();
    let (root, proof) = ordered_trie_proof(&encoded, Some(tx_index));
    if root != *receipts_root {
        bail!("rebuilt receipts root {root} does not match the header's {receipts_root}");
    }
    let receipt = encoded.get(tx_index as usize).ok_or_else(|| anyhow!("no receipt at index {tx_index}"))?;
    Ok(ReceiptProof { tx_index, receipt: Bytes::from(receipt.clone()), proof })
}
