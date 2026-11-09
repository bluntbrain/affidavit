# Live run: Affidavit on a running LEZ v0.4.0-rc1 sequencer

The state-machine test in [`../`](../README.md) drove the program through `lee::V03State` in
process. This directory is the next step up: the same `affidavit_program`, **deployed to and
driven through a running LEZ sequencer** over its RPC, with LEZ's own wallet library building,
proving, fee-declaring and submitting every transaction. Each step below is a transaction the
sequencer accepted into a block and settled.

What it is: the standalone sequencer of LEZ `v0.4.0-rc1` (commit `1806e873`), built with its
`standalone` feature (Bedrock mocked) and `prove` feature (in-process guest execution), on
the repository's `networks/local.json` genesis (15-second blocks). What it is not: a public
testnet, and (in the run below) not real proofs. Both are separate claims; see
[*What this does and does not show*](#what-this-does-and-does-not-show).

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
- **Dev-mode proofs.** Sequencer and wallet ran with `RISC0_DEV_MODE=1`, so the receipts
  the sequencer verified were dev-mode placeholders. Everything else is real. The
  cryptographic proof is covered separately: the 77-minute real proof of `Verify` in
  [`../../docs/COSTS.md`](../../docs/COSTS.md), and the real-proof live run below once it
  completes.
- **Standalone, not consensus.** Bedrock is the mocked actor of the `standalone` feature:
  no proof-of-leadership, data availability or gossip peers. Those sit below the execution
  layer and do not change how a transaction is executed or how state is mutated.
- **Local, not the public testnet.** Nothing here is a public-testnet claim.

## Real-proof run (`real-mode/`, in progress)

Same pipeline with `RISC0_DEV_MODE` unset on both sides: the sequencer verifies real
receipts; the wallet proves for real. The funding transfer, `Initialize` and one private
`Verify` (update 0) are run; a real `Verify` proof took 77 minutes on this 4-vCPU machine
in the standalone measurement, plus the privacy-preserving circuit's own proof. Results are
added to `real-mode/` when the run completes.

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
