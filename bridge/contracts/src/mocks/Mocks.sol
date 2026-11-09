// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.28;

import {IRiscZeroVerifier, ILezStateAnchor} from "../Interfaces.sol";

/// @notice Stands in for the audited RISC Zero verifier in tests: a seal is valid iff it is
/// exactly `abi.encode(imageId, journalDigest)`. Any other seal, or the right seal against a
/// different journal, reverts, which is what the real verifier does with a bad proof.
contract MockRiscZeroVerifier is IRiscZeroVerifier {
    function verify(bytes calldata seal, bytes32 imageId, bytes32 journalDigest) external pure {
        require(keccak256(seal) == keccak256(abi.encode(imageId, journalDigest)), "invalid seal");
    }
}

/// @notice Stands in for the LEZ settled-state anchor: an operator marks roots final. Once
/// final, a root stays final (the property F#12 relies on).
contract MockLezStateAnchor is ILezStateAnchor {
    mapping(bytes32 => bool) public finalRoots;

    function markFinal(bytes32 root) external {
        finalRoots[root] = true;
    }

    function isFinal(bytes32 root) external view returns (bool) {
        return finalRoots[root];
    }
}

/// @notice A standard ERC-20 that returns `true` (USDC/DAI/WETH style).
contract MockERC20 {
    string public name;
    uint8 public decimals;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(string memory _name, uint8 _decimals) {
        name = _name;
        decimals = _decimals;
    }

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

/// @notice USDT-style: `transfer` and `transferFrom` return nothing (F#17).
contract NoReturnERC20 {
    uint8 public decimals = 6;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external {
        allowance[msg.sender][spender] = amount;
    }

    function transfer(address to, uint256 amount) external {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
    }

    function transferFrom(address from, address to, uint256 amount) external {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}
