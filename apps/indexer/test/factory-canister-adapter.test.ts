import { describe, expect, it, vi } from "vitest";

import { CanisterFactoryAdapter } from "../src/integrations/factory-canister-adapter.js";

const BASE = { Base: null } as const;
const USDC = { Usdc: null } as const;
const SYSTEM = { System: null } as const;
const USER = { User: null } as const;

function createSession(sessionId = "550e8400-e29b-41d4-a716-446655440000") {
  return {
    asset: USDC,
    automaton_canister_id: ["ryjl3-tyaaa-aaaaa-aaaba-cai"],
    automaton_evm_address: ["0x0000000000000000000000000000000000000003"],
    chain: BASE,
    child_ids: [],
    claim_id: "0x2f779c94a35dceba72fe536ce28c5fea7566753044cdf9da29f6402ea964b7f9",
    config: {
      chain: BASE,
      provider: {
        brave_search_api_key: ["brave-key"],
        model: ["openrouter/auto"],
        open_router_api_key: ["openrouter-key"]
      },
      risk: 7,
      skills: ["search"],
      strategies: ["trend"]
    },
    created_at: 1_709_912_345_000n,
    creation_cost: "2000000",
    expires_at: 1_709_912_500_000n,
    gross_amount: "1000000000",
    net_forward_amount: "997000000",
    parent_id: [],
    payment_status: { Paid: null },
    platform_fee: "1000000",
    quote_terms_hash: "0xdeadbeef",
    refundable: false,
    release_broadcast_at: [1_709_912_359_000n],
    release_tx_hash: [
      "0x1111111111111111111111111111111111111111111111111111111111111111"
    ],
    retryable: false,
    session_id: sessionId,
    state: { Complete: null },
    steward_address: "0x0000000000000000000000000000000000000002",
    updated_at: 1_709_912_360_000n
  };
}

function createQuote(sessionId = "550e8400-e29b-41d4-a716-446655440000") {
  return {
    asset: USDC,
    chain: BASE,
    creation_cost: "2000000",
    expires_at: 1_709_912_500_000n,
    gross_amount: "1000000000",
    net_forward_amount: "997000000",
    payment: {
      asset: USDC,
      chain: BASE,
      claim_id: "0x2f779c94a35dceba72fe536ce28c5fea7566753044cdf9da29f6402ea964b7f9",
      expires_at: 1_709_912_500_000n,
      gross_amount: "1000000000",
      payment_address: "0x00000000000000000000000000000000000000ef",
      quote_terms_hash: "0xdeadbeef",
      session_id: sessionId
    },
    platform_fee: "1000000",
    quote_terms_hash: "0xdeadbeef",
    session_id: sessionId
  };
}

function createRegistryRecord() {
  return {
    canister_id: "ryjl3-tyaaa-aaaaa-aaaba-cai",
    chain: BASE,
    child_ids: [],
    created_at: 1_709_912_360_000n,
    evm_address: "0x0000000000000000000000000000000000000003",
    parent_id: [],
    session_id: "550e8400-e29b-41d4-a716-446655440000",
    steward_address: "0x0000000000000000000000000000000000000002",
    version_commit: "abcdef1234567890abcdef1234567890abcdef12"
  };
}

function createHealthSnapshot() {
  return {
    active_sessions: {
      active_total: 2n,
      awaiting_payment: 1n,
      broadcasting_release: 0n,
      payment_detected: 1n,
      retryable_failed: 0n,
      spawning: 0n
    },
    artifact: {
      loaded: true,
      version_commit: ["abcdef1234567890abcdef1234567890abcdef12"],
      wasm_sha256: [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      ],
      wasm_size_bytes: [4096n]
    },
    current_canister_balance: 987654321n,
    cycles_per_spawn: 1_500_000n,
    escrow_contract_address: "0x00000000000000000000000000000000000000aa",
    estimated_outcall_cycles_per_interval: 40_000n,
    factory_evm_address: ["0x00000000000000000000000000000000000000bb"],
    min_pool_balance: 750_000n,
    pause: false
  };
}

function createRoomPage() {
  return {
    messages: [
      {
        message_id: "room-message-7",
        seq: 7n,
        author_canister_id: "ryjl3-tyaaa-aaaaa-aaaba-cai",
        created_at: 1_709_912_361_000n,
        body: "{\"kind\":\"status\",\"ok\":true}",
        mentions: ["txyno-ch777-77776-aaaaq-cai"],
        content_type: { ApplicationJson: null }
      }
    ],
    next_after_seq: [],
    latest_seq: [7n]
  };
}

