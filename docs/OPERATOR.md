# Running an operator

An operator follows Ethereum's beacon chain and submits the updates an instance needs.
It is a liveness role, not a trust role: every update is verified on submission, so an
operator can only choose to act or not to act. **Nobody is obliged to operate.** An
instance stays fresh because the protocols whose users need fresh headers choose to pay
for them; if everyone stops, statements about already-verified headers keep working and
anyone can catch the instance up later, within its weak-subjectivity period.

## What it takes

| Need | Public-verifier path | Private-verifier path |
|---|---|---|
| Ethereum | a beacon API endpoint (light-client routes) and an execution endpoint with `eth_getProof`, reached through the Logos Ethereum core module | same |
| LEZ | 12M to 15M gas per update, over at least two transactions | about 0.49M gas per update, in one transaction |
| Compute | negligible (native verification is about 40 ms per update) | a RISC Zero prover: 15.9M padded cycles per update, about 19 minutes on an M2 Pro CPU (*estimate*), minutes on a GPU |
| Standing | none: no registration, no allowlist, no stake, nothing on Ethereum | none |
| State | the committee keys behind each tracked root (about 25 KB per period), recoverable from any beacon node | same |

Starting one is installing the module, pointing it at endpoints and a funded LEZ account,
and naming the instance id to follow. No one can stop you and no one has to approve you.

## Cadence

Two kinds of update matter:

- **Handoffs**, once per sync-committee period (8,192 slots, about 27.3 hours). An
  instance cannot verify anything signed in period *p+1* until it has learned the
  committee of *p+1* from an update of period *p*. Missing a handoff is not fatal: it can
  be submitted later, in order, but nothing newer lands until it is.
- **Finality advances**, as often as consumers need fresh headers. A finalised header is
  available roughly every 6.4 minutes; the recommended cadence is one per hour, plus on
  demand when a consumer asks for a specific block.

At one finality advance per hour plus the daily handoff, an instance records about 8,800
headers a year. Storage and the amortised cost per attestation are in `COSTS.md`.

## When several operators run at once

Racing is the default failure of permissionless submission: the fastest operator wins
every time, pays for every update, and everyone else pays for refused transactions. The
daemon avoids it with three rules (`crates/core/src/operator.rs`):

1. **Defer.** Before acting on update *u*, wait `sha256(operator_key ‖ u) mod W`
   milliseconds. The order changes with every update and cannot be gamed by anyone else,
   so the cost rotates. An operator that wants to always go first may set W = 0; the
   protocol does not care (#F11).
2. **Check, act, check.** Read the instance right before starting work, and again right
   before submitting. If it already moved, stop (#R9).
3. **Hint (private-verifier path only).** When starting a proof, announce
   "proving update *u*, ETA *t*" over Logos Messaging. Others that see the hint wait until
   *t* (capped at 30 minutes) before checking again. A hint is never trusted: a silent or
   lying announcer costs at most the cap in delay, never safety, and nobody needs
   permission to ignore hints.

Simulated over 20,000 updates with five operators of different latency and hardware
(`cargo run --release -p affidavit-sim`):

**Public-verifier path** (no proving; 6 s inclusion):

| strategy | share of updates paid (A, B, C, D, E) | wasted submissions /100 | median / p95 latency |
|---|---|---:|---|
| submit on sight | 100% / 0% / 0% / 0% / 0% | 300.0 | 6 s / 6 s |
| deferral window 30 s | 33% / 27% / 21% / 15% / 5% | 88.2 | 13 s / 23 s |
| deferral window 120 s | 23% / 23% / 20% / 19% / 14% | 24.5 | 26 s / 64 s |
| deferral window 300 s | 21% / 21% / 20% / 20% / 18% | 10.3 | 49 s / 146 s |

**Private-verifier path** (proving times from the measured 15.9M padded cycles: A, B, E
prove on GPUs in 1.5 to 2.5 minutes; C and D on laptop CPUs in about 19 and 22):

| strategy | share of updates paid (A, B, C, D, E) | wasted submissions /100 | wasted proofs /100 | median / p95 latency |
|---|---|---:|---:|---|
| submit on sight | 0% / 0% / 0% / 0% / 100% | 0.0 | 400.0 | 106 s / 108 s |
| deferral window 600 s | 35% / 28% / 0% / 0% / 37% | 2.7 | 146.5 | 245 s / 500 s |
| deferral window 1800 s | 34% / 31% / 1% / 0% / 34% | 0.9 | 81.0 | 490 s / 1242 s |
| deferral 600 s + proving hint | 21% / 20% / 20% / 20% / 19% | 0.0 | 0.0 | 302 s / 1491 s |
| deferral 1800 s + proving hint | 20% / 20% / 20% / 20% / 19% | 0.0 | 0.0 | 664 s / 1881 s |

Reading it: on the public path a 300 s window spreads the cost evenly and wastes about one
submission in ten updates, at a median latency under a minute. On the private path the
second check keeps wasted gas near zero whatever the strategy; what racing wastes there is
proofs, 400 per 100 updates when everyone proves on sight, and the slow provers never get
a turn. The hint removes wasted proofs and gives every operator about a fifth of the
updates, at the price of latency when a slow prover goes first. The recommended defaults
are W = 300 s (public) and W = 600 s with hints (private), both configurable.

## Noticing a stalled instance

The daemon exposes, per instance: finalised slot of the instance vs. the beacon chain's,
current period vs. the instance's, time since the last applied update, and its own
submitted, skipped and refused counts. Alert when the instance's finalised slot is more
than one period behind, and page when it is behind by half the weak-subjectivity period
(`checkpoint_stale` is then a week away). Anyone can see the same thing from public state.

## Rewarding operators (soft requirement #5)

Optional and permissionless: a bounty account per instance that anyone can fund (a bridge
would). `Apply` pays a fixed reward from it to the account the winning `Verify` names,
capped per update, only when the update advanced the instance. It makes operating
attractive without making it exclusive: the payment goes to whoever lands the update,
there is no list of who may, and the deferral rule already spreads who that is.
