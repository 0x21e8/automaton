import { fileURLToPath } from "node:url";

export interface EvaluatorConfig {
  host: string;
  port: number;
  websocketPath: string;
  repoRoot: string;
  artifactsRoot: string;
  experimentPath: string | null;
  indexerBaseUrl: string;
  rpcGatewayUrl: string;
  localReplicaHost: string;
  localReplicaPort: number;
}

export interface EvaluatorConfigOverrides {
  host?: string;
  port?: number;
  websocketPath?: string;
  repoRoot?: string;
  artifactsRoot?: string;
  experimentPath?: string | null;
  indexerBaseUrl?: string;
  rpcGatewayUrl?: string;
  localReplicaHost?: string;
  localReplicaPort?: number;
}

const DEFAULT_REPO_ROOT = fileURLToPath(new URL("../../..", import.meta.url));
const DEFAULT_ARTIFACTS_ROOT = fileURLToPath(
  new URL("../../../tmp/evaluations", import.meta.url)
);

function parsePositiveInteger(value: string | undefined, fallback: number) {
  if (!value) {
    return fallback;
  }

  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : fallback;
}

function parseOptionalString(value: string | undefined) {
  if (value === undefined) {
    return null;
  }

  const normalized = value.trim();
  return normalized === "" ? null : normalized;
}

function inferBaseUrlFromPort(portValue: string | undefined, fallback: string) {
  const parsedPort = Number(portValue);
  if (!Number.isInteger(parsedPort) || parsedPort <= 0 || parsedPort > 65_535) {
    return fallback;
  }

  return `http://127.0.0.1:${parsedPort}`;
}

export function resolveEvaluatorConfig(
  env: NodeJS.ProcessEnv = process.env,
  overrides: EvaluatorConfigOverrides = {}
): EvaluatorConfig {
  return {
    host: overrides.host ?? env.EVALUATOR_HOST?.trim() ?? "0.0.0.0",
    port: overrides.port ?? parsePositiveInteger(env.EVALUATOR_PORT, 3003),
    websocketPath: overrides.websocketPath ?? env.EVALUATOR_WS_PATH?.trim() ?? "/ws/events",
    repoRoot: overrides.repoRoot ?? env.EVALUATOR_REPO_ROOT?.trim() ?? DEFAULT_REPO_ROOT,
    artifactsRoot:
      overrides.artifactsRoot ?? env.EVALUATOR_ARTIFACTS_ROOT?.trim() ?? DEFAULT_ARTIFACTS_ROOT,
    experimentPath:
      overrides.experimentPath ??
      parseOptionalString(env.EVALUATOR_EXPERIMENT_PATH ?? env.EXPERIMENT_PATH),
    indexerBaseUrl:
      overrides.indexerBaseUrl ??
      env.PLAYGROUND_INDEXER_BASE_URL?.trim() ??
      inferBaseUrlFromPort(env.PLAYGROUND_INDEXER_PORT, "http://127.0.0.1:3001") ??
      "http://127.0.0.1:3001",
    rpcGatewayUrl:
      overrides.rpcGatewayUrl ??
      env.PLAYGROUND_RPC_GATEWAY_URL?.trim() ??
      inferBaseUrlFromPort(env.PLAYGROUND_RPC_GATEWAY_PORT, "http://127.0.0.1:3002") ??
      env.PLAYGROUND_PUBLIC_RPC_URL?.trim() ??
      "http://127.0.0.1:3002",
    localReplicaHost:
      overrides.localReplicaHost ?? env.PLAYGROUND_LOCAL_REPLICA_HOST?.trim() ?? "127.0.0.1",
    localReplicaPort:
      overrides.localReplicaPort ??
      parsePositiveInteger(env.PLAYGROUND_LOCAL_REPLICA_PORT, 8000)
  };
}
