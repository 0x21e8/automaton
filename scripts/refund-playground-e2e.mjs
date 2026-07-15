import path from "node:path";
import { randomBytes } from "node:crypto";
import { fileURLToPath } from "node:url";
import { readFile } from "node:fs/promises";

import {
  assert,
  runCast,
  writeJsonOutput,
  createEphemeralWallet
} from "./lib/playground-e2e.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tmpDir = process.env.PLAYGROUND_TMP_DIR ?? path.join(rootDir, "tmp");
const outputPath =
  process.env.PLAYGROUND_REFUND_E2E_OUTPUT_FILE ??
  path.join(tmpDir, "refund-playground-e2e.json");
const deploymentPath =
  process.env.LOCAL_EVM_DEPLOYMENT_FILE ??
  path.join(rootDir, "tmp", "local-escrow-deployment.json");
const depositAmount = process.env.PLAN013_REFUND_DEPOSIT_AMOUNT ?? "75000000";
const pollTimeoutMs = Number.parseInt(process.env.PLAN013_POLL_TIMEOUT_MS ?? "120000", 10);
const releaserPrivateKey =
  process.env.PLAN013_REFUND_RELEASER_PRIVATE_KEY?.trim() ?? null;
const payerPrivateKey = process.env.PLAN013_REFUND_PAYER_PRIVATE_KEY?.trim() ?? null;

const deploymentText = await readFile(deploymentPath, "utf8");
const deployment = JSON.parse(deploymentText);
const rpcUrl = deployment.rpcUrl;
const usdcAddress = deployment.usdcTokenAddress ?? deployment.mockUsdcAddress;
const escrowAddress = deployment.escrowContractAddress;
const releaser = deployment.releaser;
const deployer = deployment.deployer;
const wallet = payerPrivateKey
  ? {
      privateKey: payerPrivateKey,
      address: runCast(rootDir, ["wallet", "address", "--private-key", payerPrivateKey]).toLowerCase()
    }
  : createEphemeralWallet(rootDir);

if (!usdcAddress) {
  throw new Error("deployment is missing USDC token address");
}
if (!escrowAddress) {
  throw new Error("deployment is missing escrow contract address");
}
if (!releaser) {
  throw new Error("deployment is missing releaser");
}
if (!deployer) {
  throw new Error("deployment is missing deployer");
}
const ensureReleaserSigner = async () => {
  if (releaserPrivateKey) {
    return;
  }

  try {
    await rpc("anvil_impersonateAccount", [releaser]);
    await rpc("anvil_setBalance", [releaser, `0x${(10n * 10n ** 18n).toString(16)}`]);
  } catch {
    throw new Error(
      "environment is missing PLAN013_REFUND_RELEASER_PRIVATE_KEY and releaser impersonation failed"
    );
  }
};

async function rpc(method, params = []) {
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: Date.now(),
      method,
      params
    })
  });
  const body = await response.json();

  if (body.error) {
    throw new Error(`${method} failed: ${body.error.message}`);
  }

  return body.result;
}

async function sendRawTx(transaction) {
  const txHash = await rpc("eth_sendTransaction", [transaction]);
  return waitForReceipt(txHash);
}

async function callHex(method, params = []) {
  return rpc(method, params);
}

