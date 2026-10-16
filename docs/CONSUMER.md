# Recommended consumer pattern

How a LEZ program should consume affidavit statements. Supportability #4 and Verification
Security #3 of RFP-021.

## Bind to an instance by constant

Hold two constants in your program's code, never in an account:

- the affidavit **program id**, and
- the **instance id** you verified (see `TRUST.md`, "How a verifier confirms it
  independently").

A program that reads the verifier's program id from mutable state lets whoever controls
that state substitute a program that attests to anything. The same is true of the
instance id. Changing either is a new version of your program, deployed and migrated to
like any other upgrade.

## Verify inside the action's own transaction

Call affidavit from the transaction that takes the action, and act on the statement the
call returns, never on a statement recorded earlier by someone else. Then the check and
the action land or fail together.

- **Public consumer:** send `AttestRecord { chain, evidence, predicate }` to the header
  record you want, and handle the `Reply::Attested(statement)` sent back to you.
- **Private consumer:** send `AttestPrivate { chain, instance_id, headers_root, header,
  membership, evidence, predicate }` to a privately placed affidavit actor. The only thing
  that leaves your private execution is `CheckRoot(headers_root)`, shared with every other
  consumer of the instance.

When the reply arrives, check that it came from the affidavit program (`from.program_account_id`
equals your constant) and call `statement.check_binding(&chain, &instance_id)`, which
refuses another network (`AFF-101`) or another instance (`AFF-103`).

## Enforce uniqueness with a nullifier

affidavit proves existence; whether a fact was already used is your job, because only you
know what "the same action" means. Key a spent-marker on `statement.id`:

- a bridge mint: once per deposit (use the **log** statement of the deposit event, whose
  identifier is its position on the chain, not the vault's balance);
- a read-only escrow check: as often as you like, no marker.

**On a private path, never publish `statement.id` itself.** Anyone can compute it from
public Ethereum data, so a public nullifier equal to it links your private action to the
Ethereum event. Derive the nullifier from a secret the depositor committed to on the
Ethereum side instead, for example `nullifier = H(secret ‖ statement.id)` where the
deposit event carries `H(secret)`, and prove inside your private execution that the
statement's log contains that commitment. That is the consumer-side half of
Functionality #8, and the shape RFP-022's privacy requirement needs.

State statements (a balance, a storage slot) are identified by what they say, not when:
the same slot with the same value has the same identifier at any block. That is what you
want for "this escrow holds X"; if you need "this deposit happened", attest the event.

## Handle every error

| Code | Meaning | What your program should do |
|---|---|---|
| 101 chain_mismatch, 103 instance_mismatch | wrong network or instance | refuse; this is a configuration or attack signal |
| 200, 201, 203, 204, 205, 209 | consensus material invalid | refuse; the attestation cannot be produced from this material |
| 202 committee_handoff_not_verified, 207 update_not_newer, 210 header_unknown | instance not at that header yet | refuse now; retry once an operator advances the instance |
| 208 checkpoint_stale | instance outside its weak-subjectivity period | refuse; migrate to a replacement instance |
| 300 inclusion_proof_invalid, 302 state_not_covered | proof does not cover the claim | refuse; rebuild the proof |
| 301 predicate_not_satisfied | the fact is real but does not meet your condition | refuse |
| 303 transaction_reverted, 304 transaction_chain_mismatch | the transfer did not happen on this chain | refuse |

**When an attestation cannot be produced, refuse the action.** Never fall back to an
unverified input, a relayer's word, or an RPC answer. A delayed action is recoverable; an
action on a false fact is not.

## Bound your exposure

A two-thirds collusion of one sync committee can sign a fabricated header (`TRUST.md`).
Size what that could cost you and cap it: per-period and total limits on what can be
minted or released, a freeze authority that can pause your program when instances disagree
or a header contradicts independent nodes, and a delay on large actions longer than one
committee period. RFP-022 specifies these for the bridge.

## Migrating to a replacement instance

When your instance goes stale (`AFF-208`) or a fork needs a new program version: deploy
your program's next version with the new constants, keep your nullifier set (statement
identifiers do not depend on the instance, so nothing already consumed becomes consumable
again), and point users at it.

## Reference consumers

Two reference consumers come with the grant (Usability #6); their attestation halves are
already exercised on mainnet data in `crates/io/tests/mainnet_attestation.rs`:

- **ERC-20 transfer to a contract:** the `Transfer(from, to, amount)` log of a pinned token
  contract, with `log.address == token`, `log.topic0 == Transfer`, `log.topic2 == vault`
  and `log.data0 >= amount`, plus a nullifier on the log statement.
- **Native ETH transfer to a contract:** the transaction itself, proven under the payload's
  `transactions_root`, sender by signature recovery, `tx.to == vault`, `tx.value`, and
  `tx.success == 1` from the receipt at the same index. ETH sent to the vault through an
  internal call does not appear here; a vault that must accept those emits an event and
  is consumed like the ERC-20 case.
