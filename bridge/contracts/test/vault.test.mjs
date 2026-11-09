// CausewayVault behaviour, requirement by requirement, on an in-process EVM. Every hard
// requirement the vault is responsible for has a case here (Reliability #11); gas is
// measured per operation for the cost report (Performance #3).
import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { before, describe, it } from "node:test";
import { ethers } from "ethers";
import { compile } from "./compile.mjs";
import { Evm } from "./evm.mjs";

const abi = ethers.AbiCoder.defaultAbiCoder();
const JOURNAL = "tuple(bytes32,bytes32,uint256,address,bytes32,address,uint256,address,bytes32)";
const ZONE = ethers.id("lez-zone-testnet-0.4");
const PROGRAM = ethers.id("causeway-bridge-program");
const IMAGE_ID = ethers.id("causeway-burn-guest-image-id");
const ETH = ethers.ZeroAddress;
const gas = {};

let evm, admin, freezer, alice, relayer, recipient, verifier, anchor, usdc, usdt, vault, contracts;

function commitment(label) {
  return ethers.sha256(ethers.toUtf8Bytes(`commitment:${label}`));
}

/// A burn journal bound to this deployment, and the mock seal the verifier accepts for it.
function burn({ token, amount, nullifier, root, to = recipient.hex, chainId = 1n, vaultAddr, zone = ZONE, program = PROGRAM }) {
  const j = [zone, program, chainId, vaultAddr ?? vault.address, nullifier, token, amount, to, root];
  const digest = ethers.sha256(abi.encode([JOURNAL], [j]));
  const seal = abi.encode(["bytes32", "bytes32"], [IMAGE_ID, digest]);
  return { j, seal, digest };
}

async function expectRevert(promise, errorName) {
  await assert.rejects(promise, (e) => {
    assert.equal(e.errorName, errorName, `expected ${errorName}, got ${e.errorName ?? e.message}`);
    return true;
  });
}

before(async () => {
  ({ contracts } = compile());
  evm = await Evm.create();
  [admin, freezer, alice, relayer, recipient] = await Promise.all([1, 2, 3, 4, 5].map((b) => evm.account(b)));
  verifier = await evm.deploy(admin, contracts.MockRiscZeroVerifier);
  anchor = await evm.deploy(admin, contracts.MockLezStateAnchor);
  usdc = await evm.deploy(admin, contracts.MockERC20, ["USD Coin", 6]);
  usdt = await evm.deploy(admin, contracts.NoReturnERC20);
  vault = await evm.deploy(admin, contracts.CausewayVault, [
    ZONE,
    PROGRAM,
    IMAGE_ID,
    verifier.address,
    anchor.address,
    admin.hex,
    freezer.hex,
    64n,
  ]);
  // Registry (F#10): ETH and USDC with fixed denominations (F#8); USDT-style token too (F#17).
  await vault.send(admin, "registerToken", [ETH, 18, [ethers.parseEther("0.1"), ethers.parseEther("1")], 0n, 0n]);
  await vault.send(admin, "registerToken", [usdc.address, 6, [10_000000n, 100_000000n, 1000_000000n], 0n, 2000_000000n]);
  await vault.send(admin, "registerToken", [usdt.address, 6, [100_000000n], 0n, 0n]);
  await usdc.send(admin, "mint", [alice.hex, 100_000_000000n]);
  await usdt.send(admin, "mint", [alice.hex, 1_000_000000n]);
  await usdc.send(alice, "approve", [vault.address, ethers.MaxUint256]);
  await usdt.send(alice, "approve", [vault.address, ethers.MaxUint256]);
});

describe("pairing is immutable and bound to this deployment (F#15, Security #8)", () => {
  it("exposes the pairing it was deployed with", async () => {
    assert.equal(await vault.call("lezZoneId"), ZONE);
    assert.equal(await vault.call("lezProgramId"), PROGRAM);
    assert.equal(await vault.call("burnImageId"), IMAGE_ID);
    assert.equal(await vault.call("chainId"), 1n);
    assert.equal((await vault.call("verifier")).toLowerCase(), verifier.address.toLowerCase());
  });
});