async function waitForReceipt(txHash) {
  const timeoutAt = Date.now() + pollTimeoutMs;
  while (Date.now() < timeoutAt) {
    const receipt = await rpc("eth_getTransactionReceipt", [txHash]);
    if (receipt !== null) {
      return receipt;
    }

    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  throw new Error(`timed out waiting for receipt ${txHash}`);
}

async function callUint(to, data) {
  const result = await rpc("eth_call", [{ to, data }, "latest"]);
  return BigInt(result);
}

async function getBalanceWei(address) {
  const result = await callHex("eth_getBalance", [address, "latest"]);
  return BigInt(result);
}

function calldata(signature, args) {
  return runCast(rootDir, ["calldata", signature, ...args]);
}

function assertReceiptSuccess(label, receipt) {
  if (receipt.status !== "0x1") {
    throw new Error(`${label} receipt failed: ${JSON.stringify(receipt)}`);
  }
}

function isClaimReleased(claimId) {
  return runCast(
    rootDir,
    ["call", "--rpc-url", rpcUrl, escrowAddress, "releasedClaims(bytes32)(bool)", claimId]
  ).trim().toLowerCase() === "true";
}

async function sendSignedTx({
  privateKey,
  from = null,
  to,
  data,
  value
}) {
  if (!privateKey) {
    await ensureReleaserSigner();
    const txHash = await callHex("eth_sendTransaction", [
      {
        from: from ?? releaser,
        to,
        data: data || "0x",
        ...(value ? { value } : {})
      }
    ]);
    assert(/^0x[0-9a-fA-F]{64}$/.test(txHash), `invalid tx hash for ${to}: ${txHash}`);
    return txHash;
  }

  const txHash = runCast(
    rootDir,
    [
      "send",
      "--async",
      "--rpc-url",
      rpcUrl,
      "--private-key",
      privateKey,
      to,
      data,
      ...(value ? [value] : [])
    ].filter(Boolean)
  );

  assert(/^0x[0-9a-fA-F]{64}$/.test(txHash), `invalid tx hash for ${to}: ${txHash}`);
  return txHash;
}

async function topUpPayerForGas({
  payerAddress,
  minimumWei = 1_000_000_000_000_000n,
  topUpWei = 5_000_000_000_000_000n
}) {
  const existingBalance = await getBalanceWei(payerAddress);

  if (existingBalance < minimumWei) {
    try {
      const txHash = await sendSignedTx({
        privateKey: releaserPrivateKey,
        to: payerAddress,
        data: "0x",
        value: `0x${topUpWei.toString(16)}`
      });
      const receipt = await waitForReceipt(txHash);
      assertReceiptSuccess("payer top-up", receipt);
      return;
    } catch (error) {
      process.stderr.write(`refund-e2e: payer gas top-up via transfer failed: ${error.message}\n`);
    }

    await rpc("anvil_setBalance", [
      payerAddress,
      `0x${(minimumWei + topUpWei).toString(16)}`
    ]);
  }
}

const payer = wallet.address.toLowerCase();
const sessionId = `refund-${Date.now()}-${randomBytes(6).toString("hex")}`;
let claimId = `0x${randomBytes(32).toString("hex")}`;

for (let attempt = 0; attempt < 8; attempt += 1) {
  const claimReleased = isClaimReleased(claimId);

  if (!claimReleased) {
    break;
  }

  claimId = `0x${randomBytes(32).toString("hex")}`;
}

const initialClaimReleased = isClaimReleased(claimId);
assert(!initialClaimReleased, "generated claim id is already marked released; please rerun");
const payerInitialBalanceBeforeFunding = await callUint(usdcAddress, calldata("balanceOf(address)", [payer]));
let fundTxHash = null;
let fundedBalanceAfterTopUp = null;

if (payerInitialBalanceBeforeFunding < BigInt(depositAmount)) {
  const neededBalance = BigInt(depositAmount) - payerInitialBalanceBeforeFunding;
  let fundReceipt = null;

  try {
    fundTxHash = await sendSignedTx({
      privateKey: releaserPrivateKey,
      to: usdcAddress,
      data: calldata("transfer(address,uint256)", [payer, neededBalance.toString()])
    });
    fundReceipt = await waitForReceipt(fundTxHash);
    assertReceiptSuccess("payer top-up", fundReceipt);
  } catch (error) {
    process.stderr.write(`refund-e2e: payer top-up transfer failed: ${error.message}\n`);

    try {
      fundTxHash = await (async () => {
        const txReceipt = await sendRawTx({
          from: releaser,
          to: usdcAddress,
          data: calldata("mint(address,uint256)", [payer, neededBalance.toString()])
        });
        return txReceipt.transactionHash;
      })();
      fundReceipt = await waitForReceipt(fundTxHash);
      assertReceiptSuccess("payer top-up", fundReceipt);
    } catch (releaserMintError) {
      process.stderr.write(`refund-e2e: payer top-up releaser mint failed: ${releaserMintError.message}\n`);

      fundTxHash = await (async () => {
        const txReceipt = await sendRawTx({
          from: deployer,
          to: usdcAddress,
          data: calldata("mint(address,uint256)", [payer, neededBalance.toString()])
        });
        return txReceipt.transactionHash;
      })();
      fundReceipt = await waitForReceipt(fundTxHash);
      assertReceiptSuccess("payer top-up", fundReceipt);
    }
  }

  fundedBalanceAfterTopUp = await callUint(usdcAddress, calldata("balanceOf(address)", [payer]));
}

await topUpPayerForGas({ payerAddress: payer });

const payerBalanceBeforeDeposit = await callUint(usdcAddress, calldata("balanceOf(address)", [payer]));
process.stderr.write(`refund-e2e: payer=${payer} claimId=${claimId} preDeposit=${payerBalanceBeforeDeposit.toString()}\\n`);

const approveTxHash = await sendSignedTx({
  privateKey: wallet.privateKey,
  to: usdcAddress,
  data: calldata("approve(address,uint256)", [escrowAddress, depositAmount])
});
process.stderr.write(`refund-e2e: approveTxHash=${approveTxHash}\n`);
const approveReceipt = await waitForReceipt(approveTxHash);
assertReceiptSuccess("approval", approveReceipt);

const depositTxHash = await sendSignedTx({
  privateKey: wallet.privateKey,
  to: escrowAddress,
  data: calldata("deposit(bytes32,uint256)", [claimId, depositAmount])
});
process.stderr.write(`refund-e2e: depositTxHash=${depositTxHash}\n`);
const depositReceipt = await waitForReceipt(depositTxHash);
assertReceiptSuccess("deposit", depositReceipt);

const depositorBalanceAfterDeposit = await callUint(usdcAddress, calldata("balanceOf(address)", [payer]));
const claimBalanceAfterDeposit = await callUint(escrowAddress, calldata("claimBalances(bytes32)", [claimId]));

const depositedTopic = runCast(rootDir, ["keccak", "Deposited(bytes32,address,uint256)"]);
const depositedLogs = (depositReceipt.logs ?? []).filter((log) => log.topics?.[0] === depositedTopic);
assert(Array.isArray(depositedLogs) && depositedLogs.length > 0, "deposit event was not emitted");

const refundTxHash = await sendSignedTx({
  privateKey: releaserPrivateKey,
  to: escrowAddress,
  data: calldata("refund(bytes32)", [claimId])
});
process.stderr.write(`refund-e2e: refundTxHash=${refundTxHash}\n`);
const refundReceipt = await waitForReceipt(refundTxHash);
assertReceiptSuccess("refund", refundReceipt);

const refundedTopic = runCast(rootDir, ["keccak", "Refunded(bytes32,address,uint256)"]);
const refundedLogs = (refundReceipt.logs ?? []).filter((entry) => {
  const topic0 = entry?.topics?.[0]?.toLowerCase();
  const topic1 = entry?.topics?.[1];
  return topic0 === refundedTopic.toLowerCase() && topic1 === claimId;
});
assert(refundedLogs.length > 0, "refund event was not emitted");

const payerBalanceAfterRefund = await callUint(usdcAddress, calldata("balanceOf(address)", [payer]));
const claimBalanceAfterRefund = await callUint(escrowAddress, calldata("claimBalances(bytes32)", [claimId]));
process.stderr.write(`refund-e2e: preDeposit=${payerBalanceBeforeDeposit.toString()} postRefund=${payerBalanceAfterRefund.toString()} claimBalance=${claimBalanceAfterRefund.toString()}\\n`);

assert(
  payerBalanceAfterRefund === payerBalanceBeforeDeposit,
  "payer balance was not restored after refund cycle"
);
assert(claimBalanceAfterDeposit > 0n, "claim should have been funded before refund");
assert(claimBalanceAfterRefund === 0n, "claim balance should be zero after refund");

let secondRefundTxHash = null;
let secondRefundReceipt = null;
const secondRefundNoop = isClaimReleased(claimId);
if (!secondRefundNoop) {
  secondRefundTxHash = await sendSignedTx({
    privateKey: releaserPrivateKey,
    to: escrowAddress,
    data: calldata("refund(bytes32)", [claimId])
  });
  secondRefundReceipt = await waitForReceipt(secondRefundTxHash);
}

assert(
  secondRefundNoop ||
    secondRefundReceipt?.status === "0x0",
  "second refund should be rejected as a no-op"
);

const payerBalanceAfterSecondAttempt = await callUint(usdcAddress, calldata("balanceOf(address)", [payer]));
assert(
  payerBalanceAfterSecondAttempt === payerBalanceAfterRefund,
  "payer balance changed after a rejected second refund"
);

const summary = {
  ok: true,
  deploymentRpcUrl: rpcUrl,
  usdcAddress,
  escrowAddress,
  releaser,
  payer,
  claimId,
  sessionId,
  depositAmount,
  payerInitialBalanceBeforeFunding: payerInitialBalanceBeforeFunding.toString(),
  payerBalanceBeforeDeposit: payerBalanceBeforeDeposit.toString(),
  fundTxHash,
  fundedBalanceAfterTopUp: fundedBalanceAfterTopUp?.toString() ?? null,
  depositorBalanceAfterDeposit: depositorBalanceAfterDeposit.toString(),
  payerBalanceAfterRefund: payerBalanceAfterRefund.toString(),
  claimBalanceAfterDeposit: claimBalanceAfterDeposit.toString(),
  claimBalanceAfterRefund: claimBalanceAfterRefund.toString(),
  approveTxHash,
  depositTxHash,
  depositReceipt,
  refundTxHash,
  refundReceipt,
  secondRefundTxHash,
  secondRefundReceipt,
  secondRefundNoop,
  refundedEventLogs: refundedLogs
};

writeJsonOutput(outputPath, summary);
process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
