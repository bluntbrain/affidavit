# RFP-021 traceability

Every requirement of RFP-021, where it stands in this prototype, and which grant
milestone completes it. **Done** means implemented and tested here; **Designed** means
specified in the docs with the mechanism chosen; **Mn** is the milestone in the proposal.

## Functionality

| # | Requirement | Status | Evidence / milestone |
|---|---|---|---|
| F1 | Light client inside the LEZ program, from a trusted checkpoint, checks the aggregate itself | Done (program handlers, mainnet data); on testnet M1 | `lez/program` `Verify`/`Apply`; `lez/program/tests/program.rs::whole_mainnet_flow_through_the_handlers` |
| F2 | Verify a header is finalised, reject otherwise | Done | `adversarial.rs::non_finalised_header_is_not_attestable` |
| F3 | Account, storage slot, log by position in receipt; roots bound to the finalised header | Done | `mainnet_attestation.rs`; `adversarial.rs::tampered_execution_roots_are_rejected` |
| F4 | Predicate with equality and inequality, expressed by the consumer | Done | `statement::Predicate`; `erc20_transfer_logs_attest` |
| F5 | Chain binding; mainnet and a testnet with the same address | Done for identifiers and refusals; Hoodi fixtures M2 | `statement_identifiers`; `replay_against_another_chain_is_rejected` |
| F6 | Deterministic, collision-resistant statement identifier | Done | `statement::statement_id`; `statement_identifiers` |
| F7 | LEZ program on testnet, published interface, inclusion and predicate in the consumer's transaction | Done in handlers (`AttestRecord`, `AttestPrivate`); testnet M2 | `public_and_private_consumers_get_the_same_statement` |
| F8 | No correlation signal beyond Ethereum's | Designed (accumulator, single `CheckRoot` crossing); state-diff audit on testnet M2 | `DESIGN.md` §3, `TRUST.md` §4 |
| F9 | Checkpoint, chain and finality parameters fixed at init; core module API to verify the checkpoint | Done (address = configuration; `affidavit checkpoint`); module API M3 | `instance_lives_at_its_config_and_initialises_once` |
| F10 | Stale instance replaced, not refreshed; migration documented | Done | `stale_instance_stops_advancing_but_keeps_its_attestations`; `TRUST.md` §2 |
| F11 | Permissionless advancing | Done | `Verify` accepted from any actor; `whole_mainnet_flow_through_the_handlers` uses an unknown account |
| F12 | Every off-chain participant liveness-only, justified | Done | `TRUST.md` §4 |
| F13 | No instruction rewrites an instance | Done | no such message; `D6` |
| F14 | Same program, any Ethereum-consensus chain | Done | `ChainSpec` mainnet/Sepolia/Hoodi; minimal-preset spec vectors through the same code |
| F15 | Independent instances coexist; consumers bound to one | Done | PDA per configuration; `AFF-103` |

## Usability

| # | Requirement | Status | Evidence / milestone |
|---|---|---|---|
| U1 | Logos core module: produce, verify, submit, read config | M3 | `DESIGN.md` §6 |
| U2 | CLI incl. dry run with the on-chain error codes | Done offline (`demo`, `sync`, `attest`, `checkpoint`); LEZ submission M3 | `crates/cli` |
| U3 | Update daemon as a Logos module + headless CLI | Policy done (`operator.rs`, simulation); daemon M3 | `OPERATOR.md` |
| U4 | SPEL IDL | M2 | IDL of the `Message` enum |
| U5 | Clear, actionable errors for every failure mode | Done | `error.rs`; table in `DESIGN.md` §5 |
| U6 | Reference consumers: ERC-20 and native ETH to a contract | Attestation halves done on mainnet data; consumer programs M2 | `erc20_transfer_logs_attest`, `native_eth_transfer_under_the_light_client_header` |

## Reliability

