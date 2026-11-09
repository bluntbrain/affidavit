// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.28;

/// @notice The audited RISC Zero on-chain verifier (risc0-ethereum). A seal is a Groth16
/// proof wrapping a zkVM receipt; `journalDigest` is `sha256(journal)`.
interface IRiscZeroVerifier {
    function verify(bytes calldata seal, bytes32 imageId, bytes32 journalDigest) external view;
}

/// @notice The anchor of LEZ settled state on Ethereum: which LEZ state roots are final at
/// the configured depth. How roots get here is the outbound leg's proof system (see
/// docs/OUTBOUND.md); the vault only asks the question. Once a root is final it stays final,
/// so a later change of depth never invalidates a release that was already valid (F#12).
interface ILezStateAnchor {
    function isFinal(bytes32 lezStateRoot) external view returns (bool);
}

/// @notice Minimal ERC-20 surface the vault needs. Transfers are made through low-level
/// calls that tolerate tokens whose `transfer`/`transferFrom` return nothing (USDT, F#17).
interface IERC20Minimal {
    function balanceOf(address) external view returns (uint256);
}
