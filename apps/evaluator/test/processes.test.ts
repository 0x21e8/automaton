import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import {
  PlaygroundProcessManager,
  resolveUsdcAddressFromDeployment
} from "../src/lib/processes.js";

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

describe("playground process manager", () => {
  it("includes recent script output when bootstrap fails", async () => {
    const repoRoot = await createTempDirectory("evaluator-processes-repo-");
    const scriptsDir = join(repoRoot, "scripts");
    const bootstrapScriptPath = join(scriptsDir, "playground-bootstrap.sh");
    const stopScriptPath = join(scriptsDir, "playground-stop.sh");

    await mkdir(scriptsDir, { recursive: true });
    await writeFile(
      bootstrapScriptPath,
      [
        "#!/bin/sh",
        "echo \"boot log line\"",
        "echo \"Missing required tool: ic-wasm\" >&2",
        "exit 1"
      ].join("\n"),
      "utf8"
    );
    await writeFile(stopScriptPath, "#!/bin/sh\nexit 0\n", "utf8");
    await chmod(bootstrapScriptPath, 0o755);
    await chmod(stopScriptPath, 0o755);

    const manager = new PlaygroundProcessManager(repoRoot, process.env);

    await expect(manager.bootstrapPlayground()).rejects.toMatchObject({
      message: expect.stringContaining("Missing required tool: ic-wasm"),
      details: expect.objectContaining({
        scriptPath: bootstrapScriptPath,
        exitCode: 1,
        recentOutput: expect.stringContaining("Missing required tool: ic-wasm")
      })
    });
  });
});

describe("resolveUsdcAddressFromDeployment", () => {
  it("prefers usdcAddress when present", () => {
    expect(
      resolveUsdcAddressFromDeployment({
        usdcAddress: "0x111",
        usdcTokenAddress: "0x222",
        mockUsdcAddress: "0x333"
      })
    ).toBe("0x111");
  });

  it("falls back to usdcTokenAddress and mockUsdcAddress", () => {
    expect(
      resolveUsdcAddressFromDeployment({
        usdcTokenAddress: "0x222"
      })
    ).toBe("0x222");

    expect(
      resolveUsdcAddressFromDeployment({
        mockUsdcAddress: "0x333"
      })
    ).toBe("0x333");
  });

  it("returns an empty string when no deployment address exists", () => {
    expect(resolveUsdcAddressFromDeployment({})).toBe("");
  });
});
