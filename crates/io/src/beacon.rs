//! Beacon API JSON (`/eth/v1/beacon/light_client/*`, `/eth/v2/beacon/blocks/*`) into core types.

use affidavit_core::types::{
    BeaconBlockHeader, ExecutionPayloadHeader, LightClientBootstrap, LightClientHeader, LightClientUpdate,
    SyncAggregate, SyncCommittee,
};
use alloy_primitives::{Address, B256, U256};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::hex_bytes;

#[derive(Deserialize)]
struct Versioned<T> {
    data: T,
}

#[derive(Deserialize)]
struct JsonBeaconHeader {
    slot: String,
    proposer_index: String,
    parent_root: B256,
    state_root: B256,
    body_root: B256,
}

#[derive(Deserialize)]
struct JsonExecutionHeader {
    parent_hash: B256,
    fee_recipient: Address,
    state_root: B256,
    receipts_root: B256,
    logs_bloom: String,
    prev_randao: B256,
    block_number: String,
    gas_limit: String,
    gas_used: String,
    timestamp: String,
    extra_data: String,
    base_fee_per_gas: String,
    block_hash: B256,
    transactions_root: B256,
    withdrawals_root: B256,
    #[serde(default)]
    blob_gas_used: Option<String>,
    #[serde(default)]
    excess_blob_gas: Option<String>,
}

#[derive(Deserialize)]
struct JsonLcHeader {
    beacon: JsonBeaconHeader,
    execution: JsonExecutionHeader,
    execution_branch: Vec<B256>,
}

#[derive(Deserialize)]
struct JsonCommittee {
    pubkeys: Vec<String>,
    aggregate_pubkey: String,
}

#[derive(Deserialize)]
struct JsonAggregate {
    sync_committee_bits: String,
    sync_committee_signature: String,
}

#[derive(Deserialize)]
struct JsonBootstrap {
    header: JsonLcHeader,
    current_sync_committee: JsonCommittee,
    current_sync_committee_branch: Vec<B256>,
}

#[derive(Deserialize)]
struct JsonUpdate {
    attested_header: JsonLcHeader,
    #[serde(default)]
    next_sync_committee: Option<JsonCommittee>,
    #[serde(default)]
    next_sync_committee_branch: Option<Vec<B256>>,
    finalized_header: JsonLcHeader,
    finality_branch: Vec<B256>,
    sync_aggregate: JsonAggregate,
    signature_slot: String,
}

fn num(s: &str) -> Result<u64> {
    s.parse::<u64>().with_context(|| format!("not a decimal u64: {s}"))
}

fn fixed<const N: usize>(s: &str) -> Result<[u8; N]> {
    let bytes = hex_bytes(s)?;
    bytes.try_into().map_err(|b: Vec<u8>| anyhow!("expected {N} bytes, got {}", b.len()))
}

impl JsonBeaconHeader {
    fn into_core(self) -> Result<BeaconBlockHeader> {
        Ok(BeaconBlockHeader {
            slot: num(&self.slot)?,
            proposer_index: num(&self.proposer_index)?,
            parent_root: self.parent_root,
            state_root: self.state_root,
            body_root: self.body_root,
        })
    }
}

impl JsonExecutionHeader {
    fn into_core(self) -> Result<ExecutionPayloadHeader> {
        Ok(ExecutionPayloadHeader {
            parent_hash: self.parent_hash,
            fee_recipient: self.fee_recipient,
            state_root: self.state_root,
            receipts_root: self.receipts_root,
            logs_bloom: hex_bytes(&self.logs_bloom)?,
            prev_randao: self.prev_randao,
            block_number: num(&self.block_number)?,
            gas_limit: num(&self.gas_limit)?,
            gas_used: num(&self.gas_used)?,
            timestamp: num(&self.timestamp)?,
            extra_data: hex_bytes(&self.extra_data)?,
            base_fee_per_gas: U256::from_str_radix(&self.base_fee_per_gas, 10)?,
            block_hash: self.block_hash,
            transactions_root: self.transactions_root,
            withdrawals_root: self.withdrawals_root,
            blob_gas_used: self.blob_gas_used.as_deref().map(num).transpose()?.unwrap_or(0),
            excess_blob_gas: self.excess_blob_gas.as_deref().map(num).transpose()?.unwrap_or(0),
        })
    }
}

impl JsonLcHeader {
    fn into_core(self) -> Result<LightClientHeader> {
        Ok(LightClientHeader {
            beacon: self.beacon.into_core()?,
            execution: self.execution.into_core()?,
            execution_branch: self.execution_branch,
        })
    }
}

impl JsonCommittee {
    fn into_core(self) -> Result<SyncCommittee> {
        Ok(SyncCommittee {
            pubkeys: self.pubkeys.iter().map(|pk| fixed::<48>(pk)).collect::<Result<_>>()?,
            aggregate_pubkey: fixed::<48>(&self.aggregate_pubkey)?,
        })
    }
}

impl JsonUpdate {
    fn into_core(self) -> Result<LightClientUpdate> {
        Ok(LightClientUpdate {
            attested_header: self.attested_header.into_core()?,
            next_sync_committee: self.next_sync_committee.map(JsonCommittee::into_core).transpose()?,
            next_sync_committee_branch: self.next_sync_committee_branch.unwrap_or_default(),
            finalized_header: Some(self.finalized_header.into_core()?),
            finality_branch: self.finality_branch,
            sync_aggregate: SyncAggregate {
                sync_committee_bits: hex_bytes(&self.sync_aggregate.sync_committee_bits)?,
                sync_committee_signature: fixed::<96>(&self.sync_aggregate.sync_committee_signature)?,
            },
            signature_slot: num(&self.signature_slot)?,
        })
    }
}

/// `GET /eth/v1/beacon/light_client/bootstrap/{block_root}`.
pub fn parse_bootstrap(json: &str) -> Result<LightClientBootstrap> {
    let v: Versioned<JsonBootstrap> = serde_json::from_str(json).context("light client bootstrap")?;
    Ok(LightClientBootstrap {
        header: v.data.header.into_core()?,
        current_sync_committee: v.data.current_sync_committee.into_core()?,
        current_sync_committee_branch: v.data.current_sync_committee_branch,
    })
}

/// `GET /eth/v1/beacon/light_client/updates?start_period=..&count=..` (a JSON array).
pub fn parse_updates(json: &str) -> Result<Vec<LightClientUpdate>> {
    let list: Vec<Versioned<JsonUpdate>> = serde_json::from_str(json).context("light client updates")?;
    list.into_iter().map(|v| v.data.into_core()).collect()
}

/// `GET /eth/v1/beacon/light_client/finality_update`.
pub fn parse_finality_update(json: &str) -> Result<LightClientUpdate> {
    let v: Versioned<JsonUpdate> = serde_json::from_str(json).context("light client finality update")?;
    v.data.into_core()
}

/// Raw transactions of the execution payload in `GET /eth/v2/beacon/blocks/{id}`.
pub fn parse_block_transactions(json: &str) -> Result<(u64, Vec<Vec<u8>>)> {
    let v: Value = serde_json::from_str(json).context("beacon block")?;
    let message = &v["data"]["message"];
    let slot = num(message["slot"].as_str().ok_or_else(|| anyhow!("slot"))?)?;
    let txs = message["body"]["execution_payload"]["transactions"]
        .as_array()
        .ok_or_else(|| anyhow!("execution_payload.transactions"))?
        .iter()
        .map(|t| hex_bytes(t.as_str().ok_or_else(|| anyhow!("transaction"))?))
        .collect::<Result<_>>()?;
    Ok((slot, txs))
}
