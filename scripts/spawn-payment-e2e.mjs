import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  assert,
  claimPlaygroundFaucet,
  createDefaultSpawnSessionRequest,
  createEphemeralWallet,
  createSpawnSession,
  ensurePlaygroundSmokePrecondition,
  fetchJson,
  getSpawnSessionDetail,
  resolvePlaygroundRuntime,
  submitSpawnPayment,
  waitForSessionCompletion,
  waitForSpawnSession,
  writeJsonOutputAndPrint
} from "./lib/playground-e2e.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const smokePrecondition = await ensurePlaygroundSmokePrecondition(rootDir, process.env);
const runtime = await resolvePlaygroundRuntime(rootDir, process.env);

const wallet = createEphemeralWallet(rootDir);
const faucetClaim = await claimPlaygroundFaucet(runtime.indexerBaseUrl, wallet.address);

assert(faucetClaim?.ok === true, "faucet claim did not succeed", faucetClaim);

const createSessionResponse = await createSpawnSession(
  runtime.indexerBaseUrl,
  createDefaultSpawnSessionRequest({
    stewardAddress: wallet.address,
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

const sessionId = createSessionResponse.session.sessionId;
const observedStates = new Set([createSessionResponse.session.state]);
const observedPaymentStatuses = new Set([createSessionResponse.session.paymentStatus]);

function trackDetail(detail) {
  if (detail?.session?.state) {
    observedStates.add(detail.session.state);
  }

  if (detail?.session?.paymentStatus) {
    observedPaymentStatuses.add(detail.session.paymentStatus);
  }
}

const initialDetail = await getSpawnSessionDetail(runtime.indexerBaseUrl, sessionId);
trackDetail(initialDetail);

assert(
  initialDetail.session.state === "awaiting_payment",
  "spawn session did not start in awaiting_payment",
  initialDetail
);
assert(
  initialDetail.session.paymentStatus === "unpaid",
  "spawn session did not start in unpaid status",
  initialDetail
);

const submittedPayment = await submitSpawnPayment({
  rootDir,
  rpcUrl: runtime.activeRpcUrl,
  expectedChainId: runtime.metadata.chain.id,
  usdcAddress,
  payment,
  wallet,
  sessionDetail: initialDetail,
  pollTimeoutMs: runtime.pollTimeoutMs,
  pollIntervalMs: runtime.pollIntervalMs
});

const mirroredDetail = await waitForSpawnSession(runtime.indexerBaseUrl, sessionId, {
  pollTimeoutMs: runtime.pollTimeoutMs,
  pollIntervalMs: runtime.pollIntervalMs,
  description: "to mirror a confirmed payment",
  onDetail: trackDetail,
  accept: (detail) => {
    return detail?.session?.paymentStatus === "paid";
  }
});

const advancedDetail =
  mirroredDetail.session.state !== "awaiting_payment"
    ? mirroredDetail
    : await waitForSpawnSession(runtime.indexerBaseUrl, sessionId, {
        pollTimeoutMs: runtime.pollTimeoutMs,
        pollIntervalMs: runtime.pollIntervalMs,
        description: "to leave awaiting_payment after payment mirroring",
        onDetail: trackDetail,
        accept: (detail) => detail?.session?.state !== "awaiting_payment"
      });

assert(
  ["payment_detected", "spawning", "broadcasting_release", "complete"].includes(
    advancedDetail.session.state
  ),
  "spawn session did not advance into the post-payment state machine",
  advancedDetail
);

let rejectedSecondPayment = null;
try {
  await submitSpawnPayment({
    rootDir,
    rpcUrl: runtime.activeRpcUrl,
    expectedChainId: runtime.metadata.chain.id,
    usdcAddress,
    payment,
    wallet,
    sessionDetail: advancedDetail,
    pollTimeoutMs: runtime.pollTimeoutMs,
    pollIntervalMs: runtime.pollIntervalMs
  });
} catch (error) {
  rejectedSecondPayment = error instanceof Error ? error : new Error(String(error));
}

assert(
  rejectedSecondPayment !== null,
  "second payment attempt was not rejected after the session advanced"
);

const completedSession = await waitForSessionCompletion(runtime.indexerBaseUrl, sessionId, {
  pollTimeoutMs: runtime.pollTimeoutMs,
  pollIntervalMs: runtime.pollIntervalMs,
  onDetail: trackDetail
});

assert(
  completedSession.session.state === "complete",
  "spawn session did not complete successfully",
  completedSession
);
assert(
  completedSession.session.paymentStatus === "paid",
  "completed spawn session did not remain paid",
  completedSession
);
assert(
  typeof completedSession.session.releaseTxHash === "string" &&
    completedSession.session.releaseTxHash.length > 0,
  "completed spawn session is missing release tx hash",
  completedSession
);
assert(
  completedSession.registryRecord?.canisterId,
  "completed spawn session is missing registry record",
  completedSession
);

const registryRecord = await fetchJson(
  `${runtime.indexerBaseUrl}/api/spawned-automatons/${completedSession.registryRecord.canisterId}`
);

assert(
  registryRecord?.sessionId === sessionId,
  "registry record session linkage did not match the completed session",
  registryRecord
);

const summary = {
  ok: true,
  precondition: {
    smokeMode: smokePrecondition.mode,
    smokeOutputPath: smokePrecondition.outputPath,
    smokeSessionId: smokePrecondition.summary?.spawnSmoke?.sessionId ?? null
  },
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
  faucetClaim,
  spawnPaymentE2e: {
    walletAddress: wallet.address,
    sessionId,
    claimId: payment.claimId,
    approvalTxHash: submittedPayment.approvalTxHash,
    depositTxHash: submittedPayment.depositTxHash,
    mirroredState: mirroredDetail.session.state,
    mirroredPaymentStatus: mirroredDetail.session.paymentStatus,
    advancedState: advancedDetail.session.state,
    finalState: completedSession.session.state,
    finalPaymentStatus: completedSession.session.paymentStatus,
    releaseTxHash: completedSession.session.releaseTxHash,
    automatonCanisterId: completedSession.session.automatonCanisterId,
    secondPaymentRejection: rejectedSecondPayment.message,
    observedStates: Array.from(observedStates),
    observedPaymentStatuses: Array.from(observedPaymentStatuses),
    registryRecord
  }
};

writeJsonOutputAndPrint(runtime.spawnPaymentE2eOutputPath, summary);
