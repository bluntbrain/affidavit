# Costs

Cost measurement is a primary output of RFP-021 (Performance #1 to #9). This page has
what is measured in this prototype, how to reproduce it, and what the grant measures on
LEZ testnet. Every number here comes from a command in this repository.

## How LEZ charges

From LEZ `main` (`lez/programs/fee/core`):

- **Execution gas is metered cycles.** A public transaction declares a `gas_limit` that is
  its cycle budget; `MAX_GAS_EXEC = 10,000,000` per transaction, and a block's execution
  total is capped at the same 10,000,000 (target 5,000,000).
- **A private transaction costs a fixed `PRIVATE_VERIFY_GAS = 409,764`** of execution gas
  (STARK receipt verification, RISC Zero 3.0.5; marked provisional in LEZ) plus storage gas
  for the proof bytes (about 224 KB; `MAX_GAS_STOR = 1,000,000` per block).
- Private executions are not cycle-capped: they cost the submitter proving time instead.

## Verification cycles in the RISC Zero zkVM

`zkvm/` builds `affidavit-core` unchanged into a RISC Zero guest (risc0-zkvm 3.0.5, upstream
nightly Rust's `riscv32im-risc0-zkvm-elf` target with `build-std`, linked with the v1compat
kernel) and runs it in the RISC Zero executor on the mainnet fixtures. RISC Zero's
accelerated forks are patched in: `bls12_381` and `blst` on the bigint2 coprocessor,
`sha2` on the SHA-256 accelerator, `k256`, `crypto-bigint`. Keccak runs in software.

```console
zkvm/build-guest.sh                                        # zkcrypto guest
CARGO_TARGET_DIR=target-blst zkvm/build-guest.sh --features blst   # blst guest
cd zkvm/host && RISC0_SKIP_BUILD_KERNELS=1 cargo run --release     # executor only
```

{{CYCLES}}

## Gas per action

{{GAS}}

## Amortised cost per attestation (Performance #3)

{{AMORTISED}}

## Storage growth (Performance #7)

Measured borsh sizes (`lez/program/tests/program.rs::storage_sizes`):

| Account | Size | Grows? |
|---|---:|---|
| Instance | 841 bytes today, at most 4,585 bytes (64 recent roots, 64 accumulator peaks) | no: bounded |
| Header record | 248 bytes | one per finalised header the instance records |

| Cadence | Records per year | 1 year | 5 years |
|---|---:|---:|---:|
| one finalised header per hour (recommended) | ≈ 8,800 | ≈ 2.2 MB | ≈ 11 MB |
| one per finalised epoch (the maximum useful) | ≈ 82,200 | ≈ 20 MB | ≈ 102 MB |
| one per committee period (handoffs only) | ≈ 321 | ≈ 80 KB | ≈ 0.4 MB |

No action reads more than a bounded amount of it (#P9): an update reads the instance (at
most 4.6 KB) and the 512 keys it is given; a public attestation reads one record; a
private attestation reads nothing but the instance's ring of recent roots, and carries a
membership proof whose length grows with the logarithm of the record count (16 hashes at
65,536 records, 17 at 131,072).

## End-to-end latency (Performance #5)

From an Ethereum event to a statement usable on LEZ, at the recommended settings:

| Stage | Typical | Driven by |
|---|---|---|
| Finality on Ethereum | about 12.8 min (two epochs) | Ethereum |
| Finality update available on beacon nodes | within a slot or two | beacon nodes |
| Operator deferral | median 49 s (public path, W = 300 s); about 5 min (private path, W = 600 s, including proving) | `OPERATOR.md` |
| Proving the update (private path) | about 2 min on a GPU, about 20 min on a laptop CPU | operator hardware |
| Inclusion on LEZ | one block | LEZ |
| Consumer's own transaction | proving time of the consumer path on the user's device (below) | consumer |

So roughly 15 to 20 minutes from a deposit to a usable statement, dominated by Ethereum
finality, when an operator is running at the recommended cadence. A consumer that needs a
specific block sooner than the hourly cadence can submit the update itself.

## Off-chain compute (Performance #6)

| Component | CPU | Memory | Time |
|---|---|---|---|
| Native verification of an update (CLI, daemon dry run; blst) | one core | < 100 MB | about 40 ms |
| Update daemon, public path | negligible | committee keys: about 25 KB per period | — |
| Update daemon, private path | a RISC Zero prover | {{PROVER_MEMORY}} | about 2 min (GPU) to 20 min (CPU) per update |

## In-program BLS versus a precompile (Performance #8)

{{PRECOMPILE}}
