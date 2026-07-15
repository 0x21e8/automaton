import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import {
  assert,
  buildRpcClient,
  claimPlaygroundFaucet,
  createDefaultSpawnSessionRequest,
  createEphemeralWallet,
  createSpawnSession,
  fetchJson,
  resolvePlaygroundRuntime,
  runCast,
  runExistingEscrowSmoke,
  submitSpawnPayment,
  waitForSessionCompletion,
  writeJsonOutputAndPrint
} from "./lib/playground-e2e.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const runtime = await resolvePlaygroundRuntime(rootDir, process.env);

const directEscrowSmokeAvailable =
  runtime.deployment.releaser?.toLowerCase() ===
  runtime.deployment.deployer?.toLowerCase();
const escrowSmoke = directEscrowSmokeAvailable
  ? runExistingEscrowSmoke(rootDir, process.env)
  : {
      skipped: true,
      reason: "escrow release authority is the factory threshold-ECDSA signer"
    };
const smokeWallet = createEphemeralWallet(rootDir);
const faucetClaim = await claimPlaygroundFaucet(runtime.indexerBaseUrl, smokeWallet.address);

assert(faucetClaim?.ok === true, "faucet claim did not succeed", faucetClaim);

const createSessionResponse = await createSpawnSession(
  runtime.indexerBaseUrl,
  createDefaultSpawnSessionRequest({
    stewardAddress: smokeWallet.address,
    grossAmount: runtime.spawnGrossAmount,
    parentId: null
  })
);

const payment = createSessionResponse?.quote?.payment;
assert(
  payment?.claimId,
  "spawn session response did not include payment instructions",
  createSessionResponse
);

const usdcAddress =
  runtime.deployment.usdcTokenAddress ?? runtime.deployment.mockUsdcAddress;
assert(
  typeof usdcAddress === "string" && usdcAddress.length > 0,
  "deployment is missing USDC address",
  runtime.deployment
);

const submittedPayment = await submitSpawnPayment({
  rootDir,
  rpcUrl: runtime.activeRpcUrl,
  expectedChainId: runtime.metadata.chain.id,
  usdcAddress,
  payment,
  wallet: smokeWallet,
  sessionDetail: {
    session: createSessionResponse.session,
    payment,
    audit: [],
    registryRecord: null
  },
  pollTimeoutMs: runtime.pollTimeoutMs,
  pollIntervalMs: runtime.pollIntervalMs
});

const completedSession = await waitForSessionCompletion(
  runtime.indexerBaseUrl,
  createSessionResponse.session.sessionId,
  {
    pollTimeoutMs: runtime.pollTimeoutMs,
    pollIntervalMs: runtime.pollIntervalMs
  }
);
const registryRecord = await fetchJson(
  `${runtime.indexerBaseUrl}/api/spawned-automatons/${completedSession.registryRecord.canisterId}`,
);
assert(registryRecord?.name === "Meridian", "registry did not surface genesis name", registryRecord);
assert(/^[0-9a-f]{64}$/.test(registryRecord?.constitutionHash ?? ""), "registry did not surface constitution hash", registryRecord);