describe("deposits (F#1, F#2, F#8, F#17)", () => {
  it("escrows native ETH of a registered denomination and logs only token, amount, commitment", async () => {
    const c = commitment("alice-eth-0");
    const { gas: g, logs } = await vault.send(alice, "depositETH", [c], ethers.parseEther("1"));
    gas.deposit_eth = g;
    const dep = logs.find((l) => l.parsed?.name === "Deposit");
    assert.ok(dep, "Deposit event");
    assert.deepEqual(Object.keys(dep.parsed.args.toObject()), ["index", "token", "amount", "commitment"]);
    assert.equal(dep.parsed.args.token, ETH);
    assert.equal(dep.parsed.args.amount, ethers.parseEther("1"));
    assert.equal(dep.parsed.args.commitment, c);
    assert.equal(await vault.call("escrowed", [ETH]), ethers.parseEther("1"));
  });

  it("escrows an ERC-20 (USDC) denomination", async () => {
    const { gas: g } = await vault.send(alice, "depositERC20", [usdc.address, 100_000000n, commitment("alice-usdc-0")]);
    gas.deposit_erc20 = g;
    assert.equal(await vault.call("escrowed", [usdc.address]), 100_000000n);
  });

  it("escrows a USDT-style token whose transferFrom returns nothing", async () => {
    const { gas: g } = await vault.send(alice, "depositERC20", [usdt.address, 100_000000n, commitment("alice-usdt-0")]);
    gas.deposit_usdt_style = g;
    assert.equal(await vault.call("escrowed", [usdt.address]), 100_000000n);
  });

  it("refuses an unregistered token, an amount off the denomination grid, and zero", async () => {
    await expectRevert(vault.send(alice, "depositERC20", [recipient.hex, 100n, commitment("x")]), "UnsupportedToken");
    await expectRevert(vault.send(alice, "depositETH", [commitment("x")], ethers.parseEther("0.5")), "InvalidAmount");
    await expectRevert(vault.send(alice, "depositERC20", [usdc.address, 123_456789n, commitment("x")]), "InvalidAmount");
  });

  it("the depositor's commitment is never checked or stored: the vault knows nothing to leak", async () => {
    // Two deposits with the same commitment are both accepted; uniqueness of the *mint* is
    // the LEZ program's job, keyed on the Affidavit statement id of each log (F#3).
    const c = commitment("same");
    await vault.send(alice, "depositERC20", [usdc.address, 10_000000n, c]);
    await vault.send(alice, "depositERC20", [usdc.address, 10_000000n, c]);
    assert.equal(await vault.call("depositCount"), 5n);
  });
});

describe("freeze authority is independent of admin (F#14, Security #5)", () => {
  it("only the freezer can pause, only the admin can administer", async () => {
    await expectRevert(vault.send(admin, "setFrozen", [usdc.address, false, true, false]), "NotFreezer");
    await expectRevert(vault.send(freezer, "registerToken", [recipient.hex, 18, [1n], 0n, 0n]), "NotAdmin");
  });

  it("pauses one token's deposits, then everything, then resumes", async () => {
    await vault.send(freezer, "setFrozen", [usdc.address, false, true, false]);
    await expectRevert(vault.send(alice, "depositERC20", [usdc.address, 10_000000n, commitment("f")]), "FrozenError");
    await vault.send(alice, "depositETH", [commitment("eth-while-usdc-frozen")], ethers.parseEther("0.1"));
    await vault.send(freezer, "setFrozen", [usdc.address, false, false, false]);

    await vault.send(freezer, "setFrozen", [ETH, true, true, false]);
    await expectRevert(vault.send(alice, "depositETH", [commitment("g")], ethers.parseEther("0.1")), "FrozenError");
    await vault.send(freezer, "setFrozen", [ETH, true, false, false]);
    await vault.send(alice, "depositETH", [commitment("eth-after-unfreeze")], ethers.parseEther("0.1"));
  });
});

