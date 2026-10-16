//! Network configuration. An instance is initialised with one `ChainSpec` and never
//! reconfigured (Functionality #9, #13). Everything that differs between Ethereum
//! networks lives here, so the same program supports mainnet and every testnet (#14).

use alloy_primitives::{b256, B256};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub enum ForkName {
    Phase0,
    Altair,
    Bellatrix,
    Capella,
    Deneb,
    Electra,
    Fulu,
    Gloas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct Fork {
    pub epoch: u64,
    pub version: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct Preset {
    pub slots_per_epoch: u64,
    pub epochs_per_sync_committee_period: u64,
    pub sync_committee_size: usize,
}

impl Preset {
    pub const MAINNET: Preset =
        Preset { slots_per_epoch: 32, epochs_per_sync_committee_period: 256, sync_committee_size: 512 };
    pub const MINIMAL: Preset =
        Preset { slots_per_epoch: 8, epochs_per_sync_committee_period: 8, sync_committee_size: 32 };
}

const FAR_FUTURE: u64 = u64::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct ChainSpec {
    /// Execution-layer chain id (EIP-155). Bound into every statement (Functionality #5).
    pub chain_id: u64,
    pub genesis_time: u64,
    pub seconds_per_slot: u64,
    pub genesis_validators_root: B256,
    /// Fork schedule, indexed by `ForkName as usize`.
    pub forks: [Fork; 8],
    pub preset: Preset,
}

impl ChainSpec {
    pub fn mainnet() -> Self {
        Self {
            chain_id: 1,
            genesis_time: 1_606_824_023,
            seconds_per_slot: 12,
            genesis_validators_root: b256!("4b363db94e286120d76eb905340fdd4e54bfe9f06bf33ff6cf5ad27f511bfe95"),
            forks: [
                Fork { epoch: 0, version: [0x00, 0, 0, 0] },
                Fork { epoch: 74_240, version: [0x01, 0, 0, 0] },
                Fork { epoch: 144_896, version: [0x02, 0, 0, 0] },
                Fork { epoch: 194_048, version: [0x03, 0, 0, 0] },
                Fork { epoch: 269_568, version: [0x04, 0, 0, 0] },
                Fork { epoch: 364_032, version: [0x05, 0, 0, 0] },
                Fork { epoch: 411_392, version: [0x06, 0, 0, 0] },
                Fork { epoch: FAR_FUTURE, version: [0x07, 0, 0, 0] },
            ],
            preset: Preset::MAINNET,
        }
    }

    pub fn sepolia() -> Self {
        Self {
            chain_id: 11_155_111,
            genesis_time: 1_655_733_600,
            seconds_per_slot: 12,
            genesis_validators_root: b256!("d8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078"),
            forks: [
                Fork { epoch: 0, version: [0x90, 0, 0, 0x69] },
                Fork { epoch: 50, version: [0x90, 0, 0, 0x70] },
                Fork { epoch: 100, version: [0x90, 0, 0, 0x71] },
                Fork { epoch: 56_832, version: [0x90, 0, 0, 0x72] },
                Fork { epoch: 132_608, version: [0x90, 0, 0, 0x73] },
                Fork { epoch: 222_464, version: [0x90, 0, 0, 0x74] },
                Fork { epoch: 272_640, version: [0x90, 0, 0, 0x75] },
                Fork { epoch: 353_024, version: [0x90, 0, 0, 0x76] },
            ],
            preset: Preset::MAINNET,
        }
    }

    pub fn hoodi() -> Self {
        Self {
            chain_id: 560_048,
            genesis_time: 1_742_213_400,
            seconds_per_slot: 12,
            genesis_validators_root: b256!("212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f"),
            forks: [
                Fork { epoch: 0, version: [0x10, 0, 0x09, 0x10] },
                Fork { epoch: 0, version: [0x20, 0, 0x09, 0x10] },
                Fork { epoch: 0, version: [0x30, 0, 0x09, 0x10] },
                Fork { epoch: 0, version: [0x40, 0, 0x09, 0x10] },
                Fork { epoch: 0, version: [0x50, 0, 0x09, 0x10] },
                Fork { epoch: 2_048, version: [0x60, 0, 0x09, 0x10] },
                Fork { epoch: 50_688, version: [0x70, 0, 0x09, 0x10] },
                Fork { epoch: FAR_FUTURE, version: [0x80, 0, 0x09, 0x10] },
            ],
            preset: Preset::MAINNET,
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "mainnet" => Some(Self::mainnet()),
            "sepolia" => Some(Self::sepolia()),
            "hoodi" => Some(Self::hoodi()),
            _ => None,
        }
    }

    pub fn slots_per_period(&self) -> u64 {
        self.preset.slots_per_epoch * self.preset.epochs_per_sync_committee_period
    }

    pub fn epoch_at_slot(&self, slot: u64) -> u64 {
        slot / self.preset.slots_per_epoch
    }

    pub fn period_at_slot(&self, slot: u64) -> u64 {
        slot / self.slots_per_period()
    }

    pub fn fork_at_epoch(&self, epoch: u64) -> ForkName {
        const ORDER: [ForkName; 8] = [
            ForkName::Phase0,
            ForkName::Altair,
            ForkName::Bellatrix,
            ForkName::Capella,
            ForkName::Deneb,
            ForkName::Electra,
            ForkName::Fulu,
            ForkName::Gloas,
        ];
        let mut current = ForkName::Phase0;
        for name in ORDER {
            if epoch >= self.forks[name as usize].epoch {
                current = name;
            }
        }
        current
    }

    pub fn fork_at_slot(&self, slot: u64) -> ForkName {
        self.fork_at_epoch(self.epoch_at_slot(slot))
    }

    pub fn fork_version_at_epoch(&self, epoch: u64) -> [u8; 4] {
        self.forks[self.fork_at_epoch(epoch) as usize].version
    }

    /// Unix time of a slot's start.
    pub fn slot_time(&self, slot: u64) -> u64 {
        self.genesis_time + slot * self.seconds_per_slot
    }

    /// Digest binding a statement to this network: chain id, genesis validators root and
    /// fork schedule. Two networks that share a contract address never share this digest.
    pub fn digest(&self) -> B256 {
        let mut h = Sha256::new();
        h.update(b"affidavit/chain/v1");
        h.update(self.chain_id.to_le_bytes());
        h.update(self.genesis_validators_root);
        for fork in &self.forks {
            h.update(fork.epoch.to_le_bytes());
            h.update(fork.version);
        }
        h.update(self.preset.slots_per_epoch.to_le_bytes());
        h.update(self.preset.epochs_per_sync_committee_period.to_le_bytes());
        h.update((self.preset.sync_committee_size as u64).to_le_bytes());
        B256::from_slice(&h.finalize())
    }
}

/// Generalised indices of the light-client proofs. They moved when the beacon state grew
/// past 32 fields at Electra, and move again at Gloas.
pub struct Gindex {
    pub depth: usize,
    pub index: u64,
}

pub fn finalized_root_gindex(fork: ForkName) -> Gindex {
    if fork >= ForkName::Electra {
        Gindex { depth: 7, index: 41 }
    } else {
        Gindex { depth: 6, index: 41 }
    }
}

pub fn current_sync_committee_gindex(fork: ForkName) -> Gindex {
    if fork >= ForkName::Electra {
        Gindex { depth: 6, index: 22 }
    } else {
        Gindex { depth: 5, index: 22 }
    }
}

pub fn next_sync_committee_gindex(fork: ForkName) -> Gindex {
    if fork >= ForkName::Electra {
        Gindex { depth: 6, index: 23 }
    } else {
        Gindex { depth: 5, index: 23 }
    }
}

/// `execution_payload` in `BeaconBlockBody`, Capella through Fulu.
pub const EXECUTION_PAYLOAD_GINDEX: Gindex = Gindex { depth: 4, index: 9 };

/// Weak-subjectivity period, in epochs, from the Electra formula
/// (`MIN_VALIDATOR_WITHDRAWABILITY_DELAY + SAFETY_DECAY * T / (2 * delta * 100)`), where `T`
/// is the total active balance and `delta` the balance churn limit, both in gwei.
/// The value is computed off-chain from public state at initialisation and frozen into the
/// instance configuration; see `docs/TRUST.md` for the procedure and the mainnet figure.
pub fn weak_subjectivity_period_epochs(total_active_balance_gwei: u64) -> u64 {
    const MIN_VALIDATOR_WITHDRAWABILITY_DELAY: u64 = 256;
    const SAFETY_DECAY: u128 = 10;
    const CHURN_LIMIT_QUOTIENT: u64 = 65_536;
    const MIN_PER_EPOCH_CHURN_LIMIT_ELECTRA: u64 = 128_000_000_000;
    const EFFECTIVE_BALANCE_INCREMENT: u64 = 1_000_000_000;
    let churn = (total_active_balance_gwei / CHURN_LIMIT_QUOTIENT).max(MIN_PER_EPOCH_CHURN_LIMIT_ELECTRA);
    let churn = churn - churn % EFFECTIVE_BALANCE_INCREMENT;
    let epochs = SAFETY_DECAY * total_active_balance_gwei as u128 / (2 * churn as u128 * 100);
    MIN_VALIDATOR_WITHDRAWABILITY_DELAY + epochs as u64
}
