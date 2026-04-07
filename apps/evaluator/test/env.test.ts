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
        "IC_AUTOMATON_REPO=/tmp/from-dotenv"
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
    expect(merged.IC_AUTOMATON_REPO).toBe("/tmp/from-dotenv");
    expect(merged.EXTRA_VALUE).toBe("kept");

    const runtimeEnv = loadEvaluatorEnv(repoRoot, {
      EVAL_OPENROUTER_API_KEY: "env-openrouter"
    });

    expect(runtimeEnv).toEqual({
      stewardAddress: "0x00000000000000000000000000000000000000aa",
      openRouterApiKey: "env-openrouter",
      braveSearchApiKey: null,
      localEvmForkUrl: "https://dotenv.invalid/base",
      automatonRepoPath: "/tmp/from-dotenv"
    });
  });
});