describe("caps bound value per rolling window, independently of the freeze (F#11, Security #4)", () => {
  it("refuses a deposit that would exceed the per-token window cap", async () => {
    const already = await vault.call("depositedInWindow", [usdc.address, 0n]);
    await vault.send(admin, "setCaps", [usdc.address, already + 1000_000000n, 2000_000000n]);
    await vault.send(alice, "depositERC20", [usdc.address, 1000_000000n, commitment("cap-ok")]);
    await expectRevert(vault.send(alice, "depositERC20", [usdc.address, 10_000000n, commitment("cap-over")]), "CapExceeded");
    await vault.send(admin, "setCaps", [usdc.address, 0n, 2000_000000n]);
  });

  it("the global cap counts operations across all tokens", async () => {
    const n = await vault.call("globalDepositsInWindow", [0n]);
    await vault.send(admin, "setGlobalCaps", [n + 1n, 0n, 86400n]);
    await vault.send(alice, "depositETH", [commitment("global-ok")], ethers.parseEther("0.1"));
    await expectRevert(vault.send(alice, "depositERC20", [usdc.address, 10_000000n, commitment("global-over")]), "CapExceeded");
    await vault.send(admin, "setGlobalCaps", [0n, 0n, 86400n]);
  });
});

describe("release on a proven LEZ burn (F#5, F#7, Security #2, #3, #6)", () => {
  const rootA = ethers.id("lez-state-root-A");
  const nullA = ethers.id("burn-nullifier-A");

  it("pays the recipient, spends the nullifier and decrements escrow (ERC-20)", async () => {
    await anchor.send(admin, "markFinal", [rootA]);
    const { j, seal } = burn({ token: usdc.address, amount: 100_000000n, nullifier: nullA, root: rootA });
    const before = await usdc.call("balanceOf", [recipient.hex]);
    const escrowBefore = await vault.call("escrowed", [usdc.address]);
    const { gas: g, logs } = await vault.send(relayer, "release", [seal, j]);
    gas.release_erc20 = g;
    assert.equal(await usdc.call("balanceOf", [recipient.hex]), before + 100_000000n);
    assert.equal(await vault.call("escrowed", [usdc.address]), escrowBefore - 100_000000n);
    assert.equal(await vault.call("spentNullifier", [nullA]), true);
    assert.ok(logs.some((l) => l.parsed?.name === "Release"));
  });

  it("pays native ETH too", async () => {
    const nullEth = ethers.id("burn-nullifier-ETH");
    const { j, seal } = burn({ token: ETH, amount: ethers.parseEther("1"), nullifier: nullEth, root: rootA });
    const { gas: g } = await vault.send(relayer, "release", [seal, j]);
    gas.release_eth = g;
    assert.equal(await vault.call("spentNullifier", [nullEth]), true);
  });

  it("refuses a replay of the same burn", async () => {
    const { j, seal } = burn({ token: usdc.address, amount: 100_000000n, nullifier: nullA, root: rootA });
    await expectRevert(vault.send(relayer, "release", [seal, j]), "AlreadyReleased");
  });

  it("a relayer cannot redirect, inflate, or re-target: any change breaks the proof", async () => {
    const nullB = ethers.id("burn-nullifier-B");
    const { seal } = burn({ token: usdc.address, amount: 100_000000n, nullifier: nullB, root: rootA });
    const redirected = burn({ token: usdc.address, amount: 100_000000n, nullifier: nullB, root: rootA, to: relayer.hex }).j;
    await expectRevert(vault.send(relayer, "release", [seal, redirected]), "VerificationFailed");
    const inflated = burn({ token: usdc.address, amount: 1000_000000n, nullifier: nullB, root: rootA }).j;
    await expectRevert(vault.send(relayer, "release", [seal, inflated]), "VerificationFailed");
    await expectRevert(vault.send(relayer, "release", ["0x1234", redirected]), "VerificationFailed");
  });

  it("refuses a proof for another chain, vault, zone or program (F#15)", async () => {
    const nullC = ethers.id("burn-nullifier-C");
    for (const variant of [
      { chainId: 11155111n },
      { vaultAddr: relayer.hex },
      { zone: ethers.id("other-zone") },
      { program: ethers.id("other-program") },
    ]) {
      const { j, seal } = burn({ token: usdc.address, amount: 10_000000n, nullifier: nullC, root: rootA, ...variant });
      await expectRevert(vault.send(relayer, "release", [seal, j]), "WrongPairing");
    }
  });

  it("refuses a burn under a LEZ state root that is not final (F#12)", async () => {
    const { j, seal } = burn({ token: usdc.address, amount: 10_000000n, nullifier: ethers.id("n-D"), root: ethers.id("unknown-root") });
    await expectRevert(vault.send(relayer, "release", [seal, j]), "InsufficientFinality");
  });

  it("never releases more than is escrowed (supply soundness, Security #6)", async () => {
    const escrowNow = await vault.call("escrowed", [usdc.address]);
    const { j, seal } = burn({ token: usdc.address, amount: escrowNow + 10_000000n, nullifier: ethers.id("n-E"), root: rootA });
    await expectRevert(vault.send(relayer, "release", [seal, j]), "InvalidAmount");
  });

  it("respects the release freeze and the release cap", async () => {
    await vault.send(freezer, "setFrozen", [usdc.address, false, false, true]);
    const { j, seal } = burn({ token: usdc.address, amount: 10_000000n, nullifier: ethers.id("n-F"), root: rootA });
    await expectRevert(vault.send(relayer, "release", [seal, j]), "FrozenError");
    await vault.send(freezer, "setFrozen", [usdc.address, false, false, false]);

    const over = burn({ token: usdc.address, amount: 1000_000000n, nullifier: ethers.id("n-G"), root: rootA });
    const over2 = burn({ token: usdc.address, amount: 1000_000000n, nullifier: ethers.id("n-H"), root: rootA });
    await vault.send(relayer, "release", [over.seal, over.j]); // 100 + 1000 released so far: under 2000 cap
    await expectRevert(vault.send(relayer, "release", [over2.seal, over2.j]), "CapExceeded");
  });
});

