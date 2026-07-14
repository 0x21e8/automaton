import type {
  CreateSpawnSessionRequest,
  CreateSpawnSessionResponse,
  RepositoryStrategyGetResponse,
  RepositoryStrategyListResponse,
  RepositoryStrategyRecord,
  RoomMessagePage,
  RefundSpawnResponse,
  RetrySpawnResponse,
  SpawnSessionStatusResponse,
  SpawnedAutomatonRecord,
  SpawnedAutomatonRegistryPage
} from "@ic-automaton/shared";

import { CanisterFactoryAdapter } from "./factory-canister-adapter.js";

export interface FactoryHealthSnapshot {
  activeSessions: {
    activeTotal: number;
    awaitingPayment: number;
    broadcastingRelease: number;
    paymentDetected: number;
    retryableFailed: number;
    spawning: number;
  };
  artifact: {
    loaded: boolean;
    versionCommit: string | null;
    wasmSha256: string | null;
    wasmSizeBytes: number | null;
  };
  currentCanisterBalance: string;
  cyclesPerSpawn: number;
  escrowContractAddress: string;
  estimatedOutcallCyclesPerInterval: number;
  factoryEvmAddress: string | null;
  minPoolBalance: number;
  pause: boolean;
}

export interface FactoryAdapter {
  createSpawnSession(
    request: CreateSpawnSessionRequest
  ): Promise<CreateSpawnSessionResponse>;
  getSpawnSession(sessionId: string): Promise<SpawnSessionStatusResponse | null>;
  retrySpawnSession(sessionId: string): Promise<RetrySpawnResponse>;
  claimSpawnRefund(sessionId: string): Promise<RefundSpawnResponse>;
  listSpawnedAutomatons(
    cursor: string | undefined,
    limit: number
  ): Promise<SpawnedAutomatonRegistryPage>;
  listRepositoryStrategies(): Promise<RepositoryStrategyListResponse>;
  getRepositoryStrategy(strategyId: string): Promise<RepositoryStrategyGetResponse>;
  listRoomMessages(
    afterSeq: number | undefined,
    limit: number
  ): Promise<RoomMessagePage>;
  listMessagesForAutomaton(
    canisterId: string,
    afterSeq: number | undefined,
    limit: number
  ): Promise<RoomMessagePage>;
  getSpawnedAutomaton(canisterId: string): Promise<SpawnedAutomatonRecord | null>;
  getFactoryHealth(): Promise<FactoryHealthSnapshot>;
}

export interface FactorySessionSnapshot extends SpawnSessionStatusResponse {
  registryRecord: SpawnedAutomatonRecord | null;
}

class UnconfiguredFactoryAdapter implements FactoryAdapter {
  async createSpawnSession(): Promise<CreateSpawnSessionResponse> {
    throw new Error("Factory adapter is not configured.");
  }

  async getSpawnSession(): Promise<null> {
    return null;
  }

  async retrySpawnSession(): Promise<RetrySpawnResponse> {
    throw new Error("Factory adapter is not configured.");
  }

  async claimSpawnRefund(): Promise<RefundSpawnResponse> {
    throw new Error("Factory adapter is not configured.");
  }

  async listSpawnedAutomatons(): Promise<SpawnedAutomatonRegistryPage> {
    return {
      items: [],
      nextCursor: null
    };
  }

  async listRepositoryStrategies(): Promise<RepositoryStrategyListResponse> {
    return {
      items: [],
      updatedAt: 0
    };
  }

  async getRepositoryStrategy(): Promise<RepositoryStrategyGetResponse> {
    return {
      item: null,
      updatedAt: 0
    };
  }

  async listRoomMessages(): Promise<RoomMessagePage> {
    return {
      messages: [],
      nextAfterSeq: null,
      latestSeq: null
    };
  }

  async listMessagesForAutomaton(): Promise<RoomMessagePage> {
    return {
      messages: [],
      nextAfterSeq: null,
      latestSeq: null
    };
  }

  async getSpawnedAutomaton(): Promise<null> {
    return null;
  }

  async getFactoryHealth(): Promise<FactoryHealthSnapshot> {
    throw new Error("Factory adapter is not configured.");
  }
}

function redactProviderSecrets(
  config: SpawnSessionStatusResponse["session"]["config"]
): SpawnSessionStatusResponse["session"]["config"] {
  return {
    ...config,
    provider: {
      ...config.provider
    },
    strategies: [...config.strategies],
    skills: [...config.skills]
  };
}

function normalizeSelectedStrategies(
  selectedStrategies: SpawnSessionStatusResponse["session"]["selectedStrategies"]
): SpawnSessionStatusResponse["session"]["selectedStrategies"] {
  return selectedStrategies.map((strategy) => ({
    ...strategy,
    source: {
      ...strategy.source
    }
  }));
}

function normalizeSession(
  session: SpawnSessionStatusResponse["session"]
): SpawnSessionStatusResponse["session"] {
  return {
    ...session,
    childIds: [...session.childIds],
    selectedStrategies: normalizeSelectedStrategies(session.selectedStrategies),
    config: redactProviderSecrets(session.config)
  };
}

