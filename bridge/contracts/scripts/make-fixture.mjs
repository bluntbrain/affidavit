// Writes bridge/testdata/vault/deposit-receipts.json: the real receipts of real deposits into
// the compiled CausewayVault on the test EVM, in `eth_getBlockReceipts` shape, for the Rust
// seam test (`bridge/tests/seam.rs`) to attest with Affidavit's real receipt-proof path.
//
//   npm run fixture
//
// Deterministic: same accounts, same order, same bytecode → same file.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { ethers } from "ethers";
import { compile } from "../test/compile.mjs";
import { Evm } from "../test/evm.mjs";
import { depositCommitment, depositSecret } from "../test/recovery.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "..", "..", "testdata", "vault", "deposit-receipts.json");
const ETH = ethers.ZeroAddress;
const ALICE_SEED = "0x" + "a1".repeat(32);
const BOB_SEED = "0x" + "b0".repeat(32);

const { contracts } = compile();
const evm = await Evm.create();
const [admin, alice, bob] = await Promise.all([1, 3, 6].map((b) => evm.account(b)));
const verifier = await evm.deploy(admin, contracts.MockRiscZeroVerifier);
const anchor = await evm.deploy(admin, contracts.MockLezStateAnchor);
const vault = await evm.deploy(admin, contracts.CausewayVault, [
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
const chainId = await vault.call("chainId");
const at = { chainId, vault: vault.address, token: ETH };

// tx 0: Bob (nonce 0) · tx 1: Alice (nonce 0) · tx 2: Bob (nonce 1). Alice's is the one the
// Rust test claims; Bob's are the ones it must not be able to claim with her secret.
const plan = [
  [bob, BOB_SEED, 0],
  [alice, ALICE_SEED, 0],
  [bob, BOB_SEED, 1],
];
const receipts = [];
let cumulative = 0n;
for (const [from, seed, nonce] of plan) {
  const c = depositCommitment(depositSecret(seed, at, nonce));
  const res = await vault.send(from, "depositETH", [c], ethers.parseEther("0.1"));
  cumulative += res.gas;
  receipts.push({
    type: "0x0",
    status: res.receipt.status === 1 ? "0x1" : "0x0",
    cumulativeGasUsed: ethers.toQuantity(cumulative),
    logsBloom: ethers.hexlify(res.receipt.bitvector),
    logs: res.logs.map((l) => ({ address: l.address, topics: l.topics, data: l.data })),
  });
}
mkdirSync(dirname(out), { recursive: true });
writeFileSync(
  out,
  JSON.stringify({ jsonrpc: "2.0", id: 1, vault: vault.address, chainId: ethers.toQuantity(chainId), result: receipts }, null, 2) + "\n",
);
console.log(`wrote ${out}: ${receipts.length} receipts from vault ${vault.address}`);
