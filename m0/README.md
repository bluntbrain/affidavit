# M0: the private-verifier placement, validated on LEZ v0.4

**Question.** The proposal's main advantage — advancing an instance in **one transaction** —
rests on one LEZ behaviour: a `Verify` actor, executed and proven **privately** by the
submitter, can send `Apply` to the **public** instance, and the instance's
`from_own_program()` check accepts it so its public state advances. The 77-minute real proof
([`docs/COSTS.md`](../docs/COSTS.md)) shows the receipt is producible and self-verifies; it
does **not** show a runtime accepts that private transaction and mutates public state. This
is what M0 settles.

**Result.** Running the real `affidavit_program` through LEZ v0.4's actual execution state
machine (`lee::V03State`, the code the sequencer applies), against `lee_core` **v0.4.0-rc1**:

```
[1] Initialize (public): instance PDA created, finalized slot 7069376, period 862   PASS
[2] Verify -> Apply (public): updates_applied 1, next committee learned true         PASS
    Apply accepted: the instance's from_own_program() authorization held.
[3] Verify -> Apply (PRIVATE): a privately-proven Verify advanced the PUBLIC instance PASS
```

([`run-output.txt`](run-output.txt), produced by [`driver.rs`](driver.rs).)

Step 3 is the one that matters: the submitter runs `Verify` as a **privacy-preserving
transaction** whose private actor emits `Apply` to the public instance; the state machine
delivers it with the sender presented canonically, the instance's `from_own_program()` check
holds across the private→public boundary, and the public instance advances (it learns the
next sync committee). This is the single-transaction advance the proposal is built on,
working on v0.4 with the real program — not a mock, and not inferred from a similar test.

## What this is, and what it is not

- **It is** the real LEZ v0.4 execution state machine (`lee::V03State::transition_from_public_transaction`
  and `transition_from_privacy_preserving_transaction`) — the same code path the sequencer
  applies to settle a block — running the real, unmodified `affidavit_program` guest
  (`lez/build-program.sh`, `lee_core` v0.4.0-rc1) on the mainnet light-client fixtures.
- **Dev mode for the proof only.** The run sets `RISC0_DEV_MODE=1`, so the *cryptographic
  proof* is a dev-mode placeholder. Everything the question is about — the guest execution,
  the private→public message delivery, the `from_own_program()` authorization, and the
  public state transition — runs for real. The *validity* of the proof itself is the
  separately-measured 77-minute real proof in `docs/COSTS.md`; the two together cover the
  claim.
- **State machine, not the full networked sequencer.** The full standalone sequencer could
  not be built in this environment: its consensus-circuit dependency
  (`logos-blockchain-circuits-pol-sys`) downloads prebuilt artifacts from a Logos host the
  sandbox network blocks. That stack (Cryptarchia proof-of-leadership, Bedrock, gossip, RPC)
  sits *below* the execution state machine and does not change how a private transition is
  executed or how public state is mutated — but the end-to-end confirmation on a running
  sequencer, and on the public testnet, is the runbook below, for a machine with network
  access.
- **Local, not a public-testnet deployment.** No claim here is a public-testnet claim.

## Reproduce

Needs a checkout of LEZ at tag `v0.4.0-rc1` and this repo side by side, plus nightly Rust
with `rust-src`, `clang`/`llvm` (to build the program guest), and `~16 GB` RAM (the
in-process executor is pulled in via risc0-zkvm's `prove` feature).

```console
# 1. the program guest, against lee_core v0.4.0-rc1 (this repo)
lez/build-program.sh

# 2. a LEZ v0.4.0-rc1 checkout, with this driver added as a workspace member
git clone --branch v0.4.0-rc1 https://github.com/logos-blockchain/logos-execution-zone lez-src
mkdir -p lez-src/affidavit_m0/src
cp m0/driver.rs        lez-src/affidavit_m0/src/main.rs
cp m0/driver-Cargo.toml lez-src/affidavit_m0/Cargo.toml
#   add "affidavit_m0" to lez-src/Cargo.toml [workspace].members, and unify lee_core:
#   [patch."https://github.com/logos-blockchain/logos-execution-zone"]
#   lee_core = { path = "lee/state_machine/core" }

# 3. run it (dev mode skips only the proof; the mechanism runs for real)
cd lez-src && RISC0_DEV_MODE=1 cargo run -p affidavit_m0
```

The driver's paths assume this repo at `/home/user/affidavit`; adjust if it is elsewhere.

## Runbook: the end-to-end confirmation (on a machine with network access)

This is the full version of step 3, for the user's own machine, where the Logos circuit
host and the public testnet are reachable:

1. Build the standalone sequencer: `just run-bedrock` and `just run-sequencer-standalone`
   (LEZ `v0.4.0-rc1`). In standalone mode Bedrock is mocked, so no live consensus is needed.
2. Deploy `affidavit_program` with the wallet (`wallet deploy-program affidavit_program.bin`).
3. `Init` the instance (a public transaction to the instance PDA).
4. Prove `Verify` privately (`wallet send_privacy_preserving_tx`, real proof ≈ tens of
   minutes on a laptop / minutes on a GPU) and submit it; confirm the instance's `Apply`
   lands and public state advances.
5. Run a public attestation against a recorded header.
6. Repeat against the **public LEZ testnet** (`testnet.lez.logos.co`) with a funded account.
   Record the program address and the transaction hashes; that is a public-testnet result,
   to be reported as such, separate from this local state-machine validation.

## What it means for the proposal

The one open question behind RFP-021's central claim — can a privately-proven `Verify`
advance the public instance in one transaction on LEZ v0.4 — is answered **yes** at the
execution-state-machine level, with the real program. Combined with the 77-minute real proof
of `Verify` and LEZ's own `NestedBoundary` test (a public root → private actor → public
actor crossing, in `lee/state_machine/src/state/tests/privacy_preserving.rs`), the private
placement is substantially de-risked ahead of the milestone that confirms it on a running
sequencer and the public testnet.
