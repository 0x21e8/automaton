import { readFileSync } from "node:fs";
import path, { join } from "node:path";

import type { EvaluatorRuntimeEnv } from "../types.js";

const FACTORY_PROXY_ENV_ALIASES = [
  [
    "EVAL_INFERENCE_PROXY_WORKER_BASE_URL",
    "FACTORY_CHILD_INFERENCE_PROXY_WORKER_BASE_URL"
  ],
  [
    "EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL",
    "FACTORY_CHILD_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL"
  ]
] as const;

function parseDotEnv(source: string): Record<string, string> {
  const entries: Record<string, string> = {};

  for (const rawLine of source.split(/\r?\n/u)) {
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) {
      continue;
    }

    const separatorIndex = line.indexOf("=");
    if (separatorIndex < 1) {
      continue;
    }

    const key = line.slice(0, separatorIndex).trim();
    let value = line.slice(separatorIndex + 1).trim();

    if (
      (value.startsWith("\"") && value.endsWith("\"")) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }

    if (key !== "") {
      entries[key] = value;
    }
  }

  return entries;
}

function applyFactoryProxyEnvAliases(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const merged = {
    ...env
  };

  for (const [sourceKey, targetKey] of FACTORY_PROXY_ENV_ALIASES) {
    const targetValue = merged[targetKey]?.trim();
    if (targetValue) {
      continue;
    }

    const sourceValue = merged[sourceKey]?.trim();
    if (sourceValue) {
      merged[targetKey] = sourceValue;
    }
  }

  return merged;
}

function readRepoDotEnv(repoRoot: string) {
  try {
    const source = readFileSync(join(repoRoot, ".env"), "utf8");
    return parseDotEnv(source);
  } catch {
    return {};
  }
}

export function loadRepoEnv(
  repoRoot: string,
  baseEnv: NodeJS.ProcessEnv = process.env
): NodeJS.ProcessEnv {
  return applyFactoryProxyEnvAliases({
    ...readRepoDotEnv(repoRoot),
    ...baseEnv
  });
}

function requireValue(
  value: string | undefined,
  key: string,
  issues: string[]
) {
  if (typeof value !== "string" || value.trim() === "") {
    issues.push(`${key} is required in the repo-root .env or process environment.`);
    return "";
  }

  return value.trim();
}

export function loadEvaluatorEnv(
  repoRoot: string,
  baseEnv: NodeJS.ProcessEnv = process.env
): EvaluatorRuntimeEnv {
  const merged = loadRepoEnv(repoRoot, baseEnv);
  const issues: string[] = [];

  const result: EvaluatorRuntimeEnv = {
    stewardAddress: requireValue(merged.EVAL_STEWARD_ADDRESS, "EVAL_STEWARD_ADDRESS", issues),
    openRouterApiKey: requireValue(
      merged.EVAL_OPENROUTER_API_KEY,
      "EVAL_OPENROUTER_API_KEY",
      issues
    ),
    braveSearchApiKey: merged.EVAL_BRAVE_SEARCH_API_KEY?.trim() || null,
    inferenceProxyWorkerBaseUrl:
      merged.EVAL_INFERENCE_PROXY_WORKER_BASE_URL?.trim() || null,
    inferenceProxyTrustedCallbackPrincipal:
      merged.EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL?.trim() || null,
    localEvmForkUrl: requireValue(merged.LOCAL_EVM_FORK_URL, "LOCAL_EVM_FORK_URL", issues),
    automatonRepoPath: path.resolve(
      repoRoot,
      merged.AUTOMATON_COMPONENT_ROOT?.trim() ||
        merged.IC_AUTOMATON_REPO?.trim() ||
        "components/ic-automaton"
    )
  };

  if (issues.length > 0) {
    throw new Error(issues.join("\n"));
  }

  return result;
}
