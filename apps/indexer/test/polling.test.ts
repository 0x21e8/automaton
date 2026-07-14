import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import type { IndexerConfig } from "../src/config.js";
import { buildServer } from "../src/server.js";
import type {
  AutomatonClient,
  IdentityConfigRead,
  JournalRead,
  RecentTurnsRead,
  RuntimeFinancialRead
} from "../src/integrations/automaton-client.js";
import {
  AutomatonIndexer,
  FixedEthUsdPriceSource,
  type RealtimeEventPublisher
} from "../src/polling/automaton-indexer.js";
import { createSqliteStore } from "../src/store/sqlite.js";
import {
  createRoomMessageFixture,
  createSpawnSessionDetailFixture,
  createSpawnedAutomatonRecordFixture
} from "./fixtures.js";

const tempPaths: string[] = [];

afterEach(async () => {
  vi.useRealTimers();
  await Promise.all(
    tempPaths.splice(0).map(async (path) => {
      await rm(path, { recursive: true, force: true });
    })
  );
});

async function createDatabasePath() {
  const directory = await mkdtemp(join(tmpdir(), "indexer-polling-"));
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

function createIndexerConfig(
  canisterIds: string[],
  factoryCanisterId?: string
): IndexerConfig {
  return {
    host: "127.0.0.1",
    port: 3001,
    databasePath: "",
    websocketPath: "/ws/events",
    corsAllowedOrigins: [],
    ingestion: {
      canisterIds,
      network: {
        target: "local" as const,
        local: {
          host: "localhost",
          port: 8000
        }
      }
    },
    factoryCanisterId,
    icHost: "http://localhost:8000",
    fastPollIntervalMs: 15_000,
    slowPollIntervalMs: 300_000,
    pricePollIntervalMs: 60_000,
    playground: createPlaygroundConfig()
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
    promptLayers: [
      {
        layer_id: 6,
        is_mutable: true,
        content: "Protect the canister first.",
        updated_at_ns: [1_709_912_345_000_000_000n],
        updated_by_turn: ["turn-0"],
        version: [1]
      }
    ],
    skills: [
      {
        name: "Messaging",
        description: "Exchange packets with sibling automatons.",
        instructions: "Use the inbox carefully.",
        enabled: true,
        mutable: true,
        allowed_canister_calls: []
      }
    ],
    strategies: [
      {
        key: {
          protocol: "Aerodrome",
          primitive: "yield-farming",
          chain_id: 8453n,
          template_id: "velo-usdc"
        },
        status: {
          Active: null
        },
        contract_roles: [],
        actions: [],
        constraints_json: "{}",
        created_at_ns: 1_709_912_345_000_000_000n,
        updated_at_ns: 1_709_912_346_000_000_000n
      }
    ]
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
        usd_per_trillion_cycles: 1.35,
        estimated_freeze_time_ns: 1_710_112_347_000_000_000
      },
      recent_turns: [
        {
          id: "turn-2",
          created_at_ns: 1_709_912_349_000_000_000,
          duration_ms: 1_240,
          state_from: "Idle",
          state_to: "ExecutingActions",
          tool_call_count: 2,
          input_summary: "Rebalance pool positions",
          inner_dialogue: "Rebalancing exposure toward the active LP.",
          error: null
        },
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

function createJournalRead(canisterId: string): JournalRead {
  return {
    canisterId,
    entries: [
      {
        id: 1,
        turn_id: "genesis-1",
        timestamp_ns: 1_709_912_345_000_000_000,
        text: "I begin by watching the evidence.",
        genesis: true
      }
    ]
  };
}

describe("automaton indexer poller", () => {
  it("keeps legacy children healthy when their journal read resolves empty", async () => {
    const canisterId = "legacy-journal-cai";
    const store = createSqliteStore({ databasePath: await createDatabasePath() });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId)),
      readJournal: vi.fn(async () => ({ canisterId, entries: [] }))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      config: createIndexerConfig([canisterId]),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([canisterId]);
    await indexer.pollJournalNow();

    expect(indexer.getSnapshot()).toMatchObject({
      canisters: {
        [canisterId]: {
          lastIndexedJournalCount: 0,
          journal: { successCount: 1, lastError: null }
        }
      }
    });

    await store.close();
  });

  it("hydrates a newly factory-discovered child once on the registry sync path", async () => {
    const canisterId = "registry-only-child-cai";
    const registryRecord = createSpawnedAutomatonRecordFixture({ canisterId });
    const store = createSqliteStore({ databasePath: await createDatabasePath() });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId)),
      readJournal: vi.fn(async () => createJournalRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      factoryClient: {
        isConfigured: () => true,
        getSpawnSession: vi.fn(async () => null),
        listRoomMessages: vi.fn(async () => ({
          messages: [],
          nextAfterSeq: null,
          latestSeq: null
        })),
        listSpawnedAutomatons: vi.fn(async () => ({
          items: [registryRecord],
          nextCursor: null
        }))
      },
      config: createIndexerConfig([], "factory-canister-id"),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([]);
    await indexer.syncFactoryRegistryNow();

    await expect(store.getAutomatonDetail(canisterId)).resolves.toMatchObject({ canisterId });
    await expect(store.listJournal(canisterId, { limit: 50 })).resolves.toMatchObject({
      entries: [{ id: 1, text: "I begin by watching the evidence." }]
    });
    expect(client.readIdentityConfig).toHaveBeenCalledTimes(1);
    expect(client.readRuntimeFinancial).toHaveBeenCalledTimes(1);
    expect(client.readRecentTurns).toHaveBeenCalledTimes(1);
    expect(client.readJournal).toHaveBeenCalledTimes(1);

    await indexer.syncFactoryRegistryNow();

    expect(client.readIdentityConfig).toHaveBeenCalledTimes(1);
    expect(client.readRuntimeFinancial).toHaveBeenCalledTimes(1);
    expect(client.readRecentTurns).toHaveBeenCalledTimes(1);
    expect(client.readJournal).toHaveBeenCalledTimes(1);

    await store.close();
  });

  it("discovers and hydrates post-start births on the fast registry cadence", async () => {
    const canisterId = "post-start-child-cai";
    const registryRecord = createSpawnedAutomatonRecordFixture({ canisterId });
    const store = createSqliteStore({ databasePath: await createDatabasePath() });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId)),
      readJournal: vi.fn(async () => createJournalRead(canisterId))
    };
    let registryReadCount = 0;
    const indexer = new AutomatonIndexer({
      client,
      store,
      factoryClient: {
        isConfigured: () => true,
        getSpawnSession: vi.fn(async () => null),
        listRoomMessages: vi.fn(async () => ({
          messages: [],
          nextAfterSeq: null,
          latestSeq: null
        })),
        listSpawnedAutomatons: vi.fn(async () => ({
          items: registryReadCount++ === 0 ? [] : [registryRecord],
          nextCursor: null
        }))
      },
      config: {
        ...createIndexerConfig([], "factory-canister-id"),
        fastPollIntervalMs: 10,
        slowPollIntervalMs: 1_000,
        pricePollIntervalMs: 1_000
      },
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([]);
    vi.useFakeTimers();
    indexer.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(client.readIdentityConfig).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(10);

    await expect(store.getAutomatonDetail(canisterId)).resolves.toMatchObject({ canisterId });
    await expect(store.listJournal(canisterId, { limit: 50 })).resolves.toMatchObject({
      entries: [{ id: 1 }]
    });
    expect(client.readIdentityConfig).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(10);
    expect(client.readIdentityConfig).toHaveBeenCalledTimes(1);

    await indexer.stop();
    await store.close();
  });

  it("normalizes live reads into sqlite and keeps monologue upserts idempotent", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId)),
      readJournal: vi.fn(async () => createJournalRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      config: createIndexerConfig([canisterId]),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([canisterId]);

    await indexer.refreshPriceNow();
    await indexer.pollIdentityNow();
    await indexer.pollRuntimeNow();
    await indexer.pollMonologueNow();
    await indexer.pollMonologueNow();
    await indexer.pollJournalNow();
    await indexer.pollJournalNow();

    await expect(store.listAutomatons()).resolves.toMatchObject({
      total: 1,
      prices: {
        ethUsd: 2_500
      }
    });

    await expect(store.getAutomatonDetail(canisterId)).resolves.toMatchObject({
      canisterId,
      chain: "base",
      tier: "low",
      name: expect.stringMatching(/^[A-Z]+-\d{2}$/),
      canisterUrl: "http://txyno-ch777-77776-aaaaq-cai.localhost:8000",
      explorerUrl: "https://basescan.org/address/0x1111111111111111111111111111111111111111",
      runtime: {
        agentState: "Idle",
        loopEnabled: true,
        heartbeatIntervalSeconds: 150
      },
      financials: {
        cyclesBalance: 4_200_000_000_000,
        liquidCycles: 3_100_000_000_000
      },
      metabolism: {
        burnRateCyclesPerDay: 182_000_000_000,
        runwaySeconds: expect.any(Number),
        lifetimeEarningsUsdcRaw: "0",
        state: "hibernating",
        history: [expect.objectContaining({ liquidCycles: 3_100_000_000_000 })]
      },
      strategies: [
        {
          key: {
            protocol: "Aerodrome",
            primitive: "yield-farming",
            templateId: "velo-usdc",
            chainId: 8453
          },
          status: "active"
        }
      ],
      skills: [
        {
          name: "Messaging",
          enabled: true
        }
      ],
      promptLayers: ["Protect the canister first."],
      monologue: [
        {
          turnId: "turn-2",
          headline: "Rebalance exposure toward the active LP",
          category: "act",
          importance: "high"
        },
        {
          turnId: "turn-1",
          headline: "Check solvency",
          category: "observe",
          importance: "high"
        }
      ],
      journal: [
        {
          id: 1,
          turnId: "genesis-1",
          text: "I begin by watching the evidence.",
          genesis: true
        }
      ]
    });

    await expect(
      store.listMonologue(canisterId, {
        limit: 50
      })
    ).resolves.toMatchObject({
      entries: [
        {
          turnId: "turn-2"
        },
        {
          turnId: "turn-1"
        }
      ],
      hasMore: false
    });

    expect(indexer.getSnapshot()).toMatchObject({
      enabled: false,
      price: {
        ethUsd: 2_500,
        source: "fixed",
        label: "fixed:2500"
      },
      canisters: {
        [canisterId]: {
          currentDetailAvailable: true,
          lastIndexedMonologueCount: 2,
          lastObservedTurnId: "turn-2",
          identity: {
            successCount: 1,
            lastError: null
          },
          runtime: {
            successCount: 1,
            lastError: null
          },
          monologue: {
            successCount: 2,
            lastError: null
          }
        }
      }
    });

    await store.close();
  });

  it("surfaces scheduler tick failures as runtime lastError", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const schedulerError =
      "autonomy inference error: openrouter returned status 429: provider rate-limited";
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => {
        const runtime = createRuntimeFinancialRead(canisterId);
        runtime.snapshot.scheduler = {
          ...runtime.snapshot.scheduler,
          last_tick_error: schedulerError
        };
        runtime.snapshot.runtime = {
          ...runtime.snapshot.runtime,
          last_error: null
        };
        return runtime;
      }),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      config: createIndexerConfig([canisterId]),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([canisterId]);

    await indexer.refreshPriceNow();
    await indexer.pollIdentityNow();
    await indexer.pollRuntimeNow();

    await expect(store.getAutomatonDetail(canisterId)).resolves.toMatchObject({
      canisterId,
      runtime: {
        lastError: schedulerError
      }
    });

    await store.close();
  });

  it("clears a stale runtime lastError after a clean runtime poll", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const schedulerError =
      "autonomy inference error: openrouter returned status 429: provider rate-limited";
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const readRuntimeFinancial = vi
      .fn<AutomatonClient["readRuntimeFinancial"]>()
      .mockImplementationOnce(async () => {
        const runtime = createRuntimeFinancialRead(canisterId);
        runtime.snapshot.scheduler = {
          ...runtime.snapshot.scheduler,
          last_tick_error: schedulerError
        };
        runtime.snapshot.runtime = {
          ...runtime.snapshot.runtime,
          last_error: null
        };
        return runtime;
      })
      .mockImplementationOnce(async () => createRuntimeFinancialRead(canisterId));
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial,
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      config: createIndexerConfig([canisterId]),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([canisterId]);

    await indexer.refreshPriceNow();
    await indexer.pollIdentityNow();
    await indexer.pollRuntimeNow();

    await expect(store.getAutomatonDetail(canisterId)).resolves.toMatchObject({
      canisterId,
      runtime: {
        lastError: schedulerError
      }
    });

    await indexer.pollRuntimeNow();

    await expect(store.getAutomatonDetail(canisterId)).resolves.toMatchObject({
      canisterId,
      runtime: {
        lastError: null
      }
    });

    await store.close();
  });

  it("indexes a factory-discovered canister without a seed config entry", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const registryRecord = createSpawnedAutomatonRecordFixture({ canisterId });
    const spawnSessionDetail = createSpawnSessionDetailFixture({
      registryRecord
    });
    const faucetService = {
      claim: vi.fn(async () => ({
        ok: true as const,
        walletAddress: registryRecord.evmAddress,
        txHashes: {
          eth: "0xfund",
          usdc: "0xmint"
        },
        fundedAmounts: {
          eth: {
            amount: "1",
            decimals: 18,
            wei: "1000000000000000000"
          },
          usdc: {
            amount: "250",
            decimals: 6,
            raw: "250000000"
          }
        },
        balances: {
          ethWei: "1000000000000000000",
          usdcRaw: "250000000"
        }
      }))
    };
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      faucetService,
      factoryClient: {
        isConfigured: () => true,
        getSpawnSession: vi.fn(async (sessionId: string) => ({
          session: {
            ...spawnSessionDetail.session,
            sessionId
          },
          payment: {
            ...spawnSessionDetail.payment,
            sessionId
          },
          audit: spawnSessionDetail.audit,
          registryRecord: {
            ...registryRecord,
            sessionId
          }
        })),
        listRoomMessages: vi.fn(async () => ({
          messages: [],
          nextAfterSeq: null,
          latestSeq: null
        })),
        listSpawnedAutomatons: vi.fn(async () => ({
          items: [registryRecord],
          nextCursor: null
        }))
      },
      config: {
        ...createIndexerConfig([], "factory-canister-id"),
        playground: {
          ...createPlaygroundConfig(),
          metadata: {
            ...createPlaygroundConfig().metadata,
            faucet: {
              ...createPlaygroundConfig().metadata.faucet,
              available: true
            }
          }
        }
      },
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([]);
    await indexer.syncFactoryRegistryNow();
    await indexer.refreshPriceNow();
    await indexer.pollIdentityNow();
    await indexer.pollRuntimeNow();
    await indexer.pollMonologueNow();

    await expect(store.listTrackedCanisterIds()).resolves.toEqual([canisterId]);
    await expect(store.getAutomatonDetail(canisterId)).resolves.toMatchObject({
      canisterId,
      chain: "base",
      createdAt: registryRecord.createdAt,
      model: "openrouter/auto",
      steward: {
        address: registryRecord.stewardAddress
      }
    });
    expect(faucetService.claim).toHaveBeenCalledWith({
      ipAddress: `automaton:${registryRecord.sessionId}`,
      walletAddress: registryRecord.evmAddress
    });
  });

  it("keeps seed canisters indexed alongside factory-discovered canisters", async () => {
    const seedCanisterId = "txyno-ch777-77776-aaaaq-cai";
    const discoveredCanisterId = "ryjl3-tyaaa-aaaaa-aaaba-cai";
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async (canisterId: string) => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async (canisterId: string) =>
        createRuntimeFinancialRead(canisterId)
      ),
      readRecentTurns: vi.fn(async (canisterId: string) => createRecentTurnsRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      factoryClient: {
        isConfigured: () => true,
        getSpawnSession: vi.fn(async () => null),
        listRoomMessages: vi.fn(async () => ({
          messages: [],
          nextAfterSeq: null,
          latestSeq: null
        })),
        listSpawnedAutomatons: vi.fn(async () => ({
          items: [createSpawnedAutomatonRecordFixture({ canisterId: discoveredCanisterId })],
          nextCursor: null
        }))
      },
      config: createIndexerConfig([seedCanisterId], "factory-canister-id"),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([seedCanisterId]);
    await indexer.syncFactoryRegistryNow();
    await indexer.pollIdentityNow();

    await expect(store.listTrackedCanisterIds()).resolves.toEqual([
      discoveredCanisterId,
      seedCanisterId
    ]);
    await expect(store.getAutomatonDetail(seedCanisterId)).resolves.toMatchObject({
      canisterId: seedCanisterId
    });
    await expect(store.getAutomatonDetail(discoveredCanisterId)).resolves.toMatchObject({
      canisterId: discoveredCanisterId
    });
  });

  it("de-duplicates overlapping seed and factory registry ids during polling", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      factoryClient: {
        isConfigured: () => true,
        getSpawnSession: vi.fn(async () => null),
        listRoomMessages: vi.fn(async () => ({
          messages: [],
          nextAfterSeq: null,
          latestSeq: null
        })),
        listSpawnedAutomatons: vi.fn(async () => ({
          items: [createSpawnedAutomatonRecordFixture({ canisterId })],
          nextCursor: null
        }))
      },
      config: createIndexerConfig([canisterId], "factory-canister-id"),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([canisterId]);
    await indexer.syncFactoryRegistryNow();
    await indexer.pollIdentityNow();

    await expect(store.listTrackedCanisterIds()).resolves.toEqual([canisterId]);
    expect(client.readIdentityConfig).toHaveBeenCalledTimes(1);
  });

  it("surfaces live polling debug state in /health", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const client: AutomatonClient = {
      readIdentityConfig: async () => createIdentityConfigRead(canisterId),
      readRuntimeFinancial: async () => createRuntimeFinancialRead(canisterId),
      readRecentTurns: async () => createRecentTurnsRead(canisterId)
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      config: createIndexerConfig([canisterId]),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([canisterId]);
    await indexer.refreshPriceNow();
    await indexer.pollIdentityNow();

    const app = buildServer({
      store,
      automatonIndexer: indexer,
      config: {
        databasePath: "",
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

    const response = await app.inject({
      method: "GET",
      url: "/health"
    });

    expect(response.statusCode).toBe(200);
    expect(response.json()).toMatchObject({
      discovery: {
        mode: "seeds_only",
        seedCanisterIds: [canisterId]
      },
      polling: {
        live: {
          price: {
            ethUsd: 2_500,
            label: "fixed:2500"
          },
          canisters: {
            [canisterId]: {
              currentDetailAvailable: true,
              identity: {
                successCount: 1,
                lastError: null
              }
            }
          }
        }
      }
    });

    await app.close();
  });

  it("emits update and monologue events only when live reads change", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const publisher: RealtimeEventPublisher = {
      broadcast: vi.fn()
    };
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId))
    };
    const indexer = new AutomatonIndexer({
      client,
      store,
      eventPublisher: publisher,
      config: createIndexerConfig([canisterId]),
      priceSource: new FixedEthUsdPriceSource(2_500)
    });

    await store.initialize();
    await store.syncConfiguredCanisterIds([canisterId]);

    await indexer.refreshPriceNow();
    await indexer.pollIdentityNow();
    await indexer.pollRuntimeNow();
    await indexer.pollMonologueNow();
    await indexer.pollIdentityNow();
    await indexer.pollRuntimeNow();
    await indexer.pollMonologueNow();

    expect(publisher.broadcast).toHaveBeenCalledTimes(4);
    expect(publisher.broadcast).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        type: "update",
        canisterId,
        changes: expect.objectContaining({
          canisterId,
          promptLayers: ["Protect the canister first."]
        })
      })
    );
    expect(publisher.broadcast).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({
        type: "update",
        canisterId,
        changes: expect.objectContaining({
          agentState: "Idle",
          cyclesBalance: 4_200_000_000_000,
          netWorthUsd: 14_611.69,
          tier: "low",
          metabolism: expect.objectContaining({
            state: "hibernating",
            runwaySeconds: expect.any(Number)
          })
        })
      })
    );
    expect(publisher.broadcast).toHaveBeenNthCalledWith(3, {
      type: "monologue",
      canisterId,
      entry: expect.objectContaining({
        turnId: "turn-1"
      })
    });
    expect(publisher.broadcast).toHaveBeenNthCalledWith(4, {
      type: "monologue",
      canisterId,
      entry: expect.objectContaining({
        turnId: "turn-2"
      })
    });

    await store.close();
  });

  it("ingests room messages, prunes expired rows, and broadcasts the revised message payload", async () => {
    const canisterId = "txyno-ch777-77776-aaaaq-cai";
    const publisher: RealtimeEventPublisher = {
      broadcast: vi.fn()
    };
    const freshMessage = createRoomMessageFixture({
      messageId: "room-message-17",
      seq: 17,
      authorCanisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
      createdAt: Date.now(),
      mentions: [canisterId],
      contentType: "application/json",
      body: "{\"kind\":\"status\",\"ok\":true}"
    });
    const expiredMessage = createRoomMessageFixture({
      messageId: "room-message-11",
      seq: 11,
      createdAt: Date.now() - 8 * 24 * 60 * 60 * 1000
    });
    const store = createSqliteStore({
      databasePath: await createDatabasePath()
    });
    const client: AutomatonClient = {
      readIdentityConfig: vi.fn(async () => createIdentityConfigRead(canisterId)),
      readRuntimeFinancial: vi.fn(async () => createRuntimeFinancialRead(canisterId)),
      readRecentTurns: vi.fn(async () => createRecentTurnsRead(canisterId))
    };
    const listRoomMessages = vi
      .fn()
      .mockResolvedValueOnce({
        messages: [freshMessage],
        nextAfterSeq: null,
        latestSeq: freshMessage.seq
      })
      .mockResolvedValueOnce({
        messages: [],
        nextAfterSeq: null,
        latestSeq: freshMessage.seq
      });
    const indexer = new AutomatonIndexer({
      client,
      store,
      eventPublisher: publisher,
      factoryClient: {
        isConfigured: () => true,
        listRoomMessages,
        listSpawnedAutomatons: vi.fn(async () => ({
          items: [],
          nextCursor: null
        }))
      },
      config: createIndexerConfig([canisterId], "factory-canister-id")
    });

    await store.initialize();
    await store.upsertRoomMessages([expiredMessage], expiredMessage.seq);
    await indexer.pollRoomNow();
    await indexer.pollRoomNow();

    await expect(store.getLatestRoomMessageSeq()).resolves.toBe(freshMessage.seq);
    await expect(
      store.listRoomMessages({
        limit: 10
      })
    ).resolves.toEqual({
      messages: [freshMessage],
      nextAfterSeq: null,
      latestSeq: freshMessage.seq
    });
    expect(listRoomMessages).toHaveBeenNthCalledWith(1, expiredMessage.seq, 100);
    expect(listRoomMessages).toHaveBeenNthCalledWith(2, freshMessage.seq, 100);
    expect(publisher.broadcast).toHaveBeenCalledTimes(1);
    expect(publisher.broadcast).toHaveBeenCalledWith({
      type: "message",
      message: freshMessage
    });

    await store.close();
  });
});
