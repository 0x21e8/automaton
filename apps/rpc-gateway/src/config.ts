export interface RpcGatewayConfig {
  host: string;
  port: number;
  upstreamUrl: string;
  chainId: number;
  bodyLimitBytes: number;
  rateLimitWindowMs: number;
  rateLimitMaxRequests: number;
}

export interface RpcGatewayConfigOverrides {
  host?: string;
  port?: number;
  upstreamUrl?: string;
  chainId?: number;
  bodyLimitBytes?: number;
  rateLimitWindowMs?: number;
  rateLimitMaxRequests?: number;
}

const DEFAULT_HOST = "0.0.0.0";
const DEFAULT_PORT = 3002;
const DEFAULT_UPSTREAM_URL = "http://127.0.0.1:8545";
const DEFAULT_CHAIN_ID = 8453;
const DEFAULT_BODY_LIMIT_BYTES = 32 * 1024;
const DEFAULT_RATE_LIMIT_WINDOW_MS = 60_000;
const DEFAULT_RATE_LIMIT_MAX_REQUESTS = 2_000;

function parsePositiveInteger(value: string | undefined, fallback: number) {
  if (!value) {
    return fallback;
  }

  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : fallback;
}

export function resolveRpcGatewayConfig(
  env: NodeJS.ProcessEnv = process.env,
  overrides: RpcGatewayConfigOverrides = {}
): RpcGatewayConfig {
  return {
    host: overrides.host ?? env.RPC_GATEWAY_HOST ?? DEFAULT_HOST,
    port: overrides.port ?? parsePositiveInteger(env.RPC_GATEWAY_PORT, DEFAULT_PORT),
    upstreamUrl:
      overrides.upstreamUrl ?? env.RPC_GATEWAY_UPSTREAM_URL?.trim() ?? DEFAULT_UPSTREAM_URL,
    chainId:
      overrides.chainId ??
      parsePositiveInteger(
        env.PLAYGROUND_CHAIN_ID ?? env.RPC_GATEWAY_CHAIN_ID ?? env.LOCAL_EVM_CHAIN_ID,
        DEFAULT_CHAIN_ID
      ),
    bodyLimitBytes:
      overrides.bodyLimitBytes ??
      parsePositiveInteger(env.RPC_GATEWAY_BODY_LIMIT_BYTES, DEFAULT_BODY_LIMIT_BYTES),
    rateLimitWindowMs:
      overrides.rateLimitWindowMs ??
      parsePositiveInteger(env.RPC_GATEWAY_RATE_LIMIT_WINDOW_MS, DEFAULT_RATE_LIMIT_WINDOW_MS),
    rateLimitMaxRequests:
      overrides.rateLimitMaxRequests ??
      parsePositiveInteger(
        env.RPC_GATEWAY_RATE_LIMIT_MAX_REQUESTS,
        DEFAULT_RATE_LIMIT_MAX_REQUESTS
      )
  };
}
