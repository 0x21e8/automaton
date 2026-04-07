import { readFileSync } from "node:fs";
import { join } from "node:path";

import type { EvaluatorRuntimeEnv } from "../types.js";

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
  return {
    ...readRepoDotEnv(repoRoot),
    ...baseEnv
  };
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
    localEvmForkUrl: requireValue(merged.LOCAL_EVM_FORK_URL, "LOCAL_EVM_FORK_URL", issues),
    automatonRepoPath: requireValue(merged.IC_AUTOMATON_REPO, "IC_AUTOMATON_REPO", issues)
  };

  if (issues.length > 0) {
    throw new Error(issues.join("\n"));
  }

  return result;
}
