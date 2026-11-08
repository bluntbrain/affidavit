# [PROPOSAL] RFP-021 — Affidavit: trustless Ethereum state attestation for LEZ

The text submitted to [logos-co/rfp](https://github.com/logos-co/rfp/issues/new?template=proposal.yml),
field by field.

---

## RFP ID

RFP-021 — Trustless Ethereum State Attestation for LEZ

## Your Project Name

Affidavit

## Team or Organization Name

Independent

## Primary Contact

ishan.lakhwani@gmail.com · Telegram @bluntbrainsol

## Team Members

**Ishan Lakhwani** (bluntbrain)
- Social links: [github.com/bluntbrain](https://github.com/bluntbrain) · [bluntbrain.com](https://bluntbrain.com) · [linkedin.com/in/ishanl](https://linkedin.com/in/ishanl) · [x.com/bluntbrain](https://x.com/bluntbrain)
- Role: everything in this proposal: light client and LEZ program, inclusion proofs and statements, core module, update daemon, CLI, cost measurement, docs
- Status: full-time for the whole grant
- One accountable engineer who already built and shipped the prototype: a lower total cost to Logos than a multi-person team, with no coordination overhead and a single owner across the light client, the modules and the daemon.

The independent audit (Supportability #8) is done by a third-party firm, scoped and quoted
during the grant so Logos can contract it directly (see Milestones).

## Project Summary

Every LEZ program that wants to act on something that happened on Ethereum needs the same
guarantee: the fact is true, and nobody had to be trusted to say so. An inclusion proof
alone does not give that, because anyone can fabricate a header and a proof that agree with
each other. It needs an anchor in Ethereum's own finality, checked once, shared by every
consumer, and audited once. The bridge in RFP-022 is the first consumer; escrow checks,
collateral checks and DAO-mirrored governance are the same primitive with a different
predicate.

Affidavit is that primitive. A LEZ program runs an Ethereum light client from a
weak-subjectivity checkpoint. Anyone can submit sync-committee updates; the program checks
the aggregate BLS signature and every committee handoff itself, and records each finalised
header. A consumer then proves an account, a storage slot, a log or a native ETH transfer
under a recorded header, evaluates its own predicate inside its own transaction, and gets
back a chain-bound statement with a deterministic identifier to key its nullifier on.
After initialisation nothing is trusted: operators, RPC providers and relayers can delay,
never lie.

I built the prototype before asking: **[github.com/bluntbrain/affidavit](https://github.com/bluntbrain/affidavit)**.
On real mainnet data it follows five committee handoffs (periods 862 to 867) from a
published checkpoint to finality, proves a native ETH transfer under the header it just
verified, attests WETH and USDC transfers and a storage slot, passes the official
consensus-spec-tests light-client vectors for Deneb and Electra, and refuses every attack
the RFP lists (fabricated chain, 341/512 signatures, a foreign committee, tampered roots,
replays, other networks) with a typed error code. `cargo run -p affidavit-cli -- demo`
shows all of it in one run. The LEZ program is written for the actor model of LEZ's
testnet-0.4 line and **builds and passes its tests against `lee_core` at v0.4.0-rc1**;
driven through LEZ's own message types across all the mainnet updates, it ends in exactly
the state the native light client reaches. Its binary is measured action by action in the
RISC Zero executor, and both private actions are proven for real: the whole update proves
in 77 minutes on a 4-vCPU VM into a 222,668-byte succinct receipt that verifies against the
program's image id — one transaction, one ~224 KB receipt the sequencer checks.

Two design choices matter most for LEZ in particular. First, verification is separated
from application, so the 15M-cycle update check can run in a privately placed actor whose
execution the submitter proves, while the instance stays public: **one transaction at
about 0.49M gas per update**, where verifying publicly costs 12M to 15M gas split over at
least two transactions under the 10M cap, and no BLS12-381 precompile is needed. Second,
private consumers prove their header is in the instance's header accumulator, so the only
thing they disclose is a root every consumer shares, which is what Functionality #8 asks
for.

## Technical Approach

**Stack.** Rust; LEZ programs on the actor model of LEZ's testnet-0.4 line (`lee_core` at
v0.4.0-rc1; SPEL for the IDL); RISC Zero 3.0 with RISC Zero's accelerated forks of `blst`, `sha2` and
`k256`; `alloy-trie`/`alloy-consensus` for Merkle-Patricia proofs and decoding; the Logos
Ethereum core module (`eth_rpc_module` with the Nimbus verified proxy) for every Ethereum
read; Logos modules for the core module and daemon, with the Rust library behind a C ABI
the way `logos-verified-proxy-module` wraps `libverifproxy`.

**Program actors** ([design](https://github.com/bluntbrain/affidavit/blob/main/docs/DESIGN.md)):
- *Instance*, at the PDA of the hash of its own configuration (chain id, genesis validators
  root, fork schedule, checkpoint, weak-subjectivity limit). There is no reconfiguration
  instruction and could not be one: a different configuration is a different address.
  Any number coexist; consumers bind to an instance id as a constant (#F9, #F13, #F15).
- *Verify*, any actor of the program: checks participation (≥ 342/512), slot ordering,
  that the signing committee is the one the instance tracks, the execution, finality and
  handoff branches, and the aggregate signature; produces an `Advance` that names the
  state it was computed against. Permissionless (#F11, #VS6).
- *Apply*, on the instance, accepted only from the program's own actors: a
  compare-and-swap, so duplicates and racing operators change nothing (#R8).
- *Header records*, write-once, and a header accumulator (at most 64 peaks) on the instance.

**Where the update cost lands** ([measured costs](https://github.com/bluntbrain/affidavit/blob/main/docs/COSTS.md)).
The signers' key is the committee's committed aggregate key minus the non-signers' keys,
all supplied as uncompressed points that must compress to the committed bytes, so no key
is ever decompressed: that took a whole update from 240M cycles (a straight port) to
13.7M. In the program binary, `Verify` is 14.6M cycles with a handoff and 12.3M without,
above LEZ's 10M per-transaction cap. LEZ runs private actors uncapped and proven by the
sender, with the sequencer verifying one receipt at a fixed 409,764 gas. So the preferred
path places `Verify` privately: one transaction, about 0.49M gas including `Apply` (57K
cycles) and the header record (20K), 25 to 30 times less than verifying publicly. The
submitter proves 15.9M padded cycles into one 222,668-byte succinct receipt (measured: 77
minutes on a 4-vCPU VM with no GPU; minutes on a GPU or a proving service). The
fallback, if testnet 0.4 does not allow that placement, splits the public verification
along the pairing's seams. M0 settles this on testnet first, and both paths are measured,
since the comparison is the input to any precompile RFP (#P8).

**Consumers.** Inclusion and the predicate run in the consumer's own transaction (#F7).
Public consumers call a header record. Private consumers call a privately placed actor
that proves accumulator membership; the only public crossing is `CheckRoot(root)` against
the instance, which answers for its last 64 roots so private proofs survive a new header
landing (#R11). Facts: accounts and storage (MPT), logs read from the receipt by index,
and native ETH transfers by SSZ proof under the payload's `transactions_root`, sender
recovery and the receipt's status. Measured: 0.6M to 1.3M cycles per attestation, within
one transaction (#P1); a private attestation proves in 8.3 minutes on a 4-vCPU cloud VM
with 9.4 GB of memory, about 2 to 3 minutes on a recent laptop (#P4). Statement identifiers depend on the chain and the fact, never on the
instance, so nullifiers survive migration to a replacement instance.

**Reuse** ([assessment](https://github.com/bluntbrain/affidavit/blob/main/docs/REUSE.md)).
Reused unchanged: blst and zkcrypto BLS (RISC Zero forks in the guest), alloy's trie, RLP
and transaction code, RustCrypto hashes and curves, the Logos verified proxy. Helios is
the reference: its mainnet fixtures are my test data. Written here: the light-client rules
(Altair sync protocol restricted to what an on-chain instance should do: supermajority
finality only, no optimistic header, no force update, committees as roots), SSZ for six
containers, statements and the accumulator. Helios's monolithic `verify_update` cannot be
split for a public path and r0vm-helios targets EVM contracts; M0 re-checks whether the
private path can embed Helios's logic verbatim to shrink the audit surface further.

**Operators** ([operator guide](https://github.com/bluntbrain/affidavit/blob/main/docs/OPERATOR.md)).
The daemon waits a deferral of `sha256(key ‖ update) mod W` before acting, reads the
instance before working and again before submitting, and on the private path announces
"proving update *u*, ETA *t*" over Logos Messaging. Simulated with five operators over
20,000 updates: a 300 s window spreads the public-path cost about 20% each and cuts wasted
submissions 30×; on the private path, hints bring wasted proofs from 400 to 0 per 100
updates while every operator, laptop or GPU, still lands about a fifth. Hints are never
trusted; a liar costs bounded delay (#R9, #R10).

**Trust** ([trust assumptions](https://github.com/bluntbrain/affidavit/blob/main/docs/TRUST.md)).
Honest two-thirds of a 512-member committee, which today is not backed by slashing:
capturing a committee by stake sampling needs about 55 to 60% of all stake (1.1 × 10⁻⁵ per
year at 55%, 0.25 at 60%); the realistic risk is bribing the known members of one period,
which consumers bound with caps and freezes. Weak-subjectivity limit 3,533 epochs (about
15.7 days) for mainnet, frozen into the instance id, with independent checkpoint
verification exposed through the core module.

**Anticipated challenges.**
1. *LEZ 0.4 semantics for private placement*: validated first, with a measured fallback.
2. *Forks.* Sepolia moved to Gloas on 2026-10-06; Gloas headers commit only to the
   execution block hash, so roots come from the RLP header checked against it (first piece
   already in the core). Hoodi (Fulu) is the first testnet target; the Gloas program version
   ships before mainnet schedules it.
3. *Beacon light-client routes through the Logos Ethereum core module*: if the module does
   not expose them, I add them upstream rather than calling beacon nodes around it.
4. *Proving hardware (#P4, #P6).* A private update proves in minutes on a GPU but takes
   tens of minutes on a laptop CPU, so operators on the private path use a GPU, a fast
   desktop or a proving service (outsourcing a proof is trustless), while a private
   attestation is already practical on a laptop. M0 and M2 measure both on real hardware
   and declare the minimum spec (16 GB of memory today).

## Milestones, Payout and Timeline

Twelve weeks of build, inside the RFP's 8 to 12, plus remediation after the audit. Every
milestone is a tagged release with green CI, updated docs and the decision log. The
prototype already builds and passes its tests against `lee_core` at **v0.4.0-rc1**, so M0
starts from a program that targets the testnet-0.4 line rather than porting to it later.

**M0 — LEZ 0.4 placement and cost baseline** · 2 weeks · $3,000
On a standalone LEZ 0.4 sequencer: the private placement of `Verify` end to end (the one
LEZ behaviour the one-transaction cost depends on), or the public split if it does not
hold; measured cycles, gas, proof time and proof size for update, apply and each consumer
path on 0.4; whether LEZ can verify an arbitrary RISC Zero receipt (composition);
reproducible program build with its image id checked in CI; reuse assessment finished with
those numbers; design freeze.

**M1 — Light client on testnet 0.4** · 3 weeks · $7,000
Program deployed to the public LEZ testnet (0.4): initialise, verify, apply, header
records, accumulator; live instances for Hoodi, Sepolia (Gloas) and mainnet; permissionless
advancing from unknown accounts; the whole adversarial suite run against the sequencer, with
an on-chain verification script and a mutation test; typed errors.

**M2 — Attestation, consumers, cost report** · 3 weeks · $7,000
Public and private consumer paths on testnet; reference consumers (ERC-20 and native ETH to
a contract) with the recommended consumer pattern; the same contract address across mainnet
and a testnet (#F5); SPEL IDL; end-to-end tests against a standalone sequencer and Hoodi in
CI; the full cost report (#P1–#P9) including private proving on a mid-range laptop and the
minimum spec; audit quotes collected.

**M3 — Core module, CLI, update daemon** · 3 weeks · $7,000
Logos core module (produce, verify, submit, read configuration, verify checkpoint);
CLI on top of it with dry runs returning on-chain error codes; update daemon as a Logos
module plus headless CLI: deferral, check-before-submit, proving hints over Logos
Messaging, metrics and stall alerts, restart without re-verification, clean shutdown;
operator and CLI docs.

**M4 — Release and handover** · 1 week · $3,000
Module catalog from the template, built and released with the release action; a public
operator running for the testnet instances; final docs; audit-ready package (spec, threat
model, vectors, traceability matrix).

**M5 — Audit remediation** · 1 week after the report · $3,000
Fixes for every finding, re-verification, published report and responses.

The audit itself is not in this budget: I will scope it in M2 with a firm that has
audited this code family (zkSecurity audited r0vm-helios, Zellic SP1 Helios, Veridise
Telepathy) for Logos to contract directly, as the RFP anticipates.

**Companion proposal.** I am also applying for
[RFP-022](https://github.com/logos-co/rfp/blob/master/RFPs/RFP-022-wrapped-erc20.md), the
privacy-preserving bridge that is this attestation primitive's first consumer. RFP-022
states that combined proposals are preferred, and the two share one verification core: the
bridge mints only against a statement this primitive produces. Either can be funded alone;
funded together, M3's core module and daemon serve both, and I would discount the combined
scope accordingly.

Each milestone stands on its own. If Logos wants to combine work across proposals, M3
(core module, CLI, daemon) and the private-verifier and private-consumer paths can be
delivered separately.

## Total Requested Budget (USD)

$30,000

## Relevant Experience

- **This prototype:** [github.com/bluntbrain/affidavit](https://github.com/bluntbrain/affidavit). Light client, inclusion, statements and the LEZ program, on mainnet data and the official spec vectors, with zkVM cycle counts per program action and an operator simulation.
- **Infrastructure Lead at Spout Finance** (tokenised real-world assets, 2026–): own the cloud and release path (Terraform, AWS, Fargate, RDS, Kubernetes, CI/CD) for infrastructure that carries real money. The update daemon, its operator journey and the CI this RFP asks for are this job.
- **Smart contracts, Solidity and Rust:** an [exogenously collateralised stablecoin with liquidations](https://github.com/bluntbrain/stable-coin-foundry) and [invariant testing](https://github.com/bluntbrain/invariant-testing-foundry) in Foundry; [Solana programs in Anchor and `no_std` Pinocchio](https://github.com/bluntbrain/solana-projects) (vaults, escrow, AMM, flash loans, CPI); a [Solana lending protocol](https://github.com/bluntbrain/solana-lending-protocol).
- **Hackathon wins:** ETHGlobal SuperHack ($10,000, Repo Rewards on Base), StarkHack ($4,000, Cairo), ETH Bangkok ($2,000, a ZK credit score proving creditworthiness without revealing balances).
- **Shipped products:** Senior engineer at SendAI (Solana consumer apps) and DxSale (DeFi launchpad: 500+ token sales, $10M+ volume); co-founder of Krane Apps (team of 10; blockchain and DeFi products including Glitter Finance); frontend lead at Jar (10M+ users); [NearMe](https://bluntbrain.com/seeker) (20,000+ downloads on the Solana Seeker dApp Store).
- **Open source:** [45 merged PRs](https://github.com/crestalnetwork/intentkit/pulls?q=is%3Apr+author%3Abluntbrain) to IntentKit, the most from outside its core team.

What I have not done before is ship an Ethereum consensus client, so I did the hard part
first: the prototype implements the light client to the specification and proves it on
mainnet data and the spec vectors before asking for anything.

## Post-Delivery Plan

For 12 months after M4, at no extra cost:
- Security reports acknowledged within 48 hours; fixes released as a new program version
  with the consumer migration documented (the program is immutable by design).
- Fork readiness: each consensus fork that changes light-client containers gets a program
  version released before it activates on Hoodi and mainnet, starting with Gloas.
- Track LEZ testnet releases and the Logos module SDK; keep CI green.
- Run a public operator for the testnet instances, and publish the daemon's metrics.
- Answer issues and integration questions, starting with the RFP-022 bridge team.

After that, the code is MIT/Apache-2.0 and documented for anyone to operate and maintain.

## Permissions and Consent

- [x] Logos may contact me using the primary contact above.
- [x] Logos may use information from this proposal publicly.

## Program Requirements

- [x] I have read and agree to the Logos RFP Terms and Conditions, and understand no grant is awarded until a Grant Agreement is executed.
- [x] I understand RFP specifications are proposals rather than instructions, and that I am responsible for what I build, deploy or operate.
- [x] I understand the project must be open-sourced under MIT and Apache 2.0.
- [x] I am prepared to deliver milestone-based outcomes.
