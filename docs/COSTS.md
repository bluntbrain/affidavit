# Costs

Cost measurement is a primary output of RFP-021 (Performance #1 to #9). Everything below
is measured in this repository on the mainnet fixtures, except where a line says
*estimate*, which says what it is derived from. On LEZ, public execution gas is cycles,
so the cycle counts are the gas. Raw outputs are in [`raw/`](raw/).

## How LEZ charges

From LEZ `main` (`lez/programs/fee/core`):

- **Execution gas is metered cycles.** `MAX_GAS_EXEC = 10,000,000` per transaction, and a
  block's execution total is capped at the same 10,000,000 (target 5,000,000).
- **A private transaction costs a fixed `PRIVATE_VERIFY_GAS = 409,764`** of execution gas
  (STARK receipt verification, RISC Zero 3.0.5; marked provisional in LEZ), plus storage
  gas for about 224 KB of proof (`MAX_GAS_STOR = 1,000,000` per block).
- **Private executions are not cycle-capped** (`Simulator::handle_message`): they cost
  the sender proving time instead.

## The LEZ program, per action

The program binary (`lez/build-program.sh`, LEZ `lee_core` at v0.4.0-rc1, blst through
RISC Zero's accelerated fork) executed in the RISC Zero executor with LEZ's own input
framing (`to_borsh_frame` of a `ReceiveInput`):

```console
lez/build-program.sh
cd zkvm/host && RISC0_SKIP_BUILD_KERNELS=1 cargo run --release --bin measure_lez
```

| Action | Runs | User cycles | Padded |
|---|---|---:|---:|
| `Initialize` (bootstrap check, once per instance) | public | 2,523,572 | 3,145,728 |
| `Verify`, update with committee handoff | **private**, proven by the submitter | **14,583,345** | 15,859,712 |
| `Verify`, finality update | **private**, proven by the submitter | **12,251,079** | 13,631,488 |
| `Apply` (instance compare-and-swap) | public | 55,296 to 58,786 | 131,072 |
| `Record` (write-once header record) | public | 19,600 | 65,536 |
| `AttestRecord`, native ETH transfer | public, in the consumer's transaction | 813,571 | 1,048,576 |
| `AttestPrivate`, native ETH transfer | private, proven by the user | 818,024 | 1,048,576 |
| `CheckRoot` (the private path's one public crossing) | public | 27,855 | 131,072 |

The fixture's updates are signed by 510 to 512 of 512 members. Each non-signer adds about
12,000 cycles to `Verify` (a witness check and a point subtraction; see the phases below),
so an update at the 342/512 minimum costs about 2M cycles more.

## Gas per action

| Action | Path | LEZ execution gas | Transactions |
|---|---|---:|---|
| Advance the instance | **private verifier** (preferred) | 409,764 + `Apply` ≈ 57,000 + `Record` 19,600 ≈ **487,000**, plus about 224 KB storage gas | **1** |
| Advance the instance | public verifier (fallback) | ≈ 14,700,000 with a handoff, ≈ 12,300,000 without | at least 2, in at least 2 blocks (10M cap per transaction and per block) |
| Attest, public consumer | public | 600,000 to 1,300,000 (813,571 for a native transfer) | inside the consumer's own |
| Attest, private consumer | private | 409,764 + `CheckRoot` 27,855 | inside the consumer's own |

So the private-verifier path costs the sequencer **25 to 30 times less** per update than
the public one, and fits one transaction in one block.

## Proving time

Measured: the program's `AttestPrivate` action, proven with RISC Zero 3.0.5's CPU prover
into a succinct receipt and verified against the program's image id, on a 4-vCPU cloud VM
(Xeon, 2.8 GHz, 15 GB RAM, no GPU; [raw](raw/prove-lez-program-2026-10-16.txt)):

```console
cd zkvm/host && PROVE_ONLY=AttestPrivate CARGO_TARGET_DIR=target-prove cargo run --release --features prove --bin measure_lez
```

| Proof | Segments | Time | Receipt | Peak memory |
|---|---:|---:|---:|---:|
| `AttestPrivate`, native ETH transfer (a private consumer's proof) | 1 (2^20 cycles) | **495 s** | succinct, **222,668 bytes** | 9.4 GB |

The receipt matches the roughly 224 KB of proof LEZ charges storage gas for. The rest is
derived from that measurement and from LEZ's own `cycle_bench` (about 73 µs per padded
cycle on an Apple M2 Pro CPU before recursion, and 18 to 25 s for the privacy-preserving
composition); M0 measures it on target hardware:

| What | Padded cycles | 4-vCPU VM | Recent laptop CPU | GPU |
|---|---:|---:|---:|---|
| Private consumer attestation | 1.05M (1 segment) | 8.3 min, *measured* | about 2 to 3 min, *estimate* | seconds, *assumed* |
| Update with handoff, private verifier | 15.9M (16 segments) | about 2.5 h, *extrapolated* | about 20 to 40 min, *estimate* | 1.5 to 2.5 min, *assumed* |
| Finality update, private verifier | 13.6M (14 segments) | about 2 h, *extrapolated* | about 17 to 35 min, *estimate* | same |

So an operator on the private path needs a GPU, a fast desktop or a proving service
(proving is trustless to outsource) to keep an hourly cadence; any machine with 16 GB of
memory keeps up with the one handoff a day that an instance cannot do without.

## Verification core, phase by phase

`zkvm/` builds `affidavit-core` unchanged into a measurement guest (upstream nightly
Rust's `riscv32im-risc0-zkvm-elf` target with `build-std`, v1compat kernel) with RISC
Zero's accelerated `bls12_381`/`blst`, `sha2`, `k256` and `crypto-bigint`; keccak in
software.

```console
zkvm/build-guest.sh && CARGO_TARGET_DIR=target-blst zkvm/build-guest.sh --features blst
cd zkvm/host && RISC0_SKIP_BUILD_KERNELS=1 cargo run --release --bin affidavit-zkvm-host
```

| `verify_update`, real mainnet update with a handoff | blst | zkcrypto |
|---|---:|---:|
| A straight port: decompress and subgroup-check every signer's key | 239,908,734 | 549,236,839 |
| All 512 keys as uncompressed witnesses (earlier design, [raw](raw/measure-core-2026-10-16-all-keys-as-witnesses.txt)) | 18,031,068 | 33,823,787 |
| **Committed aggregate minus witnessed non-signers** (the program's path, `DECISIONS.md` D3), 512/512 signed | **13,652,631** | 27,608,953 |
| Same update cut to the 342/512 minimum: 170 non-signers | 15,648,098 | 29,729,640 |
| Finality update without handoff, 512/512 signed | 11,717,409 | 25,674,089 |

The 342/512 row clears signature bits of the real update, so its signature is refused;
every step still runs in full, so its cycles are those of a valid update.

Phases of the update with handoff, measured with the zkcrypto primitives:

| Phase | Cycles |
|---|---:|
| Committee root (512 keys, SSZ) | 1,928,665 |
| Execution, finality and handoff branches | 2,012,745 |
| Aggregate key and non-signer witnesses checked: 512/512 signed | **3,945** |
| — at 342/512 (170 non-signers) | 567,267 |
| Signers' key: aggregate minus non-signers, 512/512 signed | **1,795** |
| — at 342/512 | 1,548,338 |
| *for comparison:* decompress and subgroup-check every signer's key | *517,156,501* |
| *for comparison:* add up 512 signers' keys | *4,535,026* |
| Hash to G2 | 2,748,192 |
| Signature decode and subgroup check | 1,426,154 |
| Miller loops (2) | 8,745,906 |
| Final exponentiation | 10,693,562 |

Consumer paths (`attest_with_membership`, including a 16-level membership proof in a
65,536-header accumulator): native ETH transfer (SSZ transaction proof and signature
recovery) 801,229; ERC-20 log (receipt MPT proof) 591,001; storage slot (account and
storage MPT proofs) 1,280,030. Keccak is in software here; RISC Zero's keccak
accelerator would cut the MPT paths further.

## Amortised cost per attestation (Performance #3)

At the recommended cadence (one finality advance an hour plus the daily handoff, about 25
updates a day), tracking costs about 25 × 487,000 ≈ **12.2M gas a day** on the private
path, whatever the number of consumers. Per attestation, that is 12.2M / *N* for *N*
attestations a day (122,000 gas at 100 a day), on top of the attestation's own
0.6M to 1.3M. Tracking is the minor part of an attestation's cost from about 20
attestations a day upward. The public fallback would cost 25 to 30 times more to track.

## Storage growth (Performance #7)

Measured borsh sizes (`lez/program/tests/program.rs::storage_sizes`):

| Account | Size | Grows? |
|---|---:|---|
| Instance | 841 bytes today, at most 4,585 (64 recent roots, 64 accumulator peaks) | no: bounded |
| Header record | 248 bytes | one per finalised header the instance records |

| Cadence | Records per year | 1 year | 5 years |
|---|---:|---:|---:|
| one finalised header per hour (recommended) | ≈ 8,800 | ≈ 2.2 MB | ≈ 11 MB |
| one per finalised epoch (the maximum useful) | ≈ 82,200 | ≈ 20 MB | ≈ 102 MB |
| one per committee period (handoffs only) | ≈ 321 | ≈ 80 KB | ≈ 0.4 MB |

No action reads more than a bounded amount (#P9): an update reads the instance (at most
4.6 KB) and the keys it is given; a public attestation reads one record; a private
attestation reads only the instance's ring of recent roots and carries a membership proof
that grows with the logarithm of the record count (16 hashes at 65,536 records).

## End-to-end latency (Performance #5)

| Stage | Typical | Driven by |
|---|---|---|
| Finality on Ethereum | about 12.8 min (two epochs) | Ethereum |
| Finality update on beacon nodes | within a slot or two | beacon nodes |
| Operator deferral, proving and submission | median 49 s (public path, W = 300 s); median about 5 min (private path, W = 600 s with hints) | `OPERATOR.md` |
| Inclusion on LEZ | one block | LEZ |
| Consumer's own transaction | public: one block; private: proving about 2 to 3 min on a laptop | consumer |

About 15 to 25 minutes from an Ethereum event to a usable statement, dominated by
Ethereum's own finality, when an operator runs at the recommended cadence. A consumer that
needs a block sooner than the hourly cadence can submit the update itself.

## Off-chain compute (Performance #6)

| Component | Needs | Time |
|---|---|---|
| Native verification of an update (CLI, daemon dry run; blst) | one core, under 100 MB | about 40 ms |
| Update daemon, public path | committee keys, about 25 KB per period | negligible |
| Update daemon, private path | a RISC Zero prover, 16 GB of memory | minutes per update on a GPU, about 20 to 40 min on a recent laptop CPU (*estimate*) |

## In-program BLS versus a precompile (Performance #8)

Of the 13.7M cycles of a blst update, about 9.7M are BLS12-381 work (hash to G2,
signature decoding, the pairing, the signers' key) and about 3.9M are SSZ hashing that no
precompile would remove. A BLS12-381 precompile would bring a whole update to about 4M
cycles plus its own gas, so it would fit one public transaction. A precompile is
reachable only from public transactions. The private-verifier path already lands an
update in one transaction without one, so a precompile RFP is only worth writing if M0
finds the private placement unavailable on testnet 0.4, or if operators cannot be expected
to run provers.
