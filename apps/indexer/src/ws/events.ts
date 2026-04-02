import {
  AUTOMATON_EVENT_TYPES,
  SPAWN_EVENT_TYPES,
  type RealtimeEvent
} from "@ic-automaton/shared";

interface RealtimeSocket {
  close(): void;
  on(event: "close" | "error", listener: () => void): void;
  readyState: number;
  send(payload: string): void;
}

export interface RealtimeFilter {
  canisterId?: string;
  sessionId?: string;
}

interface RealtimeClient {
  filter: RealtimeFilter;
  socket: RealtimeSocket;
}

export interface RealtimeSnapshot {
  websocketPath: string;
  clientCount: number;
  supportedEventTypes: string[];
}

function extractEventCanisterIds(event: RealtimeEvent) {
  switch (event.type) {
    case "spawn":
      return [event.automaton.canisterId];
    case "update":
    case "action":
    case "monologue":
    case "offline":
      return [event.canisterId];
    case "message":
      return [...event.message.mentions];
    case "spawn.session.updated":
    case "spawn.session.completed":
    case "spawn.session.failed":
    case "spawn.session.expired":
      return event.session.automatonCanisterId ? [event.session.automatonCanisterId] : [];
    default:
      return [];
  }
}

function extractEventSessionIds(event: RealtimeEvent) {
  switch (event.type) {
    case "spawn.session.updated":
    case "spawn.session.completed":
    case "spawn.session.failed":
    case "spawn.session.expired":
      return [event.session.sessionId];
    default:
      return [];
  }
}

export function shouldDeliverEvent(
  filter: RealtimeFilter | string | undefined,
  event: RealtimeEvent
) {
  const normalizedFilter =
    typeof filter === "string" || filter === undefined ? { canisterId: filter } : filter;

  if (
    normalizedFilter.canisterId &&
    event.type === "message" &&
    event.message.mentions.length === 0
  ) {
    return true;
  }

  if (
    normalizedFilter.canisterId &&
    !extractEventCanisterIds(event).includes(normalizedFilter.canisterId)
  ) {
    return false;
  }

  if (
    normalizedFilter.sessionId &&
    !extractEventSessionIds(event).includes(normalizedFilter.sessionId)
  ) {
    return false;
  }

  return true;
}

export class RealtimeHub {
  private readonly clients = new Set<RealtimeClient>();

  constructor(readonly websocketPath = "/ws/events") {}

  connect(socket: RealtimeSocket, filter: RealtimeFilter = {}) {
    const client: RealtimeClient = {
      socket,
      filter
    };

    this.clients.add(client);

    const cleanup = () => {
      this.clients.delete(client);
    };

    socket.on("close", cleanup);
    socket.on("error", cleanup);
  }

  async close() {
    for (const client of this.clients) {
      client.socket.close();
    }

    this.clients.clear();
  }

  broadcast(event: RealtimeEvent) {
    const payload = JSON.stringify(event);

    for (const client of this.clients) {
      if (client.socket.readyState !== 1) {
        this.clients.delete(client);
        continue;
      }

      if (!shouldDeliverEvent(client.filter, event)) {
        continue;
      }

      client.socket.send(payload);
    }
  }

  getSnapshot(): RealtimeSnapshot {
    return {
      websocketPath: this.websocketPath,
      clientCount: this.clients.size,
      supportedEventTypes: [
        ...AUTOMATON_EVENT_TYPES,
        ...SPAWN_EVENT_TYPES
      ]
    };
  }
}
