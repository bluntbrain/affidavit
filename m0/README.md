# M0: the private-verifier placement, validated on LEZ v0.4

**Question.** The proposal's main advantage — advancing an instance in **one transaction** —
rests on one LEZ behaviour: a `Verify` actor, executed and proven **privately** by the
submitter, can send `Apply` to the **public** instance, and the instance's
`from_own_program()` check accepts it so its public state advances. The 77-minute real proof
([`docs/COSTS.md`](../docs/COSTS.md)) shows the receipt is producible and self-verifies; it
does **not** show a runtime accepts that private transaction and mutates public state. This
is what M0 settles — and, once settled, the same run carries the instance through the whole
mainnet fixture chain and exercises both consumer paths against the state it produced.

**Result.** Running the real `affidavit_program` through LEZ v0.4's actual execution state
machine (`lee::V03State`, the code the sequencer applies), against `lee_core` **v0.4.0-rc1**:

```
[1] Initialize (public): instance PDA created, finalized slot 7069376, period 862           PASS
[2] Verify -> Apply (public): updates_applied 1, next committee learned true                 PASS
    Apply accepted: the instance's from_own_program() authorization held.
[3] Verify -> Apply (PRIVATE): a privately-proven Verify advanced the PUBLIC instance         PASS
[4] The whole fixture chain, privately: updates 1..6 as private Verify -> public Apply (+ Record)
    update 1 (committee handoff): Apply + Record landed: finalized slot 7070047 (block 17883995)
    update 2 (committee handoff): Apply + Record landed: finalized slot 7078240 (block 17892141)
    update 3 (committee handoff): Apply + Record landed: finalized slot 7089280 (block 17903093)
    update 4 (committee handoff): Apply + Record landed: finalized slot 7094272 (block 17908060)
    update 5 (committee handoff): Apply + Record landed: finalized slot 7104096 (block 17917816)
    update 6 (finality update):   Apply + Record landed: finalized slot 7109344 (block 17923026)
    on-chain instance == native follower: 7 updates applied, 7 header records                PASS
[5] AttestRecord (public): accepted at header record slot 7109344 (block 17923026)          PASS
    refused as expected with AFF-301: a predicate the evidence does not satisfy
    refused as expected with AFF-101: a Sepolia consumer at a mainnet record
[6] AttestPrivate -> CheckRoot (PRIVATE): accepted; the public crossing disclosed only the
    accumulator root; instance state unchanged                                               PASS
    refused as expected with AFF-210: an accumulator root the instance never produced
total wall-clock 226 s
```

([`run-output.txt`](run-output.txt), produced by [`driver.rs`](driver.rs). The three expected
refusals also appear on stderr as the guest's `AFF-301`, `AFF-101` and `AFF-210` panics,
which is how a refused transaction fails.)

Step 3 is the one that matters: the submitter runs `Verify` as a **privacy-preserving
transaction** whose private actor emits `Apply` to the public instance; the state machine
delivers it with the sender presented canonically, the instance's `from_own_program()` check
holds across the private→public boundary, and the public instance advances (it learns the
next sync committee). This is the single-transaction advance the proposal is built on,
working on v0.4 with the real program — not a mock, and not inferred from a similar test.

Steps 4–6 take the validated mechanism to the end of the fixtures and through both consumer
paths:

- **4 — the whole chain, privately.** Five more committee handoffs (periods 863–867) and the
  finality update, each a private `Verify` whose `Apply` lands on the public instance and
  writes the newly finalised header as a **write-once record PDA** — a public account the
  privacy-preserving transaction creates, admitted by its seed. After seven updates the
  on-chain instance is **equal** to a native `Follower` run alongside, with seven header
  records at their deterministic addresses and accumulator root `0xcc69d5…de958`.
- **5 — public consumer path.** `AttestRecord` to the record of block 17,923,026, with the
  block's first transaction (a native ETH transfer of 0.4796 ETH) as evidence and the
  predicate `value > 0`. At the top level nobody receives the `Reply`, so acceptance *is* the
  result: the same message is refused — the transaction fails and the record is untouched —
  under a predicate the evidence does not satisfy (`AFF-301`) and for a Sepolia consumer at a
  mainnet record (`AFF-101`). The statement a consumer program receives is computed offline
  for the same inputs: id `0xeda294…987c`.
- **6 — private consumer path.** `AttestPrivate` from a private actor proves the header's
  membership in the instance's accumulator and the evidence under it; the only public crossing
  is `CheckRoot`, carrying the accumulator root, which the instance confirms without changing
  its state (the instance bytes are identical before and after). A forged accumulator — the
  same headers plus one invented leaf, under which the header's membership proof still
  verifies — is refused by the instance's `CheckRoot` (`AFF-210`): a consumer cannot attest
  against a root the instance never produced.

## What this is, and what it is not

- **It is** the real LEZ v0.4 execution state machine (`lee::V03State::transition_from_public_transaction`
  and `transition_from_privacy_preserving_transaction`) — the same code path the sequencer
  applies to settle a block — running the real, unmodified `affidavit_program` guest
  (`lez/build-program.sh`, `lee_core` v0.4.0-rc1) on the mainnet light-client fixtures:
  the bootstrap, six committee-period updates, the finality update, and the finalised block's
  transactions, from initialisation to both consumer paths, with every refusal exercised
  carrying the `AFF-xxx` code the CLI dry run prints.
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
A full run takes about four minutes here (226 s): each private `Verify` is ≈25 s in the
in-process executor, the attestations 2–10 s.

## Runbook: the end-to-end confirmation (on a machine with network access)

This is the full version of steps 3–6, for the user's own machine, where the Logos circuit
host and the public testnet are reachable:

1. Build the standalone sequencer: `just run-bedrock` and `just run-sequencer-standalone`
   (LEZ `v0.4.0-rc1`). In standalone mode Bedrock is mocked, so no live consensus is needed.
2. Deploy `affidavit_program` with the wallet (`wallet deploy-program affidavit_program.bin`).
3. `Init` the instance (a public transaction to the instance PDA).
4. Prove `Verify` privately (`wallet send_privacy_preserving_tx`, real proof ≈ tens of
   minutes on a laptop / minutes on a GPU) and submit it; confirm the instance's `Apply`
   lands and public state advances.
5. Run a public attestation (`AttestRecord`) and a private one (`AttestPrivate` →
   `CheckRoot`) against a recorded header — the on-sequencer counterpart of steps 5–6 here.
6. Repeat against the **public LEZ testnet** (`testnet.lez.logos.co`) with a funded account.
   Record the program address and the transaction hashes; that is a public-testnet result,
   to be reported as such, separate from this local state-machine validation.

## What it means for the proposal

The one open question behind RFP-021's central claim — can a privately-proven `Verify`
advance the public instance in one transaction on LEZ v0.4 — is answered **yes** at the
execution-state-machine level, with the real program, and it keeps being yes across the
whole fixture chain: seven updates, seven header records, an on-chain instance identical to
the native follower. Both consumer interfaces the RFP asks for work on the same state
machine — the public record path and the private accumulator path — and every refusal a
consumer could run into comes back as the program's own `AFF-xxx` code with public state
untouched. Combined with the 77-minute real proof of `Verify` and LEZ's own `NestedBoundary`
test (a public root → private actor → public actor crossing, in
`lee/state_machine/src/state/tests/privacy_preserving.rs`), the private placement is
substantially de-risked ahead of the milestone that confirms it on a running sequencer and
the public testnet.
