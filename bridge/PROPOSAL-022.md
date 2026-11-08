# [PROPOSAL] RFP-022 — Causeway: a privacy-preserving Ethereum ⇄ LEZ bridge on Affidavit

The text submitted to [logos-co/rfp](https://github.com/logos-co/rfp/issues/new?template=proposal.yml),
field by field. This proposal is the **companion to my RFP-021 proposal (Affidavit)**;
RFP-022 states that combined proposals are preferred, and this bridge is Affidavit's first
consumer. The attestation core is the same one, audited once and shared.

---

## RFP ID

RFP-022 — Privacy-Preserving Wrapped ERC-20 and Ether Bridge for LEZ

## Your Project Name

Causeway

## Team or Organization Name

Independent

## Primary Contact

ishan.lakhwani@gmail.com · Telegram @bluntbrainsol

## Team Members

**Ishan Lakhwani** (bluntbrain)
- Social links: [github.com/bluntbrain](https://github.com/bluntbrain) · [bluntbrain.com](https://bluntbrain.com) · [linkedin.com/in/ishanl](https://linkedin.com/in/ishanl) · [x.com/bluntbrain](https://x.com/bluntbrain)
- Role: everything in this proposal — the Ethereum vault, the LEZ bridge and wrapped-token programs, the privacy construction, the Logos core and UI modules, the CLI and relayer, and the cost and privacy test suites. The Ethereum state attestation this bridge consumes is my RFP-021 submission (Affidavit).
- Status: full-time for the RFP-022 engagement. This is a tier-L, 16–20 week project and I am scoping it as my primary work for that window.
- One accountable engineer across the Ethereum vault, the LEZ programs and the modules: no cross-team hand-offs on a security-critical bridge, and the same person who built the Affidavit core it depends on.

The independent smart-contract audit (Security #1) is scoped during the grant with a firm
that audits both Solidity and RISC Zero code, for Logos to contract directly (see
Milestones).

## Project Summary

A bridge is where bridges get robbed: cross-chain bridges were 64% of the $3.1B stolen
from DeFi in 2022. Most of that was either a trusted key that got compromised or
verification logic that was wrong. Causeway removes the first entirely — no signer,
federation or relayer is ever trusted with a user's funds — and treats the second as the
real work: the verification path is fixed at deployment, built on an attestation core that
is proven on mainnet data and audited once for every LEZ program that reads Ethereum.

Causeway is a trustless lock-and-mint bridge. An Ethereum **vault** escrows an ERC-20 or
native ETH. A LEZ **bridge program** mints the wrapped token only after cryptographically
verifying the deposit was finalised on Ethereum — using the RFP-021 attestation primitive,
not its own consensus code (Functionality #3). Burning the wrapped token on LEZ entitles
the holder to release the original from the vault, on proof of the burn verified natively
on Ethereum. Neither leg publishes a link between its two ends: a deposit never names its
LEZ destination, a burn never names its Ethereum destination, and the only public signals
are amount, token and timing, which the design works to make uninformative.

The privacy is not bolted on; it falls out of how the attestation is consumed. LEZ already
ships the cross-zone plumbing for a wrapped token — `wrapped_token`, `bridge`,
`cross_zone_inbox/outbox/marker` in `lez/programs` — but its inbox carries the comment
`l1_inclusion_witness must be None in v1`: today that mint **trusts** that the Ethereum
deposit happened. Causeway fills that `None` with a verified Affidavit statement, and does
it on the **private** attestation path, where the consumer proves its deposit is in the
instance's header accumulator and discloses only a root every consumer shares. That is what
makes inbound unlinkability (Privacy Preservation #1) actually hold: the public transaction
reveals a nullifier, an amount and a token, never which Ethereum deposit funded the mint.

I have built the attestation core and the seam already:
**[github.com/bluntbrain/affidavit](https://github.com/bluntbrain/affidavit)** is the
RFP-021 primitive — a full Ethereum light client inside a LEZ program, proven on mainnet
data and the official consensus-spec-tests, building on `lee_core` v0.4.0-rc1 — and
`bridge/` in that repo is a runnable demonstration of the inbound mint authorisation:
it takes an Affidavit statement about an ERC-20 `Transfer` into the vault, checks the
registry, denomination and finality predicate, derives the one-time deposit nullifier, and
authorises the wrapped mint, refusing a replay, a wrong amount, a wrong token and an
unfinalised deposit. The bridge proper is the grant.

## Technical Approach

**Stack.** Solidity for the Ethereum vault (prepared for audit, no upgradeable proxy on the
verification path, per Security #8); Rust LEZ programs on `lee_core` v0.4.0-rc1 reusing
LEZ's own `wrapped_token` and `cross_zone` crates; the RFP-021 Affidavit primitive for the
inbound proof; RISC Zero 3.0 (LEZ's own zkVM) for LEZ-side proving, with the burn proof
wrapped to Groth16 for cheap verification in the vault; the admin authority (RFP-001) for
the registry and caps and the freeze authority (RFP-002) for the circuit breaker; the Logos
Ethereum core module for all Ethereum access; Logos core and UI modules for everything
off-chain; SPEL for the IDL.

**Inbound — deposit to mint (the private path).**
1. A user deposits amount *N* of a registered token to the vault. *N* is one of the token's
   fixed denominations (Privacy #1 baseline). The deposit's only secret is a commitment
   `c = H(s)` where `s = KDF(seed, token, nonce)` is derived from the user's own wallet
   seed — so the deposit names no LEZ destination (#F2), and the user can rediscover every
   unclaimed deposit by scanning vault events and recomputing `c` from their seed, with no
   server index and no separately stored secret (#F9).
2. Affidavit attests the deposit: the vault's `Deposit` log (or the ERC-20 `Transfer` to
   the vault) is included under an Ethereum header finalised to the configured depth (#F12).
3. The LEZ bridge program mints *N* wrapped to a destination the user chooses at claim
   time, as a **private** LEZ execution. The proof establishes, in zero knowledge: the
   Affidavit statement is valid against the instance's shared accumulator root (one
   `CheckRoot`, the only public crossing); the deposit is to this configuration's vault, of
   a registered token, for denomination *N*; and the one-time nullifier `H(s)` has not been
   spent. The public transaction shows a nullifier, *N*, and the token — never the deposit
   or the destination (#Privacy 1, 3). The nullifier keyed on the statement enforces
   one-mint-per-deposit (#Reliability 3), and because the destination is bound inside the
   proof, a relayer who submits the receipt cannot redirect or overcharge (#Security 3).

This is exactly LEZ's `wrapped_token::MintFrom` authorised by a delivery — but the delivery
is authorised by an Affidavit statement instead of a trusted `None`.

**Outbound — burn to release.**
1. The holder burns *N* wrapped on LEZ (private execution), producing a burn record bound to
   an Ethereum recipient `r` secret-committed from their seed (#F5, #F9), and a nullifier so
   a burn releases once (#Reliability 3).
2. The vault releases *N* of the token to `r` on a proof of the burn, verified **natively on
   Ethereum**: a RISC Zero receipt of the burn, wrapped to Groth16, checked by a verifier
   fixed in the vault (#F7, #Security 2, 8). The burn must be final on LEZ to the configured
   depth before release (#F12). This LEZ→Ethereum direction is the bridge's own; RFP-021
   covers only Ethereum→LEZ, so the outbound verifier is new work here.

**Why the private path matters for the bridge.** A bridge built on an attestation that
discloses *which* finalised header it read leaks the deposit's timing-and-block to anyone
watching, shrinking the anonymity set toward one. Affidavit's accumulator path (RFP-021
Functionality #8) discloses only a root shared by every consumer of the instance, so the
mint is unlinkable to the deposit beyond amount-token-timing. The two RFPs are designed to
fit: this is the strongest reason to build them as one body of work.

**Privacy, stated precisely.** Fixed per-token denominations are the baseline (#F8); the
mini-app defaults to them, to delayed submission, and to a permissionless relayer paid out
of the bridged amount, and shows what each choice would leak (#Usability 5, 6, #Privacy 7).
Value commitments (Soft #1) are the stronger successor; the vault and accounts are laid out
so they can be adopted without redeploying or resetting the accumulated anonymity set, and
the migration path is documented.

**Governance and safety.** The token registry, caps and finality depth are admin-governed
via RFP-001; the freeze authority via RFP-002 is independent on each leg (#F14, #Security 5).
Caps are per-token and global over a rolling window (#F11). The failsafe for a discovered
verification bug (#Security 9) lets in-flight depositors and burners recover by proving
their own entitlement through the ordinary claim path — never an admin redirect — so the
failsafe is not a backdoor.

**Anticipated challenges.**
1. *The LEZ→Ethereum burn proof.* Verifying a LEZ receipt on Ethereum is the heaviest new
   piece. M0 settles the proof system (RISC Zero → Groth16 via the LEZ proving stack) and
   measures on-chain verification gas before the design freezes.
2. *Fixed denominations vs. fees.* A protocol fee must not re-fingerprint the amount
   (#F13). I handle fees as separate fixed-denomination flows or value-conserving deductions
   that keep the public amount on the denomination grid; M2 measures the anonymity-set cost.
3. *Finality on both chains.* Reorg-safe depths each way, configurable, with the residual
   risk documented (#F12, Design Rationale).
4. *Position recovery completeness.* Every unclaimed deposit and unreleased burn must be
   rediscoverable from the seed alone (#F9, #Reliability 5); the recovery scanner is a
   first-class deliverable with its own tests, not an afterthought.

## Milestones, Payout and Timeline

Eighteen weeks of build, inside the RFP's 16–20, plus remediation after the audit. Every
milestone is a tagged release with green CI, updated docs and the decision log. This
assumes the RFP-021 attestation primitive (my companion proposal) is in progress or
funded; M0 pins the exact interface between them.

**M0 — Design, threat model, proof-system baseline** · 2 weeks · $7,000
The full architecture and threat model; the Ethereum→LEZ and LEZ→Ethereum proof interfaces
pinned against Affidavit and the LEZ proving stack; on-chain burn-proof verification gas
measured; the privacy model written as testable anonymity-set claims; the inbound
mint-authorisation demo (already prototyped) extended to the chosen commitment scheme;
design freeze.

**M1 — Ethereum vault + inbound mint on testnet 0.4** · 4 weeks · $16,000
The vault (deposit, registry hooks, native ETH + USDT-style no-boolean tokens, #F1, #F17);
the LEZ bridge + wrapped-token programs; the private inbound mint against a live Affidavit
instance; one-mint-per-deposit and the destination binding under adversarial retry
(#Reliability 1–3, #Security 2–3). Deployed to a public Ethereum testnet and LEZ testnet 0.4.

**M2 — Outbound burn + release, privacy suite** · 4 weeks · $16,000
The burn path and the vault's native burn-proof verification and release (#F5–F7); fixed
denominations, delayed submission, permissionless relayer on by default; the privacy test
suite (inbound/outbound unlinkability, indistinguishable error paths, full round-trip
state-diff, #Privacy 1–7); caps, fees and finality depth configurable (#F8, F11–F14).

**M3 — Modules, mini-app, recovery, CLI** · 4 weeks · $16,000
Logos core module (deposit, claim, redeem, release, recovery, registry read/admin); the
mini-app and admin UI (#Usability 1, 2); position recovery from seed alone, with its tests
(#F9, #Reliability 5); the relayer/daemon as a Logos module plus headless CLI; SPEL IDL;
error messages that reveal nothing (#Usability 8).

**M4 — Hardening, e2e CI, release, audit-ready** · 3 weeks · $9,000
End-to-end round-trip tests against a standalone sequencer and an Ethereum fork in CI
(#Reliability 10); multi-entity independent configurations (#F16); the failsafe (#Security 9);
module catalog release; the deployer, operator and developer docs; audit package.

**M5 — Audit remediation** · 1 week after the report · $4,000
Fixes for every finding, re-verification, published report.

Planned build total: **$68,000**. The independent audit is not in this budget; I will scope
it in M2 for Logos to contract directly, as the RFP anticipates. **Combined with RFP-021**,
the core module, CLI and daemon (RFP-021 M3) serve both projects, so I would discount the
combined build by $6,000.

## Total Requested Budget (USD)

$68,000

## Relevant Experience

- **The attestation core and the seam, already built:** [github.com/bluntbrain/affidavit](https://github.com/bluntbrain/affidavit) — the RFP-021 primitive on mainnet data and the official spec vectors, building on `lee_core` v0.4.0-rc1, plus `bridge/`, the runnable inbound mint-authorisation demo this proposal rests on.
- **Infrastructure Lead at Spout Finance** (tokenised real-world assets, 2026–): own the cloud and release path (Terraform, AWS, Fargate, RDS, Kubernetes, CI/CD) for infrastructure that moves real money.
- **Solidity and smart-contract security:** an [exogenously collateralised stablecoin with liquidations](https://github.com/bluntbrain/stable-coin-foundry) and [invariant/fuzz testing](https://github.com/bluntbrain/invariant-testing-foundry) in Foundry — directly the escrow-vault, cap and invariant work this RFP needs.
- **ZK and privacy:** ETH Bangkok win ($2,000) for a ZK credit score proving creditworthiness without revealing balances — anonymity-set reasoning is the core skill RFP-022 asks for.
- **Rust and on-chain programs:** [Solana programs in Anchor and `no_std` Pinocchio](https://github.com/bluntbrain/solana-projects) (vaults, escrow, AMM, flash loans, CPI); a [Solana lending protocol](https://github.com/bluntbrain/solana-lending-protocol). LEZ's account/PDA model is close to Solana's, which is where I work daily.
- **Hackathon wins:** ETHGlobal SuperHack ($10,000, Base), StarkHack ($4,000, Cairo), ETH Bangkok ($2,000).
- **Shipped products:** senior engineer at SendAI and DxSale (DeFi launchpad, 500+ sales, $10M+ volume); co-founder of Krane Apps; frontend lead at Jar (10M+ users).
- **Open source:** [45 merged PRs](https://github.com/crestalnetwork/intentkit/pulls?q=is%3Apr+author%3Abluntbrain) to IntentKit.

## Post-Delivery Plan

For 12 months after M4, at no extra cost: security reports acknowledged within 48 hours and
fixed as a new, migrated program/contract version (the verification path is immutable by
design); fork readiness tracking the Affidavit primitive; CI kept green against LEZ testnet
releases; a public relayer run for the testnet deployment; and answers to integration
questions from the DEX, lending and stablecoin teams that consume wrapped assets. After
that the code is MIT/Apache-2.0 and documented for anyone to operate.

## Permissions and Consent

- [x] Logos may contact me using the primary contact above.
- [x] Logos may use information from this proposal publicly.

## Program Requirements

- [x] I have read and agree to the Logos RFP Terms and Conditions, and understand no grant is awarded until a Grant Agreement is executed.
- [x] I understand RFP specifications are proposals rather than instructions, and that I am responsible for what I build, deploy or operate.
- [x] I understand the project must be open-sourced under MIT and Apache 2.0.
- [x] I am prepared to deliver milestone-based outcomes.
