//! Causeway inbound leg: authorise a wrapped-token mint from a verified Affidavit statement.
//!
//! This is the seam between RFP-021 and RFP-022. LEZ already ships the cross-zone
//! wrapped-token plumbing (`lez/programs/wrapped_token`, `bridge`, `cross_zone_*`), but its
//! inbox carries the comment *"l1_inclusion_witness must be None in v1"*: the mint trusts
//! that the Ethereum deposit happened. Causeway replaces that `None` with a [`Statement`]
//! from the Affidavit primitive, so the mint is authorised by a cryptographic proof that a
//! deposit was **included under a finalised Ethereum header**, with no trusted party.
//!
//! What this crate decides, given an already-verified statement and the depositor's secret:
//!
//! - the statement is about **this** chain and the instance the bridge is bound to;
//! - the fact is the vault's own `Deposit(index, token, amount, commitment)` log, emitted by
//!   **this vault**, for a **registered token** and a **registered denomination** (fixed
//!   denominations are the RFP-022 privacy baseline);
//! - the depositor's secret **opens the commitment** the deposit carries, so only the holder
//!   of the seed that made the deposit can claim it (Security #3);
//! - a **one-time nullifier** derived from the secret and the statement identifier, so one
//!   deposit mints exactly once (Reliability #3) and no observer can compute it from public
//!   data (Privacy Preservation #1). The caller spends it in the instance's nullifier set.
//!
//! It does not re-prove inclusion; that is Affidavit's job. It is the authorisation logic the
//! LEZ bridge program runs over the proof's output, written against `affidavit-core`'s real
//! [`Statement`] type so the seam is exact, and exercised in `tests/seam.rs` on receipts the
//! compiled vault emitted on a real EVM.
//!
//! On LEZ the mint runs as a **private** execution over Affidavit's `AttestPrivate` path, so
//! the public transaction shows only the nullifier, the amount and the token — never which
//! deposit funded it. This crate is the deterministic core of that execution; the proving
//! and the account writes are the LEZ program around it.

pub mod recovery;

use affidavit_core::config::ChainSpec;
use affidavit_core::statement::{Fact, Statement};
use alloy_primitives::{Address, B256, U256};
use sha2::{Digest, Sha256};

use crate::recovery::deposit_commitment;

/// `keccak256("Deposit(uint256,address,uint256,bytes32)")`: topic 0 of the vault's deposit log
/// (`CausewayVault.sol`). Topics 1 and 2 are the deposit index and the token; the data words
/// are the amount and the commitment.
pub const DEPOSIT_TOPIC: B256 =
    B256::new(hex_literal(b"a3f79602c59a24c3a1150457898d8110e447cc0148e0f97e8e0167e2b62001cb"));

/// A token the admin authority (RFP-001) has registered for this deployment.
#[derive(Debug, Clone)]
pub struct RegisteredToken {
    /// The ERC-20 on Ethereum; `Address::ZERO` is native ETH, as in the vault.
    pub erc20: Address,
    /// The LEZ wrapped-token mint it maps to.
    pub wrapped_mint: [u8; 32],
    /// The fixed denominations a deposit may use (Functionality #8, privacy baseline).
    pub denominations: Vec<U256>,
}

/// One deployment's configuration: the vault it accepts deposits from, the chain and
/// instance it is bound to, and the registered tokens. Admin-governed (RFP-001); immutable
/// to the verification path (Security #8).
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub chain: ChainSpec,
    /// The Affidavit instance id this bridge reads (one network, one checkpoint).
    pub instance_id: B256,
    /// The Ethereum vault whose `Deposit` logs this deployment honours.
    pub vault: Address,
    pub tokens: Vec<RegisteredToken>,
}

/// The authorisation the LEZ bridge program acts on: mint `amount` of `wrapped_mint`, and
/// spend `nullifier` exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintAuthorization {
    pub wrapped_mint: [u8; 32],
    pub amount: U256,
    /// One-time key, derived from the depositor's secret and the statement identifier. The
    /// deposit is spent when this is first seen; a second authorisation with the same
    /// nullifier is a replay.
    pub nullifier: B256,
}

/// Why a mint was refused. Each maps to a distinct, testable failure of the inbound leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeError {
    /// The statement is about another network or another instance (Affidavit `check_binding`).
    WrongInstance,
    /// The attested fact is not a `Deposit(uint256,address,uint256,bytes32)` log.
    NotADeposit,
    /// The deposit log was emitted by a contract other than this deployment's vault.
    NotThisVault,
    /// The deposited token is not in the registry.
    TokenNotRegistered,
    /// The amount is not one of the token's registered denominations (Privacy #1 baseline).
    AmountNotADenomination,
    /// The secret does not open the commitment the deposit carries: not the depositor.
    WrongSecret,
    /// This deposit's nullifier was already spent (Reliability #3).
    AlreadyMinted,
}

impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            BridgeError::WrongInstance => "CWY-100 wrong chain or instance",
            BridgeError::NotADeposit => "CWY-101 attested fact is not a vault Deposit log",
            BridgeError::NotThisVault => "CWY-102 deposit log was not emitted by this vault",
            BridgeError::TokenNotRegistered => "CWY-103 token is not registered",
            BridgeError::AmountNotADenomination => "CWY-104 amount is not a registered denomination",
            BridgeError::AlreadyMinted => "CWY-105 deposit already minted (nullifier spent)",
            BridgeError::WrongSecret => "CWY-106 secret does not open the deposit's commitment",
        })
    }
}

fn addr_word(topic: &B256) -> Address {
    Address::from_slice(&topic.0[12..])
}

/// The one-time nullifier for a deposit: `sha256(domain ‖ secret ‖ statement id)`. Unique per
/// deposit because of the statement identifier; not computable from public data because of
/// the secret, so a claim cannot be matched to its deposit by recomputing nullifiers.
pub fn deposit_nullifier(secret: &B256, statement_id: &B256) -> B256 {
    let mut h = Sha256::new();
    h.update(b"causeway/deposit-nullifier/v1");
    h.update(secret);
    h.update(statement_id);
    B256::from_slice(&h.finalize())
}

/// Decide whether a verified Affidavit statement, together with the depositor's secret,
/// authorises a wrapped mint under `config`, and for how much. The statement must already be
/// verified (it is the output of Affidavit's attestation); this checks only that it is the
/// right *kind* of deposit for this deployment and that the caller is its depositor.
pub fn authorize_mint(
    config: &BridgeConfig,
    statement: &Statement,
    secret: &B256,
) -> Result<MintAuthorization, BridgeError> {
    // Right network, right instance (Functionality #15, Security #2).
    statement.check_binding(&config.chain, &config.instance_id).map_err(|_| BridgeError::WrongInstance)?;

    // The fact must be a vault `Deposit` log: three topics, two data words.
    let Fact::Log { address, topics, data, .. } = &statement.fact else {
        return Err(BridgeError::NotADeposit);
    };
    if topics.first() != Some(&DEPOSIT_TOPIC) || topics.len() != 3 || data.len() < 64 {
        return Err(BridgeError::NotADeposit);
    }

    // Emitted by this deployment's vault.
    if *address != config.vault {
        return Err(BridgeError::NotThisVault);
    }

    // Of a registered token, for a registered denomination.
    let token = addr_word(&topics[2]);
    let registered = config.tokens.iter().find(|t| t.erc20 == token).ok_or(BridgeError::TokenNotRegistered)?;
    let amount = U256::from_be_slice(&data[0..32]);
    if !registered.denominations.contains(&amount) {
        return Err(BridgeError::AmountNotADenomination);
    }

    // And the caller holds the secret the deposit was committed to.
    let commitment = B256::from_slice(&data[32..64]);
    if deposit_commitment(secret) != commitment {
        return Err(BridgeError::WrongSecret);
    }

    Ok(MintAuthorization {
        wrapped_mint: registered.wrapped_mint,
        amount,
        nullifier: deposit_nullifier(secret, &statement.id),
    })
}

/// The instance's spent-deposit set. On LEZ this is a PDA per nullifier, written once; here
/// it is the in-memory model the tests drive, showing that a deposit mints exactly once.
#[derive(Debug, Default)]
pub struct NullifierSet {
    spent: Vec<B256>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deposit_topic_is_the_keccak_of_the_vault_event_signature() {
        assert_eq!(DEPOSIT_TOPIC, alloy_primitives::keccak256(b"Deposit(uint256,address,uint256,bytes32)"));
    }

    #[test]
    fn nullifier_depends_on_both_secret_and_deposit() {
        let id = B256::repeat_byte(0x01);
        let a = deposit_nullifier(&B256::repeat_byte(0xaa), &id);
        assert_ne!(a, deposit_nullifier(&B256::repeat_byte(0xab), &id));
        assert_ne!(a, deposit_nullifier(&B256::repeat_byte(0xaa), &B256::repeat_byte(0x02)));
    }
}
