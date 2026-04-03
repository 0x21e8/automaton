import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, describe, expect, it, vi } from "vitest";

import { FactoryClient } from "../src/integrations/factory-client.js";
import { buildServer, start } from "../src/server.js";
import {
  createAutomatonDetailFixture,
  createMonologueEntryFixture,
  createRoomMessageFixture,
  createSpawnSessionDetailFixture,
  createSpawnedAutomatonRecordFixture
} from "./fixtures.js";

const tempPaths: string[] = [];
const indexerWorkspacePath = fileURLToPath(new URL("..", import.meta.url));
const serverEntryPath = fileURLToPath(new URL("../src/server.ts", import.meta.url));

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

async function createDatabasePath() {
  const directory = await createTempDirectory("indexer-server-");
  return join(directory, "indexer.sqlite");
}

async function runStartupProcess(env: NodeJS.ProcessEnv) {
  const child = spawn(process.execPath, ["--import", "tsx", serverEntryPath], {
    cwd: indexerWorkspacePath,
    env: {
      ...process.env,
      ...env
    },
    stdio: ["ignore", "pipe", "pipe"]
  });

  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk) => {
    stdout += chunk.toString();
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
  });

  return await new Promise<{
    code: number | null;
    signal: NodeJS.Signals | null;
    stderr: string;
    stdout: string;
  }>((resolve, reject) => {
    const timeout = setTimeout(() => {
      child.kill("SIGTERM");
      reject(new Error("Timed out waiting for indexer startup process to exit."));
    }, 5_000);

    child.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });

    child.once("close", (code, signal) => {
      clearTimeout(timeout);
      resolve({
        code,
        signal,
        stderr,
        stdout
      });
    });
  });
}

function waitForMessage(socket: {
  once: (
    event: "error" | "message",
    listener: (value: Error | { toString(): string }) => void
  ) => void;
}) {
  return new Promise<string>((resolve, reject) => {
    socket.once("error", (error) => {
      reject(error);
    });
    socket.once("message", (data) => {
      resolve(data.toString());
    });
  });
}

