### Public path: the sequencer executes the verification, nothing to prove

| strategy | share of updates paid (A, B, C, D, E) | wasted submissions /100 | median / p95 latency |
|---|---|---:|---|
| submit on sight | 100% / 0% / 0% / 0% / 0% | 300.0 | 6 s / 6 s |
| deferral window 30 s | 33% / 27% / 21% / 15% / 5% | 88.2 | 13 s / 23 s |
| deferral window 120 s | 23% / 23% / 20% / 19% / 14% | 24.5 | 26 s / 64 s |
| deferral window 300 s | 21% / 21% / 20% / 20% / 18% | 10.3 | 49 s / 146 s |

### Private-verifier path: prove, then submit (A, B, E on GPUs; C, D on laptop CPUs)

| strategy | share of updates paid (A, B, C, D, E) | wasted submissions /100 | wasted proofs /100 | median / p95 latency |
|---|---|---:|---:|---|
| submit on sight | 0% / 0% / 0% / 0% / 100% | 0.0 | 400.0 | 106 s / 108 s |
| deferral window 600 s | 35% / 28% / 0% / 0% / 37% | 2.7 | 146.5 | 245 s / 500 s |
| deferral window 1800 s | 34% / 31% / 1% / 0% / 34% | 0.9 | 81.0 | 490 s / 1242 s |
| deferral 600 s + proving hint | 21% / 20% / 20% / 20% / 19% | 0.0 | 0.0 | 302 s / 1491 s |
| deferral 1800 s + proving hint | 20% / 20% / 20% / 20% / 19% | 0.0 | 0.0 | 664 s / 1881 s |

