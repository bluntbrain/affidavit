// Position recovery from the seed alone (RFP-022 Functionality #9, Reliability #5), in the
// client's language. The derivation mirrors `bridge/src/recovery.rs` byte for byte and is
// pinned to the same vector, so a Rust client and a JavaScript client can never drift apart.
// The end-to-end case runs the scan over real `Deposit` logs from the vault on the test EVM:
// no index, no server, nothing remembered but the seed.
import assert from "node:assert/strict";
import { before, describe, it } from "node:test";
import { ethers } from "ethers";
import { compile } from "./compile.mjs";
import { Evm } from "./evm.mjs";

import { depositCommitment, depositSecret, recoverDeposits } from "./recovery.mjs";

const ETH = ethers.ZeroAddress;

describe("derivation matches the Rust client (bridge/src/recovery.rs)", () => {
  it("reproduces the pinned cross-language vector", () => {
    const seed = "0x" + "42".repeat(32);
    const at = {
      chainId: 1n,
      vault: "0x0000000aa232009084bd71a5797d089aa4edfad4",
      token: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
    };
    const s = depositSecret(seed, at, 7);
    assert.equal(s, "0xcb8933db0ce443e07872f406f53ccae8299a3d65f1f4f521effb0c288b2ed653");
    assert.equal(depositCommitment(s), "0x5b6c608b41ea9a69d4c57ddf44c3667a067771cf5b8d6874c219fbf56b761dbe");
  });

  it("binds the commitment to chain, vault, token, nonce and seed", () => {
    const seed = "0x" + "42".repeat(32);
    const at = { chainId: 1n, vault: "0x" + "11".repeat(20), token: ETH };
    const c = depositCommitment(depositSecret(seed, at, 0));
    assert.notEqual(c, depositCommitment(depositSecret(seed, { ...at, chainId: 11155111n }, 0)));
    assert.notEqual(c, depositCommitment(depositSecret(seed, { ...at, vault: "0x" + "22".repeat(20) }, 0)));
    assert.notEqual(c, depositCommitment(depositSecret(seed, { ...at, token: "0x" + "33".repeat(20) }, 0)));
    assert.notEqual(c, depositCommitment(depositSecret(seed, at, 1)));
    assert.notEqual(c, depositCommitment(depositSecret("0x" + "43".repeat(32), at, 0)));
  });
});

describe("a client restored from its seed finds its deposits in the vault's logs", () => {
  const ALICE_SEED = "0x" + "a1".repeat(32);
  const BOB_SEED = "0x" + "b0".repeat(32);
  let evm, admin, alice, bob, vault, at;
  const logs = [];

  async function deposit(from, seed, nonce) {
    const c = depositCommitment(depositSecret(seed, at, nonce));
    const res = await vault.send(from, "depositETH", [c], ethers.parseEther("0.1"));
    for (const l of res.logs) {
      if (l.parsed?.name === "Deposit") {
        const { index, token, amount, commitment } = l.parsed.args;
        logs.push({ index, token, amount, commitment });
      }
    }
  }

  before(async () => {
    const { contracts } = compile();
    evm = await Evm.create();
    [admin, alice, bob] = await Promise.all([1, 3, 6].map((b) => evm.account(b)));
    const verifier = await evm.deploy(admin, contracts.MockRiscZeroVerifier);
    const anchor = await evm.deploy(admin, contracts.MockLezStateAnchor);
    vault = await evm.deploy(admin, contracts.CausewayVault, [
      ethers.id("lez-zone-testnet-0.4"),
      ethers.id("causeway-bridge-program"),
      ethers.id("causeway-burn-guest-image-id"),
      verifier.address,
      anchor.address,
      admin.hex,
      admin.hex,
      64n,
    ]);
    await vault.send(admin, "registerToken", [ETH, 18, [ethers.parseEther("0.1"), ethers.parseEther("1")], 0n, 0n]);
    // The coordinates come from the chain and the deployment, never from configuration the
    // user would have to remember.
    at = { chainId: await vault.call("chainId"), vault: vault.address, token: ETH };
    assert.equal(at.chainId, 1n);

    // Alice deposits at nonces 0, 1 and 3 (nonce 2 was never used); Bob's deposits are
    // interleaved. The vault sees only commitments: nothing says which are whose.
    await deposit(bob, BOB_SEED, 0);
    await deposit(alice, ALICE_SEED, 0);
    await deposit(bob, BOB_SEED, 1);
    await deposit(alice, ALICE_SEED, 1);
    await deposit(bob, BOB_SEED, 2);
    await deposit(alice, ALICE_SEED, 3);
  });

  it("recovers exactly its own deposits, with the nonce and secret each claim needs", () => {
    const mine = recoverDeposits(ALICE_SEED, at, logs);
    assert.deepEqual(
      mine.map((d) => [d.nonce, Number(d.log.index)]),
      [
        [0, 1],
        [1, 3],
        [3, 5],
      ],
    );
    for (const d of mine) {
      assert.equal(d.log.amount, ethers.parseEther("0.1"));
      assert.equal(depositCommitment(d.secret), d.log.commitment);
    }
  });

  it("never finds anyone else's, and a wrong seed finds nothing", () => {
    assert.deepEqual(
      recoverDeposits(BOB_SEED, at, logs).map((d) => Number(d.log.index)),
      [0, 2, 4],
    );
    assert.deepEqual(recoverDeposits("0x" + "ee".repeat(32), at, logs), []);
    // The same seed on another deployment finds nothing here: commitments are bound to it.
    assert.deepEqual(recoverDeposits(ALICE_SEED, { ...at, chainId: 11155111n }, logs), []);
  });
});
