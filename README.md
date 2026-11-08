# affidavit

**Trustless Ethereum state attestation for Logos LEZ.** A LEZ program that follows
Ethereum's sync committee from a weak-subjectivity checkpoint, proves that an account,
storage slot, log or ETH transfer is included under a finalised header, evaluates a
consumer's predicate over it, and returns a verified statement. Nobody is trusted after
initialisation: operators, RPC providers and relayers can delay, never lie.

Prototype for [Logos RFP-021](https://github.com/logos-co/rfp/blob/master/RFPs/RFP-021-ethereum-state-attestation.md).

```console
$ cargo run --release -p affidavit-cli -- demo
```

runs the whole pipeline, and the attacks it refuses, on real Ethereum mainnet data in a
few seconds (output below).

## What works today

All of it is tested on **real mainnet data** (fixtures from [Helios](https://github.com/a16z/helios),
MIT) and the **official consensus-spec-tests vectors**; `cargo test --workspace` runs 38 tests,
and the LEZ program has 5 more.

| | |
|---|---|
| **Light client** | Bootstrap from the published checkpoint of slot 7,069,376, five committee handoffs (periods 862 → 867), finality at slot 7,109,344 / block 17,923,026. BLS aggregate verified with **blst** and **zkcrypto `bls12_381`**, which agree on every update. Deneb and Electra spec vectors pass: every update the instance accepts lands exactly where the reference store does. |
| **Inclusion** | Accounts and storage (`eth_getProof`, MPT via `alloy-trie`), receipts and logs (receipt trie rebuilt and checked against the header root), native ETH transfers (SSZ proof under the payload's `transactions_root` + signature recovery). Real WETH and USDC transfers (2025, Sept 2026) and the deposit contract's balance. |
| **Statements** | Predicates (`==`, `!=`, `<`, `<=`, `>`, `>=`) supplied by the consumer; chain binding; deterministic identifiers for nullifiers. |
| **Attacks refused** | Fabricated chain with every Merkle path repaired, 341/512 signatures, attacker committee, tampered roots, missing finality, replays, skipped periods, stale checkpoint, other networks, uncovered predicates: one test each, with a typed error code. |
| **LEZ program** | Handlers for LEZ's actor model, building and passing tests against `lee_core` v0.4.0-rc1 (`lez/program`): instance at the PDA of its own configuration, permissionless verification by any actor, compare-and-swap apply, write-once header records, public and private consumer paths. Driven through LEZ's `ReceiveInput`/`Response` over all mainnet updates, it ends in the same state as the native client. |
| **Runs on v0.4's state machine** | The real program driven through LEZ v0.4.0-rc1's actual execution state machine (`lee::V03State`): `Initialize` and a full `Verify → Apply` advance, **including a privately-proven `Verify` that advances the public instance** — the one-transaction design's private→public `from_own_program()` crossing, validated. See [`m0/`](m0/). |
| **Measured cost** | The LEZ program binary itself, built for RISC Zero (upstream nightly + `build-std`, no `rzup`) and executed with LEZ's input framing: `Verify` 14.6M cycles, `Apply` 57K, a consumer attestation 0.6M to 1.3M. Placed privately, an update costs the sequencer about **0.49M gas, 25 to 30× less** than verifying it publicly. Both private actions are proven for real: the whole update into a 222,668-byte succinct receipt (77 min on a 4-vCPU VM, minutes on a GPU). The signers' key is the committee's committed aggregate minus the witnessed non-signers, which took an update from 240M cycles to 13.7M. See [`docs/COSTS.md`](docs/COSTS.md). |
| **Operators** | Deferral, check-before-submit and proving hints, plus a contention simulation: five operators each pay about 20%, wasted submissions fall 30× on the public path, and wasted proofs fall from 400 to 0 per 100 updates on the private path. See [`docs/OPERATOR.md`](docs/OPERATOR.md). |

## The design in four points

1. **Verify, then apply.** `Verify` is pure and can run in any actor of the program;
   `Apply` is a compare-and-swap the instance accepts only from its own program. Placed
   privately, the 15M-cycle verification is proven by the submitter and the sequencer
   checks one receipt instead of executing it: about 0.49M gas per update, one
   transaction, no BLS precompile needed. A public, split fallback uses the same code.
   ([DESIGN §2](docs/DESIGN.md))
2. **The address is the configuration.** An instance lives at the PDA of the hash of its
   chain, checkpoint and limits: nothing can reconfigure it, any number coexist.
3. **Private consumers disclose one shared root.** A header accumulator lets a private
   consumer prove its header is verified without revealing which Ethereum block it acts
   on. ([DESIGN §3](docs/DESIGN.md), Functionality #8)
4. **Identifiers that survive migration.** Statement ids depend on the chain and the fact,
   never on the instance, so nullifiers carry over when a stale instance is replaced.

## Documents

| | |
|---|---|
| [DESIGN](docs/DESIGN.md) | actors, flows, consumer paths, errors |
| [TRUST](docs/TRUST.md) | assumptions, attack cost, checkpoint verification, participants |
| [COSTS](docs/COSTS.md) | cycles per action, amortisation, storage growth, latency |
| [OPERATOR](docs/OPERATOR.md) | running an operator, cadence, contention simulation |
| [CONSUMER](docs/CONSUMER.md) | the recommended consumer pattern |
| [REUSE](docs/REUSE.md) | what is reused, what is written, audit surface |
| [DECISIONS](docs/DECISIONS.md) | design decision log |
| [REQUIREMENTS](docs/REQUIREMENTS.md) | every RFP-021 requirement, status and evidence |

## Layout

```
crates/core   affidavit-core: light client, SSZ, inclusion, statements (no_std + alloc)
crates/io     Beacon API / JSON-RPC / spec-test decoding, proof builders, follower
crates/cli    `affidavit` CLI: demo, sync, attest, checkpoint
crates/sim    operator contention simulation
lez/program   the LEZ program (actor model, builds+tests on lee_core v0.4.0-rc1)
zkvm/         RISC Zero measurement guest, and hosts that count cycles for the core and the program
testdata/     mainnet fixtures and consensus-spec-tests vectors (provenance in testdata/README.md)
```

## Run it

```console
cargo test --workspace --release                       # 38 tests: mainnet data, spec vectors, attacks
cargo run --release -p affidavit-cli -- demo           # the narrated pipeline
cargo run --release -p affidavit-sim                   # operator contention tables
cargo test --release --manifest-path lez/program/Cargo.toml   # LEZ program handlers (5 tests)
```

Cycle counts need nightly Rust with `rust-src`, `clang` and `llvm-ar` (no `rzup`, no Docker):

```console
zkvm/build-guest.sh                                     # measurement guest, zkcrypto
CARGO_TARGET_DIR=target-blst zkvm/build-guest.sh --features blst
lez/build-program.sh                                    # the LEZ program binary
cd zkvm/host
RISC0_SKIP_BUILD_KERNELS=1 cargo run --release --bin affidavit-zkvm-host   # core, phase by phase
RISC0_SKIP_BUILD_KERNELS=1 cargo run --release --bin measure_lez           # program, action by action
```

The CLI works on files today (Beacon API and JSON-RPC responses); live fetching goes
through the Logos Ethereum core module in the grant's M3.

## What this prototype is not yet

Not deployed to a LEZ testnet; the program's handlers are tested natively against LEZ's
types rather than through a sequencer. Cycle counts come from the RISC Zero executor; one
private action (a consumer attestation) is proven end to end, and the update's proving
time is extrapolated from it. No Logos core module or update daemon yet (the submission
policy and its simulation are). Receipts for the light-client fixture's block
are not available offline, so the native-transfer demo proves inclusion and sender but not
success; the receipt path is tested on other mainnet blocks. Gloas containers (Sepolia
since 2026-10-06) are not in. These are the milestones in the proposal.

## License

MIT OR Apache-2.0.
