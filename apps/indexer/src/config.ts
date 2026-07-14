import { fileURLToPath } from "node:url";

import type { PlaygroundMetadata } from "@ic-automaton/shared";

import {
  INDEXER_NETWORK_TARGETS,
  INDEXER_TARGET_CONFIG,
  type IndexerTargetConfig
} from "./indexer.config.js";

export interface IndexerPlaygroundConfig {
  metadata: PlaygroundMetadata;
  statusFilePath: string;
}

export interface IndexerConfig {
  host: string;
  port: number;
  databasePath: string;
  websocketPath: string;
  corsAllowedOrigins: string[];
  ingestion: IndexerTargetConfig;
  factoryCanisterId?: string;
  icHost: string;
  fastPollIntervalMs: number;
  slowPollIntervalMs: number;
  pricePollIntervalMs: number;
  playground: IndexerPlaygroundConfig;
  society: {
    trustedInboxAddresses: string[];
    trustedUsdcAddresses: string[];
  };
}

export interface IndexerConfigOverrides {
  host?: string;
  port?: number;
  databasePath?: string;
  websocketPath?: string;
  corsAllowedOrigins?: string[];
  ingestion?: IndexerTargetConfig;
  factoryCanisterId?: string;
  fastPollIntervalMs?: number;
  slowPollIntervalMs?: number;
  pricePollIntervalMs?: number;
  playground?: IndexerPlaygroundConfig;
  society?: IndexerConfig["society"];
}

const DEFAULT_DATABASE_PATH = fileURLToPath(
  new URL("../data/indexer.sqlite", import.meta.url)
);
const DEFAULT_PLAYGROUND_STATUS_FILE_PATH = fileURLToPath(
  new URL("../tmp/playground-status.json", import.meta.url)
);
const DEFAULT_CORS_ALLOWED_ORIGINS = [
  "http://127.0.0.1:5173",
  "http://localhost:5173"
] as const;
const DEFAULT_PLAYGROUND_ENVIRONMENT_LABEL = "Local development";
const DEFAULT_PLAYGROUND_CHAIN_NAME = "Base Local Fork";
const DEFAULT_PLAYGROUND_PUBLIC_RPC_HOST = "127.0.0.1";
const DEFAULT_PLAYGROUND_PUBLIC_RPC_PORT = 8545;
const DEFAULT_PLAYGROUND_CHAIN_ID = 8453;
const DEFAULT_PLAYGROUND_RESET_CADENCE_LABEL = "Manual local resets";
const DEFAULT_PLAYGROUND_FAUCET_CLAIM_WINDOW_SECONDS = 86_400;
const DEFAULT_PLAYGROUND_FAUCET_MAX_CLAIMS_PER_WALLET = 1;
const DEFAULT_PLAYGROUND_FAUCET_MAX_CLAIMS_PER_IP = 1;
const DEFAULT_PLAYGROUND_FAUCET_ETH_AMOUNT = "1";
const DEFAULT_PLAYGROUND_FAUCET_USDC_AMOUNT = "250";
const BASE32_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";
const CRC32_TABLE = buildCrc32Table();

