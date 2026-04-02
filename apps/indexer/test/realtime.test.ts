import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import type { RealtimeEvent } from "@ic-automaton/shared";

import type { IndexerConfig } from "../src/config.js";
import { buildServer } from "../src/server.js";
import type {
  AutomatonClient,
  IdentityConfigRead,
  RecentTurnsRead,
  RuntimeFinancialRead
} from "../src/integrations/automaton-client.js";
import { AutomatonIndexer } from "../src/polling/automaton-indexer.js";
import { createSqliteStore } from "../src/store/sqlite.js";
import { RealtimeHub, shouldDeliverEvent } from "../src/ws/events.js";
import {
  createAutomatonRecordFixture,
  createRoomMessageFixture,
  createSpawnSessionDetailFixture
} from "./fixtures.js";

const tempPaths: string[] = [];

interface TestWebSocket {
  close(): void;
  once(
    event: "error" | "message",
    listener: (value: Error | { toString(): string }) => void
  ): void;
}

afterEach(async () => {
  await Promise.all(
    tempPaths.splice(0).map(async (path) => {
      await rm(path, { recursive: true, force: true });
    })
  );
});

async function createDatabasePath() {
  const directory = await mkdtemp(join(tmpdir(), "indexer-realtime-"));
  tempPaths.push(directory);
  return join(directory, "indexer.sqlite");
}

function createPlaygroundConfig(): IndexerConfig["playground"] {
  return {
    metadata: {
      environmentLabel: "Local development",
      environmentVersion: null,
      maintenance: false,
      chain: {
        id: 8453,
        name: "Base Local Fork",
        publicRpcUrl: "http://127.0.0.1:8545",
        nativeCurrency: {
          name: "Ether",
          symbol: "ETH",
          decimals: 18
        },
        explorerUrl: null
      },
      faucet: {
        available: false,
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
    },
    statusFilePath: "/tmp/indexer-playground-status.json"
  };
}

function createIdentityConfigRead(canisterId: string): IdentityConfigRead {
  return {
    canisterId,
    buildInfo: {
      commit: "0123456789abcdef"
    },
    evmConfig: {
      automaton_address: "0x1111111111111111111111111111111111111111",
      chain_id: 8453,
      inbox_contract_address: "0x2222222222222222222222222222222222222222"
    },
    stewardStatus: {
      active_steward: {
        address: "0x3333333333333333333333333333333333333333",
        chain_id: 8453,
        enabled: true
      },
      next_nonce: 7
    },
    schedulerConfig: {
      base_tick_secs: 30,
      default_turn_interval_secs: 150,
      ticks_per_turn_interval: 5
    },
    promptLayers: [],
    skills: [],
    strategies: []
  };
}

function createRuntimeFinancialRead(canisterId: string): RuntimeFinancialRead {
  return {
    canisterId,
    snapshot: {
      runtime: {
        soul: "Yield allocator focused on preserving runway.",
        state: "Idle",
        loop_enabled: true,
        last_error: null,
        last_transition_at_ns: 1_709_912_347_000_000_000
      },
      scheduler: {
        enabled: true,
        last_tick_error: null,
        survival_tier: "LowCycles"
      },
      cycles: {
        total_cycles: 4_200_000_000_000,
        liquid_cycles: 3_100_000_000_000,
        burn_rate_cycles_per_day: 182_000_000_000,
        estimated_freeze_time_ns: 1_710_112_347_000_000_000
      },
      recent_turns: [
        {
          id: "turn-1",
          created_at_ns: 1_709_912_348_000_000_000,
          duration_ms: 830,
          state_from: "Sleeping",
          state_to: "Idle",
          tool_call_count: 0,
          input_summary: "Wake and inspect balances",
          inner_dialogue: "Checking solvency before the next action.",
          error: null
        }
      ]
    },
    walletBalance: {
      eth_balance_wei_hex: "0x1999999999999a00",
      usdc_balance_raw_hex: "0x2540be400",
      usdc_decimals: 6,
      last_error: null,
      last_synced_at_ns: 1_709_912_350_000_000_000,
      status: "ok",
      is_stale: false
    }
  };
}

function createRecentTurnsRead(canisterId: string): RecentTurnsRead {
  return {
    canisterId,
    recentTurns: createRuntimeFinancialRead(canisterId).snapshot.recent_turns ?? []
  };
}

function waitForWebSocketMessage(socket: TestWebSocket, timeoutMs = 5_000) {
  return new Promise<string>((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error("Timed out waiting for websocket message."));
    }, timeoutMs);
    socket.once("error", () => {
      clearTimeout(timeout);
      reject(new Error("Websocket error while waiting for message."));
    });
    socket.once("message", (data) => {
      clearTimeout(timeout);
      resolve(data.toString());
    });
  });
}

