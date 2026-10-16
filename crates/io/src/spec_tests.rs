//! Reader for the official consensus-spec-tests `light_client/sync` vectors: snappy-framed
//! SSZ containers, `config.yaml`, `meta.yaml` and `steps.yaml`.

use std::fs;
use std::path::Path;

use affidavit_core::config::{ChainSpec, Fork, ForkName, Preset};
use affidavit_core::types::{
    BeaconBlockHeader, ExecutionPayloadHeader, LightClientBootstrap, LightClientHeader, LightClientUpdate,
    SyncAggregate, SyncCommittee,
};
use alloy_primitives::{Address, B256, U256};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

pub struct SyncTest {
    pub spec: ChainSpec,
    pub trusted_block_root: B256,
    pub bootstrap: LightClientBootstrap,
    pub steps: Vec<Step>,
}

pub struct Step {
    /// The reference store force-updated before this step (the instance never does).
    pub after_force_update: bool,
    pub update: LightClientUpdate,
    pub current_slot: u64,
    pub expected_finalized_slot: u64,
    pub expected_finalized_root: B256,
}

#[derive(Deserialize)]
struct Meta {
    genesis_validators_root: B256,
    trusted_block_root: B256,
}

#[derive(Deserialize)]
struct HeaderCheck {
    slot: u64,
    beacon_root: B256,
}

#[derive(Deserialize)]
struct Checks {
    finalized_header: HeaderCheck,
}

#[derive(Deserialize)]
struct ProcessUpdate {
    update: String,
    current_slot: u64,
    checks: Checks,
}

fn version(config: &serde_yaml::Value, key: &str) -> Result<[u8; 4]> {
    let v = &config[key];
    let s = match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => format!("{:#010x}", n.as_u64().unwrap_or(0)),
        _ => bail!("{key} missing"),
    };
    let bytes = hex::decode(s.trim_start_matches("0x"))?;
    bytes.try_into().map_err(|_| anyhow!("{key} is not 4 bytes"))
}

fn epoch(config: &serde_yaml::Value, key: &str) -> u64 {
    config[key].as_u64().unwrap_or(u64::MAX)
}

fn chain_spec(dir: &Path, gvr: B256) -> Result<ChainSpec> {
    let config: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(dir.join("config.yaml"))?)?;
    if config["PRESET_BASE"].as_str() != Some("minimal") {
        bail!("only minimal-preset sync vectors are generated upstream");
    }
    let names = ["GENESIS", "ALTAIR", "BELLATRIX", "CAPELLA", "DENEB", "ELECTRA", "FULU", "GLOAS"];
    let mut forks = [Fork { epoch: u64::MAX, version: [0; 4] }; 8];
    for (i, name) in names.iter().enumerate() {
        let version_key = format!("{name}_FORK_VERSION");
        forks[i] = Fork {
            epoch: if i == 0 { 0 } else { epoch(&config, &format!("{name}_FORK_EPOCH")) },
            version: version(&config, &version_key).unwrap_or([0xff; 4]),
        };
    }
    Ok(ChainSpec {
        chain_id: 0,
        genesis_time: 0,
        seconds_per_slot: config["SECONDS_PER_SLOT"].as_u64().unwrap_or(6),
        genesis_validators_root: gvr,
        forks,
        preset: Preset::MINIMAL,
    })
}

fn snappy(path: &Path) -> Result<Vec<u8>> {
    let raw = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(snap::raw::Decoder::new().decompress_vec(&raw)?)
}

/// Cursor over a fixed-layout SSZ buffer.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos + n;
        let out = self.buf.get(self.pos..end).ok_or_else(|| anyhow!("ssz buffer too short"))?;
        self.pos = end;
        Ok(out)
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn b256(&mut self) -> Result<B256> {
        Ok(B256::from_slice(self.take(32)?))
    }
    fn branch(&mut self, depth: usize) -> Result<Vec<B256>> {
        (0..depth).map(|_| self.b256()).collect()
    }
}

fn beacon_header(r: &mut Reader<'_>) -> Result<BeaconBlockHeader> {
    Ok(BeaconBlockHeader {
        slot: r.u64()?,
        proposer_index: r.u64()?,
        parent_root: r.b256()?,
        state_root: r.b256()?,
        body_root: r.b256()?,
    })
}

fn execution_header(bytes: &[u8], fork: ForkName) -> Result<ExecutionPayloadHeader> {
    let mut r = Reader::new(bytes);
    let parent_hash = r.b256()?;
    let fee_recipient = Address::from_slice(r.take(20)?);
    let state_root = r.b256()?;
    let receipts_root = r.b256()?;
    let logs_bloom = r.take(256)?.to_vec();
    let prev_randao = r.b256()?;
    let block_number = r.u64()?;
    let gas_limit = r.u64()?;
    let gas_used = r.u64()?;
    let timestamp = r.u64()?;
    let extra_offset = r.u32()? as usize;
    let base_fee_per_gas = U256::from_le_slice(r.take(32)?);
    let block_hash = r.b256()?;
    let transactions_root = r.b256()?;
    let withdrawals_root = r.b256()?;
    let (blob_gas_used, excess_blob_gas) = if fork >= ForkName::Deneb { (r.u64()?, r.u64()?) } else { (0, 0) };
    if extra_offset != r.pos {
        bail!("unexpected extra_data offset");
    }
    Ok(ExecutionPayloadHeader {
        parent_hash,
        fee_recipient,
        state_root,
        receipts_root,
        logs_bloom,
        prev_randao,
        block_number,
        gas_limit,
        gas_used,
        timestamp,
        extra_data: bytes[extra_offset..].to_vec(),
        base_fee_per_gas,
        block_hash,
        transactions_root,
        withdrawals_root,
        blob_gas_used,
        excess_blob_gas,
    })
}

