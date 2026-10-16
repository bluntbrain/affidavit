//! Statement and error rendering, human and JSON.

use affidavit_core::statement::{Fact, Statement};
use affidavit_core::Error;
use alloy_primitives::U256;
use serde_json::{json, Value};

use crate::predicate::describe;

pub fn statement_json(s: &Statement) -> Value {
    let fact = match &s.fact {
        Fact::Account { address, nonce, balance, code_hash, storage_root } => json!({
            "kind": "account", "address": address, "nonce": nonce, "balance": balance.to_string(),
            "code_hash": code_hash, "storage_root": storage_root,
        }),
        Fact::Storage { address, slot, value } => json!({
            "kind": "storage", "address": address, "slot": slot, "value": value.to_string(),
        }),
        Fact::Log { block_hash, tx_index, log_index, address, topics, data } => json!({
            "kind": "log", "block_hash": block_hash, "tx_index": tx_index, "log_index": log_index,
            "address": address, "topics": topics, "data": data,
        }),
        Fact::Transaction { block_hash, tx_index, tx_hash, from, to, value, success } => json!({
            "kind": "transaction", "block_hash": block_hash, "tx_index": tx_index, "tx_hash": tx_hash,
            "from": from, "to": to, "value": value.to_string(), "success": success,
        }),
    };
    json!({
        "statement_id": s.id,
        "chain_id": s.chain_id,
        "chain_digest": s.chain_digest,
        "instance_id": s.instance_id,
        "header": {
            "slot": s.header.slot,
            "beacon_root": s.header.beacon_root,
            "block_number": s.header.block_number,
            "block_hash": s.header.block_hash,
            "timestamp": s.header.timestamp,
        },
        "fact": fact,
        "predicate": describe(&s.predicate),
    })
}

pub fn error_json(e: &Error) -> Value {
    json!({ "error": { "code": e.code(), "name": e.name(), "message": e.to_string() } })
}

pub fn eth(wei: U256) -> String {
    let unit = U256::from(10u64).pow(U256::from(18u64));
    let whole = wei / unit;
    let frac = (wei % unit) / U256::from(10u64).pow(U256::from(12u64));
    format!("{whole}.{frac:0>6}")
}

pub fn short(h: impl std::fmt::Display) -> String {
    let s = h.to_string();
    if s.len() > 14 {
        format!("{}…{}", &s[..8], &s[s.len() - 4..])
    } else {
        s
    }
}
