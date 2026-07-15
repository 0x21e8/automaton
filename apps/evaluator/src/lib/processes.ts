import { execFile, spawn } from "node:child_process";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";

import type { PlaygroundRuntime } from "../types.js";

const execFileAsync = promisify(execFile);
const SCRIPT_OUTPUT_TAIL_LIMIT = 8_192;

interface PlaygroundE2eModule {
  resolvePlaygroundRuntime(
    rootDir: string,
    env?: NodeJS.ProcessEnv
  ): Promise<{
    activeRpcUrl: string;
    gatewayChainId: number;
    deployment: {
      usdcAddress?: string;
      usdcTokenAddress?: string;
      mockUsdcAddress?: string;
    };
  }>;
}

async function loadPlaygroundHelpers(repoRoot: string): Promise<PlaygroundE2eModule> {
  return (await import(
    pathToFileURL(join(repoRoot, "scripts/lib/playground-e2e.mjs")).href
  )) as PlaygroundE2eModule;
}

async function runScriptStreaming(options: {
  cwd: string;
  env: NodeJS.ProcessEnv;
  scriptPath: string;
}) {
  let stdoutTail = "";
  let stderrTail = "";

  const appendOutputTail = (current: string, chunk: string) => {
    const next = `${current}${chunk}`;
    return next.length <= SCRIPT_OUTPUT_TAIL_LIMIT
      ? next
      : next.slice(next.length - SCRIPT_OUTPUT_TAIL_LIMIT);
  };

  await new Promise<void>((resolve, reject) => {
    const child = spawn("sh", [options.scriptPath], {
      cwd: options.cwd,
      env: options.env,
      stdio: ["ignore", "pipe", "pipe"]
    });

    child.stdout?.on("data", (chunk: Buffer | string) => {
      const text = chunk.toString();
      process.stdout.write(text);
      stdoutTail = appendOutputTail(stdoutTail, text);
    });

    child.stderr?.on("data", (chunk: Buffer | string) => {
      const text = chunk.toString();
      process.stderr.write(text);
      stderrTail = appendOutputTail(stderrTail, text);
    });

    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }

      const suffix =
        signal === null
          ? `exit code ${code ?? "unknown"}`
          : `signal ${signal}`;
      const recentOutput = [stderrTail.trim(), stdoutTail.trim()].filter(Boolean).join("\n");
      const error = new Error(
        recentOutput === ""
          ? `Command failed: sh ${options.scriptPath} (${suffix})`
          : `Command failed: sh ${options.scriptPath} (${suffix})\n${recentOutput}`
      );
      (
        error as Error & {
          details?: {
            scriptPath: string;
            exitCode: number | null;
            signal: NodeJS.Signals | null;
            recentOutput: string | null;
          };
        }
      ).details = {
        scriptPath: options.scriptPath,
        exitCode: code,
        signal,
        recentOutput: recentOutput === "" ? null : recentOutput
      };
      reject(error);
    });
  });
}

function normalizeOptionalString(value: unknown) {
  if (typeof value !== "string") {
    return null;
  }

  const normalized = value.trim();
  return normalized === "" ? null : normalized;
}

export function resolveUsdcAddressFromDeployment(deployment: {
  usdcAddress?: string;
  usdcTokenAddress?: string;
  mockUsdcAddress?: string;
}) {
  return (
    normalizeOptionalString(deployment.usdcAddress) ??
    normalizeOptionalString(deployment.usdcTokenAddress) ??
    normalizeOptionalString(deployment.mockUsdcAddress) ??
    ""
  );
}

export class PlaygroundProcessManager {
  constructor(
    private readonly repoRoot: string,
    private readonly env: NodeJS.ProcessEnv = process.env
  ) {}

  async bootstrapPlayground() {
    await runScriptStreaming({
      cwd: this.repoRoot,
      env: this.env,
      scriptPath: join(this.repoRoot, "scripts/playground-bootstrap.sh")
    });
  }

  async stopPlayground() {
    await runScriptStreaming({
      cwd: this.repoRoot,
      env: this.env,
      scriptPath: join(this.repoRoot, "scripts/playground-stop.sh")
    });
  }

  async resolveRuntime(): Promise<PlaygroundRuntime> {
    const helpers = await loadPlaygroundHelpers(this.repoRoot);
    const runtime = await helpers.resolvePlaygroundRuntime(this.repoRoot, this.env);

    return {
      indexerBaseUrl: this.env.PLAYGROUND_INDEXER_BASE_URL?.trim() ?? "http://127.0.0.1:3001",
      paymentRpcUrl: runtime.activeRpcUrl,
      chainId: runtime.gatewayChainId,
      usdcAddress: resolveUsdcAddressFromDeployment(runtime.deployment)
    };
  }

  async readGitCommit(cwd = this.repoRoot) {
    const { stdout } = await execFileAsync("git", ["rev-parse", "HEAD"], {
      cwd
    });

    return stdout.trim();
  }
}

export interface PlaygroundProcessManagerLike {
  bootstrapPlayground(): Promise<void>;
  stopPlayground(): Promise<void>;
  resolveRuntime(): Promise<PlaygroundRuntime>;
  readGitCommit(cwd?: string): Promise<string>;
}
