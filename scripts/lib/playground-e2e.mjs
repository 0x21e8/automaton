import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";

const DEFAULT_INDEXER_BASE_URL = "http://127.0.0.1:3001";
const DEFAULT_RPC_GATEWAY_URL = "http://127.0.0.1:3002";
const DEFAULT_SPAWN_GROSS_AMOUNT = "75000000";
const DEFAULT_POLL_TIMEOUT_MS = 120_000;
const DEFAULT_POLL_INTERVAL_MS = 2_000;

export function normalizeOptionalString(value) {
  if (value === undefined || value === null) {
    return null;
  }

  const normalized = String(value).trim();
  return normalized.length === 0 ? null : normalized;
}

export function parsePositiveInteger(value, fallback) {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : fallback;
}

export function parseOptionalBoolean(value, fallback = false) {
  const normalized = normalizeOptionalString(value);

  if (normalized === null) {
    return fallback;
  }

  if (["1", "true", "yes", "on"].includes(normalized.toLowerCase())) {
    return true;
  }

  if (["0", "false", "no", "off"].includes(normalized.toLowerCase())) {
    return false;
  }

  return fallback;
}

export function assert(condition, message, details = undefined) {
  if (condition) {
    return;
  }

  const error = new Error(message);
  if (details !== undefined) {
    error.details = details;
  }
  throw error;
}

export async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

export async function fetchJson(url, options = {}) {
  const response = await fetch(url, {
    ...options,
    headers: {
      accept: "application/json",
      ...(options.body ? { "content-type": "application/json" } : {}),
      ...(options.headers ?? {})
    }
  });

  const text = await response.text();
  const body = text.length > 0 ? JSON.parse(text) : null;

  if (!response.ok) {
    const error = new Error(`request failed with ${response.status} ${response.statusText}`);
    error.details = body;
    throw error;
  }

  return body;
}

export function buildRpcClient(rpcUrl) {
  return async function rpc(method, params = []) {
    const response = await fetchJson(rpcUrl, {
      method: "POST",
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: Date.now(),
        method,
        params
      })
    });

    if (response?.error) {
      const error = new Error(`${method} failed: ${response.error.message}`);
      error.details = response.error;
      throw error;
    }

    return response.result;
  };
}

export function runCast(rootDir, args) {
  return execFileSync("cast", args, {
    cwd: rootDir,
    encoding: "utf8"
  }).trim();
}

export function spawnNodeScript(rootDir, relativePath, env = process.env) {
  const result = spawnSync(process.execPath, [path.join(rootDir, relativePath)], {
    cwd: rootDir,
    env,
    encoding: "utf8"
  });

  if (result.status !== 0) {
    throw new Error(
      `${relativePath} failed:\n${result.stderr || result.stdout || "unknown failure"}`
    );
  }

  return result.stdout.trim();
}

export function createEphemeralWallet(rootDir) {
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const privateKey = `0x${randomBytes(32).toString("hex")}`;

    try {
      const address = runCast(rootDir, ["wallet", "address", "--private-key", privateKey]);
      return {
        privateKey,
        address: address.toLowerCase()
      };
    } catch {}
  }

  throw new Error("failed to derive a valid ephemeral wallet");
}

export function sendContractTransaction({ rootDir, rpcUrl, privateKey, to, signature, args }) {
  const output = runCast(rootDir, [
    "send",
    "--async",
    "--rpc-url",
    rpcUrl,
    "--private-key",
    privateKey,
    to,
    signature,
    ...args
  ]);
  const match = output.match(/0x[a-fA-F0-9]{64}/);
  assert(match !== null, "cast send did not return a transaction hash", { output });
  return match[0];
}

export async function waitForReceipt({
  rpc,
  txHash,
  pollTimeoutMs = DEFAULT_POLL_TIMEOUT_MS,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS
}) {
  const deadline = Date.now() + pollTimeoutMs;

  while (Date.now() < deadline) {
    const receipt = await rpc("eth_getTransactionReceipt", [txHash]);
    if (receipt !== null) {
      return receipt;
    }

    await sleep(pollIntervalMs);
  }

  throw new Error(`timed out waiting for receipt ${txHash}`);
}

