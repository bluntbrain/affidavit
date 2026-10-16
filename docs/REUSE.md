# Reuse assessment

Reliability #6 of RFP-021 asks proposals to integrate mature, audited implementations,
to evaluate what exists, and to state what is reused, what is rewritten, and what that
split does to cost and audit surface. This is that evaluation as of October 2026, and it
is also the plan: the grant's first milestone finishes it with measurements on LEZ
testnet 0.4.

## What exists

| Implementation | What it is | Audit | Fit for a LEZ program |
|---|---|---|---|
| [a16z/helios](https://github.com/a16z/helios) `consensus-core` | Rust light client: bootstrap, update validation and application, Capella to Gloas containers | covered as the logic inside SP1 Helios (Zellic) and r0vm-helios (zkSecurity) | The logic is right, but the crate brings `tree_hash`, `ssz_types`, `superstruct`, alloy RPC types and zkcrypto BLS into the guest, and `verify_update` is one monolithic call: it cannot be split along the pairing's seams for a public path, and the zkcrypto final exponentiation alone exceeds a public transaction. Used here as the **reference**: its fixtures are this repository's test data and its fork tables cross-check ours. |
| [boundless-xyz/r0vm-helios](https://github.com/boundless-xyz/r0vm-helios) | Helios compiled into a RISC Zero guest; operator; Solidity contract that verifies the receipt | zkSecurity | Same zkVM as LEZ. Its guest proves *Helios state transitions for an EVM contract*; LEZ needs a program whose state lives in LEZ accounts. What carries over is the pattern this design uses on the private-verifier path: the update is proven off-chain by any operator and a thin on-chain actor applies the result. Its operator's beacon handling is a reference for the daemon. |
| [succinctlabs/sp1-helios](https://github.com/succinctlabs/sp1-helios) | the same on SP1 | Zellic (July 2025) | Different zkVM. Its execution-header mode (committing the receipts root) confirms the interface decision made here: a header record carries state, receipts *and* transactions roots. |
| Telepathy | circuits; permissionless `step` / `rotate` | Veridise | Primary source for the operator model (anyone may submit). Circuits are not reusable on RISC Zero. |
| [supranational/blst](https://github.com/supranational/blst) | BLS12-381 | NCC Group 2021; formal verification by Galois in progress | **Reused** natively (CLI, daemon dry run) and as RISC Zero's accelerated fork `v0.3.16-risczero.0` for the guest. The library Ethereum's consensus clients sign with. |
| [zkcrypto/bls12_381](https://github.com/zkcrypto/bls12_381) | BLS12-381, pure Rust | not audited (its README says so); RISC Zero's fork `v0.8.0-risczero.1` is what r0vm-helios ships | **Reused** behind the same trait; cross-checked against blst on every real update. Kept because it compiles anywhere without a C toolchain. |
| [alloy-rs/trie](https://github.com/alloy-rs/trie), `alloy-consensus`, `alloy-rlp` | MPT proof verification, receipt and transaction decoding, signature recovery | part of reth (audited by Sigma Prime) | **Reused unmodified** for every execution-layer check. |
| RustCrypto `sha2`, `k256` | SHA-256, secp256k1 | RustCrypto; RISC Zero ships accelerated forks | **Reused** through RISC Zero's forks in the guest. |
| [logos-verified-proxy-module](https://github.com/logos-co/logos-verified-proxy-module) (Nimbus `libverifproxy`) and `eth_rpc_module` | light-client-verified JSON-RPC for Logos | Nimbus's | **Reused** for all Ethereum access by the core module and daemon (a hard dependency of the RFP). |

## What is written here, and why

Under 2,000 lines in `crates/core`, comments and unit tests included:

- **The light-client rules** (`lc.rs`): the Altair `validate_light_client_update` and
  `apply_light_client_update`, restricted to an on-chain instance's needs (no optimistic
  header, no force update, committees kept as roots), with verification separated from
  application so the two can run in different actors. This is the part a reused crate
  could not provide in this shape; it follows the specification line by line and is
  checked against Helios's real mainnet fixtures and the official consensus-spec-tests.
- **SSZ merkleization for six containers** (`ssz.rs`, `types.rs`), checked against the
  published checkpoint root of the fixtures and against the spec vectors.
- **Statements, predicates, identifiers, the header accumulator** (`statement.rs`,
  `mmr.rs`): new by nature, since they are this primitive's interface.

## Consequences for the audit

The audit surface is: the light-client rules and SSZ code above, the statement and
accumulator code, the program shell (`lez/program`), and the accelerated layer of the
RISC Zero forks of the cryptography (RISC Zero's code, under their own review, but in
scope because a consensus verifier depends on it). Pairing arithmetic, MPT verification,
RLP and transaction decoding stay with their audited upstreams. Measured, the generic
crates cost cycles but not risk: alloy's trie verification is what reth runs on every
block.

## What changes if Logos prefers a different split

- **Reuse Helios `consensus-core` verbatim inside the private verifier.** Possible on the
  private-verifier path, where there is no cycle cap: the guest would call Helios's
  `verify_update` and the program would only map its output to an `Advance`. Smaller new
  code, larger binary (and a longer proof). It cannot serve the public fallback.
- **Reuse the r0vm-helios guest and verify its receipt.** Possible if LEZ lets a program
  verify an arbitrary RISC Zero receipt inside a public transaction (composition). That
  would make the audited guest the whole BLS path, at the price of a second proof system
  boundary to reason about. M0 checks whether LEZ 0.4 supports it.
