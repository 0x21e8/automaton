import fs from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultDeploymentPath = path.join(rootDir, "tmp", "local-escrow-deployment.json");
const deploymentPath = process.env.LOCAL_EVM_DEPLOYMENT_FILE ?? defaultDeploymentPath;
const localInboxDeploymentPath =
  process.env.AUTOMATON_INBOX_DEPLOYMENT_FILE ??
  path.join(rootDir, "tmp", "automaton-inbox-deployment.json");
const siblingRepo = normalizeOptionalString(process.env.IC_AUTOMATON_REPO);
const renderTarget = normalizeOptionalString(process.env.FACTORY_RENDER_TARGET) ?? "init";

const deployment = JSON.parse(fs.readFileSync(deploymentPath, "utf8"));
const deploymentUsdcAddress = normalizeOptionalString(
  deployment.usdcTokenAddress ?? deployment.mockUsdcAddress
);

function normalizeOptionalString(value) {
  if (value === undefined || value === null) {
    return null;
  }

  const trimmed = String(value).trim();
  return trimmed.length === 0 ? null : trimmed;
}

function normalizeOptionalInteger(value) {
  if (value === undefined || value === null || String(value).trim() === "") {
    return null;
  }

  const parsed = Number.parseInt(String(value).trim(), 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function normalizeOptionalBoolean(value) {
  if (value === undefined || value === null || String(value).trim() === "") {
    return null;
  }

  const normalized = String(value).trim().toLowerCase();
  if (normalized === "true") {
    return true;
  }
  if (normalized === "false") {
    return false;
  }
  return null;
}

function normalizeOptionalList(value) {
  if (value === undefined || value === null) {
    return null;
  }

  const items = String(value)
    .split(",")
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0);

  return items.length === 0 ? null : items;
}

function readOptionalTrimmedFile(filePath) {
  if (!filePath || !fs.existsSync(filePath)) {
    return null;
  }
  return normalizeOptionalString(fs.readFileSync(filePath, "utf8"));
}

function resolveOptionalWasmSha256() {
  const explicitSha = normalizeOptionalString(
    process.env.CHILD_WASM_SHA256 ?? process.env.FACTORY_WASM_SHA256
  );

  if (explicitSha !== null) {
    return explicitSha;
  }

  const childWasmPath = normalizeOptionalString(process.env.CHILD_WASM_PATH);
  if (childWasmPath === null) {
    return null;
  }

  const resolvedPath = path.resolve(rootDir, childWasmPath);
  if (!fs.existsSync(resolvedPath)) {
    throw new Error(`missing CHILD_WASM_PATH at ${resolvedPath}`);
  }

  return createHash("sha256")
    .update(fs.readFileSync(resolvedPath))
    .digest("hex");
}

function escapeText(value) {
  return value.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"");
}

function renderOptText(value) {
  return value === null ? "null" : `opt "${escapeText(value)}"`;
}

function renderOptNat(value) {
  return value === null ? "null" : `opt ${value}`;
}

function renderOptBool(value) {
  return value === null ? "null" : `opt ${value ? "true" : "false"}`;
}

function renderOptPrincipal(value) {
  return value === null ? "null" : `opt principal "${escapeText(value)}"`;
}

function renderOptVecText(values) {
  return values === null
    ? "null"
    : `opt vec { ${values.map((value) => `"${escapeText(value)}"`).join("; ")} }`;
}

const versionCommit =
  process.env.CHILD_VERSION_COMMIT ?? process.env.FACTORY_VERSION_COMMIT ?? "dev-build";
const paymentAddress = process.env.FACTORY_PAYMENT_ADDRESS ?? deployment.paymentAddress;
const escrowAddress =
  process.env.FACTORY_ESCROW_CONTRACT_ADDRESS ?? deployment.escrowContractAddress;
const baseRpcEndpoint = process.env.FACTORY_BASE_RPC_ENDPOINT ?? deployment.rpcUrl;
const childEcdsaKeyName =
  normalizeOptionalString(process.env.FACTORY_CHILD_ECDSA_KEY_NAME) ?? "key_1";
const localInboxDeployment = fs.existsSync(localInboxDeploymentPath)
  ? JSON.parse(fs.readFileSync(localInboxDeploymentPath, "utf8"))
  : null;
