// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.28;

import {IRiscZeroVerifier, ILezStateAnchor} from "./Interfaces.sol";

/// @title CausewayVault
/// @notice The Ethereum half of Causeway (Logos RFP-022): escrows ERC-20s and native ETH
/// against wrapped tokens minted on LEZ, and releases them on a cryptographic proof of a LEZ
/// burn. No signer, federation or relayer is trusted with funds.
///
/// Inbound (deposit): a user escrows a fixed denomination of a registered token with a
/// 32-byte commitment derived from their own wallet seed. Nothing here names, stores or
/// reveals the LEZ destination (F#2); the LEZ bridge program mints only after the Affidavit
/// primitive (RFP-021) proves this contract's `Deposit` log is included under a finalised
/// Ethereum header (F#3). Deposits are recoverable from the seed by scanning `Deposit` logs
/// and recomputing commitments (F#9).
///
/// Outbound (release): the LEZ burn is proven by a RISC Zero receipt whose journal binds
/// this exact deployment (chain id, vault, LEZ zone and program), the burned amount and
/// token, the recipient, and the LEZ state root the burn settled under. The receipt is
/// checked by the audited RISC Zero verifier, the root by the LEZ settled-state anchor, and
/// the burn's nullifier is spent once (F#5-7, Reliability #3, Security #2, #3, #6).
///
/// The verification logic is fixed at deployment: every verifier parameter is immutable and
/// there is no proxy (Security #8). Governance is limited to the token registry, caps,
/// finality depth and fees (admin) and pausing (a separate freezer), which can stop new
/// activity but never move funds (Security #5, #9).
contract CausewayVault {
    // ---------------------------------------------------------------- pairing (F#15)
    uint256 public immutable chainId;
    bytes32 public immutable lezZoneId;
    bytes32 public immutable lezProgramId;
    bytes32 public immutable burnImageId;
    IRiscZeroVerifier public immutable verifier;
    ILezStateAnchor public immutable anchor;

    // ---------------------------------------------------------------- roles
    address public admin; // registry, caps, finality depth, fee (RFP-001 analogue)
    address public freezer; // pause only, independent of admin (RFP-002 analogue)

    // ---------------------------------------------------------------- registry (F#10)
    struct TokenConfig {
        bool registered;
        uint8 decimals;
        uint256 depositCapPerWindow; // 0 = no per-token cap
        uint256 releaseCapPerWindow; // 0 = no per-token cap
    }

    /// token => config. `address(0)` is native ETH.
    mapping(address => TokenConfig) public tokens;
    /// token => amount => is a permitted fixed denomination (F#8)
    mapping(address => mapping(uint256 => bool)) public isDenomination;

    // ---------------------------------------------------------------- caps (F#11)
    uint256 public capWindow = 1 days;
    uint256 public globalDepositCapPerWindow; // 0 = none, in token-agnostic "units" below
    uint256 public globalReleaseCapPerWindow;
    /// token => window index => amount
    mapping(address => mapping(uint256 => uint256)) public depositedInWindow;
    mapping(address => mapping(uint256 => uint256)) public releasedInWindow;
    /// window index => count of deposits / releases (the global cap counts operations, so it
    /// is comparable across tokens of different decimals; per-token caps are in token units)
    mapping(uint256 => uint256) public globalDepositsInWindow;
    mapping(uint256 => uint256) public globalReleasesInWindow;

    // ---------------------------------------------------------------- freeze (F#14)
    bool public depositsFrozen;
    bool public releasesFrozen;
    mapping(address => bool) public tokenDepositsFrozen;
    mapping(address => bool) public tokenReleasesFrozen;

    // ---------------------------------------------------------------- accounting
    /// token => escrowed balance. Releases can never exceed it (Security #6).
    mapping(address => uint256) public escrowed;
    /// Spent burn nullifiers: a burn releases exactly once (Reliability #3).
    mapping(bytes32 => bool) public spentNullifier;
    uint256 public depositCount;
    uint256 public releaseCount;

    // ---------------------------------------------------------------- events
    /// The only public trace of a deposit: token, amount, and a commitment the depositor can
    /// recompute from their seed. No LEZ destination (F#2).
    event Deposit(uint256 indexed index, address indexed token, uint256 amount, bytes32 commitment);
    event Release(bytes32 indexed nullifier, address indexed token, uint256 amount, address indexed recipient);
    event TokenRegistered(address indexed token, uint8 decimals, uint256[] denominations);
    event TokenDeregistered(address indexed token);
    event Frozen(address indexed token, bool deposits, bool releases);
    event FinalityDepthSet(uint64 depth);

    // ---------------------------------------------------------------- errors (Usability #8)
    error NotAdmin();
    error NotFreezer();
    error UnsupportedToken();
    error InvalidAmount();
    error CapExceeded();
    error FrozenError();
    error VerificationFailed();
    error InsufficientFinality();
    error AlreadyReleased();
    error WrongPairing();
    error TransferFailed();

    /// The journal the LEZ burn-proof guest commits to (see docs/OUTBOUND.md). It binds the
    /// burn to this exact deployment so a proof for one pairing is never valid for another.
    struct BurnJournal {
        bytes32 lezZoneId;
        bytes32 lezProgramId;
        uint256 chainId;
        address vault;
        bytes32 nullifier;
        address token;
        uint256 amount;
        address recipient;
        bytes32 lezStateRoot;
    }

    uint64 public finalityDepth; // informational for clients; finality itself is the anchor's

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    modifier onlyFreezer() {
        if (msg.sender != freezer) revert NotFreezer();
        _;
    }

    constructor(
        bytes32 _lezZoneId,
        bytes32 _lezProgramId,
        bytes32 _burnImageId,
        IRiscZeroVerifier _verifier,
        ILezStateAnchor _anchor,
        address _admin,
        address _freezer,
        uint64 _finalityDepth
    ) {
        chainId = block.chainid;
        lezZoneId = _lezZoneId;
        lezProgramId = _lezProgramId;
        burnImageId = _burnImageId;
        verifier = _verifier;
        anchor = _anchor;
        admin = _admin;
        freezer = _freezer;
        finalityDepth = _finalityDepth;
    }

    // ================================================================ admin (RFP-001)

    function registerToken(
        address token,
        uint8 decimals,
        uint256[] calldata denominations,
        uint256 depositCap,
        uint256 releaseCap
    ) external onlyAdmin {
        tokens[token] = TokenConfig({
            registered: true,
            decimals: decimals,
            depositCapPerWindow: depositCap,
            releaseCapPerWindow: releaseCap
        });
        for (uint256 i = 0; i < denominations.length; i++) {
            if (denominations[i] == 0) revert InvalidAmount();
            isDenomination[token][denominations[i]] = true;
        }
        emit TokenRegistered(token, decimals, denominations);
    }

    /// Delisting stops new deposits only. Funds already escrowed stay releasable, so holders
    /// of the wrapped token are never stranded (F#10, delisting with minimum impact).
    function deregisterToken(address token) external onlyAdmin {
        tokens[token].registered = false;
        emit TokenDeregistered(token);
    }

    function setDenomination(address token, uint256 amount, bool allowed) external onlyAdmin {
        if (amount == 0) revert InvalidAmount();
        isDenomination[token][amount] = allowed;
    }

    function setCaps(
        address token,
        uint256 depositCap,
        uint256 releaseCap
    ) external onlyAdmin {
        tokens[token].depositCapPerWindow = depositCap;
        tokens[token].releaseCapPerWindow = releaseCap;
    }

    function setGlobalCaps(uint256 deposits, uint256 releases, uint256 window) external onlyAdmin {
        if (window == 0) revert InvalidAmount();
        globalDepositCapPerWindow = deposits;
        globalReleaseCapPerWindow = releases;
        capWindow = window;
    }

    /// Changing the depth never invalidates an already-valid release: finality is decided by
    /// the anchor, which keeps a root final once it is final (F#12).
    function setFinalityDepth(uint64 depth) external onlyAdmin {
        finalityDepth = depth;
        emit FinalityDepthSet(depth);
    }

    function transferAdmin(address to) external onlyAdmin {
        admin = to;
    }

    function transferFreezer(address to) external onlyFreezer {
        freezer = to;
    }

    // ================================================================ freeze (RFP-002)

    /// `token == address(0)` with `global == true` pauses everything; otherwise one token.
    /// Independent of the admin and of the LEZ side (Security #5).
    function setFrozen(address token, bool global, bool deposits, bool releases) external onlyFreezer {
        if (global) {
            depositsFrozen = deposits;
            releasesFrozen = releases;
        } else {
            tokenDepositsFrozen[token] = deposits;
            tokenReleasesFrozen[token] = releases;
        }
        emit Frozen(token, deposits, releases);
    }

    // ================================================================ deposit (inbound)

    /// Escrow native ETH. The value must be a registered denomination of `address(0)`.
    function depositETH(bytes32 commitment) external payable {
        _checkDeposit(address(0), msg.value);
        escrowed[address(0)] += msg.value;
        _recordDeposit(address(0), msg.value, commitment);
    }

    /// Escrow `amount` of an ERC-20, pulled with `transferFrom`. Works for tokens that return
    /// no boolean (USDT) and for fee-on-transfer tokens is intentionally unsupported: the
    /// received amount must equal the denomination exactly.
    function depositERC20(address token, uint256 amount, bytes32 commitment) external {
        if (token == address(0)) revert UnsupportedToken();
        _checkDeposit(token, amount);
        uint256 before = _balance(token);
        _transferFrom(token, msg.sender, address(this), amount);
        if (_balance(token) - before != amount) revert InvalidAmount();
        escrowed[token] += amount;
        _recordDeposit(token, amount, commitment);
    }

    function _checkDeposit(address token, uint256 amount) internal {
        TokenConfig memory cfg = tokens[token];
        if (!cfg.registered) revert UnsupportedToken();
        if (!isDenomination[token][amount]) revert InvalidAmount();
        if (depositsFrozen || tokenDepositsFrozen[token]) revert FrozenError();
        uint256 w = block.timestamp / capWindow;
        if (cfg.depositCapPerWindow != 0) {
            uint256 next = depositedInWindow[token][w] + amount;
            if (next > cfg.depositCapPerWindow) revert CapExceeded();
            depositedInWindow[token][w] = next;
        }
        if (globalDepositCapPerWindow != 0) {
            uint256 n = globalDepositsInWindow[w] + 1;
            if (n > globalDepositCapPerWindow) revert CapExceeded();
            globalDepositsInWindow[w] = n;
        }
    }

    function _recordDeposit(address token, uint256 amount, bytes32 commitment) internal {
        uint256 index = depositCount++;
        emit Deposit(index, token, amount, commitment);
    }

    // ================================================================ release (outbound)

    /// Release escrowed funds on proof of a LEZ burn. `seal` is the Groth16-wrapped RISC Zero
    /// receipt of the burn-proof guest (`burnImageId`), whose journal is `abi.encode(j)`.
    function release(bytes calldata seal, BurnJournal calldata j) external {
        // 1. This exact deployment (F#15): a proof for another chain, vault, zone or program
        //    is never valid here.
        if (j.chainId != chainId || j.vault != address(this) || j.lezZoneId != lezZoneId || j.lezProgramId != lezProgramId) {
            revert WrongPairing();
        }
        // 2. The burn settled under a LEZ state root that is final (F#12).
        if (!anchor.isFinal(j.lezStateRoot)) revert InsufficientFinality();
        // 3. Each burn releases once (Reliability #3). Checked before the proof so a replay
        //    and a bad proof are both refused without distinguishing which deposit was meant.
        if (spentNullifier[j.nullifier]) revert AlreadyReleased();
        // 4. The proof: the audited verifier checks the seal against the burn guest's image
        //    id and the sha256 digest of this journal (Security #2).
        bytes32 digest = sha256(abi.encode(j));
        try verifier.verify(seal, burnImageId, digest) {} catch {
            revert VerificationFailed();
        }
        // 5. Operational limits.
        TokenConfig memory cfg = tokens[j.token];
        if (releasesFrozen || tokenReleasesFrozen[j.token]) revert FrozenError();
        uint256 w = block.timestamp / capWindow;
        if (cfg.releaseCapPerWindow != 0) {
            uint256 next = releasedInWindow[j.token][w] + j.amount;
            if (next > cfg.releaseCapPerWindow) revert CapExceeded();
            releasedInWindow[j.token][w] = next;
        }
        if (globalReleaseCapPerWindow != 0) {
            uint256 n = globalReleasesInWindow[w] + 1;
            if (n > globalReleaseCapPerWindow) revert CapExceeded();
            globalReleasesInWindow[w] = n;
        }
        // 6. Supply soundness: never release more than is escrowed for this token.
        if (escrowed[j.token] < j.amount) revert InvalidAmount();
        spentNullifier[j.nullifier] = true;
        escrowed[j.token] -= j.amount;
        releaseCount++;
        _payOut(j.token, j.recipient, j.amount);
        emit Release(j.nullifier, j.token, j.amount, j.recipient);
    }

    // ================================================================ token plumbing

    function _balance(address token) internal view returns (uint256) {
        (bool ok, bytes memory data) = token.staticcall(abi.encodeWithSignature("balanceOf(address)", address(this)));
        if (!ok || data.length < 32) revert TransferFailed();
        return abi.decode(data, (uint256));
    }

    /// `transferFrom` that accepts both a `true` return and no return value at all (USDT).
    function _transferFrom(address token, address from, address to, uint256 amount) internal {
        (bool ok, bytes memory data) =
            token.call(abi.encodeWithSignature("transferFrom(address,address,uint256)", from, to, amount));
        if (!ok || (data.length != 0 && !abi.decode(data, (bool)))) revert TransferFailed();
    }

    function _payOut(address token, address to, uint256 amount) internal {
        if (token == address(0)) {
            (bool ok,) = to.call{value: amount}("");
            if (!ok) revert TransferFailed();
        } else {
            (bool ok, bytes memory data) = token.call(abi.encodeWithSignature("transfer(address,uint256)", to, amount));
            if (!ok || (data.length != 0 && !abi.decode(data, (bool)))) revert TransferFailed();
        }
    }
}
