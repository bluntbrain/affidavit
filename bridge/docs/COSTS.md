# Costs

What has been measured, on what, and what is still an estimate. Measured numbers are
reproducible from this repository; estimates say what they rest on and when they get
replaced by measurements.

## Ethereum gas (measured)

`npm test` in [`../contracts`](../contracts) compiles `CausewayVault.sol` with solc 0.8.28
(optimizer, 200 runs, `cancun`) and runs it on an in-process EVM (`@ethereumjs/vm` 8,
Cancun rules), recording the gas of each operation into `artifacts/gas.json`:

| operation | gas | notes |
|---|---:|---|
| `depositETH` | 86,035 | registry + denomination + cap checks, one `Deposit` log |
| `depositERC20` (USDC-style) | 108,984 | plus `transferFrom` and the balance-delta check |
| `depositERC20` (USDT-style, no return value) | 108,635 | same path, empty return data accepted |
| `release` → ERC-20 | 160,219 | pairing, anchor, nullifier, proof call, freeze, caps, escrow, transfer |
| `release` → ETH | 98,213 | same, native payout |

Deployed bytecode: 7,914 bytes (limit 24,576).

**What the release numbers do not include.** The tests verify against a mock that checks
the seal is the expected binding of image id and journal digest, so the figures cover the
vault's own path and its two external calls but **not** the pairing arithmetic of a real
Groth16 verification. RISC Zero's published figure for its on-chain Groth16 verifier is on
the order of 250–300k gas; the real total for a release is therefore expected around
350–450k gas (ERC-20) and 300–400k gas (ETH). M0 replaces this estimate with a measurement
against the real `RiscZeroGroth16Verifier` on a fork.

At 10 gwei these are 0.0009 ETH per deposit and, with the estimated verifier cost,
roughly 0.004 ETH per release; a permissionless relayer paid from the released amount
covers the latter for users without ETH (Usability #6).

## LEZ side (estimates anchored on Affidavit's measurements)

The inbound claim runs over Affidavit's private attestation path, which is measured in the
RFP-021 repository ([`../../docs/COSTS.md`](../../docs/COSTS.md)):

| quantity | value | status |
|---|---|---|
| Affidavit `AttestPrivate` cycles | 0.6–1.3 M | measured (RFP-021) |
| Affidavit `AttestPrivate` real proof | 495 s on a CPU dev machine, 222,668-byte succinct receipt | measured (RFP-021) |
| Claim = `AttestPrivate` + mint authorisation (registry, denomination, `c = H(s)`, nullifier) | +≈0.1 M cycles over the above: a few SHA-256s and comparisons | estimate; measured in M0 |
| LEZ gas for a private tx | ≈0.49 M per Affidavit private update | measured (RFP-021); the claim is expected in the same range |
| Burn guest cycles | small: one account path + one hash opening; dominated by the state-tree depth | estimate; measured in M0 |
| STARK → Groth16 wrap | minutes on x86 with RISC Zero's prover; GPU shortens it | estimate from RISC Zero's tooling; measured in M0 |

Keeping the authorisation logic tiny is deliberate: the mint-authorisation core in
[`../src/lib.rs`](../src/lib.rs) is a few hundred lines with no allocation-heavy paths, so
the claim's cost is Affidavit's attestation cost plus noise.

## Operating costs

- **Affidavit operator** (RFP-021): keeps the instance's headers current; its costs are in
  the RFP-021 cost report and are shared by every consumer of the instance, Causeway
  included.
- **Relayer**: pays Ethereum gas for releases (and LEZ fees for relayed claims) and is paid
  a uniform per-denomination fee out of the amount ([PRIVACY.md](PRIVACY.md)). Permissionless:
  anyone can run one; the Logos module ships one.
- **Anchor**: route-dependent (see [OUTBOUND.md](OUTBOUND.md)); the per-checkpoint gas is an
  M0 measurement.

## Not yet measured

Real Groth16 verification gas on the release path; burn guest cycles and proving time;
anchor gas per checkpoint; the claim's private proof time end to end on a standalone LEZ
sequencer. All are M0 deliverables, and the cost report is updated with each.
