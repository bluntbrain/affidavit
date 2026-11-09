# Recovery from the seed alone

RFP-022 calls loss of access between lock and mint "the largest UX risk", and asks
(Functionality #9, Reliability #5) that a user who has nothing but their wallet seed can
rediscover every unclaimed deposit and every unreleased burn from on-chain state, with no
server-side index and no secret that was generated during the flow.

Causeway's rule is simple: **nothing is created during a deposit or a burn that the user
must keep.** Every secret is a function of the seed and the public coordinates of the
operation, and every on-chain record carries a commitment to that secret and nothing else.

## Deposits

```text
s = sha256("causeway/deposit-secret/v1"     ‖ seed ‖ chain_id ‖ vault ‖ token ‖ nonce)
c = sha256("causeway/deposit-commitment/v1" ‖ s)
```

`c` is the only thing the depositor gives the vault (`depositETH(c)`, `depositERC20(token,
amount, c)`); the vault logs `Deposit(index, token, amount, c)` and never looks at `c`
again. `s` is the opening the private LEZ claim proves knowledge of, and the input to the
claim's nullifier, so only the holder of the seed can complete the claim.

Why each input is in the derivation:

| input | why |
|---|---|
| domain string | the same seed is used by the wallet for other things; nothing else can collide with Causeway's secrets, and `v1` leaves room to change the scheme without ambiguity |
| `seed` | the one thing the user keeps |
| `chain_id`, `vault` | a seed never yields the same commitment on two deployments (Functionality #15). A scan against the wrong deployment finds nothing, which the tests check |
| `token` | one nonce space per registered token, so the scanner works token by token |
| `nonce` | the BIP-44-style index that makes every deposit's secret distinct |

**Scanning.** A restored client walks `nonce = 0, 1, 2, …`, recomputes `c`, and looks for
it among the vault's `Deposit` logs (`eth_getLogs` on the vault's address, through the Logos
Ethereum module). It stops after 20 consecutive nonces with no match: the gap limit,
exactly as a BIP-44 wallet rediscovers its addresses. The limit is a client setting, and
the trade-off is the usual one (a deposit made far past the limit needs a wider scan).

For each match the client has the deposit's index, amount, nonce and secret: everything the
claim needs. Whether a deposit was already claimed is answered on LEZ, not Ethereum: the
client recomputes its nullifier `n = sha256("causeway/deposit-nullifier/v1" ‖ s ‖ id)` and
asks whether `n` is spent. The result is the complete list of unclaimed deposits, from the
seed and public state alone.

## Burns

The outbound leg uses the same shape:

```text
s_b = sha256("causeway/burn-secret/v1"    ‖ seed ‖ chain_id ‖ vault ‖ token ‖ nonce)
r   = sha256("causeway/burn-recipient/v1" ‖ s_b ‖ recipient)
```

The LEZ burn record carries `r`, a commitment to the Ethereum recipient, never the
recipient itself (Functionality #5). To recover unreleased burns, the client recomputes `r`
over its candidate nonces and its own Ethereum addresses (which come from the same seed,
through the wallet's ordinary derivation paths), matches them against the public burn
records on LEZ, and asks the vault whether each burn's release nullifier is spent
(`spentNullifier(n_b)`). Unreleased burns are the ones that are not.

## Properties

- **Deterministic and stateless.** Two clients restored from the same seed compute the
  same lists. There is no database, no backup file and no "recovery phrase" beyond the
  wallet's own.
- **Cross-deployment isolation.** Binding `chain_id` and `vault` means a Sepolia rehearsal
  and a mainnet deposit can never be confused, and a second Causeway instance (another
  operator's configuration, Functionality #16) has its own commitment space.
- **Nonce reuse is harmless.** If a client reuses a nonce (say, after a crash between
  deriving and sending), both deposits carry the same `c`; both are found, and because the
  nullifier is keyed on the deposit's statement id as well as `s`, both can be claimed.
- **The vault learns nothing.** `c` is opaque; it is not checked, stored or indexed by the
  contract, only logged. The vault tests include a depositor reusing a commitment and an
  arbitrary 32-byte value, both accepted.
- **Seed compromise is total**, exactly as for the wallet itself. Causeway adds no new
  secret to protect, which is the point.

## What is built

| piece | where | tests |
|---|---|---|
| Derivation, scan with gap limit, burn commitments (Rust) | [`../src/recovery.rs`](../src/recovery.rs) | 5: determinism and binding, a restored client finds exactly its own deposits among a stranger's, gap limit, recipient binding, pinned vector |
| The same derivation and scan in JavaScript, run end to end against **real `Deposit` logs** on the test EVM: two users make six interleaved deposits, each recovers exactly their own, a wrong seed and a wrong chain find nothing | [`../contracts/test/recovery.test.mjs`](../contracts/test/recovery.test.mjs) | 4 |

Both sides pin the same vector, so a change to the derivation has to be made deliberately
in both languages at once:

```text
seed        0x4242…42 (32 bytes)
chain_id    1
vault       0x0000000aa232009084bd71a5797d089aa4edfad4
token       0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 (USDC)
nonce       7
secret      0xcb8933db0ce443e07872f406f53ccae8299a3d65f1f4f521effb0c288b2ed653
commitment  0x5b6c608b41ea9a69d4c57ddf44c3667a067771cf5b8d6874c219fbf56b761dbe
```

SHA-256 throughout: it is the hash both the LEZ guest (RISC Zero's accelerated `sha2`) and
the wallet SDK compute cheaply, and the one Affidavit already uses for its statements.

## What the grant adds (M3)

The Logos core module wires this into the wallet: seed access through the wallet's own
key-derivation, `eth_getLogs` through the Ethereum module, nullifier and burn-record queries
against LEZ, and a "Recover positions" flow in the mini-app that lists unclaimed deposits
and unreleased burns with one action each. Its tests run the scan against a standalone LEZ
sequencer and an Ethereum fork after a simulated wallet restore (Reliability #5, #10).
