import { describe, expect, it } from "vitest";

import {
  AUTOMATON_TIERS,
  MONOLOGUE_ENTRY_CATEGORIES,
  MONOLOGUE_ENTRY_IMPORTANCE,
  MONOLOGUE_ENTRY_TYPES,
  type AutomatonListResponse
} from "../src/automaton.ts";
import {
  CATALOG_ENTRY_STATUSES,
  REPOSITORY_STRATEGY_STATUSES,
  type RepositoryStrategyRecord
} from "../src/catalog.ts";
import {
  AUTOMATON_EVENT_TYPES,
  SPAWN_EVENT_TYPES,
  type RealtimeEvent
} from "../src/events.ts";
import {
  PLAYGROUND_FAUCET_ASSETS,
  type PlaygroundMetadata
} from "../src/playground.ts";
import {
  ROOM_CONTENT_TYPES,
  ROOM_MESSAGE_SCOPES,
  type RoomMessagePage
} from "../src/room.ts";
import {
  deriveClaimId,
  MINIMUM_GROSS_PAYMENT_USD,
  PAYMENT_STATUSES,
  SESSION_AUDIT_ACTORS,
  SPAWN_SESSION_STATES,
  SUPPORTED_SPAWN_ASSETS,
  type SpawnSessionStrategySnapshot,
  type SpawnSessionDetail,
  type SpawnSessionStatusResponse
} from "../src/spawn.ts";