describe("registry changes with minimum user impact (F#10, F#12)", () => {
  it("delisting stops new deposits but escrowed funds stay releasable", async () => {
    await vault.send(admin, "deregisterToken", [usdt.address]);
    await expectRevert(vault.send(alice, "depositERC20", [usdt.address, 100_000000n, commitment("late")]), "UnsupportedToken");
    const root = ethers.id("lez-state-root-B");
    await anchor.send(admin, "markFinal", [root]);
    const { j, seal } = burn({ token: usdt.address, amount: 100_000000n, nullifier: ethers.id("n-usdt"), root });
    await vault.send(relayer, "release", [seal, j]);
    assert.equal(await usdt.call("balanceOf", [recipient.hex]), 100_000000n);
  });

  it("raising the finality depth does not invalidate a root that is already final", async () => {
    await vault.send(admin, "setFinalityDepth", [4096n]);
    const { j, seal } = burn({ token: usdc.address, amount: 10_000000n, nullifier: ethers.id("n-depth"), root: ethers.id("lez-state-root-A") });
    await vault.send(relayer, "release", [seal, j]);
  });
});

describe("gas (Performance #3)", () => {
  it("records a gas table for the cost report", () => {
    const table = Object.fromEntries(Object.entries(gas).map(([k, v]) => [k, Number(v)]));
    mkdirSync(new URL("../artifacts/", import.meta.url), { recursive: true });
    writeFileSync(new URL("../artifacts/gas.json", import.meta.url), JSON.stringify(table, null, 2));
    console.log("\nmeasured gas:", table);
    assert.ok(table.deposit_eth > 0 && table.release_erc20 > 0);
  });
});
