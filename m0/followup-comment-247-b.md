**Update: the state-machine test now covers the whole chain and both consumer paths**

The earlier comment showed one privately-proven `Verify` advancing the public instance through LEZ v0.4.0-rc1's `V03State`. The driver in [`m0/`](https://github.com/bluntbrain/affidavit/tree/main/m0) now runs:

- all seven mainnet updates (six committee periods and the finality update) as private `Verify` → public `Apply` (+ write-once `Record`), ending in the same state and accumulator root as the native client: 7 updates applied, 7 header records;
- the public consumer path, `AttestRecord`, against the head record on a real transaction of the finalised block, with its refusals (AFF-301 out-of-range predicate, AFF-101 wrong network);
- the private consumer path, `AttestPrivate → CheckRoot`, which crosses to public state with only the accumulator root and leaves the instance bytes unchanged, with its refusal of a forged accumulator carrying a valid membership proof (AFF-210).

Six steps, all PASS, about 4 minutes wall-clock, about 25 s per private `Verify` in dev mode. The log, the driver and the reproduce steps are in `m0/`. The caveats are unchanged and deliberate: dev-mode proof (the real 77-minute proof is measured separately in `docs/COSTS.md`), the execution state machine rather than the full networked sequencer, and local rather than the public testnet; the runbook for those two is in the same README.
