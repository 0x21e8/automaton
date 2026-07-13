import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  assert,
  claimPlaygroundFaucet,
  createDefaultSpawnSessionRequest,
  createEphemeralWallet,
  createSpawnSession,
  fetchJson,
  resolvePlaygroundRuntime,
  runExistingEscrowSmoke,
  submitSpawnPayment,
  waitForSessionCompletion,
  writeJsonOutputAndPrint
} from "./lib/playground-e2e.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const runtime = await resolvePlaygroundRuntime(rootDir, process.env);

const escrowSmoke = runExistingEscrowSmoke(rootDir, process.env);
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
    registryRecord
  }
};

writeJsonOutputAndPrint(runtime.smokeOutputPath, summary);