const localInboxContractAddress = normalizeOptionalString(
  localInboxDeployment?.inboxContractAddress
);
const localInboxUsdcAddress = normalizeOptionalString(localInboxDeployment?.usdcTokenAddress);
const siblingInboxContractAddress = siblingRepo
  ? readOptionalTrimmedFile(path.join(siblingRepo, ".local", "inbox_contract_address"))
  : null;
const siblingInboxUsdcAddress = siblingRepo
  ? readOptionalTrimmedFile(path.join(siblingRepo, ".local", "usdc_token_address"))
  : null;
const childInboxContractAddress =
  normalizeOptionalString(process.env.FACTORY_CHILD_INBOX_CONTRACT_ADDRESS) ??
  localInboxContractAddress ??
  siblingInboxContractAddress;
const childEvmChainId =
  normalizeOptionalInteger(process.env.FACTORY_CHILD_EVM_CHAIN_ID) ?? deployment.chainId ?? 8453;
const childEvmRpcUrl =
  normalizeOptionalString(process.env.FACTORY_CHILD_EVM_RPC_URL) ?? baseRpcEndpoint;
const childEvmConfirmationDepth = normalizeOptionalInteger(
  process.env.FACTORY_CHILD_EVM_CONFIRMATION_DEPTH
);
const childEvmBootstrapLookbackBlocks = normalizeOptionalInteger(
  process.env.FACTORY_CHILD_EVM_BOOTSTRAP_LOOKBACK_BLOCKS
);
const childHttpAllowedDomains = normalizeOptionalList(
  process.env.FACTORY_CHILD_HTTP_ALLOWED_DOMAINS
);
const childLlmCanisterId = normalizeOptionalString(process.env.FACTORY_CHILD_LLM_CANISTER_ID);
const childSearchApiKey = normalizeOptionalString(process.env.FACTORY_CHILD_SEARCH_API_KEY);
const childInferenceProxyWorkerBaseUrl = normalizeOptionalString(
  process.env.FACTORY_CHILD_INFERENCE_PROXY_WORKER_BASE_URL
);
const childInferenceProxyTrustedCallbackPrincipal = normalizeOptionalString(
  process.env.FACTORY_CHILD_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL
);
const childCycleTopupEnabled = normalizeOptionalBoolean(
  process.env.FACTORY_CHILD_CYCLE_TOPUP_ENABLED
);
const childAutoTopupCycleThreshold = normalizeOptionalInteger(
  process.env.FACTORY_CHILD_AUTO_TOPUP_CYCLE_THRESHOLD
);
const cyclesPerSpawn =
  normalizeOptionalInteger(process.env.FACTORY_CYCLES_PER_SPAWN) ?? 10_000_000_000_000;
const minPoolBalance = normalizeOptionalInteger(process.env.FACTORY_MIN_POOL_BALANCE) ?? 0;
const estimatedOutcallCyclesPerInterval =
  normalizeOptionalInteger(process.env.FACTORY_ESTIMATED_OUTCALL_CYCLES_PER_INTERVAL) ?? 0;
const releaseBroadcastChainId =
  normalizeOptionalInteger(process.env.FACTORY_RELEASE_BROADCAST_CHAIN_ID) ??
  childEvmChainId ??
  8453;
const releaseBroadcastMaxPriorityFeePerGas =
  normalizeOptionalInteger(process.env.FACTORY_RELEASE_MAX_PRIORITY_FEE_PER_GAS) ?? 1_000_000_000;
const releaseBroadcastMaxFeePerGas =
  normalizeOptionalInteger(process.env.FACTORY_RELEASE_MAX_FEE_PER_GAS) ?? 3_000_000_000;
const releaseBroadcastGasLimit =
  normalizeOptionalInteger(process.env.FACTORY_RELEASE_GAS_LIMIT) ?? 250_000;
const wasmSha256 = resolveOptionalWasmSha256();

if (!["init", "child-runtime", "operational", "release-broadcast"].includes(renderTarget)) {
  throw new Error(
    `FACTORY_RENDER_TARGET must be one of init, child-runtime, operational, release-broadcast; got ${JSON.stringify(
      renderTarget
    )}`
  );
}

