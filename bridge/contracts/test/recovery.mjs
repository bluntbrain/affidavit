// Seed-only position recovery, client side (RFP-022 Functionality #9, Reliability #5). Mirrors
// `bridge/src/recovery.rs` byte for byte; both are pinned to the same vector in the tests.
import { ethers } from "ethers";

const SECRET_DOMAIN = ethers.toUtf8Bytes("causeway/deposit-secret/v1");
const COMMITMENT_DOMAIN = ethers.toUtf8Bytes("causeway/deposit-commitment/v1");
export const DEFAULT_GAP_LIMIT = 20;

/// s = sha256(domain ‖ seed ‖ chain_id (u64 BE) ‖ vault ‖ token ‖ nonce (u32 BE))
export function depositSecret(seed, { chainId, vault, token }, nonce) {
  return ethers.sha256(
    ethers.concat([
      SECRET_DOMAIN,
      seed,
      ethers.zeroPadValue(ethers.toBeHex(chainId), 8),
      vault,
      token,
      ethers.zeroPadValue(ethers.toBeHex(nonce), 4),
    ]),
  );
}

/// c = sha256(domain ‖ s): what the vault logs.
export function depositCommitment(secret) {
  return ethers.sha256(ethers.concat([COMMITMENT_DOMAIN, secret]));
}

/// Rediscover this seed's deposits of `at.token` among the vault's `Deposit` logs, with a
/// BIP-44-style gap limit. Whether each one is already claimed is LEZ's answer (nullifier).
export function recoverDeposits(seed, at, logs, gapLimit = DEFAULT_GAP_LIMIT) {
  const found = [];
  for (let nonce = 0, gap = 0; gap < gapLimit; nonce++) {
    const secret = depositSecret(seed, at, nonce);
    const c = depositCommitment(secret);
    const hits = logs.filter((l) => l.token.toLowerCase() === at.token.toLowerCase() && l.commitment === c);
    if (hits.length === 0) {
      gap++;
      continue;
    }
    gap = 0;
    found.push(...hits.map((log) => ({ log, nonce, secret })));
  }
  return found.sort((a, b) => Number(a.log.index - b.log.index));
}
