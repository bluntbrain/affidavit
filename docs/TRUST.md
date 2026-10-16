# Trust assumptions

What an affidavit statement proves, what it rests on, who can do what to it, and when the
guarantees weaken. Supportability #6 and Verification Security #4 and #5 of RFP-021.

## The model in one paragraph

Verification is trustless after initialisation: an instance accepts a header only if a
two-thirds supermajority of the sync committee it already tracks signed it, every Merkle
path from that signature down to the fact checks out, and the committee itself was handed
off, step by step, from the checkpoint the instance was initialised with. Two things are
assumed: that the checkpoint was a real finalised block, and that at least two thirds of
each sync committee are honest. Everybody else (operators, submitters, RPC providers,
relayers) can only delay. None of them can make an instance accept anything false.

## 1. Honest sync-committee supermajority

**The assumption.** At least 342 of the 512 validators in the current sync committee do
not sign a header that is not on the canonical chain.

**Why it is weaker than Ethereum finality.** Ethereum's own finality needs two thirds of
*all* stake and is enforced by slashing. A light client trusts a 512-member sample, and
at the time of writing (Fulu) **a sync-committee member who signs a conflicting header is
not slashed**; EIP-7657 proposes to change that, but it is not scheduled. The protection
is therefore the randomness of the sample and the cost of acquiring stake or keys, not
the threat of losing them.

**What it takes to break it.** Committee seats are sampled by effective balance, so an
adversary holding a fraction *f* of all stake holds each seat with probability about *f*.
The chance it holds 342 or more seats in a given period (about 27 hours):

| adversary's share of stake | per period | per year (321 periods) |
|---:|---:|---:|
| 33% | 6 × 10⁻⁵⁵ | ≈ 0 |
| 50% | 1.2 × 10⁻¹⁴ | 3.8 × 10⁻¹² |
| 55% | 3.6 × 10⁻⁸ | 1.1 × 10⁻⁵ |
| 60% | 8.9 × 10⁻⁴ | 0.25 |
| 65% | 0.21 | ≈ 1 |
| 67% | 0.56 | ≈ 1 |

(`binomial(512, f)` tail at 342, computed in `scripts/committee_odds.py`.)

So by sampling alone the bar is roughly 55 to 60% of all staked ETH, about 19 to 21
million ETH at today's ~35 million staked: comparable in scale to attacking Ethereum
itself, but without the slashing that would burn it. The cheaper attack is targeted: the
next committee is known about one period ahead, and corrupting or bribing the keys of
342 named validators for one period forges headers for that period. Large staking
operators run many validators each, so in practice this is a question of how few
entities control two thirds of a committee, which consumers should treat as the real
attack surface.

**What a successful attack can do.** Sign a fabricated header, which then passes every
check (the instance cannot tell). Every consumer that acts on statements under headers
of that period is exposed. It cannot rewrite headers the instance already recorded, and
the signed fabricated header is public evidence of the collusion.

**What the program takes from a committee's state, under the same assumption.** The next
committee's 512 keys and its aggregate key come from the attested beacon state, which an
honest supermajority signs only if it is canonical. In a canonical state every key passed
`KeyValidate` with a proof of possession when its validator joined, and the aggregate is
the sum of the 512 keys. So the program skips per-key subgroup checks and takes the
signers' key as the aggregate minus the non-signers (`DECISIONS.md` D3). A captured
committee could hand off to a committee that breaks either property, but a captured
committee can already sign anything; neither shortcut adds a way to forge a statement
without capturing a committee first.