const releaseTxHash = completedSession.session.releaseTxHash;
assert(typeof releaseTxHash === "string", "completed spawn is missing release transaction", completedSession);
const settlementRpcUrl = process.env.LOCAL_EVM_RPC_URL?.trim() || runtime.activeRpcUrl;
const rpc = buildRpcClient(settlementRpcUrl);
const releasedTopic = runCast(rootDir, ["keccak", "Released(bytes32,address,uint256)"]).toLowerCase();
const releaseReceipt = await rpc("eth_getTransactionReceipt", [releaseTxHash]);
assert(releaseReceipt !== null, "completed spawn release is missing its transaction receipt", { releaseTxHash });
assert(releaseReceipt.status !== "0x0", "completed spawn release receipt reverted", { releaseTxHash, releaseReceipt });
const releaseLogs = (releaseReceipt.logs ?? []).filter((log) => log.topics?.[0] === releasedTopic);
const releaseLog = releaseLogs.at(-1);
assert(releaseLog !== undefined, "spawn settlement is missing the authoritative Released event", { releaseTxHash, releaseLogs });
const releasedRecipient = `0x${releaseLog.topics[2].slice(-40)}`.toLowerCase();
const releasedAmount = BigInt(releaseLog.data).toString();
const registeredRecipient = registryRecord.evmAddress.toLowerCase();
assert(releasedRecipient === registeredRecipient, "spawn release recipient differs from registry EVM address", { releasedRecipient, registeredRecipient });
assert(releasedAmount === runtime.spawnGrossAmount, "spawn release amount differs from deposited gross amount", { releasedAmount, grossAmount: runtime.spawnGrossAmount });
assert(releaseLog.transactionHash?.toLowerCase() === releaseTxHash.toLowerCase(), "spawn Released event transaction differs from the recorded release transaction", { eventTransactionHash: releaseLog.transactionHash, releaseTxHash });
const recipientBalance = runCast(rootDir, ["call", usdcAddress, "balanceOf(address)(uint256)", releasedRecipient, "--rpc-url", settlementRpcUrl]).split(/\s+/)[0];
const escrowBalance = runCast(rootDir, ["call", usdcAddress, "balanceOf(address)(uint256)", runtime.deployment.escrowContractAddress, "--rpc-url", settlementRpcUrl]).split(/\s+/)[0];
const remainingClaimBalance = runCast(rootDir, ["call", runtime.deployment.escrowContractAddress, "claimBalances(bytes32)(uint256)", payment.claimId, "--rpc-url", settlementRpcUrl]).split(/\s+/)[0];
assert(BigInt(recipientBalance) >= BigInt(releasedAmount), "released child endowment is absent from the recipient balance", { recipientBalance, releasedAmount });
assert(remainingClaimBalance === "0", "released spawn claim retained escrow principal", { remainingClaimBalance });
const childAddressOutput = execFileSync("icp", ["canister", "call", completedSession.registryRecord.canisterId, "get_automaton_evm_address", "()", "-e", process.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local"], { cwd: rootDir, encoding: "utf8", env: process.env });
const childAddress = childAddressOutput.match(/0x[a-fA-F0-9]{40}/)?.[0]?.toLowerCase();
assert(childAddress === releasedRecipient, "child-reported EVM address differs from release recipient", { childAddress, releasedRecipient });

const summary = {
  ok: true,
  indexer: {
    baseUrl: runtime.indexerBaseUrl,
    health: {
      factoryCanisterId: runtime.health.discovery.factoryCanisterId,
      factoryConfigured: runtime.health.discovery.factoryConfigured
    }
  },
  rpcGateway: {
    url: runtime.activeRpcUrl,
    chainId: runtime.gatewayChainId,
    chainIdHex: runtime.gatewayChainIdHex,
    blockNumberHex: runtime.gatewayBlockNumberHex
  },
  playground: {
    environmentLabel: runtime.metadata.environmentLabel,
    environmentVersion: runtime.metadata.environmentVersion,
    maintenance: runtime.metadata.maintenance,
    chainId: runtime.metadata.chain.id
  },
  escrowSmoke,
  faucetClaim,
  spawnSmoke: {
    walletAddress: smokeWallet.address,
    sessionId: createSessionResponse.session.sessionId,
    claimId: payment.claimId,
    approvalTxHash: submittedPayment.approvalTxHash,
    depositTxHash: submittedPayment.depositTxHash,
    finalState: completedSession.session.state,
    paymentStatus: completedSession.session.paymentStatus,
    automatonCanisterId: completedSession.session.automatonCanisterId,
    releaseTxHash: completedSession.session.releaseTxHash,
    settlementEvidence: {
      releasedRecipient,
      registeredRecipient,
      childAddress,
      releasedAmount,
      recipientBalance,
      escrowBalance,
      remainingClaimBalance,
      eventTransactionHash: releaseLog.transactionHash,
      releaseTxHashMatched: releaseLog.transactionHash?.toLowerCase() === releaseTxHash.toLowerCase(),
      settlementRpcUrl
    },
    registryRecord
  }
};

writeJsonOutputAndPrint(runtime.smokeOutputPath, summary);