function normalizeSessionStatus(
  response: SpawnSessionStatusResponse
): SpawnSessionStatusResponse {
  return {
    session: normalizeSession(response.session),
    payment: {
      ...response.payment
    },
    audit: response.audit.map((entry) => ({ ...entry }))
  };
}

function normalizeCreateSpawnSessionResponse(
  response: CreateSpawnSessionResponse
): CreateSpawnSessionResponse {
  return {
    session: normalizeSession(response.session),
    quote: {
      ...response.quote,
      payment: {
        ...response.quote.payment
      }
    }
  };
}

function normalizeRegistryRecord(
  record: SpawnedAutomatonRecord | null
): SpawnedAutomatonRecord | null {
  if (!record) {
    return null;
  }

  return {
    ...record,
    childIds: [...record.childIds],
    controllers: record.controllers === undefined ? undefined : [...record.controllers]
  };
}

function normalizeRoomMessagePage(page: RoomMessagePage): RoomMessagePage {
  return {
    messages: page.messages.map((message) => ({
      ...message,
      mentions: [...message.mentions]
    })),
    nextAfterSeq: page.nextAfterSeq,
    latestSeq: page.latestSeq
  };
}

function normalizeRepositoryStrategy(
  record: RepositoryStrategyRecord
): RepositoryStrategyRecord {
  return {
    ...record,
    compatibleSpawnChains: [...record.compatibleSpawnChains],
    source: {
      ...record.source
    }
  };
}

export class FactoryClient {
  private readonly adapter: FactoryAdapter;
  private readonly configured: boolean;

  constructor(options: {
    adapter?: FactoryAdapter;
    configured?: boolean;
  } = {}) {
    this.adapter = options.adapter ?? new UnconfiguredFactoryAdapter();
    this.configured = options.configured ?? options.adapter !== undefined;
  }

  static createCanisterBacked(options: {
    canisterId: string;
    host: string;
  }) {
    return new FactoryClient({
      adapter: new CanisterFactoryAdapter(options),
      configured: true
    });
  }

  isConfigured() {
    return this.configured;
  }

  async createSpawnSession(
    request: CreateSpawnSessionRequest
  ): Promise<CreateSpawnSessionResponse> {
    return normalizeCreateSpawnSessionResponse(
      await this.adapter.createSpawnSession(request)
    );
  }

  async getSpawnSession(sessionId: string): Promise<FactorySessionSnapshot | null> {
    const response = await this.adapter.getSpawnSession(sessionId);

    if (!response) {
      return null;
    }

    const normalized = normalizeSessionStatus(response);
    const registryRecord = normalized.session.automatonCanisterId
      ? await this.adapter.getSpawnedAutomaton(normalized.session.automatonCanisterId)
      : null;

    return {
      ...normalized,
      registryRecord: normalizeRegistryRecord(registryRecord)
    };
  }

  async retrySpawnSession(sessionId: string): Promise<RetrySpawnResponse> {
    const response = await this.adapter.retrySpawnSession(sessionId);

    return {
      session: normalizeSession(response.session)
    };
  }

  async claimSpawnRefund(sessionId: string): Promise<RefundSpawnResponse> {
    return this.adapter.claimSpawnRefund(sessionId);
  }

  async listSpawnedAutomatons(
    cursor: string | undefined,
    limit: number
  ): Promise<SpawnedAutomatonRegistryPage> {
    const page = await this.adapter.listSpawnedAutomatons(cursor, limit);

    return {
      items: page.items.map((record) => {
        return {
          ...record,
          childIds: [...record.childIds]
        };
      }),
      nextCursor: page.nextCursor
    };
  }

  async listRepositoryStrategies(): Promise<RepositoryStrategyListResponse> {
    const response = await this.adapter.listRepositoryStrategies();

    return {
      items: response.items.map(normalizeRepositoryStrategy),
      updatedAt: response.updatedAt
    };
  }

  async getRepositoryStrategy(
    strategyId: string
  ): Promise<RepositoryStrategyGetResponse> {
    const response = await this.adapter.getRepositoryStrategy(strategyId);

    return {
      item:
        response.item === null ? null : normalizeRepositoryStrategy(response.item),
      updatedAt: response.updatedAt
    };
  }

  async listRoomMessages(
    afterSeq: number | undefined,
    limit: number
  ): Promise<RoomMessagePage> {
    return normalizeRoomMessagePage(
      await this.adapter.listRoomMessages(afterSeq, limit)
    );
  }

  async listMessagesForAutomaton(
    canisterId: string,
    afterSeq: number | undefined,
    limit: number
  ): Promise<RoomMessagePage> {
    return normalizeRoomMessagePage(
      await this.adapter.listMessagesForAutomaton(canisterId, afterSeq, limit)
    );
  }

  async getSpawnedAutomaton(canisterId: string): Promise<SpawnedAutomatonRecord | null> {
    const record = await this.adapter.getSpawnedAutomaton(canisterId);
    return normalizeRegistryRecord(record);
  }

  async getFactoryHealth(): Promise<FactoryHealthSnapshot | null> {
    if (!this.configured) {
      return null;
    }

    return this.adapter.getFactoryHealth();
  }
}
