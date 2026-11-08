//! Causeway inbound leg: authorise a wrapped-token mint from a verified Affidavit statement.
//!
//! This is the seam between RFP-021 and RFP-022. LEZ already ships the cross-zone
//! wrapped-token plumbing (`lez/programs/wrapped_token`, `bridge`, `cross_zone_*`), but its
//! inbox carries the comment *"l1_inclusion_witness must be None in v1"*: the mint trusts
//! that the Ethereum deposit happened. Causeway replaces that `None` with a [`Statement`]
//! from the Affidavit primitive, so the mint is authorised by a cryptographic proof that a
//! deposit was **included under a finalised Ethereum header**, with no trusted party.
//!
//! What this crate decides, given an already-verified statement:
//!
//! - the statement is about **this** chain and the instance the bridge is bound to;
//! - the fact is an ERC-20 `Transfer` of a **registered token** into **this vault**, for a
//!   **registered denomination** (fixed denominations are the RFP-022 privacy baseline);
//! - a **one-time nullifier** derived from the statement identifier, so one deposit mints
//!   exactly once (Reliability #3). The caller spends it in the instance's nullifier set.
//!
//! It does not re-prove inclusion; that is Affidavit's job, already tested on mainnet data.
//! It is the authorisation logic the LEZ bridge program runs over the proof's output, and
//! is written against `affidavit-core`'s real [`Statement`] type so the seam is exact.
//!
//! On LEZ the mint runs as a **private** execution over Affidavit's `AttestPrivate` path, so
//! the public transaction shows only the nullifier, the amount and the token — never which
//! deposit funded it (Privacy Preservation #1). This crate is the deterministic core of that
//! execution; the proving and the account writes are the LEZ program around it.

use affidavit_core::config::ChainSpec;
use affidavit_core::statement::{Fact, Statement};
use alloy_primitives::{Address, B256, U256};
use sha2::{Digest, Sha256};

/// `keccak256("Transfer(address,address,uint256)")`, topic 0 of an ERC-20 transfer.
pub const TRANSFER_TOPIC: B256 =
    B256::new(hex_literal(b"ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"));

/// A token the admin authority (RFP-001) has registered for this deployment.
#[derive(Debug, Clone)]
pub struct RegisteredToken {
    /// The ERC-20 on Ethereum.
    pub erc20: Address,
    /// The LEZ wrapped-token mint it maps to.
    pub wrapped_mint: [u8; 32],
    /// The fixed denominations a deposit may use (Functionality #8, privacy baseline).
    pub denominations: alloc_vec::Vec<U256>,
}

/// One deployment's configuration: the vault it accepts deposits into, the chain and
/// instance it is bound to, and the registered tokens. Admin-governed (RFP-001); immutable
/// to the verification path (Security #8).
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub chain: ChainSpec,
    /// The Affidavit instance id this bridge reads (one network, one checkpoint).
    pub instance_id: B256,
    /// The Ethereum vault that escrows deposits for this deployment.
    pub vault: Address,
    pub tokens: alloc_vec::Vec<RegisteredToken>,
}

/// The authorisation the LEZ bridge program acts on: mint `amount` of `wrapped_mint`, and
/// spend `nullifier` exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintAuthorization {
    pub wrapped_mint: [u8; 32],
    pub amount: U256,
    /// One-time key, derived from the statement identifier. The deposit is spent when this
    /// is first seen; a second authorisation with the same nullifier is a replay.
    pub nullifier: B256,
}

