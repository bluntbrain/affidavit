**Update: the same chain now runs on a live LEZ v0.4.0-rc1 sequencer**

After the state-machine test, I ran the real `affidavit_program` against a **running standalone LEZ `v0.4.0-rc1` sequencer** (mock Bedrock, in-process execution), deployed through `program_loader` and driven entirely with LEZ's own wallet library. Every step is a transaction the sequencer accepted and settled ([`m0/live/`](https://github.com/bluntbrain/affidavit/tree/main/m0/live), full hashes in `dev-mode/result.json`):

- program account `3VBG9ZC9xXVg3jcKzu1N29bjd4HvSafFTZrSizFLWvmn`, instance PDA `5kkzHnNwGPAdUeqF2j4AW2QvtZao7FQetPqLwpD1KruM`
- `Initialize` (public) — block 80
- all seven mainnet updates as **privately-proven `Verify` → public `Apply` (+ `Record`)**, blocks 81–87, one update per block; on-chain instance == native follower, 7 header records, accumulator root `0xcc69d515…cde958`
- `AttestRecord` (public consumer path) — block 88; a predicate that cannot hold — block 89, **reverted by the sequencer** (`AFF-301`), record unchanged
- `AttestPrivate → CheckRoot` (private consumer path) — block 90, only the accumulator root crossed, instance bytes unchanged

Two things the live sequencer taught that the state machine couldn't: the wallet's default `gas_limit` (2M) is below `Initialize`'s metered 2,543,172 cycles (the first attempt was included with no effect), and the sequencer caps a transaction at **10M gas** — so a *public* `Verify` (14.6M cycles) cannot be a single transaction on this configuration, while the private path's public part (`Apply`+`Record`, ≈80K) is. That is the one-transaction design's reason to exist, measured.

Caveats, deliberately kept: this run used `RISC0_DEV_MODE=1` on both sides (placeholder receipts; execution, delivery, authorization, fees and state are real); it is the standalone sequencer, not the public testnet. A real-proof run on the same setup is in progress and will be reported in the same directory; the public-testnet run stays a separate claim.
