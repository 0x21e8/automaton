import type {
  AutomatonDetail,
  AutomatonListResponse,
  RoomMessagePage,
  RoomMessagesQuery
} from "@ic-automaton/shared";

const INDEXER_BASE_URL = import.meta.env.VITE_INDEXER_BASE_URL?.trim() ?? "";

function buildQueryString(
  query: Record<string, string | undefined> | undefined
): string {
  if (!query) {
    return "";
  }

  const search = new URLSearchParams();

  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value.trim() !== "") {
      search.set(key, value);
    }
  }

  const serialized = search.toString();
  return serialized === "" ? "" : `?${serialized}`;
}

export function buildIndexerUrl(
  path: string,
  query?: Record<string, string | undefined>
): string {
  const serializedQuery = buildQueryString(query);

  if (INDEXER_BASE_URL === "") {
    return `${path}${serializedQuery}`;
  }

  const url = new URL(path, INDEXER_BASE_URL);

  if (serializedQuery !== "") {
    url.search = serializedQuery.slice(1);
  }

  return url.toString();
}

export function buildIndexerWebsocketUrl(
  path: string,
  query?: Record<string, string | undefined>
): string {
  const serializedQuery = buildQueryString(query);
  const baseUrl =
    INDEXER_BASE_URL !== ""
      ? new URL(INDEXER_BASE_URL)
      : typeof window !== "undefined"
        ? new URL(window.location.origin)
        : null;

  if (baseUrl === null) {
    return `${path}${serializedQuery}`;
  }

  const protocol = baseUrl.protocol === "https:" ? "wss:" : "ws:";
  const url = new URL(path, `${protocol}//${baseUrl.host}`);

  if (serializedQuery !== "") {
    url.search = serializedQuery.slice(1);
  }

  return url.toString();
}

async function readErrorMessage(response: Response): Promise<string> {
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

export async function requestIndexerJson<T>(
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

  const response = await fetch(buildIndexerUrl(path, options.query), {
    ...options,
    headers,
    body
  });

  if (!response.ok) {
    throw new Error(await readErrorMessage(response));
  }

  return (await response.json()) as T;
}

export async function fetchAutomatons(options: {
  steward?: string;
  signal?: AbortSignal;
} = {}): Promise<AutomatonListResponse> {
  return requestIndexerJson<AutomatonListResponse>("/api/automatons", {
    query: {
      steward: options.steward
    },
    signal: options.signal
  });
}

export async function fetchAutomatonDetail(
  canisterId: string,
  signal?: AbortSignal
): Promise<AutomatonDetail> {
  return requestIndexerJson<AutomatonDetail>(`/api/automatons/${canisterId}`, {
    signal
  });
}

export async function fetchRoomMessagePage(
  query: RoomMessagesQuery = {},
  signal?: AbortSignal
): Promise<RoomMessagePage> {
  return requestIndexerJson<RoomMessagePage>("/api/room/messages", {
    query: {
      afterSeq: query.afterSeq?.toString(),
      canisterId: query.canisterId,
      limit: query.limit?.toString(),
      scope: query.scope
    },
    signal
  });
}

export async function fetchRoomHistory(signal?: AbortSignal): Promise<RoomMessagePage> {
  const messages: RoomMessagePage["messages"] = [];
  let afterSeq: number | undefined;
  let latestSeq: number | null = null;

  while (true) {
    const page = await fetchRoomMessagePage(
      {
        afterSeq,
        limit: 100
      },
      signal
    );

    messages.push(...page.messages);
    latestSeq = page.latestSeq;

    if (page.nextAfterSeq === null || page.nextAfterSeq === afterSeq) {
      return {
        messages,
        nextAfterSeq: null,
        latestSeq
      };
    }

    afterSeq = page.nextAfterSeq;
  }
}