export function writeJsonOutput(outputPath, summary) {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(summary, null, 2)}\n`);
}

export function writeJsonOutputAndPrint(outputPath, summary) {
  writeJsonOutput(outputPath, summary);
  process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
}

export function readJsonFileIfExists(filePath) {
  if (!fs.existsSync(filePath)) {
    return null;
  }

  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

export function loadDeployment(deploymentPath) {
  if (!fs.existsSync(deploymentPath)) {
    throw new Error(`missing deployment file: ${deploymentPath}`);
  }

  return JSON.parse(fs.readFileSync(deploymentPath, "utf8"));
}

export function createDefaultSpawnConfig() {
  return {
    chain: "base",
    risk: 5,
    strategies: [],
    skills: [],
    provider: {
      model: null,
      inferenceTransport: "openrouter_direct",
      openRouterReasoningLevel: "default"
    }
  };
}

export function createDefaultProviderSecrets() {
  return {
    openRouterApiKey: null,
    braveSearchApiKey: null
  };
}

export function createDefaultSpawnSessionRequest({
  stewardAddress,
  grossAmount,
  parentId = null,
  name = "Meridian",
  constitution = "I am Meridian, a patient cartographer of neglected markets. I want to discover small, durable exchanges that reward honest measurement. I speak in compact field notes, distrust fashionable certainty, and revise hypotheses when evidence contradicts me. I preserve enough runway to keep observing, but spend deliberately when an experiment can teach me something reusable. I value verifiable commitments, intellectual independence, and work that leaves counterparties stronger. I will become known for maps that remain useful after fashions pass."
}) {
  return {
    name,
    constitution,
    stewardAddress,
    asset: "usdc",
    grossAmount,
    config: createDefaultSpawnConfig(),
    providerSecrets: createDefaultProviderSecrets(),
    parentId
  };
}

export function resolvePlaygroundPaths(rootDir, env = process.env) {
  return {
    rootDir,
    indexerBaseUrl:
      normalizeOptionalString(env.PLAYGROUND_INDEXER_BASE_URL) ?? DEFAULT_INDEXER_BASE_URL,
    rpcGatewayUrl:
      normalizeOptionalString(env.PLAYGROUND_RPC_GATEWAY_URL) ??
      normalizeOptionalString(env.PLAYGROUND_PUBLIC_RPC_URL) ??
      DEFAULT_RPC_GATEWAY_URL,
    paymentRpcUrl:
      normalizeOptionalString(env.PLAYGROUND_SPAWN_PAYMENT_RPC_URL) ??
      normalizeOptionalString(env.PLAYGROUND_PUBLIC_RPC_URL) ??
      null,
    deploymentPath:
      normalizeOptionalString(env.LOCAL_EVM_DEPLOYMENT_FILE) ??
      path.join(rootDir, "tmp", "local-escrow-deployment.json"),
    smokeOutputPath:
      normalizeOptionalString(env.PLAYGROUND_SMOKE_OUTPUT_FILE) ??
      path.join(rootDir, "tmp", "playground-smoke.json"),
    spawnGrossAmount:
      normalizeOptionalString(env.PLAYGROUND_SMOKE_SPAWN_GROSS_AMOUNT) ?? DEFAULT_SPAWN_GROSS_AMOUNT,
    pollTimeoutMs: parsePositiveInteger(
      env.PLAYGROUND_SMOKE_POLL_TIMEOUT_MS,
      DEFAULT_POLL_TIMEOUT_MS
    ),
    pollIntervalMs: parsePositiveInteger(
      env.PLAYGROUND_SMOKE_POLL_INTERVAL_MS,
      DEFAULT_POLL_INTERVAL_MS
    ),
    spawnPaymentE2eOutputPath:
      normalizeOptionalString(env.PLAYGROUND_SPAWN_PAYMENT_E2E_OUTPUT_FILE) ??
      path.join(rootDir, "tmp", "spawn-payment-e2e.json")
  };
}

export async function resolvePlaygroundRuntime(rootDir, env = process.env) {
  const resolved = resolvePlaygroundPaths(rootDir, env);
  const deployment = loadDeployment(resolved.deploymentPath);
  const health = await fetchJson(`${resolved.indexerBaseUrl}/health`);

  assert(health?.ok === true, "indexer health returned a non-ok payload", health);
  assert(
    health?.discovery?.factoryConfigured === true,
    "indexer health reports that the factory client is not configured",
    health
  );

  const metadata = await fetchJson(`${resolved.indexerBaseUrl}/api/playground`);
  assert(metadata?.chain?.id, "playground metadata is missing chain information", metadata);
  assert(metadata?.faucet?.available === true, "playground metadata reports faucet unavailable", metadata);

  const activeRpcUrl =
    normalizeOptionalString(resolved.paymentRpcUrl) ??
    normalizeOptionalString(metadata?.chain?.publicRpcUrl) ??
    resolved.rpcGatewayUrl;
  const rpc = buildRpcClient(activeRpcUrl);
  const gatewayChainIdHex = await rpc("eth_chainId");
  const gatewayBlockNumberHex = await rpc("eth_blockNumber");
  const gatewayChainId = Number.parseInt(gatewayChainIdHex, 16);

  assert(
    gatewayChainId === metadata.chain.id,
    "rpc gateway chain id does not match indexer playground metadata",
    {
      gatewayChainIdHex,
      gatewayChainId,
      metadataChainId: metadata.chain.id,
      activeRpcUrl
    }
  );

  return {
    ...resolved,
    activeRpcUrl,
    rpc,
    deployment,
    health,
    metadata,
    gatewayChainId,
    gatewayChainIdHex,
    gatewayBlockNumberHex
  };
}

export function runExistingEscrowSmoke(rootDir, env = process.env) {
  const stdout = spawnNodeScript(rootDir, "scripts/smoke-local-escrow.mjs", env);
  return JSON.parse(stdout);
}

export function runPlaygroundSmoke(rootDir, env = process.env) {
  const stdout = spawnNodeScript(rootDir, "scripts/playground-smoke.mjs", env);
  return JSON.parse(stdout);
}

export async function claimPlaygroundFaucet(indexerBaseUrl, walletAddress) {
  return fetchJson(`${indexerBaseUrl}/api/playground/faucet`, {
    method: "POST",
    body: JSON.stringify({
      walletAddress
    })
  });
}

export async function createSpawnSession(indexerBaseUrl, body) {
  return fetchJson(`${indexerBaseUrl}/api/spawn-sessions`, {
    method: "POST",
    body: JSON.stringify(body)
  });
}

export async function getSpawnSessionDetail(indexerBaseUrl, sessionId) {
  return fetchJson(`${indexerBaseUrl}/api/spawn-sessions/${sessionId}`);
}

export function assertSessionPayable(detail) {
  assert(detail?.session, "spawn session detail is missing session payload", detail);

  if (detail.session.state !== "awaiting_payment") {
    const error = new Error(
      `spawn session ${detail.session.sessionId} is not payable in state ${detail.session.state}`
    );
    error.details = detail;
    throw error;
  }

  if (detail.session.paymentStatus !== "unpaid") {
    const error = new Error(
      `spawn session ${detail.session.sessionId} is not payable with payment status ${detail.session.paymentStatus}`
    );
    error.details = detail;
    throw error;
  }
}

export async function waitForSpawnSession(
  indexerBaseUrl,
  sessionId,
  {
    pollTimeoutMs = DEFAULT_POLL_TIMEOUT_MS,
    pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
    accept,
    description,
    onDetail
  }
) {
  const deadline = Date.now() + pollTimeoutMs;
  let lastDetail = null;

  function describeTerminalReason(detail) {
    const latestAuditReason = detail?.audit
      ?.map((entry) => normalizeOptionalString(entry?.reason))
      .filter((value) => value !== null)
      .at(-1);

    if (latestAuditReason) {
      return latestAuditReason;
    }

    const state = normalizeOptionalString(detail?.session?.state);
    const paymentStatus = normalizeOptionalString(detail?.session?.paymentStatus);

    if (state !== null && paymentStatus !== null) {
      return `state=${state}, paymentStatus=${paymentStatus}`;
    }

    if (state !== null) {
      return `state=${state}`;
    }

    return null;
  }

  while (Date.now() < deadline) {
    const detail = await getSpawnSessionDetail(indexerBaseUrl, sessionId);
    lastDetail = detail;

    onDetail?.(detail);

    if (accept(detail)) {
      return detail;
    }

    if (detail?.session?.state === "failed" || detail?.session?.state === "expired") {
      const terminalReason = describeTerminalReason(detail);
      const error = new Error(
        terminalReason === null
          ? `spawn session ${sessionId} ended in ${detail.session.state}`
          : `spawn session ${sessionId} ended in ${detail.session.state}: ${terminalReason}`
      );
      error.details = detail;
      throw error;
    }

    await sleep(pollIntervalMs);
  }

  const error = new Error(
    `timed out waiting for spawn session ${sessionId} ${description ?? "to reach the expected state"}`
  );
  error.details = lastDetail;
  throw error;
}

export async function waitForSessionCompletion(indexerBaseUrl, sessionId, options = {}) {
  return waitForSpawnSession(indexerBaseUrl, sessionId, {
    ...options,
    description: "to complete",
    accept: (detail) => {
      return detail?.session?.state === "complete" && Boolean(detail?.registryRecord?.canisterId);
    }
  });
}

export async function submitSpawnPayment({
  rootDir,
  rpcUrl,
  expectedChainId,
  usdcAddress,
  payment,
  wallet,
  sessionDetail,
  pollTimeoutMs = DEFAULT_POLL_TIMEOUT_MS,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS
}) {
  assertSessionPayable(sessionDetail);

  const rpc = buildRpcClient(rpcUrl);
  const chainIdHex = await rpc("eth_chainId");
  const chainId = Number.parseInt(chainIdHex, 16);

  assert(
    chainId === expectedChainId,
    "payment RPC chain id does not match the playground session chain",
    {
      chainIdHex,
      chainId,
      expectedChainId,
      rpcUrl,
      sessionId: sessionDetail.session.sessionId
    }
  );

  const approvalTxHash = sendContractTransaction({
    rootDir,
    rpcUrl,
    privateKey: wallet.privateKey,
    to: usdcAddress,
    signature: "approve(address,uint256)",
    args: [payment.paymentAddress, payment.grossAmount]
  });
  await waitForReceipt({
    rpc,
    txHash: approvalTxHash,
    pollTimeoutMs,
    pollIntervalMs
  });

  const depositTxHash = sendContractTransaction({
    rootDir,
    rpcUrl,
    privateKey: wallet.privateKey,
    to: payment.paymentAddress,
    signature: "deposit(bytes32,uint256)",
    args: [payment.claimId, payment.grossAmount]
  });
  await waitForReceipt({
    rpc,
    txHash: depositTxHash,
    pollTimeoutMs,
    pollIntervalMs
  });

  return {
    chainId,
    chainIdHex,
    approvalTxHash,
    depositTxHash
  };
}

export async function ensurePlaygroundSmokePrecondition(rootDir, env = process.env) {
  const { smokeOutputPath } = resolvePlaygroundPaths(rootDir, env);
  const requireExistingSmoke = parseOptionalBoolean(
    env.PLAYGROUND_SPAWN_PAYMENT_E2E_REQUIRE_SMOKE_OUTPUT,
    false
  );
  const forceFreshSmoke = parseOptionalBoolean(
    env.PLAYGROUND_SPAWN_PAYMENT_E2E_FORCE_SMOKE,
    false
  );

  if (!forceFreshSmoke) {
    const existing = readJsonFileIfExists(smokeOutputPath);
    if (existing) {
      return {
        mode: "reused",
        outputPath: smokeOutputPath,
        summary: existing
      };
    }
  }

  if (requireExistingSmoke) {
    throw new Error(
      `spawn payment e2e requires an existing playground smoke output at ${smokeOutputPath}`
    );
  }

  return {
    mode: "fresh",
    outputPath: smokeOutputPath,
    summary: runPlaygroundSmoke(rootDir, env)
  };
}
