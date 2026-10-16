//! I/O for affidavit: Beacon API and execution JSON-RPC payloads in, core types out, plus
//! the prover-side helpers that build proofs the RPC does not return directly.

pub mod beacon;
pub mod follower;
pub mod rpc;
pub mod spec_tests;

use anyhow::{Context, Result};

pub fn hex_bytes(s: &str) -> Result<Vec<u8>> {
    hex::decode(s.trim_start_matches("0x")).with_context(|| format!("bad hex: {}", &s[..s.len().min(20)]))
}