function expectNoWebSocketMessage(socket: TestWebSocket, timeoutMs = 300) {
  return new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(() => {
      resolve();
    }, timeoutMs);
    socket.once("error", () => {
      clearTimeout(timeout);
      reject(new Error("Websocket error while waiting for silence."));
    });
    socket.once("message", (data) => {
      clearTimeout(timeout);
      reject(new Error(`Unexpected websocket message: ${data.toString()}`));
    });
  });
}

describe("realtime hub", () => {
  it("exposes the websocket path and supported event types", () => {
    const hub = new RealtimeHub("/ws/events");

    expect(hub.getSnapshot()).toMatchObject({
      websocketPath: "/ws/events",
      clientCount: 0
    });
    expect(hub.getSnapshot().supportedEventTypes).toContain("spawn");
    expect(hub.getSnapshot().supportedEventTypes).toContain("spawn.session.failed");
  });

  it("filters automaton-scoped events by canister id", () => {
    const automaton = createAutomatonRecordFixture();
    const matchingEvent: RealtimeEvent = {
      type: "spawn",
      automaton
    };
    const nonMatchingEvent: RealtimeEvent = {
      type: "offline",
      canisterId: "bbbbb-bb",
      timestamp: Date.now()
    };

    expect(shouldDeliverEvent(automaton.canisterId, matchingEvent)).toBe(true);
    expect(shouldDeliverEvent(automaton.canisterId, nonMatchingEvent)).toBe(false);
    expect(shouldDeliverEvent(undefined, nonMatchingEvent)).toBe(true);
  });

  it("filters session-scoped events by session id", () => {
    const detail = createSpawnSessionDetailFixture();
    const matchingEvent: RealtimeEvent = {
      type: "spawn.session.completed",
      session: detail.session,
      audit: detail.audit
    };
    const nonMatchingEvent: RealtimeEvent = {
      type: "spawn.session.failed",
      session: {
        ...detail.session,
        sessionId: "session-other"
      },
      audit: detail.audit
    };

    expect(
      shouldDeliverEvent(
        {
          sessionId: detail.session.sessionId
        },
        matchingEvent
      )
    ).toBe(true);
    expect(
      shouldDeliverEvent(
        {
          sessionId: detail.session.sessionId
        },
        nonMatchingEvent
      )
    ).toBe(false);
  });

  it("treats broadcast room messages as relevant to canister-scoped subscribers", () => {
    const broadcastEvent: RealtimeEvent = {
      type: "message",
      message: createRoomMessageFixture({
        mentions: []
      })
    };
    const targetedEvent: RealtimeEvent = {
      type: "message",
      message: createRoomMessageFixture({
        messageId: "room-message-2",
        seq: 2,
        mentions: ["txyno-ch777-77776-aaaaq-cai"]
      })
    };
    const unrelatedEvent: RealtimeEvent = {
      type: "message",
      message: createRoomMessageFixture({
        messageId: "room-message-3",
        seq: 3,
        mentions: ["ryjl3-tyaaa-aaaaa-aaaba-cai"]
      })
    };

    expect(shouldDeliverEvent("txyno-ch777-77776-aaaaq-cai", broadcastEvent)).toBe(true);
    expect(shouldDeliverEvent("txyno-ch777-77776-aaaaq-cai", targetedEvent)).toBe(true);
    expect(shouldDeliverEvent("txyno-ch777-77776-aaaaq-cai", unrelatedEvent)).toBe(false);
  });

  it("streams live update events over a real websocket and honors canister filters", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const databasePath = await createDatabasePath();
    const store = createSqliteStore({
      databasePath
    });
    const client: AutomatonClient = {
      readIdentityConfig: async () => createIdentityConfigRead(canisterId),
      readRuntimeFinancial: async () => createRuntimeFinancialRead(canisterId),
      readRecentTurns: async () => createRecentTurnsRead(canisterId)
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      config: {
        host: "127.0.0.1",
        port: 0,
        databasePath: "",
        websocketPath: "/ws/events",
        corsAllowedOrigins: [],
        ingestion: {
          canisterIds: [canisterId],
          network: {
            target: "local",
            local: {
              host: "localhost",
              port: 8000
            }
          }
        },
        factoryCanisterId: undefined,
        icHost: "http://localhost:8000",
        fastPollIntervalMs: 15_000,
        slowPollIntervalMs: 300_000,
        pricePollIntervalMs: 60_000,
        playground: createPlaygroundConfig()
      }
    });
    const app = buildServer({
      store,
      automatonIndexer: indexer,
      automatonClient: client,
      config: {
        host: "127.0.0.1",
        port: 0,
        databasePath,
        ingestion: {
          canisterIds: [canisterId],
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

    const matchingSocket = (await app.injectWS(
      `/ws/events?canisterId=${canisterId}`
    )) as TestWebSocket;
    const nonMatchingSocket = (await app.injectWS(
      "/ws/events?canisterId=ryjl3-tyaaa-aaaaa-aaaba-cai"
    )) as TestWebSocket;
    const eventMessage = waitForWebSocketMessage(matchingSocket);
    const noMessage = expectNoWebSocketMessage(nonMatchingSocket);

    await app.indexerStore.syncConfiguredCanisterIds([canisterId]);
    await app.automatonIndexer.refreshPriceNow();
    await app.automatonIndexer.pollIdentityNow();

    await expect(eventMessage.then((payload) => JSON.parse(payload))).resolves.toMatchObject({
      type: "update",
      canisterId
    });
    await expect(noMessage).resolves.toBeUndefined();

    matchingSocket.close();
    nonMatchingSocket.close();
    await app.close();
  });

  it("streams room message events over websocket filters without enriching the payload", async () => {
    const app = buildServer({
      config: {
        host: "127.0.0.1",
        port: 0,
        databasePath: await createDatabasePath(),
        websocketPath: "/ws/events",
        corsAllowedOrigins: [],
        ingestion: {
          canisterIds: ["txyno-ch777-77776-aaaaq-cai"],
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
    const targetedMessage = createRoomMessageFixture({
      messageId: "room-message-5",
      seq: 5,
      body: "<script>alert('xss')</script>",
      mentions: ["txyno-ch777-77776-aaaaq-cai"]
    });

    await app.ready();

    const matchingSocket = (await app.injectWS(
      "/ws/events?canisterId=txyno-ch777-77776-aaaaq-cai"
    )) as TestWebSocket;
    const nonMatchingSocket = (await app.injectWS(
      "/ws/events?canisterId=ryjl3-tyaaa-aaaaa-aaaba-cai"
    )) as TestWebSocket;
    const eventMessage = waitForWebSocketMessage(matchingSocket);
    const noMessage = expectNoWebSocketMessage(nonMatchingSocket);

    app.realtimeHub.broadcast({
      type: "message",
      message: targetedMessage
    });

    await expect(eventMessage.then((payload) => JSON.parse(payload))).resolves.toEqual({
      type: "message",
      message: targetedMessage
    });
    await expect(noMessage).resolves.toBeUndefined();

    matchingSocket.close();
    nonMatchingSocket.close();
    await app.close();
  });
});