/// `LightClientHeader` (Capella onwards): beacon, offset of execution, execution_branch.
fn lc_header(bytes: &[u8], fork: ForkName) -> Result<LightClientHeader> {
    let mut r = Reader::new(bytes);
    let beacon = beacon_header(&mut r)?;
    let offset = r.u32()? as usize;
    let execution_branch = r.branch(4)?;
    Ok(LightClientHeader { beacon, execution: execution_header(&bytes[offset..], fork)?, execution_branch })
}

fn committee(r: &mut Reader<'_>, size: usize) -> Result<SyncCommittee> {
    let pubkeys = (0..size).map(|_| Ok(r.take(48)?.try_into()?)).collect::<Result<Vec<[u8; 48]>>>()?;
    Ok(SyncCommittee { pubkeys, aggregate_pubkey: r.take(48)?.try_into()? })
}

fn depths(fork: ForkName) -> (usize, usize) {
    // (sync committee branch, finality branch)
    if fork >= ForkName::Electra {
        (6, 7)
    } else {
        (5, 6)
    }
}

fn bootstrap(bytes: &[u8], fork: ForkName, size: usize) -> Result<LightClientBootstrap> {
    let mut r = Reader::new(bytes);
    let header_offset = r.u32()? as usize;
    let current_sync_committee = committee(&mut r, size)?;
    let current_sync_committee_branch = r.branch(depths(fork).0)?;
    Ok(LightClientBootstrap {
        header: lc_header(&bytes[header_offset..], fork)?,
        current_sync_committee,
        current_sync_committee_branch,
    })
}

fn update(bytes: &[u8], fork: ForkName, size: usize) -> Result<LightClientUpdate> {
    let (committee_depth, finality_depth) = depths(fork);
    let mut r = Reader::new(bytes);
    let attested_offset = r.u32()? as usize;
    let next = committee(&mut r, size)?;
    let next_branch = r.branch(committee_depth)?;
    let finalized_offset = r.u32()? as usize;
    let finality_branch = r.branch(finality_depth)?;
    let bits = r.take(size / 8)?.to_vec();
    let signature: [u8; 96] = r.take(96)?.try_into()?;
    let signature_slot = r.u64()?;
    let attested_header = lc_header(&bytes[attested_offset..finalized_offset], fork)?;
    let finalized_header = lc_header(&bytes[finalized_offset..], fork)?;
    // An update without a handoff carries an all-zero committee and branch.
    let has_next = next_branch.iter().any(|n| !n.is_zero());
    let has_finality = finality_branch.iter().any(|n| !n.is_zero());
    Ok(LightClientUpdate {
        attested_header,
        next_sync_committee: has_next.then_some(next),
        next_sync_committee_branch: if has_next { next_branch } else { Vec::new() },
        finalized_header: has_finality.then_some(finalized_header),
        finality_branch,
        sync_aggregate: SyncAggregate { sync_committee_bits: bits, sync_committee_signature: signature },
        signature_slot,
    })
}

/// Loads one `light_client/sync/pyspec_tests/<case>` directory for a fork.
pub fn load_sync_test(dir: &Path, fork: ForkName) -> Result<SyncTest> {
    let meta: Meta = serde_yaml::from_str(&fs::read_to_string(dir.join("meta.yaml"))?)?;
    let spec = chain_spec(dir, meta.genesis_validators_root)?;
    let size = spec.preset.sync_committee_size;
    let bootstrap = bootstrap(&snappy(&dir.join("bootstrap.ssz_snappy"))?, fork, size)?;
    // Each step is a one-key map such as `- process_update: {...}`; only updates matter here.
    let raw_steps: Vec<std::collections::BTreeMap<String, serde_yaml::Value>> =
        serde_yaml::from_str(&fs::read_to_string(dir.join("steps.yaml"))?)?;
    let mut steps = Vec::new();
    let mut forced = false;
    for step in raw_steps {
        if step.contains_key("force_update") {
            forced = true;
        }
        if let Some(value) = step.get("process_update") {
            let p: ProcessUpdate = serde_yaml::from_value(value.clone())?;
            let bytes = snappy(&dir.join(format!("{}.ssz_snappy", p.update)))?;
            steps.push(Step {
                after_force_update: forced,
                update: update(&bytes, fork, size)?,
                current_slot: p.current_slot,
                expected_finalized_slot: p.checks.finalized_header.slot,
                expected_finalized_root: p.checks.finalized_header.beacon_root,
            });
        }
    }
    Ok(SyncTest { spec, trusted_block_root: meta.trusted_block_root, bootstrap, steps })
}