**What consumers do about it** (Verification Security #4): bound what any one period can
cost them. A bridge caps deposits minted per period and in total, keeps a freeze
authority that can stop minting when two instances or an instance and the honest chain
disagree, and lets large redemptions wait more than one period. These belong to the
consumer (RFP-022 Functionality #10, #11, #14); `CONSUMER.md` lists them.

A stronger anchor than sync-committee tracking (verifying the full validator set's
attestations) would remove this assumption at a cost far beyond a transaction; the
committee path is what the light-client protocol supports at tractable cost today.

## 2. The weak-subjectivity checkpoint

**Who supplies it.** Whoever initialises an instance, as a beacon block root. It is the
one trusted input. Anyone can initialise an instance; consumers choose which instance id
to bind to.

**How a verifier confirms it independently.** The checkpoint is a claim about public
history, so it can be checked from outside before anyone relies on it:

1. Read the instance's configuration (`affidavit checkpoint`, or the core module's read
   API): checkpoint root, slot, chain digest, weak-subjectivity limit.
2. Ask at least two independent sources for the finalised block root at that slot: two
   beacon nodes run by different parties, a beacon explorer, your own node. They must all
   return the configured root, and that block must be finalised.
3. Check that the bootstrap's header hashes to that root and that its sync committee
   branch verifies against its state root (`lc::initialize` does both; it refuses with
   `AFF-209` otherwise).
4. Check the checkpoint's age against the weak-subjectivity period below.

The Logos core module exposes step 2 as an API (Functionality #9) so a UI can show the
comparison rather than a configured value.

**Staleness and replacement.** Committees are followed forward by handoff. If an
instance falls behind by more than the weak-subjectivity period, validators that have
since exited could sign an alternative history from the instance's last committee at no
risk to themselves. So an instance whose newest finalised header is older than its
configured limit stops advancing for good (`AFF-208`); it is replaced by a freshly
initialised instance, never refreshed in place (#F10).

The limit is computed at initialisation with the Electra formula from public state:
`MIN_VALIDATOR_WITHDRAWABILITY_DELAY + SAFETY_DECAY · T / (2 · churn · 100)`. For mainnet,
with about 35 million ETH staked, that is **3,533 epochs, about 15.7 days**
(`config::weak_subjectivity_period_epochs`). It is frozen into the configuration, and so
into the instance id.

**Migrating to a replacement.** A consumer bound to an instance that went stale binds
to a new instance id. Statements it already consumed keep their identifiers, because
identifiers do not depend on the instance, so its nullifiers carry over and nothing can
be consumed twice across the migration. `CONSUMER.md` describes the switch.

## 3. What a statement proves, and what it does not

A statement proves that, on the chain identified by its chain digest, under a header the
instance verified as finalised, the named fact held, and the consumer's predicate held of
it.

It does **not** prove:

- that the fact still holds now (it holds at that header, forever; later blocks may change
  the state);
- that it has not been acted on before (uniqueness is the consumer's nullifier);
- anything about Ethereum activity being private (it is public by construction);
- that a token contract behaves honestly (a `Transfer` log from a malicious token is a
  real log; consumers pin token addresses);
- that a native transfer succeeded, unless the receipt was proven too (the statement
  says so: `success` is `None` otherwise, and a predicate on it is refused).

## 4. Participants: what they see, what they can affect

| Participant | Observes | Can affect safety? | Can affect liveness? | Why not more |
|---|---|---|---|---|
| Sync committee | nothing of LEZ | **yes**, with a two-thirds supermajority (section 1) | yes, by not signing (then Ethereum itself is not finalising) | the assumption |
| Initialiser of an instance | nothing beyond public data | only of instances consumers choose to bind to | no | checkpoint verifiable before use (section 2) |
| Operator / submitter | public beacon data, the instance's public state | no: every update is verified on submission | only by not submitting, which anyone else can do instead | permissionless submission (#F11, #F12) |
| Beacon API / Ethereum RPC provider | the queries of whoever uses it, i.e. which blocks and accounts a consumer's wallet asks about | no: it supplies material, the program verifies it | yes, for its own users, by refusing or lying; switch provider | verified proxy, multiple endpoints |
| LEZ sequencer | public parts of transactions, as for any program | no more than for any LEZ program | censorship, as for any LEZ program | LEZ's own guarantees |
| Logos Messaging (proving hints) | that some operator is proving some update | no | bounded delay (the hint cap), never a block | hints are never trusted |
| Consumer's own wallet | everything about the consumer's own action | its own action only | its own action only | — |

**No participant, alone or as a class, can block an attestation forever.** Updates are
public data on any beacon node; anyone with a beacon endpoint and LEZ gas (and, for the
private-verifier path, a prover) can advance an instance. A consumer that cannot get
material from one provider uses another. A lost operator is a delay, not a loss.

**Privacy of the consumer.** On the private path the only thing that leaves the
consumer's private execution is `CheckRoot(root)` against the instance, a root shared by
every consumer of that instance. The fact, the header, the Ethereum block and the
consumer's account stay inside the proof (#F8). What the consumer's *wallet* asks an
Ethereum RPC provider is another matter: fetching proofs for a specific deposit reveals
interest in it to that provider. The core module should fetch through the verified proxy
over Tor-ready transport (`eth_rpc_module` is socks5h-ready) and may over-fetch.

## 5. When the guarantees degrade or fail

| Condition | Effect | What to do |
|---|---|---|
| Two thirds of a committee collude or are compromised | fabricated headers accepted for that period | consumer caps and freeze (section 1); monitor instance headers against independent nodes |
| Wrong checkpoint at initialisation | that instance follows whatever chain the checkpoint led to | verify before binding (section 2); bind only to checked instances |
| No updates submitted for longer than the weak-subjectivity period | instance can no longer advance; past statements remain valid | initialise a replacement and migrate |
| No updates submitted for a while, within the period | statements only about older headers | anyone submits the outstanding updates; nothing is lost (#R7) |
| A fork changes light-client containers (Gloas) | an instance on the old program version cannot follow past the fork | release the new program version before activation, consumers migrate (Verification Security #3) |
| A bug in the verification code | anything | independent audit before production (#S8); the program is immutable, a fix is a new version consumers move to |
