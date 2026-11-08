**Update — M0 de-risked: the private-verifier placement now runs on LEZ v0.4.**

The proposal's main advantage is advancing an instance in **one transaction**, which rests
on a single LEZ behaviour: a `Verify` actor proven privately by the submitter can send
`Apply` to the public instance and have `from_own_program()` accept it, so public state
advances. I've now validated that on **`lee_core` v0.4.0-rc1**, driving the real
`affidavit_program` through LEZ's actual execution state machine (`lee::V03State`, the code
the sequencer applies):

```
[1] Initialize (public): instance PDA created, finalized slot 7069376            PASS
[2] Verify -> Apply (public): instance advances; from_own_program() holds        PASS
[3] Verify -> Apply (PRIVATE): a privately-proven Verify advances the public
    instance across the private->public boundary                                 PASS
```

Reproducible from the repo: https://github.com/bluntbrain/affidavit/tree/main/m0

Scope, stated plainly: this is the execution state machine with the real program, not a
mock. `RISC0_DEV_MODE=1` skips only the cryptographic proof — the execution, the
private→public delivery, the authorization check and the state transition all run for real;
the proof's validity is the separately-measured 77-minute real proof of `Verify` (224 KB
succinct receipt). The full networked sequencer isn't exercised here, and this is not a
public-testnet deployment — both are in the M0/M1 scope, with a measured public-split
fallback if any placement detail differs on a running sequencer.

Happy to walk through any of it.
