//! Position recovery from the wallet seed alone (RFP-022 Functionality #9, Reliability #5).
//!
//! The RFP calls loss of access between lock and mint "the largest UX risk" and requires
//! that a user can rediscover every unclaimed deposit and unreleased burn from their seed
//! plus on-chain state, with no server-side index and no secret generated during the flow.
//!
//! Causeway's deposit carries one 32-byte **commitment** and nothing else. The commitment is
//! derived deterministically from the seed and the deposit's public coordinates, so the
//! client never has to remember anything: it re-derives candidates and scans the vault's
//! `Deposit` logs for matches, with a gap limit, the way a BIP-44 wallet rediscovers its
//! addresses.
//!
//! ```text
//! s  = sha256("causeway/deposit-secret/v1"     ‖ seed ‖ chain_id ‖ vault ‖ token ‖ nonce)
//! c  = sha256("causeway/deposit-commitment/v1" ‖ s)                   -- goes on Ethereum
//! ```
//!
//! `s` is the opening the private LEZ claim proves knowledge of (Security #3: a claim can
//! only be completed by whoever holds the seed that produced `c`); `c` is what the vault
//! logs. The same shape serves burns: the burn record on LEZ carries
//! `r = sha256("causeway/burn-recipient/v1" ‖ s_b ‖ recipient)`, opened at release.
//!
//! Hashing is SHA-256 throughout, the hash both the LEZ guest (RISC Zero's accelerated
//! `sha2`) and the Logos wallet SDK compute cheaply. The `cross_language_vector` test below
//! pins a vector that `contracts/test/recovery.test.mjs` reproduces with `ethers.sha256`,
//! so a Rust client and a JavaScript client can never drift apart.

use alloy_primitives::{Address, B256};
use sha2::{Digest, Sha256};

const SECRET_DOMAIN: &[u8] = b"causeway/deposit-secret/v1";
const COMMITMENT_DOMAIN: &[u8] = b"causeway/deposit-commitment/v1";
const BURN_SECRET_DOMAIN: &[u8] = b"causeway/burn-secret/v1";
const BURN_RECIPIENT_DOMAIN: &[u8] = b"causeway/burn-recipient/v1";

/// How many consecutive unused nonces end a scan (BIP-44 uses 20 for addresses).
pub const DEFAULT_GAP_LIMIT: u32 = 20;

/// The public coordinates a deposit's secret is bound to. Binding the chain and the vault
/// means a seed never produces the same commitment on two deployments (F#15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coordinates {
    pub chain_id: u64,
    pub vault: Address,
    pub token: Address,
}

fn derive(domain: &[u8], seed: &B256, at: &Coordinates, nonce: u32) -> B256 {
    let mut h = Sha256::new();
    h.update(domain);
    h.update(seed);
    h.update(at.chain_id.to_be_bytes());
    h.update(at.vault);
    h.update(at.token);
    h.update(nonce.to_be_bytes());
    B256::from_slice(&h.finalize())
}

/// The deposit secret for `nonce`: the opening of the commitment, never leaves the client.
pub fn deposit_secret(seed: &B256, at: &Coordinates, nonce: u32) -> B256 {
    derive(SECRET_DOMAIN, seed, at, nonce)
}

/// The commitment the vault logs for that deposit.
pub fn deposit_commitment(secret: &B256) -> B256 {
    let mut h = Sha256::new();
    h.update(COMMITMENT_DOMAIN);
    h.update(secret);
    B256::from_slice(&h.finalize())
}

/// The burn secret for `nonce`, and the recipient commitment the LEZ burn record carries.
pub fn burn_secret(seed: &B256, at: &Coordinates, nonce: u32) -> B256 {
    derive(BURN_SECRET_DOMAIN, seed, at, nonce)
}

pub fn burn_recipient_commitment(secret: &B256, recipient: &Address) -> B256 {
    let mut h = Sha256::new();
    h.update(BURN_RECIPIENT_DOMAIN);
    h.update(secret);
    h.update(recipient);
    B256::from_slice(&h.finalize())
}

/// One `Deposit` log as the client reads it back from the vault (index, token, amount,
/// commitment). Amount is left as the raw 256-bit value the log carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositLog {
    pub index: u64,
    pub token: Address,
    pub amount: alloy_primitives::U256,
    pub commitment: B256,
}

/// A deposit that belongs to this seed, with everything the claim needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedDeposit {
    pub log: DepositLog,
    pub nonce: u32,
    pub secret: B256,
}