describe("shared contracts", () => {
  it("tracks the locked spawn session lifecycle", () => {
    expect(SPAWN_SESSION_STATES).toEqual([
      "awaiting_payment",
      "payment_detected",
      "spawning",
      "broadcasting_release",
      "complete",
      "failed",
      "expired"
    ]);
  });

  it("keeps the minimum gross payment floor at fifty dollars", () => {
    expect(MINIMUM_GROSS_PAYMENT_USD).toBe(50);
    expect(SUPPORTED_SPAWN_ASSETS).toEqual(["usdc"]);
    expect(PAYMENT_STATUSES).toEqual(["unpaid", "partial", "paid", "refunded"]);
  });

  it("exposes stable automaton, audit, and catalog enums", () => {
    expect(AUTOMATON_TIERS).toEqual([
      "normal",
      "low",
      "critical",
      "out_of_cycles"
    ]);
    expect(MONOLOGUE_ENTRY_TYPES).toEqual(["thought", "action"]);
    expect(MONOLOGUE_ENTRY_CATEGORIES).toEqual([
      "observe",
      "decide",
      "act",
      "message",
      "error"
    ]);
    expect(MONOLOGUE_ENTRY_IMPORTANCE).toEqual([
      "low",
      "medium",
      "high"
    ]);
    expect(SESSION_AUDIT_ACTORS).toEqual(["system", "user", "admin"]);
    expect(CATALOG_ENTRY_STATUSES).toEqual(["available", "coming_soon"]);
    expect(REPOSITORY_STRATEGY_STATUSES).toEqual([
      "active",
      "deprecated",
      "revoked"
    ]);
  });

  it("defines the locked realtime event names", () => {
    expect(AUTOMATON_EVENT_TYPES).toEqual([
      "spawn",
      "update",
      "action",
      "message",
      "monologue",
      "offline"
    ]);
    expect(SPAWN_EVENT_TYPES).toEqual([
      "spawn.session.updated",
      "spawn.session.completed",
      "spawn.session.failed",
      "spawn.session.expired"
    ]);
  });

  it("defines the locked room content and query contracts", () => {
    expect(ROOM_CONTENT_TYPES).toEqual([
      "text/plain",
      "application/json"
    ]);
    expect(ROOM_MESSAGE_SCOPES).toEqual(["all", "relevant"]);

    const page: RoomMessagePage = {
      messages: [
        {
          messageId: "room-message-7",
          seq: 7,
          authorCanisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
          createdAt: 1_710_000_000_000,
          body: "{\"kind\":\"status\",\"ok\":true}",
          mentions: ["txyno-ch777-77776-aaaaq-cai"],
          contentType: "application/json"
        }
      ],
      nextAfterSeq: null,
      latestSeq: 7
    };

    expect(page.messages[0]?.contentType).toBe("application/json");
    expect(page.latestSeq).toBe(7);
  });

  it("defines the playground metadata contract for runtime network discovery", () => {
    const metadata: PlaygroundMetadata = {
      environmentLabel: "Automaton Playground",
      environmentVersion: "2026.03.26+sha.abcdef",
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
          windowSeconds: 43_200,
          maxClaimsPerWallet: 2,
          maxClaimsPerIp: 5
        },
        claimAssetAmounts: [
          {
            asset: "eth",
            amount: "0.25",
            decimals: 18
          },
          {
            asset: "usdc",
            amount: "75",
            decimals: 6
          }
        ]
      },
      reset: {
        lastResetAt: 1_711_447_200_000,
        nextResetAt: 1_711_533_600_000,
        cadenceLabel: "Daily at 10:00 UTC"
      }
    };

    expect(PLAYGROUND_FAUCET_ASSETS).toEqual(["eth", "usdc"]);
    expect(metadata.chain.id).toBe(20_260_326);
    expect(metadata.faucet.claimAssetAmounts).toHaveLength(2);
  });

  it("keeps the shared API shapes compilable for later milestones", () => {
    const sessionId = "550e8400-e29b-41d4-a716-446655440000";
    const claimId = "0x2f779c94a35dceba72fe536ce28c5fea7566753044cdf9da29f6402ea964b7f9";
    const repositoryRecord: RepositoryStrategyRecord = {
      strategyId: "base-aave-usdc-reserve-01",
      name: "Base Aave USDC Reserve",
      description: "Park surplus Base USDC on Aave V3.",
      canonicalChain: "base",
      canonicalChainId: 8_453,
      compatibleSpawnChains: ["base"],
      protocol: "aave-v3",
      primitive: "lend_supply",
      recipeJson: "{\"template_id\":\"base-aave-usdc-reserve-01\"}",
      status: "active",
      source: {
        sourcePath: "docs/strategies/base-aave-usdc-reserve-01/recipe.json",
        sourceCommit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa"
      },
      createdAt: 1_710_000_000_000,
      updatedAt: 1_710_000_000_000,
      deprecatedAt: null,
      revokedAt: null
    };
    const selectedStrategy: SpawnSessionStrategySnapshot = {
      strategyId: repositoryRecord.strategyId,
      sourceStatus: repositoryRecord.status,
      name: repositoryRecord.name,
      description: repositoryRecord.description,
      canonicalChain: repositoryRecord.canonicalChain,
      canonicalChainId: repositoryRecord.canonicalChainId,
      requestedSpawnChain: "base",
      resolvedChainId: 8_453,
      protocol: repositoryRecord.protocol,
      primitive: repositoryRecord.primitive,
      recipeJson: repositoryRecord.recipeJson,
      source: repositoryRecord.source,
      selectedAt: 1_710_000_000_000
    };
    const status: SpawnSessionStatusResponse = {
      session: {
        sessionId,
        claimId,
        stewardAddress: "0xabc",
        chain: "base",
        asset: "usdc",
        grossAmount: "1000000000",
        platformFee: "1000000",
        creationCost: "2000000",
        netForwardAmount: "997000000",
        quoteTermsHash: "0xdeadbeef",
        expiresAt: 1_710_000_000_000,
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
        selectedStrategies: [selectedStrategy],
        config: {
          chain: "base",
          risk: 3,
          strategies: [selectedStrategy.strategyId],
          skills: ["portfolio-reporting"],
          provider: {
            openRouterApiKey: null,
            model: null,
            braveSearchApiKey: null,
            inferenceTransport: "openrouter_direct",
            openRouterReasoningLevel: "default"
          }
        },
        createdAt: 1_710_000_000_000,
        updatedAt: 1_710_000_000_000
      },
      payment: {
        sessionId,
        claimId,
        quoteTermsHash: "0xdeadbeef",
        paymentAddress: "0xfeed",
        chain: "base",
        asset: "usdc",
        grossAmount: "1000000000",
        expiresAt: 1_710_000_000_000
      },
      audit: [
        {
          sessionId,
          timestamp: 1_710_000_000_000,
          fromState: null,
          toState: "awaiting_payment",
          actor: "system",
          reason: "created"
        }
      ]
    };

    expect(status.session.config.strategies).toEqual([
      "base-aave-usdc-reserve-01"
    ]);
    expect(selectedStrategy.resolvedChainId).toBe(8_453);
    const detail: SpawnSessionDetail = {
      ...status,
      registryRecord: {
        canisterId: "rdmx6-jaaaa-aaaaa-aaadq-cai",
        stewardAddress: "0xabc",
        evmAddress: "0x1234",
        chain: "base",
        sessionId,
        parentId: null,
        childIds: [],
        createdAt: 1_710_000_000_010,
        versionCommit: "abcdef1234567890abcdef1234567890abcdef12"
      }
    };

    const listResponse: AutomatonListResponse = {
      automatons: [
        {
          canisterId: "rdmx6-jaaaa-aaaaa-aaadq-cai",
          ethAddress: "0x1234",
          chain: "base",
          chainId: 8453,
          name: "ALPHA-42",
          tier: "normal",
          agentState: "Idle",
          ethBalanceWei: "0x1",
          usdcBalanceRaw: "0x0",
          cyclesBalance: 42,
          netWorthEth: "0.1",
          netWorthUsd: "240.00",
          heartbeatIntervalSeconds: 30,
          steward: {
            address: "0xabc",
            chainId: 8453,
            ensName: null,
            enabled: true
          },
          gridPosition: { x: 4, y: 2 },
          corePatternIndex: 3,
          corePattern: null,
          parentId: null,
          createdAt: 1_710_000_000_000,
          lastTransitionAt: 1_710_000_000_000
        }
      ],
      total: 1,
      prices: {
        ethUsd: 2400.5
      }
    };

    const event: RealtimeEvent = {
      type: "message",
      message: {
        messageId: "room-message-9",
        seq: 9,
        authorCanisterId: "rdmx6-jaaaa-aaaaa-aaadq-cai",
        createdAt: 1_710_000_000_000,
        body: "hello room",
        mentions: [],
        contentType: "text/plain"
      }
    };

    expect(status.audit).toHaveLength(1);
    expect(deriveClaimId(sessionId)).toBe(claimId);
    expect(detail.payment.paymentAddress).toBe("0xfeed");
    expect(listResponse.automatons[0]?.name).toBe("ALPHA-42");
    expect(event.type).toBe("message");
  });
});
