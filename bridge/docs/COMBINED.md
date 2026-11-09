# Building Affidavit and Causeway together

RFP-022 states that proposals addressing both RFP-021 and RFP-022 are preferred. These two
are designed as one body of work: Affidavit (RFP-021) is the attestation primitive,
Causeway (RFP-022) is its first consumer, and the bridge's central privacy property is a
property of how the attestation is consumed ([PRIVACY.md](PRIVACY.md)). This is the plan
for one full-time engineer delivering both, with the dependencies made explicit.

## Dependencies between the two

| Causeway needs | from Affidavit | ready at |
|---|---|---|
| a LEZ program that holds finalised Ethereum headers | the light client on testnet 0.4 | RFP-021 M1 |
| the private attestation path (`AttestPrivate` / `CheckRoot`) | attestation and consumers | RFP-021 M2 |
| the core module, CLI and update daemon it extends | module work | RFP-021 M3 |
| one audited attestation core | the RFP-021 audit package | RFP-021 M4 |

Nothing in Affidavit depends on Causeway, so Affidavit goes first and Causeway's design
work overlaps its tail.

## Timeline (one engineer, full time)

| weeks | Affidavit (RFP-021, $30,000) | Causeway (RFP-022, $68,000) |
|---|---|---|
| 1–2 | **M0** placement and cost baseline · $3,000 (the private-placement question is already answered on v0.4.0-rc1's state machine, see [`../../m0/`](../../m0/README.md)) | |
| 3–5 | **M1** light client on testnet 0.4 · $7,000 | |
| 6–8 | **M2** attestation, consumers, cost report · $7,000 | |
| 9–11 | **M3** core module, CLI, update daemon · $7,000 | **M0** design, threat model, proof-system baseline · $7,000 (weeks 10–11, overlapping: design work alongside module work) |
| 12 | **M4** release and handover · $3,000 → audit begins | |
| 13–16 | (audit in progress) | **M1** vault + inbound mint on testnet 0.4 · $16,000 |
| 17–20 | **M5** audit remediation when the report lands · $3,000 | **M2** outbound burn + release, privacy suite · $16,000 |
| 21–24 | | **M3** modules, mini-app, recovery, CLI · $16,000 |
| 25–27 | | **M4** hardening, e2e CI, release, audit-ready · $9,000 → audit begins |
| after the report | | **M5** audit remediation · $4,000 |

About 27 weeks of build plus the two remediation weeks: roughly seven months. Each
milestone is a tagged release with green CI, and payment is on acceptance, so Logos's
exposure at any point is one milestone.

## Price

| | build | combined |
|---|---:|---:|
| Affidavit | $30,000 | |
| Causeway | $68,000 | |
| **Total** | $98,000 | **$92,000** (−$6,000: the core module, CLI and daemon are built once and serve both) |

The independent audits are outside these figures and contracted by Logos directly, as both
RFPs anticipate. Combining the projects also lets **one audit engagement** cover the
Affidavit core and the bridge, with the auditor reading the attestation code once.

## What is shared, concretely

- **Attestation core**: `affidavit-core` is the only code that decides what Ethereum said,
  for Affidavit's own attestations and for Causeway's mint authorisation. One audit, one
  set of fixtures (mainnet data and the official consensus-spec-tests), one cost report.
- **LEZ deployment and test harness**: the standalone-sequencer and testnet-0.4 setup, the
  state-machine driver in `m0/`, and the CI that builds both LEZ programs.
- **Module work**: the Logos core module, CLI and update daemon are extended with the
  bridge's actions rather than written twice.
- **Documentation discipline**: the same decision log, requirement traceability and
  measured-versus-estimated separation across both repositories.

## Risks of one engineer, and what offsets them

- **Bus factor.** Everything is public from day one: the design docs, the decision log,
  reproducible measurements and CI mean another engineer could pick up any milestone from
  the repository alone. Milestone gating means Logos never pays ahead of delivered work.
- **Schedule slip on Affidavit delays Causeway.** The overlap is kept to design work, so a
  slip moves Causeway's build rather than compounding it, and the Causeway M0 deliverable
  (the pinned interface between the two) is exactly what makes a later hand-off possible.
- **Scope creep across the seam.** The interface between the two is pinned in Causeway M0
  and versioned; changes after that are decisions in the log, not drift.

## What is already in hand

Before any grant: the Affidavit light client proven on mainnet data (RFP-021), its private
`Verify` → public `Apply` path validated on LEZ v0.4.0-rc1's execution state machine
([`../../m0/`](../../m0/README.md)), the Causeway vault with 21 tests and measured gas, the
inbound mint authorisation on a real Affidavit statement, and seed-only recovery in two
languages with a shared vector. The grant builds the bridge around a core that already runs.
