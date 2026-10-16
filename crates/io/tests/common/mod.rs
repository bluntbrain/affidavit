#![allow(dead_code)]

use std::path::PathBuf;

use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::lc::InstanceConfig;
use affidavit_core::types::{LightClientBootstrap, LightClientUpdate};
use affidavit_io::beacon;
use alloy_primitives::{b256, B256};

/// Beacon block root of slot 7,069,376, the checkpoint Helios pins for the same fixtures.
pub const CHECKPOINT: B256 = b256!("5afc212a7924789b2bc86acad3ab3a6ffb1f6e97253ea50bee7f4f51422c9275");

/// A wall-clock slot just after the newest fixture (the finality update's signature slot is 7,109,431).
pub const NOW_SLOT: u64 = 7_109_500;

pub fn testdata(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(path)
}

pub fn read(path: &str) -> String {
    std::fs::read_to_string(testdata(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

pub fn bootstrap() -> LightClientBootstrap {
    beacon::parse_bootstrap(&read("mainnet/light-client/bootstrap.json")).unwrap()
}

pub fn updates() -> Vec<LightClientUpdate> {
    beacon::parse_updates(&read("mainnet/light-client/updates.json")).unwrap()
}

pub fn finality_update() -> LightClientUpdate {
    beacon::parse_finality_update(&read("mainnet/light-client/finality.json")).unwrap()
}

/// About 24M ETH was staked in August 2023.
pub fn mainnet_config() -> InstanceConfig {
    InstanceConfig {
        chain: ChainSpec::mainnet(),
        checkpoint_root: CHECKPOINT,
        ws_period_epochs: weak_subjectivity_period_epochs(24_000_000 * 1_000_000_000),
    }
}

pub use affidavit_io::follower::Follower;

/// Follows the fixtures from the checkpoint through every handoff and the finality update.
pub fn synced() -> Follower {
    let mut follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    for update in updates() {
        follower.process(&update, NOW_SLOT).unwrap();
    }
    follower.process(&finality_update(), NOW_SLOT).unwrap();
    follower
}
