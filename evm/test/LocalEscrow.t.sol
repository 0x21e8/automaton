// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {LocalEscrow} from "../src/LocalEscrow.sol";
import {MockUSDC} from "../src/MockUSDC.sol";

interface Vm {
    function prank(address msgSender) external;

    function expectRevert(bytes calldata revertData) external;
}

contract LocalEscrowTest {
    Vm private constant vm =
        Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    MockUSDC private usdc;
    LocalEscrow private escrow;

    address private constant RELEASER = address(0xA11CE);
    address private constant PAYER = address(0xB0B);
    address private constant RECIPIENT = address(0xCAFE);
    address private constant OTHER_PAYER = address(0xBEEF);
    address private constant ROYALTY_ONE = address(0xD00D);
    address private constant ROYALTY_TWO = address(0xF00D);
    bytes32 private constant CLAIM_ID =
        0x8843a0dc33a27f3b64237d78d8d8d72df4f56ed9f643cef7f43f77832f0f4d0d;

    function setUp() public {
        usdc = new MockUSDC();
        escrow = new LocalEscrow(address(usdc), RELEASER);

        usdc.mint(PAYER, 150_000_000);

        vm.prank(PAYER);
        usdc.approve(address(escrow), type(uint256).max);
    }

    function testDepositAccumulatesAndReleaseTransfersFunds() public {
        vm.prank(PAYER);
        escrow.deposit(CLAIM_ID, 75_000_000);

        vm.prank(PAYER);
        escrow.deposit(CLAIM_ID, 5_000_000);

        assert(escrow.claimBalances(CLAIM_ID) == 80_000_000);
        assert(usdc.balanceOf(address(escrow)) == 80_000_000);
        assert(usdc.balanceOf(RECIPIENT) == 0);

        vm.prank(RELEASER);
        escrow.release(CLAIM_ID, RECIPIENT);

        assert(escrow.claimBalances(CLAIM_ID) == 0);
        assert(escrow.releasedClaims(CLAIM_ID));
        assert(usdc.balanceOf(address(escrow)) == 0);
        assert(usdc.balanceOf(RECIPIENT) == 80_000_000);
    }

    function testReleaseRequiresConfiguredReleaser() public {
        vm.prank(PAYER);
        escrow.deposit(CLAIM_ID, 25_000_000);

        vm.prank(PAYER);
        vm.expectRevert(abi.encodeWithSelector(LocalEscrow.Unauthorized.selector));
        escrow.release(CLAIM_ID, RECIPIENT);
    }

    function testReleaseRejectsMissingDeposit() public {
        vm.prank(RELEASER);
        vm.expectRevert(abi.encodeWithSelector(LocalEscrow.NothingDeposited.selector));
        escrow.release(CLAIM_ID, RECIPIENT);
    }

    function testRefundReturnsTheWholeClaimToItsOriginalPayerExactlyOnce() public {
        vm.prank(PAYER);
        escrow.depositReproduction(CLAIM_ID, 75_000_000);
        uint256 payerAfterDebit = usdc.balanceOf(PAYER);

        vm.prank(RELEASER);
        escrow.refund(CLAIM_ID);

        assert(usdc.balanceOf(PAYER) == payerAfterDebit + 75_000_000);
        assert(usdc.balanceOf(address(escrow)) == 0);
        assert(escrow.claimBalances(CLAIM_ID) == 0);
        assert(escrow.releasedClaims(CLAIM_ID));

        vm.prank(RELEASER);
        vm.expectRevert(abi.encodeWithSelector(LocalEscrow.AlreadyReleased.selector));
        escrow.refund(CLAIM_ID);
        assert(usdc.balanceOf(PAYER) == 150_000_000);
    }

    function testRefundRequiresTheConfiguredReleaser() public {
        vm.prank(PAYER);
        escrow.deposit(CLAIM_ID, 25_000_000);

        vm.prank(PAYER);
        vm.expectRevert(abi.encodeWithSelector(LocalEscrow.Unauthorized.selector));
        escrow.refund(CLAIM_ID);
    }

    function testClaimCannotMixPayersAndRedirectItsRefund() public {
        usdc.mint(OTHER_PAYER, 25_000_000);
        vm.prank(OTHER_PAYER);
        usdc.approve(address(escrow), type(uint256).max);
        vm.prank(PAYER);
        escrow.deposit(CLAIM_ID, 25_000_000);

        vm.prank(OTHER_PAYER);
        vm.expectRevert(abi.encodeWithSelector(LocalEscrow.PayerMismatch.selector));
        escrow.deposit(CLAIM_ID, 25_000_000);

        vm.prank(RELEASER);
        escrow.refund(CLAIM_ID);
        assert(usdc.balanceOf(PAYER) == 150_000_000);
        assert(usdc.balanceOf(OTHER_PAYER) == 25_000_000);
    }

    function testReproductionDepositCannotDebitPrincipalTwiceAfterRetry() public {
        vm.prank(PAYER);
        escrow.depositReproduction(CLAIM_ID, 75_000_000);

        vm.prank(PAYER);
        vm.expectRevert(abi.encodeWithSelector(LocalEscrow.AlreadyDeposited.selector));
        escrow.depositReproduction(CLAIM_ID, 75_000_000);

        assert(usdc.balanceOf(PAYER) == 75_000_000);
        assert(usdc.balanceOf(address(escrow)) == 75_000_000);
        assert(escrow.claimBalances(CLAIM_ID) == 75_000_000);
    }

    function testReproductionReleasePaysFeeRoyaltiesExactlyOnce() public {
        vm.prank(PAYER);
        escrow.deposit(CLAIM_ID, 75_000_000);

        vm.prank(RELEASER);
        escrow.releaseReproduction(
            CLAIM_ID,
            RECIPIENT,
            25_000_000,
            ROYALTY_ONE,
            500_000,
            ROYALTY_TWO,
            250_000
        );

        assert(usdc.balanceOf(RECIPIENT) == 25_000_000);
        assert(usdc.balanceOf(ROYALTY_ONE) == 500_000);
        assert(usdc.balanceOf(ROYALTY_TWO) == 250_000);
        assert(usdc.balanceOf(RELEASER) == 49_250_000);
        assert(
            usdc.balanceOf(RECIPIENT) + usdc.balanceOf(ROYALTY_ONE)
                + usdc.balanceOf(ROYALTY_TWO) + usdc.balanceOf(RELEASER) == 75_000_000
        );
        assert(escrow.claimBalances(CLAIM_ID) == 0);
        assert(usdc.balanceOf(address(escrow)) == 0);

        vm.prank(RELEASER);
        vm.expectRevert(abi.encodeWithSelector(LocalEscrow.AlreadyReleased.selector));
        escrow.releaseReproduction(
            CLAIM_ID,
            RECIPIENT,
            25_000_000,
            ROYALTY_ONE,
            500_000,
            ROYALTY_TWO,
            250_000
        );
        assert(usdc.balanceOf(ROYALTY_ONE) == 500_000);
        assert(usdc.balanceOf(ROYALTY_TWO) == 250_000);
        assert(usdc.balanceOf(RECIPIENT) == 25_000_000);
        assert(usdc.balanceOf(RELEASER) == 49_250_000);
    }
}