/// Rediscover this seed's deposits of `at.token` among the vault's logs. Scans nonces from
/// zero, stopping after `gap_limit` consecutive nonces with no match, so a client restored
/// from the seed alone finds every deposit it ever made (Reliability #5). Whether each one
/// was already claimed is answered on LEZ by its nullifier, not here.
pub fn recover_deposits(seed: &B256, at: &Coordinates, logs: &[DepositLog], gap_limit: u32) -> Vec<OwnedDeposit> {
    let mut found = Vec::new();
    let mut gap = 0u32;
    let mut nonce = 0u32;
    while gap < gap_limit {
        let secret = deposit_secret(seed, at, nonce);
        let commitment = deposit_commitment(&secret);
        let mut hit = false;
        for log in logs.iter().filter(|l| l.token == at.token && l.commitment == commitment) {
            found.push(OwnedDeposit { log: *log, nonce, secret });
            hit = true;
        }
        if hit {
            gap = 0;
        } else {
            gap += 1;
        }
        nonce += 1;
    }
    found.sort_by_key(|d| d.log.index);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, U256};

    fn at() -> Coordinates {
        Coordinates {
            chain_id: 1,
            vault: address!("0000000aa232009084bd71a5797d089aa4edfad4"),
            token: address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"),
        }
    }

    #[test]
    fn derivation_is_deterministic_and_bound_to_the_coordinates() {
        let seed = B256::repeat_byte(0x42);
        let a = deposit_commitment(&deposit_secret(&seed, &at(), 0));
        assert_eq!(a, deposit_commitment(&deposit_secret(&seed, &at(), 0)));
        assert_ne!(a, deposit_commitment(&deposit_secret(&seed, &at(), 1)));
        let other_chain = Coordinates { chain_id: 11_155_111, ..at() };
        assert_ne!(a, deposit_commitment(&deposit_secret(&seed, &other_chain, 0)));
        let other_seed = B256::repeat_byte(0x43);
        assert_ne!(a, deposit_commitment(&deposit_secret(&other_seed, &at(), 0)));
    }

    #[test]
    fn a_restored_client_finds_exactly_its_own_deposits() {
        let seed = B256::repeat_byte(0x42);
        let stranger = B256::repeat_byte(0x99);
        // The vault's log: the user's deposits at nonces 0, 1 and 3 (nonce 2 was never
        // used), interleaved with other people's.
        let mine = |nonce: u32, index: u64| DepositLog {
            index,
            token: at().token,
            amount: U256::from(100_000_000u64),
            commitment: deposit_commitment(&deposit_secret(&seed, &at(), nonce)),
        };
        let theirs = |nonce: u32, index: u64| DepositLog {
            index,
            token: at().token,
            amount: U256::from(100_000_000u64),
            commitment: deposit_commitment(&deposit_secret(&stranger, &at(), nonce)),
        };
        let logs = vec![theirs(0, 0), mine(0, 1), theirs(1, 2), mine(1, 3), theirs(2, 4), mine(3, 5), theirs(3, 6)];

        let found = recover_deposits(&seed, &at(), &logs, DEFAULT_GAP_LIMIT);
        assert_eq!(found.iter().map(|d| (d.nonce, d.log.index)).collect::<Vec<_>>(), vec![(0, 1), (1, 3), (3, 5)]);
        // And the stranger's seed finds the stranger's, never the user's.
        let others = recover_deposits(&stranger, &at(), &logs, DEFAULT_GAP_LIMIT);
        assert_eq!(others.iter().map(|d| d.log.index).collect::<Vec<_>>(), vec![0, 2, 4, 6]);
        // A deposit of another token at the same coordinates is not confused in.
        let other_token = Coordinates { token: address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"), ..at() };
        assert!(recover_deposits(&seed, &other_token, &logs, DEFAULT_GAP_LIMIT).is_empty());
    }

    #[test]
    fn the_gap_limit_ends_the_scan() {
        let seed = B256::repeat_byte(0x42);
        // One deposit far past the gap limit is, by design, not found: that is the trade-off
        // every gap-limit scheme makes, and the client exposes the limit as a setting.
        let far = DepositLog {
            index: 0,
            token: at().token,
            amount: U256::from(1u64),
            commitment: deposit_commitment(&deposit_secret(&seed, &at(), DEFAULT_GAP_LIMIT + 5)),
        };
        assert!(recover_deposits(&seed, &at(), &[far], DEFAULT_GAP_LIMIT).is_empty());
        assert_eq!(recover_deposits(&seed, &at(), &[far], DEFAULT_GAP_LIMIT + 10).len(), 1);
    }

    #[test]
    fn burn_recipient_commitment_binds_the_recipient() {
        let seed = B256::repeat_byte(0x42);
        let s = burn_secret(&seed, &at(), 0);
        let alice = address!("1111111111111111111111111111111111111111");
        let bob = address!("2222222222222222222222222222222222222222");
        assert_ne!(burn_recipient_commitment(&s, &alice), burn_recipient_commitment(&s, &bob));
    }

    /// The vector `contracts/test/recovery.test.mjs` reproduces with `ethers.sha256`.
    /// Pinned so a change to the derivation is a deliberate, visible decision in both
    /// languages at once.
    #[test]
    fn cross_language_vector() {
        let seed = B256::repeat_byte(0x42);
        let s = deposit_secret(&seed, &at(), 7);
        let c = deposit_commitment(&s);
        assert_eq!(format!("{s}"), "0xcb8933db0ce443e07872f406f53ccae8299a3d65f1f4f521effb0c288b2ed653");
        assert_eq!(format!("{c}"), "0x5b6c608b41ea9a69d4c57ddf44c3667a067771cf5b8d6874c219fbf56b761dbe");
    }
}
