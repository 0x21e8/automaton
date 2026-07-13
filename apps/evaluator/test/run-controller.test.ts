import Fastify from "fastify";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import type { AutomatonClientLike } from "../src/lib/automaton-client.js";
import { ArtifactStore } from "../src/lib/files.js";
import {
  buildEvaluationGenesis,
  RunController
} from "../src/runtime/run-controller.js";
import type { EvaluatorRuntimeEnv } from "../src/types.js";
import { EvaluationEventHub } from "../src/ws/events.js";

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

async function writeExperiment(
  directory: string,
  yaml: string,
  filename = "experiment.yaml"
) {
  const filePath = join(directory, filename);
  await writeFile(filePath, yaml, "utf8");
  return filePath;
}

function createRuntimeEnv(): EvaluatorRuntimeEnv {
  return {
    stewardAddress: "0x0000000000000000000000000000000000000001",
    openRouterApiKey: "test-openrouter",
    braveSearchApiKey: null,
    inferenceProxyWorkerBaseUrl: "https://proxy.example.com",
    inferenceProxyTrustedCallbackPrincipal: "aaaaa-aa",
    localEvmForkUrl: "https://example.invalid/base-fork",
    automatonRepoPath: "/tmp/ic-automaton"
  };
}

function createController(options: {
  repoRoot: string;
  artifactsRoot: string;
  experimentPath: string | null;
  env?: Partial<EvaluatorRuntimeEnv>;
  onCreateSpawnSession?: (body: unknown) => void;
  loadPlaygroundHelpers: () => Promise<{
    claimPlaygroundFaucet(indexerBaseUrl: string, walletAddress: string): Promise<unknown>;
    createEphemeralWallet(rootDir: string): {
      address: string;
      privateKey: string;
    };
    submitSpawnPayment(options: unknown): Promise<unknown>;
    waitForSessionCompletion(indexerBaseUrl: string, sessionId: string): Promise<{
      registryRecord?: {
        canisterId?: string | null;
        evmAddress?: string | null;
        versionCommit?: string | null;
      } | null;
    }>;
  }>;
  automatonClient?: AutomatonClientLike;
  sleep?: (ms: number) => Promise<void>;
}) {
  const loggerApp = Fastify({ logger: false });
  const artifacts = new ArtifactStore(options.artifactsRoot);

  return {
    loggerApp,
    artifacts,
    controller: new RunController({
      logger: loggerApp.log,
      config: {
        host: "127.0.0.1",
        port: 3003,
        websocketPath: "/ws/events",
        repoRoot: options.repoRoot,
        artifactsRoot: options.artifactsRoot,
        experimentPath: options.experimentPath,
        indexerBaseUrl: "http://127.0.0.1:3001",
        rpcGatewayUrl: "http://127.0.0.1:3002",
        localReplicaHost: "127.0.0.1",
        localReplicaPort: 8000
      },
      env: {
        ...createRuntimeEnv(),
        ...options.env
      },
      artifacts,
      processes: {
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
            items: [
              {
                strategyId: "momentum",
                name: "Momentum",
                description: "Momentum strategy",
                canonicalChain: "base",
                canonicalChainId: 8453,
                compatibleSpawnChains: ["base"],
                protocol: "uniswap",
                primitive: "swap",
                recipeJson: "{}",
                status: "active",
                source: {
                  sourcePath: "strategy-seeds/momentum.yaml",
                  sourceCommit: "0123456789abcdef0123456789abcdef01234567"
                },
                createdAt: 1,
                updatedAt: 1,
                deprecatedAt: null,
                revokedAt: null
              }
            ],
            updatedAt: 1
          } as const;
        },
        async createSpawnSession(body) {
          options.onCreateSpawnSession?.(body);
          const sessionId = `session-${body.config.provider.model}`;
          return {
            session: {
              sessionId,
              claimId: `claim-${sessionId}`,
              stewardAddress: body.stewardAddress,
              chain: "base",
              asset: "usdc",
              grossAmount: body.grossAmount,
              platformFee: "0",
              creationCost: body.grossAmount,
              netForwardAmount: body.grossAmount,
              quoteTermsHash: "0x1234",
              expiresAt: 60_000,
              state: "awaiting_payment",
              retryable: false,
              refundable: false,
              paymentStatus: "unpaid",
              automatonCanisterId: null,
              automatonEvmAddress: null,
              releaseTxHash: null,
              releaseBroadcastAt: null,
              parentId: null,
              childIds: [],
              selectedStrategies: [],
              config: body.config
            } as const,
            quote: {
              sessionId,
              chain: "base",
              asset: "usdc",
              grossAmount: body.grossAmount,
              platformFee: "0",
              creationCost: body.grossAmount,
              netForwardAmount: body.grossAmount,
              quoteTermsHash: "0x1234",
              expiresAt: 60_000,
              payment: {
                sessionId,
                claimId: `claim-${sessionId}`,
                chain: "base",
                asset: "usdc",
                paymentAddress: "0x00000000000000000000000000000000000000bb",
                grossAmount: body.grossAmount,
                quoteTermsHash: "0x1234",
                expiresAt: 60_000
              }
            },
          } as any;
        },
        async fetchSpawnSession(sessionId) {
          return {
            session: {
              sessionId,
              claimId: `claim-${sessionId}`,
              stewardAddress: "0x0000000000000000000000000000000000000001",
              chain: "base",
              asset: "usdc",
              grossAmount: "75000000",
              platformFee: "0",
              creationCost: "75000000",
              netForwardAmount: "75000000",
              quoteTermsHash: "0x1234",
              expiresAt: 60_000,
              state: "awaiting_payment",
              retryable: false,
              refundable: false,
              paymentStatus: "unpaid"
            } as const,
            payment: {
              sessionId,
              claimId: `claim-${sessionId}`,
              chain: "base",
              asset: "usdc",
              paymentAddress: "0x00000000000000000000000000000000000000bb",
              grossAmount: "75000000",
              quoteTermsHash: "0x1234",
              expiresAt: 60_000
            },
            audit: [],
            registryRecord: null
          } as any;
        },
        async fetchAutomatonDetail(canisterId) {
          return {
            canisterId,
            ethAddress: "0x0000000000000000000000000000000000000aaa",
            chain: "base",
            chainId: 8453,
            name: canisterId,
            tier: "normal",
            agentState: "running",
            ethBalanceWei: "200",
            usdcBalanceRaw: "300",
            cyclesBalance: 900,
            netWorthEth: "0",
            netWorthUsd: canisterId === "canister-alpha" ? "110" : "90",
            heartbeatIntervalSeconds: 60,
            steward: {
              address: "0x0000000000000000000000000000000000000001",
              chainId: 8453,
              ensName: null,
              enabled: true
            },
            gridPosition: { x: 0, y: 0 },
            corePatternIndex: 0,
            corePattern: null,
            parentId: null,
            createdAt: 1,
            lastTransitionAt: 1,
            soul: "test",
            canisterUrl: "http://127.0.0.1:8000/?canisterId=test",
            explorerUrl: null,
            model: "test-model",
            financials: {
              ethBalanceWei: "200",
              usdcBalanceRaw: "300",
              cyclesBalance: 900,
              liquidCycles: 900,
              burnRatePerDay: null,
              estimatedFreezeTime: null,
              netWorthEth: "0",
              netWorthUsd: canisterId === "canister-alpha" ? "110" : "90"
            },
            runtime: {
              agentState: "running",
              loopEnabled: true,
              lastTransitionAt: 1,
              lastError: null
            },
            version: {
              commitHash: "89abcdef0123456789abcdef0123456789abcdef",
              shortCommitHash: "89abcdef"
            },
            strategies: [],
            skills: [],
            promptLayers: [],
            monologue: [],
            childIds: [],
            lastPolledAt: 1
          } as any;
        },
        async fetchRoomMessages() {
          return {
            messages: [],
            nextAfterSeq: null,
            latestSeq: null
          };
        }
      },
      automatonClient: options.automatonClient ?? {
        async readEvidence(canisterId) {
          return {
            buildInfo: {
              commit: "89abcdef0123456789abcdef0123456789abcdef"
            },
            evmConfig: {
              automaton_address:
                canisterId === "canister-alpha"
                  ? "0x0000000000000000000000000000000000000aaa"
                  : "0x0000000000000000000000000000000000000bbb"
            },
            inferenceConfig: {
              provider: "OpenRouter",
              model: "test-model",
              reasoning_level: "default"
            },
            inferenceProxyStatus: null,
            snapshot: {
              cycles: {
                total_cycles: canisterId === "canister-alpha" ? 900 : 850
              }
            },
            walletBalance: {
              usdc_contract_address: "0x00000000000000000000000000000000000000aa"
            },
            recentTurns: [
              {
                id: `${canisterId}-turn-1`,
                tool_call_count: 2,
                created_at_ns: 5_000_000_000
              }
            ]
          };
        }
      } satisfies AutomatonClientLike,
      evmClient: {
        async observeAddress(address) {
          return {
            ethBalanceWei: address?.endsWith("aaa") ? "200" : "150",
            usdcBalanceRaw: address?.endsWith("aaa") ? "300" : "250",
            txCount: address?.endsWith("aaa") ? 2 : 1
          };
        }
      },
      events: new EvaluationEventHub("/ws/events"),
      loadPlaygroundHelpers: options.loadPlaygroundHelpers,
      now: (() => {
        let current = 0;
        return () => {
          current += 1_000;
          return current;
        };
      })(),
      sleep: options.sleep
    })
  };
}

