import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

import { afterEach, describe, expect, it } from "vitest";

import { ArtifactStore } from "../src/lib/files.js";
import { buildServer } from "../src/server.js";
import type { EvaluatorRuntimeEnv } from "../src/types.js";

const tempPaths: string[] = [];
const openApps = new Set<ReturnType<typeof buildServer>>();

afterEach(async () => {
  await Promise.all(
    [...openApps].map(async (app) => {
      openApps.delete(app);
      await app.close();
    })
  );

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

function createRuntimeEnv(): EvaluatorRuntimeEnv {
  return {
    stewardAddress: "0x0000000000000000000000000000000000000001",
    openRouterApiKey: "test-openrouter",
    braveSearchApiKey: null,
    localEvmForkUrl: "https://example.invalid/base-fork",
    automatonRepoPath: "/tmp/ic-automaton"
  };
}

function trackApp(app: ReturnType<typeof buildServer>) {
  openApps.add(app);
  return app;
}

describe("evaluator server", () => {
  it("serves health and empty-run routes without an active evaluation", async () => {
    const repoRoot = await createTempDirectory("evaluator-server-repo-");
    const artifactsRoot = await createTempDirectory("evaluator-server-artifacts-");
    const artifactStore = new ArtifactStore(artifactsRoot);
    const app = trackApp(
      buildServer({
        config: {
          repoRoot,
          artifactsRoot,
          experimentPath: null
        },
        runtimeEnv: createRuntimeEnv(),
        artifactStore,
        processManager: {
          async bootstrapPlayground() {},
          async stopPlayground() {},
          async resolveRuntime() {
            return {
              indexerBaseUrl: "http://127.0.0.1:3001",
              paymentRpcUrl: "http://127.0.0.1:3002",
              chainId: 8453,
              usdcAddress: "0x00000000000000000000000000000000000000aa"
            };
          },
          async readGitCommit() {
            return "0123456789abcdef0123456789abcdef01234567";
          }
        },
        indexerClient: {
          async fetchRepositoryStrategies() {
            return {
              items: [],
              updatedAt: 0
            } as const;
          },
          async createSpawnSession() {
            throw new Error("not used");
          },
          async fetchSpawnSession() {
            throw new Error("not used");
          },
          async fetchAutomatonDetail() {
            throw new Error("not used");
          },
          async fetchRoomMessages() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            } as const;
          }
        },
        automatonClient: {
          async readEvidence() {
            throw new Error("not used");
          }
        },
        evmClient: {
          async observeAddress() {
            return {
              ethBalanceWei: null,
              usdcBalanceRaw: null,
              txCount: null
            };
          }
        }
      })
    );

    const health = await app.inject({
      method: "GET",
      url: "/health"
    });
    const currentRun = await app.inject({
      method: "GET",
      url: "/api/run"
    });
    const stop = await app.inject({
      method: "POST",
      url: "/api/run/stop"
    });
    const websocketUpgrade = await app.inject({
      method: "GET",
      url: "/ws/events"
    });

    expect(health.statusCode).toBe(200);
    expect(health.json()).toMatchObject({
      ok: true,
      service: "evaluator",
      run: null,
      realtime: {
        websocketPath: "/ws/events"
      }
    });

    expect(currentRun.statusCode).toBe(404);
    expect(stop.statusCode).toBe(200);
    expect(stop.json()).toMatchObject({
      ok: true,
      accepted: false,
      run: null
    });

    expect(websocketUpgrade.statusCode).toBe(426);
    expect(websocketUpgrade.json()).toMatchObject({
      ok: false,
      error: "Upgrade Required"
    });
  });

  it("allows dashboard cross-origin requests from the local web host", async () => {
    const repoRoot = await createTempDirectory("evaluator-server-repo-cors-");
    const artifactsRoot = await createTempDirectory("evaluator-server-artifacts-cors-");
    const artifactStore = new ArtifactStore(artifactsRoot);
    const app = trackApp(
      buildServer({
        config: {
          repoRoot,
          artifactsRoot,
          experimentPath: null
        },
        runtimeEnv: createRuntimeEnv(),
        artifactStore,
        processManager: {
          async bootstrapPlayground() {},
          async stopPlayground() {},
          async resolveRuntime() {
            return {
              indexerBaseUrl: "http://127.0.0.1:3001",
              paymentRpcUrl: "http://127.0.0.1:3002",
              chainId: 8453,
              usdcAddress: "0x00000000000000000000000000000000000000aa"
            };
          },
          async readGitCommit() {
            return "0123456789abcdef0123456789abcdef01234567";
          }
        },
        indexerClient: {
          async fetchRepositoryStrategies() {
            return {
              items: [],
              updatedAt: 0
            } as const;
          },
          async createSpawnSession() {
            throw new Error("not used");
          },
          async fetchSpawnSession() {
            throw new Error("not used");
          },
          async fetchAutomatonDetail() {
            throw new Error("not used");
          },
          async fetchRoomMessages() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            } as const;
          }
        },
        automatonClient: {
          async readEvidence() {
            throw new Error("not used");
          }
        },
        evmClient: {
          async observeAddress() {
            return {
              ethBalanceWei: null,
              usdcBalanceRaw: null,
              txCount: null
            };
          }
        }
      })
    );

    const origin = "http://127.0.0.1:4173";
    const currentRun = await app.inject({
      method: "GET",
      url: "/api/run",
      headers: {
        origin
      }
    });
    const preflight = await app.inject({
      method: "OPTIONS",
      url: "/api/run",
      headers: {
        origin
      }
    });

    expect(currentRun.statusCode).toBe(404);
    expect(currentRun.headers["access-control-allow-origin"]).toBe(origin);
    expect(currentRun.headers["access-control-allow-methods"]).toBe("GET,POST,OPTIONS");
    expect(currentRun.headers["access-control-allow-headers"]).toBe("accept,content-type");
    expect(currentRun.headers.vary).toBe("Origin");

    expect(preflight.statusCode).toBe(204);
    expect(preflight.body).toBe("");
    expect(preflight.headers["access-control-allow-origin"]).toBe(origin);
  });

  it("lists historical runs from the artifact store", async () => {
    const repoRoot = await createTempDirectory("evaluator-server-repo-history-");
    const artifactsRoot = await createTempDirectory("evaluator-server-artifacts-history-");
    const artifactStore = new ArtifactStore(artifactsRoot);
    const first = await artifactStore.createRunArtifacts("run-1");
    const second = await artifactStore.createRunArtifacts("run-2");

    await artifactStore.writeManifest(first, {
      run: {
        runId: "run-1",
        experimentPath: "evaluations/experiments/smoke.yaml",
        experimentHash: "abc",
        runState: "completed",
        abortReason: null,
        startedAt: 1,
        endedAt: 2,
        launchpadCommit: "0123456789abcdef0123456789abcdef01234567",
        childCommit: null,
        requestedAutomatonCount: 1,
        successfulSpawnCount: 1,
        samplingIntervalSeconds: 15,
        maxRuntimeMinutes: 240
      },
      completionReason: "stopped_manually",
      comparisonValid: true
    });
    await artifactStore.writeManifest(second, {
      run: {
        runId: "run-2",
        experimentPath: "evaluations/experiments/smoke.yaml",
        experimentHash: "def",
        runState: "aborted",
        abortReason: "failed",
        startedAt: 10,
        endedAt: 11,
        launchpadCommit: "fedcba9876543210fedcba9876543210fedcba98",
        childCommit: null,
        requestedAutomatonCount: 2,
        successfulSpawnCount: 1,
        samplingIntervalSeconds: 15,
        maxRuntimeMinutes: 240
      },
      completionReason: "aborted",
      comparisonValid: false
    });

    const app = trackApp(
      buildServer({
        config: {
          repoRoot,
          artifactsRoot,
          experimentPath: null
        },
        runtimeEnv: createRuntimeEnv(),
        artifactStore,
        processManager: {
          async bootstrapPlayground() {},
          async stopPlayground() {},
          async resolveRuntime() {
            return {
              indexerBaseUrl: "http://127.0.0.1:3001",
              paymentRpcUrl: "http://127.0.0.1:3002",
              chainId: 8453,
              usdcAddress: "0x00000000000000000000000000000000000000aa"
            };
          },
          async readGitCommit() {
            return "0123456789abcdef0123456789abcdef01234567";
          }
        },
        indexerClient: {
          async fetchRepositoryStrategies() {
            return {
              items: [],
              updatedAt: 0
            } as const;
          },
          async createSpawnSession() {
            throw new Error("not used");
          },
          async fetchSpawnSession() {
            throw new Error("not used");
          },
          async fetchAutomatonDetail() {
            throw new Error("not used");
          },
          async fetchRoomMessages() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            } as const;
          }
        },
        automatonClient: {
          async readEvidence() {
            throw new Error("not used");
          }
        },
        evmClient: {
          async observeAddress() {
            return {
              ethBalanceWei: null,
              usdcBalanceRaw: null,
              txCount: null
            };
          }
        }
      })
    );

    const history = await app.inject({
      method: "GET",
      url: "/api/runs"
    });

    expect(history.statusCode).toBe(200);
    expect(history.json()).toEqual({
      runs: [
        {
          runId: "run-2",
          experimentPath: "evaluations/experiments/smoke.yaml",
          startedAt: 10,
          endedAt: 11,
          runState: "aborted",
          completionReason: "aborted"
        },
        {
          runId: "run-1",
          experimentPath: "evaluations/experiments/smoke.yaml",
          startedAt: 1,
          endedAt: 2,
          runState: "completed",
          completionReason: "stopped_manually"
        }
      ]
    });
  });

  it("serves requests while a configured run is still booting", async () => {
    const repoRoot = await createTempDirectory("evaluator-server-repo-boot-");
    const artifactsRoot = await createTempDirectory("evaluator-server-artifacts-boot-");
    let releaseStart = () => {};
    let startCalls = 0;

    const app = trackApp(
      buildServer({
        config: {
          repoRoot,
          artifactsRoot,
          experimentPath: "evaluations/experiments/smoke.yaml"
        },
        runtimeEnv: createRuntimeEnv(),
        runController: {
          async startConfiguredRun() {
            startCalls += 1;
            await new Promise<void>((resolve) => {
              releaseStart = resolve;
            });
            return "run-booting";
          },
          getDashboard() {
            return {
              run: {
                runId: "run-booting",
                experimentPath: "evaluations/experiments/smoke.yaml",
                experimentHash: "abc",
                runState: "booting",
                abortReason: null,
                startedAt: 1,
                endedAt: null,
                launchpadCommit: "0123456789abcdef0123456789abcdef01234567",
                childCommit: null,
                requestedAutomatonCount: 1,
                successfulSpawnCount: 0,
                samplingIntervalSeconds: 15,
                maxRuntimeMinutes: 240
              },
              report: null,
              fleet: {
                requestedSpawns: 1,
                successfulSpawns: 0,
                stalledAutomatons: 0,
                activeAutomatons: 0,
                totalTurns: 0,
                totalToolCalls: 0,
                totalErrors: 0,
                totalNetWorthUsdDelta: null,
                totalCyclesConsumed: null
              },
              automatons: []
            };
          },
          async requestStop() {
            return true;
          },
          async listHistoricalRuns() {
            return [];
          },
          async close() {},
          async waitForCompletion() {}
        } as any
      })
    );

    const health = await app.inject({
      method: "GET",
      url: "/health"
    });

    expect(health.statusCode).toBe(200);
    expect(startCalls).toBe(1);
    expect(health.json()).toMatchObject({
      ok: true,
      service: "evaluator",
      run: {
        runId: "run-booting",
        runState: "booting"
      }
    });

    releaseStart();
    await delay(0);
  });
});