function parseNumber(value: string | undefined, fallback: number) {
  if (!value) {
    return fallback;
  }

  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function parseOptionalInteger(value: string | undefined) {
  if (value === undefined) {
    return undefined;
  }

  return Number(value);
}

function parseOptionalBoolean(value: string | undefined) {
  if (value === undefined) {
    return undefined;
  }

  const normalized = value.trim().toLowerCase();

  if (["1", "true", "yes", "on"].includes(normalized)) {
    return true;
  }

  if (["0", "false", "no", "off"].includes(normalized)) {
    return false;
  }

  return undefined;
}

function parseOptionalTimestamp(value: string | undefined) {
  if (value === undefined) {
    return undefined;
  }

  const normalized = value.trim();
  if (normalized === "") {
    return null;
  }

  const numericValue = Number(normalized);
  if (Number.isFinite(numericValue)) {
    return numericValue;
  }

  const parsed = Date.parse(normalized);
  return Number.isNaN(parsed) ? undefined : parsed;
}

function parseOriginList(value: string | undefined) {
  if (value === undefined) {
    return undefined;
  }

  return value
    .split(",")
    .map((origin) => origin.trim())
    .filter((origin) => origin.length > 0);
}

function parseCanisterIdList(value: string | undefined) {
  if (value === undefined) {
    return undefined;
  }

  return value
    .split(",")
    .map((canisterId) => canisterId.trim())
    .filter((canisterId) => canisterId.length > 0);
}

function parseEvmAddressList(value: string | undefined) {
  if (!value) return [];
  return [...new Set(value.split(",").map((entry) => entry.trim().toLowerCase()).filter((entry) => /^0x[0-9a-f]{40}$/.test(entry)))];
}

function buildCrc32Table() {
  const table = new Uint32Array(256);

  for (let index = 0; index < table.length; index += 1) {
    let value = index;

    for (let bit = 0; bit < 8; bit += 1) {
      value = (value & 1) === 0 ? value >>> 1 : (value >>> 1) ^ 0xedb88320;
    }

    table[index] = value >>> 0;
  }

  return table;
}

function computeCrc32(bytes: Uint8Array) {
  let crc = 0xffffffff;

  for (const byte of bytes) {
    crc = CRC32_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }

  return (crc ^ 0xffffffff) >>> 0;
}

function decodeBase32(input: string) {
  const output: number[] = [];
  let buffer = 0;
  let bits = 0;

  for (const character of input) {
    const value = BASE32_ALPHABET.indexOf(character);

    if (value === -1) {
      return null;
    }

    buffer = (buffer << 5) | value;
    bits += 5;

    while (bits >= 8) {
      bits -= 8;
      output.push((buffer >>> bits) & 0xff);
    }
  }

  if (bits > 0 && (buffer & ((1 << bits) - 1)) !== 0) {
    return null;
  }

  return Uint8Array.from(output);
}

function isValidCanisterId(value: string) {
  if (!/^[a-z2-7-]+$/.test(value)) {
    return false;
  }

  const groups = value.split("-");
  if (groups.length === 0 || groups.some((group) => group.length === 0 || group.length > 5)) {
    return false;
  }

  if (groups.slice(0, -1).some((group) => group.length !== 5)) {
    return false;
  }

  const decoded = decodeBase32(groups.join(""));
  if (decoded === null || decoded.length < 4) {
    return false;
  }

  const checksum =
    (decoded[0] << 24) | (decoded[1] << 16) | (decoded[2] << 8) | decoded[3];
  const principal = decoded.slice(4);

  return (checksum >>> 0) === computeCrc32(principal);
}

function validateTargetConfig(
  config: IndexerTargetConfig,
  options: {
    allowEmptyCanisterIds?: boolean;
  } = {}
) {
  const errors: string[] = [];

  if (!Array.isArray(config.canisterIds)) {
    errors.push("Indexer ingestion config must include a canisterIds array.");
  } else if (config.canisterIds.length === 0 && !options.allowEmptyCanisterIds) {
    errors.push(
      "Indexer ingestion config must include at least one canister ID when factory discovery is not configured."
    );
  }

  for (const [index, canisterId] of config.canisterIds.entries()) {
    if (typeof canisterId !== "string" || !isValidCanisterId(canisterId.trim())) {
      errors.push(
        `Indexer ingestion config canisterIds[${index}] must be a valid canister ID. Received: ${JSON.stringify(canisterId)}.`
      );
    }
  }

  if (!INDEXER_NETWORK_TARGETS.includes(config.network.target)) {
    errors.push(
      `Indexer ingestion config network.target must be one of ${INDEXER_NETWORK_TARGETS.map((target) => JSON.stringify(target)).join(", ")}. Received: ${JSON.stringify(config.network.target)}.`
    );
  }

  if (config.network.target === "local") {
    if (typeof config.network.local.host !== "string" || config.network.local.host.trim() === "") {
      errors.push(
        'Indexer ingestion config network.local.host must be set when network.target is "local".'
      );
    }

    if (!Number.isInteger(config.network.local.port) || config.network.local.port <= 0) {
      errors.push(
        'Indexer ingestion config network.local.port must be set to a positive integer when network.target is "local".'
      );
    }
  }

  if (errors.length > 0) {
    throw new Error(`Invalid indexer ingestion config:\n- ${errors.join("\n- ")}`);
  }
}

function cloneTargetConfig(config: IndexerTargetConfig): IndexerTargetConfig {
  return {
    canisterIds: [...config.canisterIds],
    network: {
      target: config.network.target,
      local: {
        host: config.network.local.host,
        port: config.network.local.port
      }
    }
  };
}

function applyEnvIngestionOverrides(
  config: IndexerTargetConfig,
  env: NodeJS.ProcessEnv
): IndexerTargetConfig {
  const nextConfig = cloneTargetConfig(config);

  if (env.INDEXER_INGESTION_NETWORK_TARGET !== undefined) {
    nextConfig.network.target =
      env.INDEXER_INGESTION_NETWORK_TARGET as IndexerTargetConfig["network"]["target"];
  }

  if (env.INDEXER_INGESTION_LOCAL_HOST !== undefined) {
    nextConfig.network.local.host = env.INDEXER_INGESTION_LOCAL_HOST;
  }
  if (env.INDEXER_INGESTION_CANISTER_IDS !== undefined) {
    nextConfig.canisterIds = parseCanisterIdList(env.INDEXER_INGESTION_CANISTER_IDS) ?? [];
  }

  const localPort = parseOptionalInteger(env.INDEXER_INGESTION_LOCAL_PORT);

  if (localPort !== undefined) {
    nextConfig.network.local.port = localPort;
  }

  return nextConfig;
}

function resolveIcHost(config: IndexerTargetConfig) {
  if (config.network.target === "mainnet") {
    return "https://ic0.app";
  }

  return `http://${config.network.local.host}:${config.network.local.port}`;
}

function resolveLocalPlaygroundRpcUrl(env: NodeJS.ProcessEnv) {
  const explicitRpcUrl = env.LOCAL_EVM_RPC_URL?.trim();

  if (explicitRpcUrl) {
    return explicitRpcUrl;
  }

  const host = env.LOCAL_EVM_HOST?.trim() || DEFAULT_PLAYGROUND_PUBLIC_RPC_HOST;
  const port = parseNumber(env.LOCAL_EVM_PORT, DEFAULT_PLAYGROUND_PUBLIC_RPC_PORT);

  return `http://${host}:${port}`;
}

function resolvePlaygroundConfig(env: NodeJS.ProcessEnv): IndexerPlaygroundConfig {
  const localChainId = parseNumber(env.LOCAL_EVM_CHAIN_ID, DEFAULT_PLAYGROUND_CHAIN_ID);
  const chainId = parseNumber(env.PLAYGROUND_CHAIN_ID, localChainId);
  const lastResetAt = parseOptionalTimestamp(env.PLAYGROUND_LAST_RESET_AT);
  const nextResetAt = parseOptionalTimestamp(env.PLAYGROUND_NEXT_RESET_AT);

  return {
    metadata: {
      environmentLabel:
        env.PLAYGROUND_ENV_LABEL?.trim() || DEFAULT_PLAYGROUND_ENVIRONMENT_LABEL,
      environmentVersion: env.PLAYGROUND_ENV_VERSION?.trim() || null,
      maintenance: parseOptionalBoolean(env.PLAYGROUND_MAINTENANCE) ?? false,
      chain: {
        id: chainId,
        name: env.PLAYGROUND_CHAIN_NAME?.trim() || DEFAULT_PLAYGROUND_CHAIN_NAME,
        publicRpcUrl:
          env.PLAYGROUND_PUBLIC_RPC_URL?.trim() || resolveLocalPlaygroundRpcUrl(env),
        nativeCurrency: {
          name: env.PLAYGROUND_NATIVE_CURRENCY_NAME?.trim() || "Ether",
          symbol: env.PLAYGROUND_NATIVE_CURRENCY_SYMBOL?.trim() || "ETH",
          decimals: parseNumber(env.PLAYGROUND_NATIVE_CURRENCY_DECIMALS, 18)
        },
        explorerUrl: env.PLAYGROUND_EXPLORER_URL?.trim() || null
      },
      faucet: {
        available: parseOptionalBoolean(env.PLAYGROUND_FAUCET_ENABLED) ?? false,
        claimLimits: {
          windowSeconds: parseNumber(
            env.PLAYGROUND_FAUCET_CLAIM_WINDOW_SECONDS,
            DEFAULT_PLAYGROUND_FAUCET_CLAIM_WINDOW_SECONDS
          ),
          maxClaimsPerWallet: parseNumber(
            env.PLAYGROUND_FAUCET_MAX_CLAIMS_PER_WALLET,
            DEFAULT_PLAYGROUND_FAUCET_MAX_CLAIMS_PER_WALLET
          ),
          maxClaimsPerIp: parseNumber(
            env.PLAYGROUND_FAUCET_MAX_CLAIMS_PER_IP,
            DEFAULT_PLAYGROUND_FAUCET_MAX_CLAIMS_PER_IP
          )
        },
        claimAssetAmounts: [
          {
            asset: "eth",
            amount: env.PLAYGROUND_FAUCET_ETH_AMOUNT?.trim() || DEFAULT_PLAYGROUND_FAUCET_ETH_AMOUNT,
            decimals: 18
          },
          {
            asset: "usdc",
            amount:
              env.PLAYGROUND_FAUCET_USDC_AMOUNT?.trim() || DEFAULT_PLAYGROUND_FAUCET_USDC_AMOUNT,
            decimals: 6
          }
        ]
      },
      reset: {
        lastResetAt: lastResetAt ?? null,
        nextResetAt: nextResetAt ?? null,
        cadenceLabel:
          env.PLAYGROUND_RESET_CADENCE_LABEL?.trim() ||
          DEFAULT_PLAYGROUND_RESET_CADENCE_LABEL
      }
    },
    statusFilePath:
      env.PLAYGROUND_STATUS_FILE?.trim() || DEFAULT_PLAYGROUND_STATUS_FILE_PATH
  };
}

export function resolveIndexerConfig(
  env: NodeJS.ProcessEnv = process.env,
  overrides: IndexerConfigOverrides = {}
): IndexerConfig {
  const factoryCanisterId =
    overrides.factoryCanisterId ?? env.INDEXER_FACTORY_CANISTER_ID ?? undefined;
  const ingestion = applyEnvIngestionOverrides(
    overrides.ingestion ?? INDEXER_TARGET_CONFIG,
    env
  );
  validateTargetConfig(ingestion, {
    allowEmptyCanisterIds: factoryCanisterId !== undefined
  });

  return {
    host: overrides.host ?? env.HOST ?? "0.0.0.0",
    port: overrides.port ?? parseNumber(env.PORT, 3001),
    databasePath: overrides.databasePath ?? env.INDEXER_DB_PATH ?? DEFAULT_DATABASE_PATH,
    websocketPath: overrides.websocketPath ?? env.INDEXER_WS_PATH ?? "/ws/events",
    corsAllowedOrigins:
      overrides.corsAllowedOrigins ??
      parseOriginList(env.INDEXER_CORS_ALLOWED_ORIGINS) ?? [...DEFAULT_CORS_ALLOWED_ORIGINS],
    ingestion,
    factoryCanisterId,
    icHost: resolveIcHost(ingestion),
    fastPollIntervalMs:
      overrides.fastPollIntervalMs ?? parseNumber(env.INDEXER_FAST_POLL_INTERVAL_MS, 15_000),
    slowPollIntervalMs:
      overrides.slowPollIntervalMs ?? parseNumber(env.INDEXER_SLOW_POLL_INTERVAL_MS, 300_000),
    pricePollIntervalMs:
      overrides.pricePollIntervalMs ?? parseNumber(env.INDEXER_PRICE_POLL_INTERVAL_MS, 60_000),
    playground: overrides.playground ?? resolvePlaygroundConfig(env),
    society: overrides.society ?? {
      trustedInboxAddresses: parseEvmAddressList(env.INDEXER_TRUSTED_INBOX_ADDRESSES),
      trustedUsdcAddresses: parseEvmAddressList(env.INDEXER_TRUSTED_USDC_ADDRESSES)
    }
  };
}