describe("run controller", () => {
  it("authors deterministic, principal-specific Genesis documents for evaluator births", () => {
    const config = {
      id: "alpha-principal",
      label: "Alpha",
      model: "model-alpha",
      transport: "openrouter_direct" as const,
      reasoningLevel: "default" as const,
      strategies: []
    };

    const first = buildEvaluationGenesis(config);
    const second = buildEvaluationGenesis(config);

    expect(second).toEqual(first);
    expect(first.name).toContain("alpha-principal");
    expect(first.constitution).toContain("principal identified as alpha-principal");
    expect([...first.constitution].length).toBeGreaterThanOrEqual(400);
  });

  it("aborts and writes artifacts when the spawn threshold becomes unreachable", async () => {
    const repoRoot = await createTempDirectory("evaluator-controller-repo-");
    const artifactsRoot = await createTempDirectory("evaluator-controller-artifacts-");
    const experimentPath = await writeExperiment(
      repoRoot,
      [
        "name: smoke",
        "description: abort scenario",
        "maxRuntimeMinutes: 240",
        "samplingIntervalSeconds: 15",
        "stallAfterMinutes: 10",
        "spawn:",
        "  grossAmount: \"75000000\"",
        "  minSuccessRatio: 1",
        "automatons:",
        "  - id: alpha",
        "    label: Alpha",
        "    model: model-alpha",
        "    strategies:",
        "      - momentum",
        "  - id: beta",
        "    label: Beta",
        "    model: model-beta",
        "    strategies:",
        "      - momentum"
      ].join("\n")
    );

    const { controller, loggerApp } = createController({
      repoRoot,
      artifactsRoot,
      experimentPath,
      loadPlaygroundHelpers: async () => ({
        async claimPlaygroundFaucet() {},
        createEphemeralWallet() {
          return {
            address: "0x0000000000000000000000000000000000000aaa",
            privateKey: "0x01"
          };
        },
        async submitSpawnPayment() {},
        async waitForSessionCompletion(_indexerBaseUrl, sessionId) {
          if (sessionId === "session-model-beta") {
            const error = new Error("spawn session failed");
            (
              error as Error & {
                details?: unknown;
              }
            ).details = {
              session: {
                sessionId,
                state: "spawning",
                paymentStatus: "paid"
              },
              audit: [
                {
                  reason: "child bootstrap verification still pending"
                }
              ]
            };
            throw error;
          }

          return {
            registryRecord: {
              canisterId: "canister-alpha",
              evmAddress: "0x0000000000000000000000000000000000000aaa",
              versionCommit: "89abcdef0123456789abcdef0123456789abcdef"
            }
          };
        }
      })
    });

    try {
      await controller.startRun(experimentPath);
      await controller.waitForCompletion();

      const dashboard = controller.getDashboard();
      expect(dashboard?.run.runState).toBe("aborted");
      expect(dashboard?.run.successfulSpawnCount).toBe(1);
      expect(dashboard?.report?.comparisonValid).toBe(false);
      expect(dashboard?.run.abortReason).toContain("Spawn success threshold missed");
      expect(dashboard?.run.abortReason).toContain("beta: spawn session failed");

      const runDirectory = join(artifactsRoot, dashboard?.run.runId ?? "");
      const manifest = JSON.parse(await readFile(join(runDirectory, "manifest.json"), "utf8")) as {
        completionReason: string;
        comparisonValid: boolean;
      };
      const summary = JSON.parse(await readFile(join(runDirectory, "summary.json"), "utf8")) as {
        automatonResults: Array<{
          errorHistogram?: Array<{
            source: string;
            count: number;
          }>;
          lastErrorDetails?: unknown;
        }>;
      };
      const report = await readFile(join(runDirectory, "report.md"), "utf8");
      const events = await readFile(join(runDirectory, "events.ndjson"), "utf8");
      const sampleLines = await readFile(join(runDirectory, "samples/alpha.jsonl"), "utf8");

      expect(manifest.completionReason).toBe("aborted");
      expect(manifest.comparisonValid).toBe(false);
      expect(summary.automatonResults).toHaveLength(2);
      expect(summary.automatonResults[1]?.errorHistogram).toEqual([
        expect.objectContaining({
          source: "spawn",
          count: 1
        })
      ]);
      expect(summary.automatonResults[1]?.lastErrorDetails).toEqual({
        session: {
          sessionId: "session-model-beta",
          state: "spawning",
          paymentStatus: "paid"
        },
        audit: [
          {
            reason: "child bootstrap verification still pending"
          }
        ]
      });
      expect(report).toContain("## Rankings");
      expect(events.trim().split("\n").length).toBeGreaterThanOrEqual(4);
      expect(events).toContain("\"lastErrorDetails\":{\"session\":{\"sessionId\":\"session-model-beta\"");
      expect(sampleLines.trim().split("\n")).toHaveLength(1);
    } finally {
      await loggerApp.close();
    }
  });

  it("continues attempting the fleet before aborting on an unmet spawn threshold", async () => {
    const repoRoot = await createTempDirectory("evaluator-controller-repo-fleet-");
    const artifactsRoot = await createTempDirectory("evaluator-controller-artifacts-fleet-");
    const experimentPath = await writeExperiment(
      repoRoot,
      [
        "name: smoke",
        "description: full fleet attempt scenario",
        "maxRuntimeMinutes: 240",
        "samplingIntervalSeconds: 15",
        "stallAfterMinutes: 10",
        "spawn:",
        "  grossAmount: \"75000000\"",
        "  minSuccessRatio: 0.8",
        "automatons:",
        "  - id: alpha",
        "    label: Alpha",
        "    model: model-alpha",
        "    strategies:",
        "      - momentum",
        "  - id: beta",
        "    label: Beta",
        "    model: model-beta",
        "    strategies:",
        "      - momentum",
        "  - id: gamma",
        "    label: Gamma",
        "    model: model-gamma",
        "    strategies:",
        "      - momentum"
      ].join("\n")
    );

    const attemptedModels: string[] = [];

    const { controller, loggerApp } = createController({
      repoRoot,
      artifactsRoot,
      experimentPath,
      loadPlaygroundHelpers: async () => ({
        async claimPlaygroundFaucet() {},
        createEphemeralWallet() {
          return {
            address: "0x0000000000000000000000000000000000000aaa",
            privateKey: "0x01"
          };
        },
        async submitSpawnPayment() {},
        async waitForSessionCompletion(_indexerBaseUrl, sessionId) {
          attemptedModels.push(sessionId.replace("session-", ""));
          throw new Error(`spawn failed for ${sessionId}`);
        }
      })
    });

    try {
      await controller.startRun(experimentPath);
      await controller.waitForCompletion();

      const dashboard = controller.getDashboard();
      expect(dashboard?.run.runState).toBe("aborted");
      expect(attemptedModels).toEqual(["model-alpha", "model-beta", "model-gamma"]);
      expect(dashboard?.run.abortReason).toContain("alpha: spawn failed for session-model-alpha");
      expect(dashboard?.run.abortReason).toContain("beta: spawn failed for session-model-beta");
      expect(dashboard?.run.abortReason).toContain("gamma: spawn failed for session-model-gamma");
    } finally {
      await loggerApp.close();
    }
  });

  it("keeps successful spawns active when baseline capture fails", async () => {
    const repoRoot = await createTempDirectory("evaluator-controller-repo-baseline-");
    const artifactsRoot = await createTempDirectory("evaluator-controller-artifacts-baseline-");
    const experimentPath = await writeExperiment(
      repoRoot,
      [
        "name: smoke",
        "description: baseline failure scenario",
        "maxRuntimeMinutes: 240",
        "samplingIntervalSeconds: 15",
        "stallAfterMinutes: 10",
        "spawn:",
        "  grossAmount: \"75000000\"",
        "  minSuccessRatio: 1",
        "automatons:",
        "  - id: alpha",
        "    label: Alpha",
        "    model: model-alpha",
        "    strategies:",
        "      - momentum"
      ].join("\n")
    );

    const { controller, loggerApp } = createController({
      repoRoot,
      artifactsRoot,
      experimentPath,
      loadPlaygroundHelpers: async () => ({
        async claimPlaygroundFaucet() {},
        createEphemeralWallet() {
          return {
            address: "0x0000000000000000000000000000000000000aaa",
            privateKey: "0x01"
          };
        },
        async submitSpawnPayment() {},
        async waitForSessionCompletion() {
          return {
            registryRecord: {
              canisterId: "canister-alpha",
              evmAddress: "0x0000000000000000000000000000000000000aaa",
              versionCommit: "89abcdef0123456789abcdef0123456789abcdef"
            }
          };
        }
      }),
      automatonClient: {
        async readEvidence() {
          throw new Error("baseline capture failed");
        }
      },
      sleep: async () => {}
    });

    try {
      await controller.startRun(experimentPath);

      for (let attempt = 0; attempt < 20; attempt += 1) {
        if (controller.getDashboard()?.run.successfulSpawnCount === 1) {
          break;
        }

        await new Promise<void>((resolve) => {
          setTimeout(resolve, 0);
        });
      }

      await controller.requestStop();
      await controller.waitForCompletion();

      const dashboard = controller.getDashboard();
      expect(dashboard?.run.successfulSpawnCount).toBe(1);
      expect(dashboard?.automatons[0]?.spawnStatus).toBe("active");
      expect(dashboard?.automatons[0]?.lastError).toContain("baseline capture failed");
      expect(dashboard?.automatons[0]?.errorHistogram[0]).toMatchObject({
        source: "sampling"
      });
      expect(dashboard?.automatons[0]?.errorHistogram[0]?.message).toContain(
        "baseline capture failed"
      );
      expect(dashboard?.automatons[0]?.errorHistogram[0]?.count).toBeGreaterThanOrEqual(1);
    } finally {
      await loggerApp.close();
    }
  });

  it("surfaces scheduler tick failures in automaton lastError", async () => {
    const repoRoot = await createTempDirectory("evaluator-controller-repo-scheduler-error-");
    const artifactsRoot = await createTempDirectory("evaluator-controller-artifacts-scheduler-error-");
    const experimentPath = await writeExperiment(
      repoRoot,
      [
        "name: smoke",
        "description: scheduler error scenario",
        "maxRuntimeMinutes: 240",
        "samplingIntervalSeconds: 15",
        "stallAfterMinutes: 10",
        "spawn:",
        "  grossAmount: \"75000000\"",
        "  minSuccessRatio: 1",
        "automatons:",
        "  - id: alpha",
        "    label: Alpha",
        "    model: model-alpha",
        "    strategies:",
        "      - momentum"
      ].join("\n")
    );

    const schedulerError =
      "autonomy inference error: openrouter returned status 429: provider rate-limited";

    let resumeSleep = () => {};
    let notifySleep: (() => void) | null = null;
    const sleepStarted = new Promise<void>((resolve) => {
      notifySleep = resolve;
    });

    const { controller, loggerApp } = createController({
      repoRoot,
      artifactsRoot,
      experimentPath,
      loadPlaygroundHelpers: async () => ({
        async claimPlaygroundFaucet() {},
        createEphemeralWallet() {
          return {
            address: "0x0000000000000000000000000000000000000aaa",
            privateKey: "0x01"
          };
        },
        async submitSpawnPayment() {},
        async waitForSessionCompletion() {
          return {
            registryRecord: {
              canisterId: "canister-alpha",
              evmAddress: "0x0000000000000000000000000000000000000aaa",
              versionCommit: "89abcdef0123456789abcdef0123456789abcdef"
            }
          };
        }
      }),
      automatonClient: {
        async readEvidence() {
          return {
            buildInfo: {
              commit: "89abcdef0123456789abcdef0123456789abcdef"
            },
            evmConfig: {
              automaton_address: "0x0000000000000000000000000000000000000aaa"
            },
            inferenceConfig: {
              provider: "OpenRouter",
              model: "model-alpha",
              reasoning_level: "default"
            },
            inferenceProxyStatus: null,
            snapshot: {
              cycles: {
                total_cycles: 900
              },
              runtime: {
                last_error: null
              },
              scheduler: {
                last_tick_error: schedulerError
              }
            },
            walletBalance: {
              usdc_contract_address: "0x00000000000000000000000000000000000000aa"
            },
            recentTurns: [
              {
                id: "canister-alpha-turn-1",
                tool_call_count: 0,
                created_at_ns: 5_000_000_000
              }
            ]
          };
        }
      },
      sleep: async () =>
        await new Promise<void>((resolve) => {
          notifySleep?.();
          resumeSleep = resolve;
        })
    });

    try {
      await controller.startRun(experimentPath);
      await sleepStarted;
      await controller.requestStop();
      resumeSleep();
      await controller.waitForCompletion();

      const dashboard = controller.getDashboard();
      expect(dashboard?.automatons[0]?.spawnStatus).toBe("active");
      expect(dashboard?.automatons[0]?.lastError).toBe(schedulerError);
      expect(dashboard?.automatons[0]?.errorHistogram).toEqual([
        expect.objectContaining({
          source: "scheduler",
          message: schedulerError,
          count: 1
        })
      ]);
    } finally {
      await loggerApp.close();
    }
  });

  it("maps proxy transport and reasoning into the spawn provider config", async () => {
    const repoRoot = await createTempDirectory("evaluator-controller-repo-proxy-config-");
    const artifactsRoot = await createTempDirectory("evaluator-controller-artifacts-proxy-config-");
    const experimentPath = await writeExperiment(
      repoRoot,
      [
        "name: smoke",
        "description: proxy config mapping",
        "maxRuntimeMinutes: 1",
        "samplingIntervalSeconds: 15",
        "stallAfterMinutes: 10",
        "spawn:",
        "  grossAmount: \"75000000\"",
        "  minSuccessRatio: 1",
        "automatons:",
        "  - id: alpha",
        "    label: Alpha",
        "    model: model-alpha",
        "    transport: openrouter_proxy_worker",
        "    reasoningLevel: medium",
        "    strategies:",
        "      - momentum"
      ].join("\n")
    );

    const createdRequests: Array<Record<string, any>> = [];

    const { controller, loggerApp } = createController({
      repoRoot,
      artifactsRoot,
      experimentPath,
      onCreateSpawnSession(body) {
        createdRequests.push(body as Record<string, any>);
      },
      loadPlaygroundHelpers: async () => ({
        async claimPlaygroundFaucet() {},
        createEphemeralWallet() {
          return {
            address: "0x0000000000000000000000000000000000000aaa",
            privateKey: "0x01"
          };
        },
        async submitSpawnPayment() {},
        async waitForSessionCompletion() {
          return {
            registryRecord: {
              canisterId: "canister-alpha",
              evmAddress: "0x0000000000000000000000000000000000000aaa",
              versionCommit: "89abcdef0123456789abcdef0123456789abcdef"
            }
          };
        }
      }),
      sleep: async () => {}
    });

    try {
      await controller.startRun(experimentPath);
      await controller.waitForCompletion();

      expect(createdRequests).toHaveLength(1);
      expect(createdRequests[0]?.config.provider).toMatchObject({
        model: "model-alpha",
        inferenceTransport: "openrouter_proxy_worker",
        openRouterReasoningLevel: "medium"
      });
      expect(createdRequests[0]?.providerSecrets).toEqual({
        openRouterApiKey: "test-openrouter",
        braveSearchApiKey: null
      });
    } finally {
      await loggerApp.close();
    }
  });

  it("fails before spawning when proxy transport is requested without proxy runtime config", async () => {
    const repoRoot = await createTempDirectory("evaluator-controller-repo-proxy-missing-env-");
    const artifactsRoot = await createTempDirectory("evaluator-controller-artifacts-proxy-missing-env-");
    const experimentPath = await writeExperiment(
      repoRoot,
      [
        "name: smoke",
        "description: proxy missing env",
        "maxRuntimeMinutes: 1",
        "samplingIntervalSeconds: 15",
        "stallAfterMinutes: 10",
        "spawn:",
        "  grossAmount: \"75000000\"",
        "  minSuccessRatio: 1",
        "automatons:",
        "  - id: alpha",
        "    label: Alpha",
        "    model: model-alpha",
        "    transport: openrouter_proxy_worker",
        "    strategies:",
        "      - momentum"
      ].join("\n")
    );

    const { controller, loggerApp } = createController({
      repoRoot,
      artifactsRoot,
      experimentPath,
      env: {
        inferenceProxyWorkerBaseUrl: null,
        inferenceProxyTrustedCallbackPrincipal: null
      },
      loadPlaygroundHelpers: async () => ({
        async claimPlaygroundFaucet() {},
        createEphemeralWallet() {
          return {
            address: "0x0000000000000000000000000000000000000aaa",
            privateKey: "0x01"
          };
        },
        async submitSpawnPayment() {},
        async waitForSessionCompletion() {
          return {
            registryRecord: null
          };
        }
      })
    });

    try {
      await controller.startRun(experimentPath);
      await controller.waitForCompletion();

      const dashboard = controller.getDashboard();
      expect(dashboard?.run.runState).toBe("failed");
      expect(dashboard?.run.successfulSpawnCount).toBe(0);
      expect(dashboard?.run.abortReason).toContain(
        "EVAL_INFERENCE_PROXY_WORKER_BASE_URL"
      );
      expect(dashboard?.run.abortReason).toContain(
        "EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL"
      );
    } finally {
      await loggerApp.close();
    }
  });

  it("honors manual stop requests during the sampling loop", async () => {
    const repoRoot = await createTempDirectory("evaluator-controller-repo-stop-");
    const artifactsRoot = await createTempDirectory("evaluator-controller-artifacts-stop-");
    const experimentPath = await writeExperiment(
      repoRoot,
      [
        "name: smoke",
        "description: stop scenario",
        "maxRuntimeMinutes: 240",
        "samplingIntervalSeconds: 15",
        "stallAfterMinutes: 10",
        "spawn:",
        "  grossAmount: \"75000000\"",
        "  minSuccessRatio: 1",
        "automatons:",
        "  - id: alpha",
        "    label: Alpha",
        "    model: model-alpha",
        "    strategies:",
        "      - momentum"
      ].join("\n")
    );

    let resumeSleep = () => {};
    let notifySleep: (() => void) | null = null;
    const sleepStarted = new Promise<void>((resolve) => {
      notifySleep = resolve;
    });

    const { controller, loggerApp } = createController({
      repoRoot,
      artifactsRoot,
      experimentPath,
      loadPlaygroundHelpers: async () => ({
        async claimPlaygroundFaucet() {},
        createEphemeralWallet() {
          return {
            address: "0x0000000000000000000000000000000000000aaa",
            privateKey: "0x01"
          };
        },
        async submitSpawnPayment() {},
        async waitForSessionCompletion() {
          return {
            registryRecord: {
              canisterId: "canister-alpha",
              evmAddress: "0x0000000000000000000000000000000000000aaa",
              versionCommit: "89abcdef0123456789abcdef0123456789abcdef"
            }
          };
        }
      }),
      sleep: async () =>
        await new Promise<void>((resolve) => {
          notifySleep?.();
          resumeSleep = resolve;
        })
    });

    try {
      await controller.startRun(experimentPath);
      await sleepStarted;
      await controller.requestStop();
      resumeSleep();
      await controller.waitForCompletion();

      const dashboard = controller.getDashboard();
      expect(dashboard?.run.runState).toBe("completed");
      expect(dashboard?.report?.completionReason).toBe("stopped_manually");
      expect(dashboard?.report?.comparisonValid).toBe(false);
    } finally {
      await loggerApp.close();
    }
  });
});
