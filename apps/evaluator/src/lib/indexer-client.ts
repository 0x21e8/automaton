import type {
  AutomatonDetail,
  CreateSpawnSessionRequest,
  CreateSpawnSessionResponse,
  RepositoryStrategyListResponse,
  RoomMessagePage,
  ChronicleFeed,
  SpawnSessionDetail
} from "@ic-automaton/shared";

async function readErrorMessage(response: Response) {
  try {
    const payload = (await response.json()) as {
      error?: string;
      message?: string;
    };

    return payload.error ?? payload.message ?? `Request failed with ${response.status}.`;
  } catch {
    return `Request failed with ${response.status}.`;
  }
}

export class IndexerHttpError extends Error {
  constructor(
    readonly status: number,
    message: string
  ) {
    super(message);
    this.name = "IndexerHttpError";
  }
}

export class IndexerClient {
  constructor(
    private readonly baseUrl: string,
    private readonly fetchImpl: typeof fetch = fetch
  ) {}

  private buildUrl(path: string, query?: Record<string, string | undefined>) {
    const url = new URL(path, this.baseUrl);

    if (query) {
      for (const [key, value] of Object.entries(query)) {
        if (typeof value === "string" && value.trim() !== "") {
          url.searchParams.set(key, value);
        }
      }
    }

    return url;
  }

  async requestJson<T>(
    path: string,
    options: Omit<RequestInit, "body"> & {
      body?: unknown;
      query?: Record<string, string | undefined>;
    } = {}
  ): Promise<T> {
    const headers = new Headers(options.headers);
    headers.set("accept", "application/json");
    let body: string | undefined;

    if (options.body !== undefined) {
      headers.set("content-type", "application/json");
      body = JSON.stringify(options.body);
    }

    const response = await this.fetchImpl(this.buildUrl(path, options.query), {
      ...options,
      headers,
      body
    });

    if (!response.ok) {
      throw new IndexerHttpError(response.status, await readErrorMessage(response));
    }

    return (await response.json()) as T;
  }

  async fetchRepositoryStrategies() {
    return this.requestJson<RepositoryStrategyListResponse>("/api/repository/strategies");
  }

  async createSpawnSession(body: CreateSpawnSessionRequest) {
    return this.requestJson<CreateSpawnSessionResponse>("/api/spawn-sessions", {
      method: "POST",
      body
    });
  }

  async fetchSpawnSession(sessionId: string) {
    return this.requestJson<SpawnSessionDetail>(`/api/spawn-sessions/${sessionId}`);
  }

  async fetchAutomatonDetail(canisterId: string) {
    return this.requestJson<AutomatonDetail>(`/api/automatons/${canisterId}`);
  }

  async fetchRoomMessages(canisterId: string, limit = 20) {
    return this.requestJson<RoomMessagePage>("/api/room/messages", {
      query: {
        canisterId,
        limit: String(limit)
      }
    });
  }

  async fetchChronicle() {
    return this.requestJson<ChronicleFeed>("/api/chronicle");
  }
}

export interface IndexerClientLike {
  fetchRepositoryStrategies(): Promise<RepositoryStrategyListResponse>;
  createSpawnSession(body: CreateSpawnSessionRequest): Promise<CreateSpawnSessionResponse>;
  fetchSpawnSession(sessionId: string): Promise<SpawnSessionDetail>;
  fetchAutomatonDetail(canisterId: string): Promise<AutomatonDetail>;
  fetchRoomMessages(canisterId: string, limit?: number): Promise<RoomMessagePage>;
  fetchChronicle?(): Promise<ChronicleFeed>;
}