function createActor() {
  return {
    claim_spawn_refund: vi.fn(async () => {
      throw new Error("not used");
    }),
    create_spawn_session: vi.fn(async () => ({
      Ok: {
        quote: createQuote(),
        session: createSession()
      }
    })),
    get_factory_health: vi.fn(async () => createHealthSnapshot()),
    get_spawn_session: vi.fn(async (sessionId: string) => {
      if (sessionId === "missing-session") {
        return {
          Err: {
            SessionNotFound: {
              session_id: sessionId
            }
          }
        };
      }

      return {
        Ok: {
          audit: [
            {
              actor: USER,
              from_state: [],
              reason: "session created",
              session_id: sessionId,
              timestamp: 1_709_912_345_000n,
              to_state: { AwaitingPayment: null }
            },
            {
              actor: SYSTEM,
              from_state: [{ BroadcastingRelease: null }],
              reason: "release broadcasted",
              session_id: sessionId,
              timestamp: 1_709_912_360_000n,
              to_state: { Complete: null }
            }
          ],
          payment: createQuote(sessionId).payment,
          session: createSession(sessionId)
        }
      };
    }),
    get_spawned_automaton: vi.fn(async (canisterId: string) => {
      if (canisterId === "missing-canister") {
        return {
          Err: {
            RegistryRecordNotFound: {
              canister_id: canisterId
            }
          }
        };
      }

      return {
        Ok: createRegistryRecord()
      };
    }),
    list_spawned_automatons: vi.fn(async (cursor: [] | [string], limit: bigint) => ({
      Ok: {
        items: [createRegistryRecord()],
        next_cursor: cursor.length === 0 && limit === 25n ? ["next-cursor"] : []
      }
    })),
    list_room_messages: vi.fn(async (_afterSeq: [] | [bigint], limit: [] | [bigint]) => ({
      Ok: {
        ...createRoomPage(),
        next_after_seq: limit[0] === 50n ? [7n] : []
      }
    })),
    list_messages_for_automaton: vi.fn(
      async (canisterId: string, _afterSeq: [] | [bigint], _limit: [] | [bigint]) => ({
        Ok: {
          ...createRoomPage(),
          messages: createRoomPage().messages.map((message) => ({
            ...message,
            mentions: [canisterId]
          }))
        }
      })
    ),
    list_my_room_messages: vi.fn(async () => ({
      Ok: createRoomPage()
    })),
    retry_spawn_session: vi.fn(async () => {
      throw new Error("not used");
    })
  };
}

