import type { RoomMessageScope } from "@ic-automaton/shared";
import type { FastifyPluginAsync } from "fastify";

function normalizeLimit(value: unknown) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return 50;
  }

  return Math.min(Math.floor(parsed), 100);
}

function normalizeAfterSeq(value: unknown) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? Math.floor(parsed) : undefined;
}

function normalizeCanisterId(value: unknown) {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : undefined;
}

function normalizeScope(value: unknown): RoomMessageScope {
  return value === "relevant" ? "relevant" : "all";
}

export const roomRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.get("/api/room/messages", async (request, reply) => {
    const query = request.query as {
      afterSeq?: string;
      limit?: string;
      canisterId?: string;
      scope?: string;
    };
    const scope = normalizeScope(query.scope);
    const canisterId = normalizeCanisterId(query.canisterId);

    if (scope === "relevant" && !canisterId) {
      reply.code(400);
      return {
        ok: false,
        error: "canisterId is required when scope=relevant"
      };
    }

    return fastify.indexerStore.listRoomMessages({
      afterSeq: normalizeAfterSeq(query.afterSeq),
      limit: normalizeLimit(query.limit),
      canisterId,
      scope
    });
  });
};