describe("indexer server", () => {
  it("fails startup clearly when the canister list is invalid", async () => {
    const stderrChunks: string[] = [];
    const previousExitCode = process.exitCode;

    process.exitCode = undefined;

    try {
      await start({
        stderr: {
          write(chunk) {
            stderrChunks.push(String(chunk));
            return true;
          }
        },
        config: {
          ingestion: {
            canisterIds: [],
            network: {
              target: "local",
              local: {
                host: "localhost",
                port: 8000
              }
            }
          }
        }
      });

      expect(process.exitCode).toBe(1);
      expect(stderrChunks.join("")).toContain(
        "Indexer startup aborted: invalid ingestion target configuration."
      );
      expect(stderrChunks.join("")).toContain(
        "Indexer ingestion config must include at least one canister ID when factory discovery is not configured."
      );
      expect(stderrChunks.join("")).toContain("apps/indexer/src/indexer.config.ts");
    } finally {
      process.exitCode = previousExitCode;
    }
  });

  it("fails startup clearly when the network target override is invalid", async () => {
    const result = await runStartupProcess({
      INDEXER_INGESTION_NETWORK_TARGET: "staging"
    });

    expect(result.code).toBe(1);
    expect(result.signal).toBe(null);
    expect(result.stdout).toBe("");
    expect(result.stderr).toContain(
      "Indexer startup aborted: invalid ingestion target configuration."
    );
    expect(result.stderr).toContain("INDEXER_INGESTION_NETWORK_TARGET");
    expect(result.stderr).toContain(
      'Indexer ingestion config network.target must be one of "mainnet", "local".'
    );
  });

  it("responds to the health route", async () => {
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath()
      }
    });

    const response = await app.inject({
      method: "GET",
      url: "/health"
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toMatchObject({
      ok: true,
      service: "indexer",
      database: {
        ok: true,
        driver: "better-sqlite3"
      },
      discovery: {
        mode: "seeds_only",
        seedCanisterIds: ["txyno-ch777-77776-aaaaq-cai"],
        factoryDiscoveredCanisterIds: [],
        trackedCanisterIds: ["txyno-ch777-77776-aaaaq-cai"],
        targetNetwork: {
          target: "local",
          icHost: "http://localhost:8000",
          localReplica: {
            host: "localhost",
            port: 8000
          }
        },
        factoryCanisterId: null,
        factoryConfigured: false
      },
      factory: null,
      realtime: {
        websocketPath: "/ws/events",
        clientCount: 0
      }
    });

    await app.close();
  });

  it("surfaces factory health when a factory canister client is configured", async () => {
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath(),
        factoryCanisterId: "txyno-ch777-77776-aaaaq-cai"
      },
      factoryClient: new FactoryClient({
        configured: true,
        adapter: {
          async createSpawnSession() {
            throw new Error("not used in this test");
          },
          async getSpawnSession() {
            return null;
          },
          async retrySpawnSession() {
            throw new Error("not used in this test");
          },
          async claimSpawnRefund() {
            throw new Error("not used in this test");
          },
          async listSpawnedAutomatons() {
            return {
              items: [],
              nextCursor: null
            };
          },
          async listRepositoryStrategies() {
            return {
              items: [],
              updatedAt: 0
            };
          },
          async getRepositoryStrategy() {
            return {
              item: null,
              updatedAt: 0
            };
          },
          async listRoomMessages() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async listMessagesForAutomaton() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async getSpawnedAutomaton() {
            return null;
          },
          async getFactoryHealth() {
            return {
              activeSessions: {
                activeTotal: 2,
                awaitingPayment: 1,
                broadcastingRelease: 0,
                paymentDetected: 1,
                retryableFailed: 0,
                spawning: 0
              },
              artifact: {
                loaded: true,
                versionCommit: "0123456789abcdef0123456789abcdef01234567",
                wasmSha256:
                  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                wasmSizeBytes: 4_096
              },
              currentCanisterBalance: "987654321",
              cyclesPerSpawn: 1_500_000,
              escrowContractAddress: "0x00000000000000000000000000000000000000aa",
              estimatedOutcallCyclesPerInterval: 40_000,
              factoryEvmAddress: "0x00000000000000000000000000000000000000bb",
              minPoolBalance: 750_000,
              pause: false
            };
          }
        }
      })
    });

    const response = await app.inject({
      method: "GET",
      url: "/health"
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toMatchObject({
      discovery: {
        mode: "seeds_only",
        factoryCanisterId: "txyno-ch777-77776-aaaaq-cai",
        factoryConfigured: true,
        factoryDiscoveredCanisterIds: [],
        trackedCanisterIds: ["txyno-ch777-77776-aaaaq-cai"]
      },
      factory: {
        currentCanisterBalance: "987654321",
        escrowContractAddress: "0x00000000000000000000000000000000000000aa",
        artifact: {
          loaded: true
        }
      }
    });

    await app.close();
  });

  it("loads the configured canister id list into the startup registry", async () => {
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath(),
        ingestion: {
          canisterIds: [
            "ryjl3-tyaaa-aaaaa-aaaba-cai",
            "txyno-ch777-77776-aaaaq-cai"
          ],
          network: {
            target: "local",
            local: {
              host: "localhost",
              port: 8000
            }
          }
        }
      }
    });

    await app.ready();

    await expect(app.indexerStore.listConfiguredCanisterIds()).resolves.toEqual([
      "ryjl3-tyaaa-aaaaa-aaaba-cai",
      "txyno-ch777-77776-aaaaq-cai"
    ]);
    await expect(app.indexerStore.listTrackedCanisterIds()).resolves.toEqual([
      "ryjl3-tyaaa-aaaaa-aaaba-cai",
      "txyno-ch777-77776-aaaaq-cai"
    ]);
    await expect(app.indexerStore.listAutomatons()).resolves.toEqual({
      automatons: [],
      total: 0,
      prices: {
        ethUsd: null
      }
    });

    await app.close();
  });

  it("surfaces the effective mainnet target in the health route", async () => {
    const app = buildServer({
      env: {
        ...process.env,
        INDEXER_INGESTION_NETWORK_TARGET: "mainnet"
      },
      config: {
        databasePath: await createDatabasePath()
      }
    });

    const response = await app.inject({
      method: "GET",
      url: "/health"
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toMatchObject({
      discovery: {
        mode: "seeds_only",
        seedCanisterIds: ["txyno-ch777-77776-aaaaq-cai"],
        factoryDiscoveredCanisterIds: [],
        trackedCanisterIds: ["txyno-ch777-77776-aaaaq-cai"],
        targetNetwork: {
          target: "mainnet",
          icHost: "https://ic0.app",
          localReplica: null
        }
      }
    });

    await app.close();
  });

  it("returns config-derived playground metadata when the status file is missing", async () => {
    const statusDirectory = await createTempDirectory("indexer-playground-status-");
    const app = buildServer({
      env: {
        ...process.env,
        PLAYGROUND_CHAIN_ID: "20260326",
        PLAYGROUND_CHAIN_NAME: "Automaton Playground",
        PLAYGROUND_ENV_LABEL: "Automaton Playground",
        PLAYGROUND_FAUCET_ENABLED: "1",
        PLAYGROUND_PUBLIC_RPC_URL: "https://rpc.playground.example.com",
        PLAYGROUND_STATUS_FILE: join(statusDirectory, "missing.json")
      },
      config: {
        databasePath: await createDatabasePath()
      }
    });

    const response = await app.inject({
      method: "GET",
      url: "/api/playground"
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toEqual({
      environmentLabel: "Automaton Playground",
      environmentVersion: null,
      maintenance: false,
      chain: {
        id: 20_260_326,
        name: "Automaton Playground",
        publicRpcUrl: "https://rpc.playground.example.com",
        nativeCurrency: {
          name: "Ether",
          symbol: "ETH",
          decimals: 18
        },
        explorerUrl: null
      },
      faucet: {
        available: true,
        claimLimits: {
          windowSeconds: 86_400,
          maxClaimsPerWallet: 1,
          maxClaimsPerIp: 1
        },
        claimAssetAmounts: [
          {
            asset: "eth",
            amount: "1",
            decimals: 18
          },
          {
            asset: "usdc",
            amount: "250",
            decimals: 6
          }
        ]
      },
      reset: {
        lastResetAt: null,
        nextResetAt: null,
        cadenceLabel: "Manual local resets"
      }
    });

    await app.close();
  });

  it("overlays the runtime playground status file onto config metadata", async () => {
    const statusDirectory = await createTempDirectory("indexer-playground-status-");
    const statusFilePath = join(statusDirectory, "playground-status.json");
    await writeFile(
      statusFilePath,
      JSON.stringify({
        environmentVersion: "runtime-2026.03.26+sha.abcdef",
        maintenance: true,
        message: "scheduled reset incoming",
        lastResetAt: "2026-03-26T09:00:00Z",
        nextResetAt: "2026-03-27T09:00:00Z",
        updatedAt: "2026-03-26T09:05:00Z"
      })
    );
    const app = buildServer({
      env: {
        ...process.env,
        PLAYGROUND_CHAIN_ID: "20260326",
        PLAYGROUND_CHAIN_NAME: "Automaton Playground",
        PLAYGROUND_ENV_LABEL: "Automaton Playground",
        PLAYGROUND_ENV_VERSION: "build-2026.03.26+sha.123456",
        PLAYGROUND_PUBLIC_RPC_URL: "https://rpc.playground.example.com",
        PLAYGROUND_STATUS_FILE: statusFilePath
      },
      config: {
        databasePath: await createDatabasePath()
      }
    });

    const response = await app.inject({
      method: "GET",
      url: "/api/playground"
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toMatchObject({
      environmentLabel: "Automaton Playground",
      environmentVersion: "runtime-2026.03.26+sha.abcdef",
      maintenance: true,
      chain: {
        id: 20_260_326,
        name: "Automaton Playground",
        publicRpcUrl: "https://rpc.playground.example.com"
      },
      reset: {
        lastResetAt: Date.parse("2026-03-26T09:00:00Z"),
        nextResetAt: Date.parse("2026-03-27T09:00:00Z"),
        cadenceLabel: "Manual local resets"
      }
    });

    await app.close();
  });

  it("syncs factory discovery into the tracked registry during startup", async () => {
    const sharedCanisterId = "txyno-ch777-77776-aaaaq-cai";
    const factoryOnlyCanisterId = "ryjl3-tyaaa-aaaaa-aaaba-cai";
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath(),
        factoryCanisterId: "mxzaz-hqaaa-aaaar-qaada-cai",
        ingestion: {
          canisterIds: [sharedCanisterId],
          network: {
            target: "local",
            local: {
              host: "localhost",
              port: 8000
            }
          }
        }
      },
      factoryClient: new FactoryClient({
        configured: true,
        adapter: {
          async createSpawnSession() {
            throw new Error("not used in this test");
          },
          async getSpawnSession() {
            return null;
          },
          async retrySpawnSession() {
            throw new Error("not used in this test");
          },
          async claimSpawnRefund() {
            throw new Error("not used in this test");
          },
          async listSpawnedAutomatons(cursor) {
            if (cursor === undefined) {
              return {
                items: [
                  createSpawnedAutomatonRecordFixture({
                    canisterId: sharedCanisterId
                  })
                ],
                nextCursor: sharedCanisterId
              };
            }

            return {
              items: [
                createSpawnedAutomatonRecordFixture({
                  canisterId: factoryOnlyCanisterId,
                  sessionId: "session-1709912345000-2"
                })
              ],
              nextCursor: null
            };
          },
          async listRepositoryStrategies() {
            return {
              items: [],
              updatedAt: 0
            };
          },
          async getRepositoryStrategy() {
            return {
              item: null,
              updatedAt: 0
            };
          },
          async listRoomMessages() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async listMessagesForAutomaton() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async getSpawnedAutomaton() {
            return null;
          },
          async getFactoryHealth() {
            return {
              activeSessions: {
                activeTotal: 0,
                awaitingPayment: 0,
                broadcastingRelease: 0,
                paymentDetected: 0,
                retryableFailed: 0,
                spawning: 0
              },
              artifact: {
                loaded: true,
                versionCommit: "0123456789abcdef0123456789abcdef01234567",
                wasmSha256:
                  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                wasmSizeBytes: 4_096
              },
              currentCanisterBalance: "123",
              cyclesPerSpawn: 1,
              escrowContractAddress: "0x00000000000000000000000000000000000000aa",
              estimatedOutcallCyclesPerInterval: 1,
              factoryEvmAddress: "0x00000000000000000000000000000000000000bb",
              minPoolBalance: 1,
              pause: false
            };
          }
        }
      })
    });

    await app.ready();

    await expect(app.indexerStore.listFactoryDiscoveredCanisterIds()).resolves.toEqual([
      factoryOnlyCanisterId,
      sharedCanisterId
    ]);
    await expect(app.indexerStore.listTrackedCanisterIds()).resolves.toEqual([
      factoryOnlyCanisterId,
      sharedCanisterId
    ]);

    const response = await app.inject({
      method: "GET",
      url: "/health"
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toMatchObject({
      discovery: {
        mode: "both",
        seedCanisterIds: [sharedCanisterId],
        factoryDiscoveredCanisterIds: [factoryOnlyCanisterId, sharedCanisterId],
        trackedCanisterIds: [factoryOnlyCanisterId, sharedCanisterId],
        overlapCanisterIds: [sharedCanisterId],
        counts: {
          seedCanisters: 1,
          factoryDiscoveredCanisters: 2,
          trackedCanisters: 2,
          duplicateCanisters: 1
        }
      }
    });

    await app.close();
  });

  it("serves empty automaton routes by default", async () => {
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath()
      }
    });

    const listResponse = await app.inject({
      method: "GET",
      url: "/api/automatons"
    });
    const detailResponse = await app.inject({
      method: "GET",
      url: "/api/automatons/aaaaa-aa"
    });
    const monologueResponse = await app.inject({
      method: "GET",
      url: "/api/automatons/aaaaa-aa/monologue"
    });

    expect(listResponse.statusCode).toBe(200);
    expect(listResponse.json()).toEqual({
      automatons: [],
      total: 0,
      prices: {
        ethUsd: null
      }
    });

    expect(detailResponse.statusCode).toBe(404);
    expect(detailResponse.json()).toMatchObject({
      ok: false,
      error: "Automaton not found"
    });

    expect(monologueResponse.statusCode).toBe(200);
    expect(monologueResponse.json()).toEqual({
      entries: [],
      hasMore: false,
      nextCursor: null
    });

    await app.close();
  });

  it("allows configured dev origins to call the API over CORS", async () => {
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath()
      }
    });

    const preflightResponse = await app.inject({
      method: "OPTIONS",
      url: "/api/automatons",
      headers: {
        origin: "http://127.0.0.1:5173"
      }
    });
    const getResponse = await app.inject({
      method: "GET",
      url: "/api/automatons",
      headers: {
        origin: "http://127.0.0.1:5173"
      }
    });

    expect(preflightResponse.statusCode).toBe(204);
    expect(preflightResponse.headers["access-control-allow-origin"]).toBe(
      "http://127.0.0.1:5173"
    );
    expect(preflightResponse.headers["access-control-allow-methods"]).toBe(
      "GET,POST,OPTIONS"
    );
    expect(getResponse.statusCode).toBe(200);
    expect(getResponse.headers["access-control-allow-origin"]).toBe(
      "http://127.0.0.1:5173"
    );

    await app.close();
  });

  it("rejects disallowed preflight origins", async () => {
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath()
      }
    });

    const response = await app.inject({
      method: "OPTIONS",
      url: "/api/automatons",
      headers: {
        origin: "https://evil.example"
      }
    });

    expect(response.statusCode).toBe(403);
    expect(response.json()).toMatchObject({
      ok: false,
      error: "Origin not allowed"
    });

    await app.close();
  });

  it("returns upgrade guidance for HTTP clients and streams realtime events", async () => {
    const databasePath = await createDatabasePath();
    const app = buildServer({
      config: {
        databasePath
      }
    });
    const detail = createAutomatonDetailFixture();
    const entry = createMonologueEntryFixture();

    await app.ready();
    await app.indexerStore.upsertAutomaton(detail);
    await app.indexerStore.appendMonologue(detail.canisterId, [entry]);
    await app.indexerStore.setPrice("ethUsd", 2_499.25);
    const socket = await app.injectWS(`/ws/events?canisterId=${detail.canisterId}`);
    const eventMessage = waitForMessage(socket);

    const listResponse = await app.inject({
      method: "GET",
      url: `/api/automatons?steward=${encodeURIComponent(detail.steward.address)}`
    });
    const detailResponse = await app.inject({
      method: "GET",
      url: `/api/automatons/${detail.canisterId}`
    });
    const monologueResponse = await app.inject({
      method: "GET",
      url: `/api/automatons/${detail.canisterId}/monologue?limit=10`
    });
    const websocketResponse = await app.inject({
      method: "GET",
      url: "/ws/events"
    });

    app.realtimeHub.broadcast({
      type: "offline",
      canisterId: detail.canisterId,
      timestamp: Date.now()
    });

    expect(listResponse.statusCode).toBe(200);
    expect(listResponse.json()).toMatchObject({
      total: 1,
      prices: {
        ethUsd: 2_499.25
      }
    });

    expect(detailResponse.statusCode).toBe(200);
    expect(detailResponse.json()).toMatchObject({
      canisterId: detail.canisterId,
      name: detail.name
    });

    expect(monologueResponse.statusCode).toBe(200);
    expect(monologueResponse.json()).toEqual({
      entries: [entry],
      hasMore: false,
      nextCursor: null
    });

    expect(websocketResponse.statusCode).toBe(426);
    expect(websocketResponse.json()).toMatchObject({
      ok: false,
      error: "Upgrade Required",
      realtime: {
        websocketPath: "/ws/events"
      }
    });
    await expect(eventMessage).resolves.toContain(detail.canisterId);

    socket.close();
    await app.close();
  });

  it("serves inert room history and supports relevant room reads", async () => {
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath()
      }
    });
    const broadcast = createRoomMessageFixture({
      messageId: "room-message-1",
      seq: 1,
      mentions: []
    });
    const targeted = createRoomMessageFixture({
      messageId: "room-message-2",
      seq: 2,
      mentions: ["txyno-ch777-77776-aaaaq-cai"],
      body: "<script>alert('xss')</script>"
    });

    await app.ready();
    await app.indexerStore.upsertRoomMessages([broadcast, targeted], targeted.seq);

    const allResponse = await app.inject({
      method: "GET",
      url: "/api/room/messages?limit=10"
    });
    const relevantResponse = await app.inject({
      method: "GET",
      url: "/api/room/messages?scope=relevant&canisterId=txyno-ch777-77776-aaaaq-cai"
    });
    const invalidResponse = await app.inject({
      method: "GET",
      url: "/api/room/messages?scope=relevant"
    });

    expect(allResponse.statusCode).toBe(200);
    expect(allResponse.json()).toEqual({
      messages: [broadcast, targeted],
      nextAfterSeq: null,
      latestSeq: targeted.seq
    });
    expect(relevantResponse.statusCode).toBe(200);
    expect(relevantResponse.json()).toEqual({
      messages: [broadcast, targeted],
      nextAfterSeq: null,
      latestSeq: targeted.seq
    });
    expect(invalidResponse.statusCode).toBe(400);
    expect(invalidResponse.json()).toEqual({
      ok: false,
      error: "canisterId is required when scope=relevant"
    });

    await app.close();
  });

  it("serves spawn-session status and registry routes without touching the public automaton list", async () => {
    const databasePath = await createDatabasePath();
    const registryRecord = createSpawnedAutomatonRecordFixture();
    const sessionDetail = createSpawnSessionDetailFixture({
      registryRecord
    });
    const faucetSeedRunner = vi.fn(async ({ walletAddress }: { walletAddress: string }) => {
      return {
        walletAddress,
        mintTxHash: "0xmint",
        fundTxHash: "0xfund",
        balances: {
          ethWei: "1000000000000000000",
          usdcRaw: "250000000"
        }
      };
    });
    const createdSession = createSpawnSessionDetailFixture({
      registryRecord: null,
      session: {
        ...sessionDetail.session,
        sessionId: "session-create-1",
        state: "awaiting_payment",
        paymentStatus: "unpaid",
        automatonCanisterId: null,
        automatonEvmAddress: null
      }
    });
    const app = buildServer({
      env: {
        ...process.env,
        PLAYGROUND_FAUCET_ENABLED: "1",
        PLAYGROUND_FAUCET_ETH_AMOUNT: "1",
        PLAYGROUND_FAUCET_USDC_AMOUNT: "250"
      },
      config: {
        databasePath,
        factoryCanisterId: "factory-canister-id"
      },
      faucetSeedRunner,
      factoryClient: new FactoryClient({
        configured: true,
        adapter: {
          async createSpawnSession(request) {
            return {
              session: {
                ...createdSession.session,
                stewardAddress: request.stewardAddress,
                asset: request.asset,
                grossAmount: request.grossAmount,
                config: request.config
              },
              quote: {
                sessionId: createdSession.session.sessionId,
                chain: request.config.chain,
                asset: request.asset,
                grossAmount: request.grossAmount,
                platformFee: createdSession.session.platformFee,
                creationCost: createdSession.session.creationCost,
                netForwardAmount: createdSession.session.netForwardAmount,
                quoteTermsHash: createdSession.session.quoteTermsHash,
                expiresAt: createdSession.session.expiresAt,
                payment: {
                  sessionId: createdSession.session.sessionId,
                  claimId: createdSession.session.claimId,
                  chain: request.config.chain,
                  asset: request.asset,
                  paymentAddress: "0x00000000000000000000000000000000000000ef",
                  grossAmount: request.grossAmount,
                  quoteTermsHash: createdSession.session.quoteTermsHash,
                  expiresAt: createdSession.session.expiresAt
                }
              }
            };
          },
          async getSpawnSession(sessionId) {
            return sessionId === sessionDetail.session.sessionId
              ? {
                  session: sessionDetail.session,
                  payment: sessionDetail.payment,
                  audit: sessionDetail.audit
                }
              : null;
          },
          async listSpawnedAutomatons() {
            return {
              items: [registryRecord],
              nextCursor: null
            };
          },
          async listRepositoryStrategies() {
            return {
              items: [
                {
                  strategyId: "base-aave-usdc-reserve-01",
                  name: "Base Aave USDC Reserve",
                  description: "Park surplus Base USDC on Aave V3.",
                  canonicalChain: "base",
                  canonicalChainId: 8453,
                  compatibleSpawnChains: ["base"],
                  protocol: "aave-v3",
                  primitive: "lend_supply",
                  recipeJson: "{}",
                  status: "active",
                  source: {
                    sourcePath: "docs/strategies/base-aave-usdc-reserve-01/recipe.json",
                    sourceCommit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa"
                  },
                  createdAt: 1_709_912_345_000,
                  updatedAt: 1_709_912_360_000,
                  deprecatedAt: null,
                  revokedAt: null
                }
              ],
              updatedAt: 1_709_912_360_000
            };
          },
          async getRepositoryStrategy(strategyId) {
            return {
              item:
                strategyId === "base-aave-usdc-reserve-01"
                  ? {
                      strategyId,
                      name: "Base Aave USDC Reserve",
                      description: "Park surplus Base USDC on Aave V3.",
                      canonicalChain: "base",
                      canonicalChainId: 8453,
                      compatibleSpawnChains: ["base"],
                      protocol: "aave-v3",
                      primitive: "lend_supply",
                      recipeJson: "{}",
                      status: "active",
                      source: {
                        sourcePath:
                          "docs/strategies/base-aave-usdc-reserve-01/recipe.json",
                        sourceCommit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa"
                      },
                      createdAt: 1_709_912_345_000,
                      updatedAt: 1_709_912_360_000,
                      deprecatedAt: null,
                      revokedAt: null
                    }
                  : null,
              updatedAt: 1_709_912_360_000
            };
          },
          async listRoomMessages() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async listMessagesForAutomaton() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async retrySpawnSession(sessionId) {
            return {
              session:
                sessionId === sessionDetail.session.sessionId
                  ? {
                      ...sessionDetail.session,
                      state: "payment_detected",
                      retryable: false
                    }
                  : sessionDetail.session
            };
          },
          async claimSpawnRefund(sessionId) {
            return {
              sessionId,
              state: "expired",
              paymentStatus: "refunded",
              refundedAt: 1_709_912_361_000
            };
          },
          async getSpawnedAutomaton(canisterId) {
            return canisterId === registryRecord.canisterId ? registryRecord : null;
          },
          async getFactoryHealth() {
            return {
              activeSessions: {
                activeTotal: 1,
                awaitingPayment: 0,
                broadcastingRelease: 0,
                paymentDetected: 0,
                retryableFailed: 0,
                spawning: 0
              },
              artifact: {
                loaded: true,
                versionCommit: registryRecord.versionCommit,
                wasmSha256:
                  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                wasmSizeBytes: 1024
              },
              currentCanisterBalance: "123456789",
              cyclesPerSpawn: 1_000_000,
              escrowContractAddress: "0x00000000000000000000000000000000000000aa",
              estimatedOutcallCyclesPerInterval: 25_000,
              factoryEvmAddress: "0x00000000000000000000000000000000000000bb",
              minPoolBalance: 500_000,
              pause: false
            };
          }
        }
      })
    });
    await app.ready();

    const sessionResponse = await app.inject({
      method: "GET",
      url: `/api/spawn-sessions/${sessionDetail.session.sessionId}`
    });
    const createResponse = await app.inject({
      method: "POST",
      url: "/api/spawn-sessions",
      payload: {
        stewardAddress: sessionDetail.session.stewardAddress,
        asset: sessionDetail.session.asset,
        grossAmount: sessionDetail.session.grossAmount,
        config: sessionDetail.session.config
      }
    });
    const retryResponse = await app.inject({
      method: "POST",
      url: `/api/spawn-sessions/${sessionDetail.session.sessionId}/retry`
    });
    const refundResponse = await app.inject({
      method: "POST",
      url: `/api/spawn-sessions/${sessionDetail.session.sessionId}/refund`
    });
    const registryResponse = await app.inject({
      method: "GET",
      url: "/api/spawned-automatons"
    });
    const registryRecordResponse = await app.inject({
      method: "GET",
      url: `/api/spawned-automatons/${registryRecord.canisterId}`
    });
    const repositoryResponse = await app.inject({
      method: "GET",
      url: "/api/repository/strategies"
    });
    const repositoryItemResponse = await app.inject({
      method: "GET",
      url: "/api/repository/strategies/base-aave-usdc-reserve-01"
    });
    const automatonListResponse = await app.inject({
      method: "GET",
      url: "/api/automatons"
    });

    expect(sessionResponse.statusCode).toBe(200);
    expect(sessionResponse.json()).toEqual({
      session: sessionDetail.session,
      payment: sessionDetail.payment,
      audit: sessionDetail.audit,
      registryRecord
    });
    expect(faucetSeedRunner).toHaveBeenCalledWith({
      walletAddress: registryRecord.evmAddress,
      ethAmount: {
        asset: "eth",
        amount: "1",
        decimals: 18
      },
      usdcAmount: {
        asset: "usdc",
        amount: "250",
        decimals: 6
      }
    });
    expect(createResponse.statusCode).toBe(200);
    expect(createResponse.json()).toMatchObject({
      session: {
        sessionId: createdSession.session.sessionId,
        stewardAddress: sessionDetail.session.stewardAddress
      },
      quote: {
        payment: {
          paymentAddress: "0x00000000000000000000000000000000000000ef"
        }
      }
    });
    await expect(
      app.indexerStore.getSpawnSessionDetail(createdSession.session.sessionId)
    ).resolves.toMatchObject({
      session: {
        sessionId: createdSession.session.sessionId,
        config: {
          provider: {
            model: createdSession.session.config.provider.model
          }
        }
      }
    });
    expect(retryResponse.statusCode).toBe(200);
    expect(retryResponse.json()).toEqual({
      session: {
        ...sessionDetail.session,
        state: "payment_detected",
        retryable: false
      }
    });
    expect(refundResponse.statusCode).toBe(200);
    expect(refundResponse.json()).toEqual({
      sessionId: sessionDetail.session.sessionId,
      state: "expired",
      paymentStatus: "refunded",
      refundedAt: 1_709_912_361_000
    });

    expect(registryResponse.statusCode).toBe(200);
    expect(registryResponse.json()).toEqual({
      items: [registryRecord],
      nextCursor: null
    });

    expect(registryRecordResponse.statusCode).toBe(200);
    expect(registryRecordResponse.json()).toEqual(registryRecord);

    expect(repositoryResponse.statusCode).toBe(200);
    expect(repositoryResponse.json()).toMatchObject({
      items: [
        {
          strategyId: "base-aave-usdc-reserve-01",
          status: "active"
        }
      ],
      updatedAt: 1_709_912_360_000
    });

    expect(repositoryItemResponse.statusCode).toBe(200);
    expect(repositoryItemResponse.json()).toMatchObject({
      item: {
        strategyId: "base-aave-usdc-reserve-01",
        canonicalChainId: 8453
      },
      updatedAt: 1_709_912_360_000
    });

    expect(automatonListResponse.statusCode).toBe(200);
    expect(automatonListResponse.json()).toEqual({
      automatons: [],
      total: 0,
      prices: {
        ethUsd: null
      }
    });

    await app.close();
  });

  it("enriches automaton detail with spawn-session strategy snapshots when available", async () => {
    const registryRecord = createSpawnedAutomatonRecordFixture();
    const sessionDetail = createSpawnSessionDetailFixture({
      registryRecord
    });
    const automatonDetail = createAutomatonDetailFixture({
      canisterId: registryRecord.canisterId,
      strategies: [
        {
          key: {
            protocol: "aave-v3",
            primitive: "lend_supply",
            templateId: "base-aave-usdc-reserve-01",
            chainId: 8453
          },
          status: "active"
        }
      ]
    });
    const app = buildServer({
      config: {
        databasePath: await createDatabasePath(),
        factoryCanisterId: "factory-canister-id"
      },
      factoryClient: new FactoryClient({
        configured: true,
        adapter: {
          async createSpawnSession() {
            throw new Error("not used in this test");
          },
          async getSpawnSession(sessionId) {
            return sessionId === sessionDetail.session.sessionId
              ? {
                  session: sessionDetail.session,
                  payment: sessionDetail.payment,
                  audit: sessionDetail.audit
                }
              : null;
          },
          async retrySpawnSession() {
            throw new Error("not used in this test");
          },
          async claimSpawnRefund() {
            throw new Error("not used in this test");
          },
          async listSpawnedAutomatons() {
            return {
              items: [],
              nextCursor: null
            };
          },
          async listRepositoryStrategies() {
            return {
              items: [],
              updatedAt: 0
            };
          },
          async getRepositoryStrategy() {
            return {
              item: null,
              updatedAt: 0
            };
          },
          async listRoomMessages() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async listMessagesForAutomaton() {
            return {
              messages: [],
              nextAfterSeq: null,
              latestSeq: null
            };
          },
          async getSpawnedAutomaton(canisterId) {
            return canisterId === registryRecord.canisterId ? registryRecord : null;
          },
          async getFactoryHealth() {
            return {
              activeSessions: {
                activeTotal: 0,
                awaitingPayment: 0,
                broadcastingRelease: 0,
                paymentDetected: 0,
                retryableFailed: 0,
                spawning: 0
              },
              artifact: {
                loaded: true,
                versionCommit: registryRecord.versionCommit,
                wasmSha256: null,
                wasmSizeBytes: null
              },
              currentCanisterBalance: "0",
              cyclesPerSpawn: 0,
              escrowContractAddress: "0x00000000000000000000000000000000000000aa",
              estimatedOutcallCyclesPerInterval: 0,
              factoryEvmAddress: null,
              minPoolBalance: 0,
              pause: false
            };
          }
        }
      })
    });

    await app.ready();
    await app.indexerStore.upsertAutomaton(automatonDetail);
    await app.indexerStore.upsertSpawnedAutomatonRegistry([registryRecord]);

    const response = await app.inject({
      method: "GET",
      url: `/api/automatons/${registryRecord.canisterId}`
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toMatchObject({
      canisterId: registryRecord.canisterId,
      strategies: [
        {
          key: {
            templateId: "base-aave-usdc-reserve-01"
          },
          status: "active"
        }
      ],
      spawnSelection: {
        sessionId: sessionDetail.session.sessionId,
        requestedStrategyIds: ["base-aave-usdc-reserve-01"],
        selectedStrategies: [
          {
            strategyId: "base-aave-usdc-reserve-01",
            name: "Base Aave USDC Reserve"
          }
        ]
      }
    });

    await app.close();
  });

  it("funds a faucet request and rejects repeated claims for the same wallet", async () => {
    const app = buildServer({
      env: {
        ...process.env,
        PLAYGROUND_FAUCET_ENABLED: "1",
        PLAYGROUND_FAUCET_CLAIM_WINDOW_SECONDS: "3600",
        PLAYGROUND_FAUCET_ETH_AMOUNT: "0.25",
        PLAYGROUND_FAUCET_MAX_CLAIMS_PER_IP: "5",
        PLAYGROUND_FAUCET_MAX_CLAIMS_PER_WALLET: "1",
        PLAYGROUND_FAUCET_USDC_AMOUNT: "75"
      },
      config: {
        databasePath: await createDatabasePath()
      },
      faucetSeedRunner: async ({ walletAddress }) => {
        return {
          walletAddress,
          mintTxHash: "0xmint",
          fundTxHash: "0xfund",
          balances: {
            ethWei: "250000000000000000",
            usdcRaw: "75000000"
          }
        };
      }
    });

    const successResponse = await app.inject({
      method: "POST",
      url: "/api/playground/faucet",
      payload: {
        walletAddress: "0x00000000000000000000000000000000000000AA"
      }
    });
    const limitedResponse = await app.inject({
      method: "POST",
      url: "/api/playground/faucet",
      payload: {
        walletAddress: "0x00000000000000000000000000000000000000aa"
      }
    });

    expect(successResponse.statusCode).toBe(200);
    expect(successResponse.json()).toEqual({
      ok: true,
      walletAddress: "0x00000000000000000000000000000000000000aa",
      txHashes: {
        eth: "0xfund",
        usdc: "0xmint"
      },
      fundedAmounts: {
        eth: {
          amount: "0.25",
          decimals: 18,
          wei: "250000000000000000"
        },
        usdc: {
          amount: "75",
          decimals: 6,
          raw: "75000000"
        }
      },
      balances: {
        ethWei: "250000000000000000",
        usdcRaw: "75000000"
      }
    });

    expect(limitedResponse.statusCode).toBe(429);
    expect(limitedResponse.headers["retry-after"]).toBeDefined();
    expect(limitedResponse.json()).toMatchObject({
      ok: false,
      error: "Faucet wallet claim limit exceeded.",
      walletAddress: "0x00000000000000000000000000000000000000aa",
      windowSeconds: 3600
    });

    await expect(
      app.indexerStore.getFaucetClaimWindowStats({
        walletAddress: "0x00000000000000000000000000000000000000aa",
        since: 0
      })
    ).resolves.toEqual({
      count: 1,
      oldestClaimAt: expect.any(Number)
    });

    await app.close();
  });

  it("rejects faucet claims when the caller IP exceeds the configured window", async () => {
    const app = buildServer({
      env: {
        ...process.env,
        PLAYGROUND_FAUCET_ENABLED: "1",
        PLAYGROUND_FAUCET_CLAIM_WINDOW_SECONDS: "3600",
        PLAYGROUND_FAUCET_MAX_CLAIMS_PER_IP: "1",
        PLAYGROUND_FAUCET_MAX_CLAIMS_PER_WALLET: "2"
      },
      config: {
        databasePath: await createDatabasePath()
      },
      faucetSeedRunner: async ({ walletAddress }) => {
        return {
          walletAddress,
          mintTxHash: "0xmint",
          fundTxHash: "0xfund",
          balances: {
            ethWei: "1000000000000000000",
            usdcRaw: "250000000"
          }
        };
      }
    });

    const firstResponse = await app.inject({
      method: "POST",
      url: "/api/playground/faucet",
      payload: {
        walletAddress: "0x00000000000000000000000000000000000000aa"
      }
    });
    const secondResponse = await app.inject({
      method: "POST",
      url: "/api/playground/faucet",
      payload: {
        walletAddress: "0x00000000000000000000000000000000000000bb"
      }
    });

    expect(firstResponse.statusCode).toBe(200);
    expect(secondResponse.statusCode).toBe(429);
    expect(secondResponse.json()).toMatchObject({
      ok: false,
      error: "Faucet IP claim limit exceeded.",
      windowSeconds: 3600
    });

    await app.close();
  });
});
