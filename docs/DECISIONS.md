# Design decision log

Each entry: context, options, choice, trade-offs (Supportability #11). Kept current at
every milestone.

## D1. Verification and application are separate messages

**Context.** An update's verification is heavy and stateless given the tracked committee
roots; its application is a small write. LEZ can run the same program publicly (sequencer
executes, 10M-gas cap) or privately (submitter proves, sequencer checks one receipt).

**Options.** (a) One `Update` instruction that verifies and writes. (b) `Verify` (pure,
any actor) sends `Apply` (instance only, compare-and-swap).

**Chosen: (b).** It lets the expensive part run privately while the state stays public,
makes duplicate and racing updates harmless by construction (an `Advance` names the state
it was computed against), and keeps one verification core for every path. Cost: one
extra sync message per update, a few thousand cycles.

## D2. Where the BLS cost lands: private verifier first, public split as fallback

**Context.** A whole update measures above the public per-transaction cap (`COSTS.md`).

**Options.** (a) Split the public verification over several transactions with a job
account. (b) Place `Verify` privately: the submitter proves it, the sequencer verifies a
receipt at fixed cost. (c) Ask for a BLS12-381 precompile.

**Chosen: (b), with (a) as the fallback and the measured comparison.** (b) is one
transaction, about 0.41M gas plus `Apply`, needs no precompile and no protocol change,
and moves the cycles to whoever chooses to operate. It rests on one LEZ behaviour to
confirm on testnet 0.4 in M0 (a privately placed actor of a public program, sending to a
public actor that sees `from_own_program()`). (a) works without it at roughly thirty
times the sequencer gas; (c) is out of scope and only worth an RFP if both fall short.

## D3. Committee keys are bound by the state root, not re-validated

**Context.** Decompressing and subgroup-checking 512 keys is the largest single cost of
an update in a zkVM.

**Options.** (a) Full `KeyValidate` on every key, every update. (b) Decompress, skip the
subgroup check. (c) Supply uncompressed keys and check each against its compressed form.

**Chosen: (b) in the program, (a) in the native tools.** Every key is bound, through the
SSZ root the instance tracks, to a beacon state that admitted it only after verifying its
proof of possession at deposit, so a key outside the subgroup cannot be in a committee the
instance accepted. The native CLI and daemon keep full validation because it is free
there. (c) is a further optimisation to measure in M0 if (b) is not enough.

## D4. Committees as roots, keys as witnesses

**Context.** Storing 512 keys per committee would make the instance about 50 KB and every
update read it.

**Chosen.** The instance stores the current and next committee roots (64 bytes); the
submitter supplies the keys and the verifier hashes them against the root. Hashing 512
keys costs a fixed amount per update, independent of history (#P9), and the keys are
public data any operator can fetch.

## D5. Only supermajority finality updates; no optimistic header, no force update

**Context.** The reference light-client store also tracks an optimistic header and
force-updates to its best candidate after a timeout.

**Chosen.** An instance applies only what the spec applies through the two-thirds path.
A forced header is one the instance never saw finalised, which would make
`header_not_finalized` meaningless and break Reliability #3. The spec-vector tests stop
comparing at the first force-update step for exactly this reason.

## D6. The instance's address is its configuration

**Chosen.** `instance id = sha256(chain digest ‖ checkpoint ‖ weak-subjectivity limit)`,
and the instance lives at the PDA of that id. Immutability (#F9, #F13) is then structural
rather than enforced by the absence of an instruction; a different configuration is a
different instance; any number coexist (#F15).

## D7. Header accumulator for private consumers

**Context.** A private consumer that reads one header record discloses which Ethereum
block it acts on, a correlation #F8 forbids.

**Chosen.** The instance keeps a Merkle mountain range over every header it recorded
(at most 64 peaks) and answers `CheckRoot` for its last 64 roots. Private consumers prove
membership inside their own execution; the public crossing is a shared root. Cost: about
16 hashes per attestation at 65,536 headers. Public consumers keep the simpler record path.

## D8. Statement identifiers exclude the instance and the header

**Chosen.** Identifiers depend on the chain digest and the fact only. A consumer that
migrates to a replacement instance keeps its nullifiers valid, and two consumers never
derive different identifiers for the same event. Events are named by position, state by
content (`CONSUMER.md` explains when to use which).

## D9. Native ETH transfers by transaction proof

**Context.** A plain ETH transfer emits no log.

**Chosen.** Prove the transaction itself under the payload's SSZ `transactions_root`
(binding it to the finalised header with no MPT), recover the sender, check the
transaction's EIP-155 chain id against the instance, and prove the receipt at the same
index for success. Internal transfers are out of reach of any transaction proof; vaults
that must accept them emit an event.

## D10. Fork handling

**Context.** Generalised indices moved at Electra; Gloas changes the light-client header
to an execution block hash. Program code is immutable.

**Chosen.** The fork schedule is part of the configuration; Capella to Fulu containers
are code. Gloas support (roots from the RLP execution header checked against the block
hash; `execution::verify_execution_header` is the first piece) ships as a new program
version before the fork activates, with consumer migration as in D6. Hoodi, which has not
scheduled Gloas, is the first testnet target; Sepolia already moved to Gloas on
2026-10-06.