/// Why a mint was refused. Each maps to a distinct, testable failure of the inbound leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeError {
    /// The statement is about another network or another instance (Affidavit `check_binding`).
    WrongInstance,
    /// The attested fact is not an ERC-20 `Transfer` log.
    NotATransfer,
    /// The transfer is not into this deployment's vault.
    NotToVault,
    /// The transferred token is not in the registry.
    TokenNotRegistered,
    /// The amount is not one of the token's registered denominations (Privacy #1 baseline).
    AmountNotADenomination,
    /// This deposit's nullifier was already spent (Reliability #3).
    AlreadyMinted,
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            BridgeError::WrongInstance => "CWY-100 wrong chain or instance",
            BridgeError::NotATransfer => "CWY-101 attested fact is not an ERC-20 Transfer",
            BridgeError::NotToVault => "CWY-102 transfer is not into this vault",
            BridgeError::TokenNotRegistered => "CWY-103 token is not registered",
            BridgeError::AmountNotADenomination => "CWY-104 amount is not a registered denomination",
            BridgeError::AlreadyMinted => "CWY-105 deposit already minted (nullifier spent)",
        })
    }
}

fn addr_word(topic: &B256) -> Address {
    Address::from_slice(&topic.0[12..])
}

/// The one-time nullifier for a deposit, from its Affidavit statement identifier. Domain-
/// separated so it can never collide with another consumer's nullifier over the same id.
pub fn deposit_nullifier(statement_id: &B256) -> B256 {
    let mut h = Sha256::new();
    h.update(b"causeway/deposit-nullifier/v1");
    h.update(statement_id);
    B256::from_slice(&h.finalize())
}

/// Decide whether a verified Affidavit statement authorises a wrapped mint, and for how
/// much, under `config`. The statement must already be verified (it is the output of
/// Affidavit's attestation); this checks only that it is the right *kind* of deposit for
/// this deployment.
pub fn authorize_mint(config: &BridgeConfig, statement: &Statement) -> Result<MintAuthorization, BridgeError> {
    // Right network, right instance (Functionality #15, #Security 2).
    statement.check_binding(&config.chain, &config.instance_id).map_err(|_| BridgeError::WrongInstance)?;

    // The fact must be an ERC-20 Transfer log.
    let Fact::Log { address, topics, data, .. } = &statement.fact else {
        return Err(BridgeError::NotATransfer);
    };
    if topics.first() != Some(&TRANSFER_TOPIC) || topics.len() < 3 {
        return Err(BridgeError::NotATransfer);
    }

    // Into this vault (the `to` is topic 2 of a Transfer).
    if addr_word(&topics[2]) != config.vault {
        return Err(BridgeError::NotToVault);
    }

    // Of a registered token.
    let token = config.tokens.iter().find(|t| t.erc20 == *address).ok_or(BridgeError::TokenNotRegistered)?;

    // Of a registered denomination (word 0 of the data is the amount).
    let amount = U256::from_be_slice(data.get(0..32).ok_or(BridgeError::NotATransfer)?);
    if !token.denominations.contains(&amount) {
        return Err(BridgeError::AmountNotADenomination);
    }

    Ok(MintAuthorization { wrapped_mint: token.wrapped_mint, amount, nullifier: deposit_nullifier(&statement.id) })
}

/// The instance's spent-deposit set. On LEZ this is a PDA per nullifier, written once; here
/// it is the in-memory model the tests drive, showing that a deposit mints exactly once.
#[derive(Debug, Default)]
pub struct NullifierSet {
    spent: alloc_vec::Vec<B256>,
}

impl NullifierSet {
    /// Spend `auth`'s nullifier, or refuse it as an already-minted deposit (Reliability #3).
    pub fn spend(&mut self, auth: &MintAuthorization) -> Result<(), BridgeError> {
        if self.spent.contains(&auth.nullifier) {
            return Err(BridgeError::AlreadyMinted);
        }
        self.spent.push(auth.nullifier);
        Ok(())
    }
}

mod alloc_vec {
    pub use std::vec::Vec;
}

/// Compile-time hex decode for the 32-byte topic constant.
const fn hex_literal(s: &[u8; 64]) -> [u8; 32] {
    const fn nibble(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => 0,
        }
    }
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(s[i * 2]) << 4) | nibble(s[i * 2 + 1]);
        i += 1;
    }
    out
}