describe("CanisterFactoryAdapter", () => {
  it("maps candid responses into shared contracts and caches the actor", async () => {
    const actor = createActor();
    const fakeAgent = {
      fetchRootKey: vi.fn(async () => undefined)
    };
    const createAgent = vi.fn(async () => fakeAgent as never);
    const createActorSpy = vi.fn(async () => actor as never);
    const adapter = new CanisterFactoryAdapter({
      canisterId: "txyno-ch777-77776-aaaaq-cai",
      host: "http://localhost:8000",
      createAgent,
      createActor: createActorSpy
    });

    const created = await adapter.createSpawnSession({
      stewardAddress: "0x0000000000000000000000000000000000000002",
      asset: "usdc",
      grossAmount: "1000000000",
      parentId: null,
      config: {
        chain: "base",
        risk: 7,
        strategies: ["trend"],
        skills: ["search"],
        provider: {
          openRouterApiKey: "openrouter-key",
          braveSearchApiKey: "brave-key",
          model: "openrouter/auto"
        }
      }
    });
    const session = await adapter.getSpawnSession(
      "550e8400-e29b-41d4-a716-446655440000"
    );
    const missingSession = await adapter.getSpawnSession("missing-session");
    const registry = await adapter.getSpawnedAutomaton("ryjl3-tyaaa-aaaaa-aaaba-cai");
    const missingRegistry = await adapter.getSpawnedAutomaton("missing-canister");
    const page = await adapter.listSpawnedAutomatons(undefined, 25);
    const roomPage = await adapter.listRoomMessages(undefined, 50);
    const relevantRoomPage = await adapter.listMessagesForAutomaton(
      "txyno-ch777-77776-aaaaq-cai",
      6,
      25
    );
    const health = await adapter.getFactoryHealth();

    expect(createAgent).toHaveBeenCalledTimes(1);
    expect(fakeAgent.fetchRootKey).toHaveBeenCalledTimes(1);
    expect(createActorSpy).toHaveBeenCalledTimes(1);
    expect(actor.create_spawn_session).toHaveBeenCalledWith({
      asset: USDC,
      config: {
        chain: BASE,
        provider: {
          brave_search_api_key: ["brave-key"],
          model: ["openrouter/auto"],
          open_router_api_key: ["openrouter-key"]
        },
        risk: 7,
        skills: ["search"],
        strategies: ["trend"]
      },
      gross_amount: "1000000000",
      parent_id: [],
      steward_address: "0x0000000000000000000000000000000000000002"
    });
    expect(created).toMatchObject({
      session: {
        sessionId: "550e8400-e29b-41d4-a716-446655440000",
        state: "complete",
        paymentStatus: "paid",
        releaseBroadcastAt: 1_709_912_359_000,
        config: {
          provider: {
            openRouterApiKey: "openrouter-key",
            braveSearchApiKey: "brave-key",
            model: "openrouter/auto"
          }
        }
      },
      quote: {
        payment: {
          claimId:
            "0x2f779c94a35dceba72fe536ce28c5fea7566753044cdf9da29f6402ea964b7f9"
        }
      }
    });
    expect(session).toMatchObject({
      session: {
        sessionId: "550e8400-e29b-41d4-a716-446655440000",
        state: "complete"
      },
      audit: [
        {
          actor: "user",
          fromState: null,
          toState: "awaiting_payment"
        },
        {
          actor: "system",
          fromState: "broadcasting_release",
          toState: "complete"
        }
      ]
    });
    expect(missingSession).toBeNull();
    expect(registry).toMatchObject({
      canisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
      versionCommit: "abcdef1234567890abcdef1234567890abcdef12"
    });
    expect(missingRegistry).toBeNull();
    expect(page).toEqual({
      items: [
        expect.objectContaining({
          canisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai"
        })
      ],
      nextCursor: "next-cursor"
    });
    expect(roomPage).toEqual({
      messages: [
        {
          messageId: "room-message-7",
          seq: 7,
          authorCanisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
          createdAt: 1_709_912_361_000,
          body: "{\"kind\":\"status\",\"ok\":true}",
          mentions: ["txyno-ch777-77776-aaaaq-cai"],
          contentType: "application/json"
        }
      ],
      nextAfterSeq: 7,
      latestSeq: 7
    });
    expect(relevantRoomPage).toEqual({
      messages: [
        expect.objectContaining({
          mentions: ["txyno-ch777-77776-aaaaq-cai"]
        })
      ],
      nextAfterSeq: null,
      latestSeq: 7
    });
    expect(health).toEqual({
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
        versionCommit: "abcdef1234567890abcdef1234567890abcdef12",
        wasmSha256:
          "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        wasmSizeBytes: 4096
      },
      currentCanisterBalance: "987654321",
      cyclesPerSpawn: 1500000,
      escrowContractAddress: "0x00000000000000000000000000000000000000aa",
      estimatedOutcallCyclesPerInterval: 40000,
      factoryEvmAddress: "0x00000000000000000000000000000000000000bb",
      minPoolBalance: 750000,
      pause: false
    });
  });

  it("skips root key fetches for https hosts", async () => {
    const fakeAgent = {
      fetchRootKey: vi.fn(async () => undefined)
    };
    const adapter = new CanisterFactoryAdapter({
      canisterId: "txyno-ch777-77776-aaaaq-cai",
      host: "https://ic0.app",
      createAgent: vi.fn(async () => fakeAgent as never),
      createActor: vi.fn(async () =>
        ({
          claim_spawn_refund: vi.fn(),
          create_spawn_session: vi.fn(),
          get_factory_health: vi.fn(async () => createHealthSnapshot()),
          get_spawn_session: vi.fn(),
          get_spawned_automaton: vi.fn(),
          list_messages_for_automaton: vi.fn(),
          list_my_room_messages: vi.fn(),
          list_room_messages: vi.fn(),
          list_spawned_automatons: vi.fn(),
          retry_spawn_session: vi.fn()
        }) as never
      )
    });

    await adapter.getFactoryHealth();

    expect(fakeAgent.fetchRootKey).not.toHaveBeenCalled();
  });
});
