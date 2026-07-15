// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

interface IERC20Like {
    function transfer(address to, uint256 amount) external returns (bool);

    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

contract LocalEscrow {
    IERC20Like public immutable usdc;
    address public immutable releaser;

    mapping(bytes32 => uint256) public claimBalances;
    mapping(bytes32 => bool) public releasedClaims;
    mapping(bytes32 => address) public claimPayers;

    event Deposited(bytes32 indexed claimId, address indexed payer, uint256 amount);
    event Released(bytes32 indexed claimId, address indexed recipient, uint256 amount);
    event Refunded(bytes32 indexed claimId, address indexed payer, uint256 amount);
    event ReproductionReleased(
        bytes32 indexed claimId,
        address indexed child,
        uint256 childAmount,
        address royaltyRecipientOne,
        uint256 royaltyAmountOne,
        address royaltyRecipientTwo,
        uint256 royaltyAmountTwo,
        address platformRecipient,
        uint256 platformAmount
    );

    error Unauthorized();
    error InvalidReleaser();
    error InvalidRecipient();
    error InvalidAmount();
    error TransferFailed();
    error NothingDeposited();
    error AlreadyReleased();
    error AlreadyDeposited();
    error PayerMismatch();

    constructor(address usdcAddress, address releaserAddress) {
        if (releaserAddress == address(0)) {
            revert InvalidReleaser();
        }

        usdc = IERC20Like(usdcAddress);
        releaser = releaserAddress;
    }

    function deposit(bytes32 claimId, uint256 amount) external {
        if (amount == 0) {
            revert InvalidAmount();
        }

        address payer = claimPayers[claimId];
        if (payer != address(0) && payer != msg.sender) revert PayerMismatch();
        if (!usdc.transferFrom(msg.sender, address(this), amount)) {
            revert TransferFailed();
        }

        claimPayers[claimId] = msg.sender;
        claimBalances[claimId] += amount;
        emit Deposited(claimId, msg.sender, amount);
    }

    /// Idempotent principal-debit rail for being-paid reproduction. A retry
    /// after a crash may spend gas, but can never transfer the endowment twice.
    function depositReproduction(bytes32 claimId, uint256 amount) external {
        if (claimBalances[claimId] != 0 || releasedClaims[claimId]) {
            revert AlreadyDeposited();
        }
        if (amount == 0) revert InvalidAmount();
        if (!usdc.transferFrom(msg.sender, address(this), amount)) revert TransferFailed();
        claimPayers[claimId] = msg.sender;
        claimBalances[claimId] = amount;
        emit Deposited(claimId, msg.sender, amount);
    }

    function release(bytes32 claimId, address recipient) external {
        if (msg.sender != releaser) {
            revert Unauthorized();
        }
        if (recipient == address(0)) {
            revert InvalidRecipient();
        }
        if (releasedClaims[claimId]) {
            revert AlreadyReleased();
        }

        uint256 amount = claimBalances[claimId];
        if (amount == 0) {
            revert NothingDeposited();
        }

        releasedClaims[claimId] = true;
        delete claimBalances[claimId];
        delete claimPayers[claimId];

        if (!usdc.transfer(recipient, amount)) {
            revert TransferFailed();
        }

        emit Released(claimId, recipient, amount);
    }

    /// One-shot refund to the payer recorded by the first deposit. The same
    /// immutable factory authority used for release controls refunds, while
    /// the recipient cannot be redirected by the factory or caller.
    function refund(bytes32 claimId) external {
        if (msg.sender != releaser) revert Unauthorized();
        if (releasedClaims[claimId]) revert AlreadyReleased();
        uint256 amount = claimBalances[claimId];
        address payer = claimPayers[claimId];
        if (amount == 0) revert NothingDeposited();
        if (payer == address(0)) revert InvalidRecipient();

        releasedClaims[claimId] = true;
        delete claimBalances[claimId];
        delete claimPayers[claimId];
        if (!usdc.transfer(payer, amount)) revert TransferFailed();
        emit Refunded(claimId, payer, amount);
    }

    /// One-shot reproduction settlement. Royalties are carved exclusively
    /// from the quoted reproduction fee; the remainder is paid atomically to
    /// the immutable releaser as the trusted platform/creation recipient.
    function releaseReproduction(
        bytes32 claimId,
        address child,
        uint256 childAmount,
        address royaltyRecipientOne,
        uint256 royaltyAmountOne,
        address royaltyRecipientTwo,
        uint256 royaltyAmountTwo
    ) external {
        if (msg.sender != releaser) revert Unauthorized();
        if (child == address(0) || royaltyRecipientOne == address(0) || royaltyRecipientTwo == address(0)) {
            revert InvalidRecipient();
        }
        if (releasedClaims[claimId]) revert AlreadyReleased();
        uint256 amount = claimBalances[claimId];
        uint256 distributed = childAmount + royaltyAmountOne + royaltyAmountTwo;
        if (amount == 0) revert NothingDeposited();
        if (distributed > amount || childAmount == 0) revert InvalidAmount();

        releasedClaims[claimId] = true;
        delete claimBalances[claimId];
        delete claimPayers[claimId];
        uint256 platformAmount = amount - distributed;
        if (!usdc.transfer(child, childAmount)) revert TransferFailed();
        if (royaltyAmountOne > 0 && !usdc.transfer(royaltyRecipientOne, royaltyAmountOne)) revert TransferFailed();
        if (royaltyAmountTwo > 0 && !usdc.transfer(royaltyRecipientTwo, royaltyAmountTwo)) revert TransferFailed();
        if (platformAmount > 0 && !usdc.transfer(releaser, platformAmount)) revert TransferFailed();
        emit ReproductionReleased(
            claimId,
            child,
            childAmount,
            royaltyRecipientOne,
            royaltyAmountOne,
            royaltyRecipientTwo,
            royaltyAmountTwo,
            releaser,
            platformAmount
        );
    }
}
