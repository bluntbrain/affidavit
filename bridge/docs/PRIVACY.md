# Privacy: what Causeway hides, what it shows, and how it is measured

RFP-022's Privacy Preservation requirements, restated as properties that a test can check,
with the mechanism behind each and the correlations that remain. This is the document the
M2 privacy suite is written against.

## What is public, leg by leg

| event | where | public | hidden |
|---|---|---|---|
| Deposit | Ethereum | depositor's address, token, amount *N*, commitment `c` | the secret `s`, the future LEZ destination (there is none to hide: the deposit names no destination, Functionality #2) |
| Claim (mint) | LEZ | nullifier `n`, token, *N*, the instance's shared accumulator root | **which deposit** funded it, the destination, `s` |
| Burn | LEZ | the wrapped note's nullifier, token, *N*, recipient commitment `r` | the burner (LEZ private execution), the Ethereum recipient |
| Release | Ethereum | release nullifier `n_b`, token, *N*, recipient | which burn it came from (see *Outbound unlinkability* for the two stages) |

The only signals that cross both legs are **amount, token and timing**. Everything below
is about making those three as uninformative as possible, and saying precisely where they
still inform.

## Inbound unlinkability rests on the private attestation path

A bridge that proves "this deposit is included under finalised header *H*" and publishes
*H* has given the observer the block, and the block has a handful of deposits: the
anonymity set is roughly one. That is what a public attestation (Affidavit's `AttestRecord`)
would do.

Causeway consumes Affidavit on its **private** path (RFP-021 Functionality #8): the claim
proves, inside the zkVM, that the deposit's statement is in the instance's header
accumulator, and discloses only the accumulator **root**, the same value every consumer of
the instance discloses. The public claim therefore says "some finalised deposit of *N* of
this token", nothing about which. The anonymity set is every unclaimed deposit of that
(token, denomination) pair, across all time, not the few in one block.

This is why the two RFPs are proposed as one body of work: the bridge's central privacy
property is a property of how the attestation is consumed.

## Nullifiers are secret-derived

A nullifier has to be unique per deposit, so one deposit mints once (Reliability #3), and
it must not be computable from public data, or every claim is trivially matched to its
deposit. Causeway's is

```text
n = sha256("causeway/deposit-nullifier/v1" ‖ s ‖ statement.id)
```

unique because of the deposit's statement id, unlinkable because of `s`, which only the
depositor's seed yields ([RECOVERY.md](RECOVERY.md)). The claim proves `c = H(s)` for the
attested deposit's `c` and reveals `n`; nothing else about the deposit leaves the prover.

The first cut of the seam demo keyed the nullifier on the statement id alone. That is
unique but **linkable** (anyone can compute `H(id)` for every deposit and match claims), so
it was changed to the form above and the seam test now carries the secret.

## Denominations and fees

Fixed per-token denominations are the baseline (Functionality #8). The vault enforces the
grid at deposit time: an off-grid amount is refused (`InvalidAmount`, tested), so there is
never a deposit with a unique amount to fingerprint.

Fees must not re-fingerprint the amount (Functionality #13). The rule: **a fee is a
function of the denomination and nothing else**, fixed for an epoch, deducted inside the
claim so the minted amount is `N − f(N)`: still exactly one value per denomination. The
relayer is paid out of the same uniform fee, so a relayed claim and a self-submitted claim
mint the same amount. A fee change is an epoch boundary (claims before and after differ in
minted amount), so changes are rare, announced ahead, and the mini-app shows the two sets'
sizes while both exist.

Value commitments (Soft #1) are the stronger successor to denominations; the vault stores
amounts per token and the accounts carry no denomination assumptions, so they can be
adopted without redeploying the vault or resetting the accumulated anonymity set.

## Timing

Depositing and claiming in the same minute links the two by time alone. The defaults are
delayed submission with a randomised wait, and a permissionless relayer so the claim is not
paid for from an account that is itself linkable. The mini-app shows, before the user
claims, the current anonymity set for the denomination (unclaimed deposits) and what
submitting now would leak (Usability #5, #6; Privacy #7). None of this is enforced by the
protocol: a user who wants to claim immediately may, and is told what it costs.

## Outbound unlinkability, in two stages

The burn is a private LEZ execution, so the burner is hidden by LEZ itself; the burn record
carries `r`, a commitment to the recipient, never the recipient (Functionality #5).

- **Stage 1 (first M2 build):** the release journal names the burn record's nullifier
  directly. Burn and release are linkable to each other, but the burner is still anonymous
  on LEZ and the recipient still appears nowhere on LEZ.
- **Stage 2 (M2 target):** LEZ keeps an accumulator of burn records; the release proves
  membership of *a* burn of (token, *N*) at a final LEZ state and reveals a derived
  nullifier `n_b = sha256("causeway/burn-nullifier/v1" ‖ s_b ‖ burn id)`. The release then
  says only "some final burn of *N* of this token".

The vault is already shaped for stage 2: `release` takes a journal with a nullifier and a
LEZ state root, checks the root is final and the nullifier unspent, and verifies the proof
against a fixed image id. Which statement the image proves is the guest's business, not
the vault's.

## Failure paths

On Ethereum the vault's errors are distinct and public (`WrongPairing`,
`InsufficientFinality`, `AlreadyReleased`, `VerificationFailed`, `CapExceeded`, …), and
none of them involves a secret: they are the same for every user and leak nothing about
positions. On LEZ a claim or burn that would fail never reaches the chain at all: the
prover fails locally, so the sequencer only ever sees valid private transactions. The
module's error messages carry a code and a next step, never deposit data (Usability #8).

## Residual correlations, stated plainly

- **Amount, token, time** remain public signals; the anonymity set is bounded by how many
  unclaimed deposits of that denomination exist in the user's chosen window.
- **Address reuse.** A user who releases to the same Ethereum address they deposited from
  links the round trip themselves. The mini-app warns and offers a fresh address.
- **Fee epochs**, **cap exhaustion** and **freezes** change the public amount or stop
  flows, splitting sets at the boundary.
- **Relayer and sequencer collusion** sees submission timing, which is why delays are the
  default.
- **Off-protocol analysis** of the depositor's funding or of the recipient's later spending
  is outside what any bridge can hide.

## The privacy test suite (M2)

Each row is a test that runs the full flow against a standalone LEZ sequencer and an
Ethereum fork and inspects the complete public state diff:

| property | check |
|---|---|
| Inbound unlinkability (Privacy #1) | for two deposits of the same denomination, the two claims' public diffs are identical up to nullifier, and neither nullifier is computable from public data |
| No destination on chain (#F2, #F5) | no public field on either chain equals or is derived from a destination or recipient without the secret |
| Shared root only | every claim discloses the same accumulator root as every other consumer's `CheckRoot` in the epoch |
| Denomination grid holds (#F8, #F13) | the set of minted amounts per token has at most one value per denomination per fee epoch |
| Error indistinguishability (Privacy #4) | every failing claim fails before submission; vault reverts carry no position data |
| Round-trip diff (Privacy #6) | a full deposit→claim→burn→release leaves exactly the documented public fields and nothing else |
| Recovery leaks nothing (#F9) | the scan issues only public reads; no server, no indexed secret |
