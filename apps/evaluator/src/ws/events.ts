import type { EvaluationRunEvent } from "@ic-automaton/shared";
import type { FastifyPluginAsync } from "fastify";

interface EvaluationSocket {
  close(): void;
  on(event: "close" | "error", listener: () => void): void;
  readyState: number;
  send(payload: string): void;
}

export class EvaluationEventHub {
  private readonly clients = new Set<EvaluationSocket>();

  constructor(readonly websocketPath = "/ws/events") {}

  connect(socket: EvaluationSocket) {
    this.clients.add(socket);

    const cleanup = () => {
      this.clients.delete(socket);
    };

    socket.on("close", cleanup);
    socket.on("error", cleanup);
  }

  broadcast(event: EvaluationRunEvent) {
    const payload = JSON.stringify(event);

    for (const socket of this.clients) {
      if (socket.readyState !== 1) {
        this.clients.delete(socket);
        continue;
      }

      socket.send(payload);
    }
  }

  getSnapshot() {
    return {
      websocketPath: this.websocketPath,
      clientCount: this.clients.size,
      supportedEventTypes: [
        "run.updated",
        "automaton.updated",
        "sample.recorded",
        "run.finalized"
      ]
    };
  }

  async close() {
    for (const client of this.clients) {
      client.close();
    }

    this.clients.clear();
  }
}

export const evaluatorRealtimeRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.route({
    method: "GET",
    url: fastify.evaluatorConfig.websocketPath,
    handler: async (_request, reply) => {
      reply.code(426);
      reply.header("connection", "Upgrade");
      reply.header("upgrade", "websocket");

      return {
        ok: false,
        error: "Upgrade Required",
        realtime: fastify.realtimeHub.getSnapshot()
      };
    },
    wsHandler: (socket) => {
      fastify.realtimeHub.connect(socket);
    }
  });
};