if (
  childInboxContractAddress !== null &&
  deploymentUsdcAddress !== null &&
  localInboxUsdcAddress !== null &&
  localInboxUsdcAddress.toLowerCase() !== deploymentUsdcAddress.toLowerCase()
) {
  throw new Error(
    [
      "local automaton inbox deployment is wired to the wrong USDC contract for this launchpad local stack.",
      `expected inbox USDC ${deploymentUsdcAddress}, got ${localInboxUsdcAddress}.`,
      "Redeploy Inbox.sol against the launchpad configured USDC token on the same Anvil/Base-fork before rendering factory init args."
    ].join(" ")
  );
}

if (
  childInboxContractAddress !== null &&
  deploymentUsdcAddress !== null &&
  siblingInboxUsdcAddress !== null &&
  siblingInboxUsdcAddress.toLowerCase() !== deploymentUsdcAddress.toLowerCase()
) {
  throw new Error(
    [
      "ic-automaton inbox deployment is wired to the wrong USDC contract for this launchpad local stack.",
      `expected inbox USDC ${deploymentUsdcAddress}, got ${siblingInboxUsdcAddress}.`,
      "Deploy Inbox.sol against the launchpad configured USDC token on the same Anvil/Base-fork before rendering factory init args."
    ].join(" ")
  );
}

const childRuntimeRecord = `record {
  ecdsa_key_name = ${renderOptText(childEcdsaKeyName)};
  inbox_contract_address = ${renderOptText(childInboxContractAddress)};
  evm_chain_id = ${renderOptNat(childEvmChainId)};
  evm_rpc_url = ${renderOptText(childEvmRpcUrl)};
  evm_confirmation_depth = ${renderOptNat(childEvmConfirmationDepth)};
  evm_bootstrap_lookback_blocks = ${renderOptNat(childEvmBootstrapLookbackBlocks)};
  http_allowed_domains = ${renderOptVecText(childHttpAllowedDomains)};
  llm_canister_id = ${renderOptPrincipal(childLlmCanisterId)};
  search_api_key = ${renderOptText(childSearchApiKey)};
  inference_proxy_worker_base_url = ${renderOptText(childInferenceProxyWorkerBaseUrl)};
  inference_proxy_trusted_callback_principal = ${renderOptPrincipal(childInferenceProxyTrustedCallbackPrincipal)};
  cycle_topup_enabled = ${renderOptBool(childCycleTopupEnabled)};
  auto_topup_cycle_threshold = ${renderOptNat(childAutoTopupCycleThreshold)};
}`;

const initCandid = `(
  opt record {
    admin_principals = vec {};
    fee_config = null;
    creation_cost_quote = null;
    child_runtime = opt ${childRuntimeRecord};
    pause = false;
    payment_address = opt "${paymentAddress}";
    escrow_contract_address = opt "${escrowAddress}";
    base_rpc_endpoint = opt "${baseRpcEndpoint}";
    cycles_per_spawn = opt ${cyclesPerSpawn};
    min_pool_balance = opt ${minPoolBalance};
    estimated_outcall_cycles_per_interval = opt ${estimatedOutcallCyclesPerInterval};
    session_ttl_ms = null;
    version_commit = opt "${versionCommit}";
    wasm_sha256 = ${renderOptText(wasmSha256)};
  }
)`;

const childRuntimeCandid = `(
  ${childRuntimeRecord}
)`;

const operationalCandid = `(
  record {
    cycles_per_spawn = ${cyclesPerSpawn};
    min_pool_balance = ${minPoolBalance};
    estimated_outcall_cycles_per_interval = ${estimatedOutcallCyclesPerInterval};
  }
)`;

const releaseBroadcastCandid = `(
  record {
    chain_id = ${releaseBroadcastChainId};
    max_priority_fee_per_gas = ${releaseBroadcastMaxPriorityFeePerGas};
    max_fee_per_gas = ${releaseBroadcastMaxFeePerGas};
    gas_limit = ${releaseBroadcastGasLimit};
    ecdsa_key_name = "${escapeText(childEcdsaKeyName)}";
  }
)`;

const output =
  renderTarget === "child-runtime"
    ? childRuntimeCandid
    : renderTarget === "operational"
      ? operationalCandid
      : renderTarget === "release-broadcast"
        ? releaseBroadcastCandid
      : initCandid;

process.stdout.write(`${output}\n`);
