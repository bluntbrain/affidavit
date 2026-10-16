//! affidavit-core: verify that something happened on Ethereum, with no trusted party.
//!
//! 1. **Finality** ([`lc`]): an instance follows Ethereum's sync committee from a
//!    weak-subjectivity checkpoint and records the finalised headers it verified.
//! 2. **Inclusion** ([`execution`]): an account, storage slot, log or transaction is proven
//!    under one of those headers.
//! 3. **Predicate** ([`statement`]): a consumer-supplied condition is evaluated over the
//!    proven state, and a [`statement::Statement`] with a deterministic identifier comes out.
//!
//! `no_std` with `alloc`, so the same code runs natively, in the CLI dry run, and inside a
//! RISC Zero guest.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod bls;
pub mod config;
pub mod error;
pub mod execution;
pub mod lc;
pub mod mmr;
pub mod operator;
pub mod ssz;
pub mod statement;
pub mod types;

pub use error::{Error, Result};