| # | Requirement | Status | Evidence / milestone |
|---|---|---|---|
| R1 | Deterministic verification | Done | `verification_is_read_only_and_deterministic`; spec vectors |
| R2 | Read-only; a rejection consumes nothing | Done | same test; `duplicate_and_racing_updates_apply_once` |
| R3 | Statements valid forever | Done | write-once records; accumulator; `stale_instance_…keeps_its_attestations` |
| R4 | Handoff tracking survives interruption | Done for the instance; daemon persistence M3 | `a_stalled_instance_resumes_without_a_gap` |
| R5 | RPC failure leaves off-chain parts recoverable | M3 | daemon |
| R6 | Reuse assessed and budgeted | Done (assessment); finished with measurements M0 | `REUSE.md` |
| R7 | Unadvanced instance keeps safety and resumes | Done | `a_stalled_instance_resumes_without_a_gap` |
| R8 | Duplicates rejected cleanly | Done | `duplicate_and_racing_updates_apply_once` |
| R9 | Check before submitting | Done (policy) | `operator::decide`; simulation |
| R10 | Deferral spreads cost | Done (policy + simulation) | `OPERATOR.md` tables |
| R11 | Independent statements; private transactions compute results before inclusion | Designed (statements write nothing shared; 64 recent roots) | `DESIGN.md` §3 |
| R12 | CI green | Done | `.github/workflows/ci.yml` |

## Performance

| # | Requirement | Status | Evidence / milestone |
|---|---|---|---|
| P1 | Attestation within one transaction | Measured: 0.6M to 1.3M cycles per consumer path against the 10M cap | `COSTS.md`; `measure_lez` |
| P2 | Cost by action: cycles, gas, proof time, proof size | Cycles and gas measured per program action in the executor; proof time and size measured on testnet in M0 and M2 | `COSTS.md`; `zkvm/host` |
| P3 | Amortised tracking cost per attestation | Done (model on measured gas) | `COSTS.md` |
| P4 | Private proving practical on desktop; minimum spec stated | Estimated (about 1.5 to 2 min per private attestation on an M2 Pro CPU); measured on laptops in M2 | `COSTS.md` |
| P5 | End-to-end latency breakdown | Done (model) | `COSTS.md` |
| P6 | Off-chain compute resources | Done | `OPERATOR.md`, `COSTS.md` |
| P7 | Storage growth, 1 and 5 years | Done | `COSTS.md` |
| P8 | In-program BLS vs a hypothetical precompile | Done (phase breakdown) | `COSTS.md` |
| P9 | No cost scales with accumulated storage | Done by design (roots, bounded accumulator, 64-root ring) | `D4`, `D7` |

## Supportability

| # | Requirement | Status | Evidence / milestone |
|---|---|---|---|
| S1 | Deployed on LEZ testnet 0.4, attesting a public Ethereum testnet | M2 | Hoodi |
| S2 | End-to-end tests vs a standalone sequencer and an Ethereum testnet or fork, in CI | M2 | |
| S3 | Core module integration docs | M3 | |
| S4 | Recommended consumer pattern | Done | `CONSUMER.md` |
| S5 | CLI and operator docs | Done for the prototype; final M3 | `OPERATOR.md`, `README.md` |
| S6 | Trust assumptions document | Done | `TRUST.md` |
| S7 | Operator role document | Done | `OPERATOR.md` |
| S8 | Independent audit, published | M5 | |
| S9 | Module catalog from the template | M3 | |
| S10 | Released with `logos-modules-release-action` | M4 | |
| S11 | Design decision log | Done | `DECISIONS.md` |

## Verification Security

| # | Requirement | Status | Evidence |
|---|---|---|---|
| VS1 | Fabricated chain rejected, tested explicitly | Done | `fabricated_chain_is_rejected`; `affidavit demo` step 5 |
| VS2 | Never-handed-off committee; low participation; non-finalised header; tampered roots; other chain; uncovered predicate | Done | `adversarial.rs`, one test per case |
| VS3 | Verification logic cannot be swapped | Done | no program id in mutable state; `CONSUMER.md` |
| VS4 | Compromised committee failure modes and consumer bounds | Done | `TRUST.md` §1, `CONSUMER.md` |
| VS5 | Trust model stated in user and developer docs | Done | `TRUST.md`, `README.md` |
| VS6 | Accepted on proof validity, never identity | Done | `Verify` from any actor |

## Soft requirements

| # | Requirement | Status |
|---|---|---|
| 1 | Recorded-statement account sharing the verification core | M2, optional; reuses `attest` |
| 2 | Batching over one header | M2, optional: several `Evidence` under one record in one call |
| 3 | Historical state | Done for every header an instance recorded (accumulator); older headers via `historical_summaries` M2, optional |
| 4 | Richer predicates over several values | M2: conjunction across several facts under one header |
| 5 | Operator incentive | Designed (`OPERATOR.md`, bounty paid by `Apply`) |
