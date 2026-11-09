# Live run: Affidavit on a running LEZ v0.4.0-rc1 sequencer

The state-machine test in [`../`](../README.md) drove the program through `lee::V03State` in
process. This directory is the next step up: the same `affidavit_program`, **deployed to and
driven through a running LEZ sequencer** over its RPC, with LEZ's own wallet library building,
proving, fee-declaring and submitting every transaction. Each step below is a transaction the
sequencer accepted into a block and settled.

What it is: the standalone sequencer of LEZ `v0.4.0-rc1` (commit `1806e873`), built with its
`standalone` feature (Bedrock mocked) and `prove` feature (in-process guest execution), on
the repository's `networks/local.json` genesis (15-second blocks). Two runs: the **whole chain with dev-mode proofs**, and **real proofs** (the sequencer verifying
real receipts) for the funding transfer, `Initialize` and one private `Verify`. What it is
not: a public testnet. See [*What this does and does not show*](#what-this-does-and-does-not-show).

## Result (dev-mode proofs, `dev-mode/`)

Program account **`3VBG9ZC9xXVg3jcKzu1N29bjd4HvSafFTZrSizFLWvmn`** (907,108-byte ELF, 939,532
bytes wrapped, uploaded as 10 `program_loader` segments plus a header, 168 s). Instance PDA
**`5kkzHnNwGPAdUeqF2j4AW2QvtZao7FQetPqLwpD1KruM`**. Fee payers: the genesis test account for
public transactions, a private account funded by a privacy-preserving transfer for the
private ones. Wallet `gas_limit` 10,000,000 (the sequencer's per-transaction maximum).

| step | tx | block | what landed |
|---|---|---|---|
| `Initialize` (public) | `e988189b…072d54` | 80 | instance PDA created, bootstrap header recorded: finalized slot 7,069,376, period 862 |
| update 0, private `Verify` → public `Apply` | `81d956ea…70e9fa` | 81 | committee handoff, `updates_applied` 1 |
| update 1 | `8ea39b18…fc7a9a1` | 82 | `Apply` + `Record`: slot 7,070,047 (Ethereum block 17,883,995), record PDA `34QNsokB…` |
| update 2 | `4c662a6c…e7287c` | 83 | slot 7,078,240 (17,892,141), `F9Pxdtpv…` |
| update 3 | `0f704da7…5cc418` | 84 | slot 7,089,280 (17,903,093), `CU3BdVgU…` |
| update 4 | `e48706e2…3aafc0` | 85 | slot 7,094,272 (17,908,060), `82b3GNNW…` |
| update 5 | `f8dacda3…5922c3` | 86 | slot 7,104,096 (17,917,816), `8ag3c5kk…` |
| update 6 (finality update) | `9b50566a…415089` | 87 | slot 7,109,344 (17,923,026), `AMbZV9ZQ…`; **7 updates applied, 7 header records, on-chain instance == native follower**, accumulator root `0xcc69d515…cde958` |
| `AttestRecord` (public consumer path) | `826075bd…a72c46` | 88 | accepted at the head record (167 transactions in that Ethereum block); statement id in `driver-stdout.log` |
| `AttestRecord`, predicate `value > 2^256−1` | `1a73914d…d3bc8` | 89 | **reverted** by the sequencer (`reverted: [0]` in the block, `AFF-301 predicate_not_satisfied` in `sequencer.log`); record unchanged |
| `AttestPrivate` → `CheckRoot` (private consumer path) | `94401516…c9ad1d` | 90 | accepted; the only public crossing is the accumulator root; instance bytes unchanged |

Full hashes, timings and PDAs: [`dev-mode/result.json`](dev-mode/result.json); the driver's
output: [`dev-mode/driver-stdout.log`](dev-mode/driver-stdout.log); the sequencer's log:
[`dev-mode/sequencer.log`](dev-mode/sequencer.log). Wall-clock for steps 1–4 with the program
already deployed: **156 s**, of which almost all is the 15-second block cadence (each
privately-proven update settled in the next block, ≈13.5 s end to end).

### Two things the live sequencer taught that the state machine could not

1. **Gas.** The wallet's default `gas_limit` is 2,000,000; `Initialize` meters at
   **2,543,172 cycles**. The first attempt was included in block 39 and silently had no effect
   (out of gas). A dry run with the limit raised showed the cost; the sequencer then refused
   `gas_limit 40000000 above max 10000000` (`fee_core::market::MAX_GAS_EXEC`). At the
   10,000,000 cap everything landed ([`dev-mode/gas-findings.txt`](dev-mode/gas-findings.txt)).
2. **The cap decides the architecture.** A *public* `Verify` meters at 14.6M cycles, above the
   10M per-transaction maximum, so on this network configuration the public path **cannot**
   be one transaction: it has to be split. The private path's public part is `Apply` +
   `Record` (≈80K cycles), which is why one privately-proven transaction per update works
   here, and why the proposal's "one private transaction" design is not a stylistic choice.

## What this does and does not show

- **It does** show the real program, deployed through LEZ's `program_loader`, driven through
  LEZ's real sequencer (mempool, fee market, block production, execution, state) with LEZ's
  real wallet: PDA admission by seed, the private→public `from_own_program()` crossing, fee
  declarations and payers, write-once header records, both consumer paths, and a refused
  consumer transaction reverted by the sequencer and reflected in the block.
- **Dev-mode proofs in the full-chain run.** There, sequencer and wallet ran with
  `RISC0_DEV_MODE=1`, so the receipts the sequencer verified were placeholders; everything
  else was real. The real-proof run removes that caveat for the funding transfer,
  `Initialize` and update 0, with the sequencer verifying real receipts.
- **Standalone, not consensus.** Bedrock is the mocked actor of the `standalone` feature:
  no proof-of-leadership, data availability or gossip peers. Those sit below the execution
  layer and do not change how a transaction is executed or how state is mutated.
- **Local, not the public testnet.** Nothing here is a public-testnet claim.

## Result (real proofs, `real-mode/`)

Same pipeline on a fresh sequencer with `RISC0_DEV_MODE` **unset on both sides**: the
sequencer verifies real receipts (its log carries no dev-mode line), the wallet proves for
real. Program account **`4mLuS85wQVXr5CniXpioaWeTEaMjNuWx19U7TWUP5F9S`**, instance PDA
**`FDpgXb7UX49JjcZrpxQpNbnUEVo91W5RhLwRAMSySNR1`**.

| step | tx | block | proof | time |
|---|---|---|---|---|
| funding transfer to the private fee payer (privacy-preserving) | `ae89c64b…a79be9` | 70 | real | ≈17 min to prove and land |
| deploy (10 segments + header, public) | `0ef4f102…` … `f9372860…` | 71–81 | — | 156 s |
| `Initialize` (public) | `726ad58c…c74d93` | 82 | — | 24 s |
| **update 0, private `Verify` → public `Apply`** | **`0a1f4b68…f7f9e2`** | **559** | **real** | **7,156 s (119 min) from proving start to inclusion** |

Peak resident memory observed during the `Verify` proof: ≈8.5 GB; CPU time ≈7.5 hours over
the 2-hour wall-clock on 4 vCPUs. The standalone proof of the same `Verify` was 77 minutes
([`../../docs/COSTS.md`](../../docs/COSTS.md)); the difference is the privacy-preserving
circuit's own proof over the program receipt, which the wallet adds so the sequencer can
verify the transaction. On a GPU prover both shrink to minutes; the sequencer-side
verification is the same regardless.

This is the headline claim with the placeholder removed: **a really-proven private `Verify`,
submitted over RPC, verified by a running LEZ v0.4.0-rc1 sequencer, advanced the public
instance** (`updates_applied` 0 → 1). Only update 0 was run this way; the other six updates
and both consumer paths are in the dev-mode run above. Evidence: [`real-mode/result.json`](real-mode/result.json),
[`real-mode/driver-stdout.log`](real-mode/driver-stdout.log),
[`real-mode/sequencer-filtered.log`](real-mode/sequencer-filtered.log),
[`real-mode/pipeline-summary.txt`](real-mode/pipeline-summary.txt).

## Reproduce

Needs LEZ at tag `v0.4.0-rc1` beside this repository, this repository's program ELF
(`lez/build-program.sh`), stable Rust, `libpcsclite-dev` + `pkg-config` (the wallet's keycard
dependency), and ~15 GB RAM for in-process execution and proving.

```console
# 1. the LEZ workspace, with the live driver added as a member
git clone --branch v0.4.0-rc1 https://github.com/logos-blockchain/logos-execution-zone lez-src
mkdir -p lez-src/affidavit_live/src
cp m0/live/driver.rs         lez-src/affidavit_live/src/main.rs
cp m0/live/driver-Cargo.toml lez-src/affidavit_live/Cargo.toml
#   add "affidavit_live" to lez-src/Cargo.toml [workspace].members and unify lee_core:
#   [patch."https://github.com/logos-blockchain/logos-execution-zone"]
#   lee_core = { path = "lee/state_machine/core" }

# 2. the consensus circuits' prebuilt artifact (the build script downloads it itself where
#    its HTTP client can reach GitHub releases; otherwise fetch it and point LBC_ROOT_DIR at it)
curl -L -o circuits.tar.gz https://github.com/logos-blockchain/logos-blockchain-circuits/releases/download/v0.5.7/logos-blockchain-circuits-v0.5.7-linux-x86_64.tar.gz
mkdir -p lbc && tar -xzf circuits.tar.gz -C lbc
export LBC_ROOT_DIR=$PWD/lbc/logos-blockchain-circuits-v0.5.7-linux-x86_64

# 3. build the sequencer (standalone + in-process execution), the wallet and the driver
cd lez-src && cargo build --release -p sequencer_service --features sequencer_service/standalone,sequencer_service/prove -p wallet -p program_deployment -p affidavit_live

# 4. run (scripts/ holds the exact commands: env.sh, run-sequencer.sh, setup-wallet.sh, run-driver.sh)
#    sequencer: setup with networks/local.json + --create-channel, then start (RISC0_DEV_MODE=1 for dev mode)
#    wallet:    change-network local; import the genesis test key; new private account; fund it; gas_limit 10000000
#    driver:    affidavit_live <program ELF> <payer Public id> <verifier Private id>, same RISC0_DEV_MODE as the sequencer
```

[`versions.txt`](versions.txt) records the exact toolchain, crate versions and ELF hash.
