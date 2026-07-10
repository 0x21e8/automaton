import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { loadEvaluatorEnv, loadRepoEnv } from "../src/lib/env.js";

const tempPaths: string[] = [];

afterEach(async () => {
  await Promise.all(
    tempPaths.splice(0).map(async (path) => {
      await rm(path, { recursive: true, force: true });
    })
  );
});

async function createTempDirectory(prefix: string) {
  const directory = await mkdtemp(join(tmpdir(), prefix));
  tempPaths.push(directory);
  return directory;
}

describe("evaluator env loading", () => {
  it("merges repo-root .env into the process env while allowing explicit env overrides", async () => {
    const repoRoot = await createTempDirectory("evaluator-env-");
    await writeFile(
      join(repoRoot, ".env"),
      [
        "EVAL_STEWARD_ADDRESS=0x00000000000000000000000000000000000000aa",
        "EVAL_OPENROUTER_API_KEY=dotenv-openrouter",
        "LOCAL_EVM_FORK_URL=https://dotenv.invalid/base",
        "AUTOMATON_COMPONENT_ROOT=/tmp/automaton-component"
      ].join("\n"),
      "utf8"
    );

    const merged = loadRepoEnv(repoRoot, {
      EVAL_OPENROUTER_API_KEY: "env-openrouter",
      EXTRA_VALUE: "kept"
    });

    expect(merged.EVAL_STEWARD_ADDRESS).toBe("0x00000000000000000000000000000000000000aa");
    expect(merged.EVAL_OPENROUTER_API_KEY).toBe("env-openrouter");
    expect(merged.LOCAL_EVM_FORK_URL).toBe("https://dotenv.invalid/base");
    expect(merged.AUTOMATON_COMPONENT_ROOT).toBe("/tmp/automaton-component");
    expect(merged.EXTRA_VALUE).toBe("kept");

    const runtimeEnv = loadEvaluatorEnv(repoRoot, {
      EVAL_OPENROUTER_API_KEY: "env-openrouter"
    });

    expect(runtimeEnv).toEqual({
      stewardAddress: "0x00000000000000000000000000000000000000aa",
      openRouterApiKey: "env-openrouter",
      braveSearchApiKey: null,
      inferenceProxyWorkerBaseUrl: null,
      inferenceProxyTrustedCallbackPrincipal: null,
      localEvmForkUrl: "https://dotenv.invalid/base",
      automatonRepoPath: "/tmp/automaton-component"
    });
  });

  it("loads optional proxy runtime settings when present", async () => {
    const repoRoot = await createTempDirectory("evaluator-env-proxy-");
    await writeFile(
      join(repoRoot, ".env"),
      [
        "EVAL_STEWARD_ADDRESS=0x00000000000000000000000000000000000000aa",
        "EVAL_OPENROUTER_API_KEY=dotenv-openrouter",
        "EVAL_INFERENCE_PROXY_WORKER_BASE_URL=https://proxy.example.com",
        "EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL=aaaaa-aa",
        "LOCAL_EVM_FORK_URL=https://dotenv.invalid/base",
        "AUTOMATON_COMPONENT_ROOT=/tmp/automaton-component"
      ].join("\n"),
      "utf8"
    );

    const runtimeEnv = loadEvaluatorEnv(repoRoot);

    expect(runtimeEnv.inferenceProxyWorkerBaseUrl).toBe("https://proxy.example.com");
    expect(runtimeEnv.inferenceProxyTrustedCallbackPrincipal).toBe("aaaaa-aa");
  });

  it("forwards evaluator proxy env into factory child runtime env by default", async () => {
    const repoRoot = await createTempDirectory("evaluator-env-proxy-aliases-");
    await writeFile(
      join(repoRoot, ".env"),
      [
        "EVAL_INFERENCE_PROXY_WORKER_BASE_URL=https://proxy-from-eval.example.com",
        "EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL=aaaaa-aa"
      ].join("\n"),
      "utf8"
    );

    const merged = loadRepoEnv(repoRoot);

    expect(merged.FACTORY_CHILD_INFERENCE_PROXY_WORKER_BASE_URL).toBe(
      "https://proxy-from-eval.example.com"
    );
    expect(merged.FACTORY_CHILD_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL).toBe("aaaaa-aa");
  });

  it("preserves explicit factory child proxy env overrides", async () => {
    const repoRoot = await createTempDirectory("evaluator-env-proxy-factory-override-");
    await writeFile(
      join(repoRoot, ".env"),
      [
        "EVAL_INFERENCE_PROXY_WORKER_BASE_URL=https://proxy-from-eval.example.com",
        "EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL=aaaaa-aa"
      ].join("\n"),
      "utf8"
    );

    const merged = loadRepoEnv(repoRoot, {
      FACTORY_CHILD_INFERENCE_PROXY_WORKER_BASE_URL: "https://proxy-from-factory.example.com",
      FACTORY_CHILD_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL: "bbbbb-bb"
    });

    expect(merged.FACTORY_CHILD_INFERENCE_PROXY_WORKER_BASE_URL).toBe(
      "https://proxy-from-factory.example.com"
    );
    expect(merged.FACTORY_CHILD_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL).toBe("bbbbb-bb");
  });
});
