import type {
  CreateSpawnSessionRequest,
  SpawnEventType,
  SpawnSessionDetail,
  SpawnedAutomatonRecord
} from "@ic-automaton/shared";
import type { FastifyInstance, FastifyPluginAsync } from "fastify";

import { FaucetError } from "../lib/faucet.js";

function normalizeLimit(value: unknown) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return 50;
  }

  return Math.min(Math.floor(parsed), 100);
}

function normalizeCursor(value: unknown) {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : undefined;
}

function selectSpawnSessionEventType(detail: SpawnSessionDetail): SpawnEventType {
  switch (detail.session.state) {
    case "complete":
      return "spawn.session.completed";
    case "failed":
      return "spawn.session.failed";
    case "expired":
      return "spawn.session.expired";
    default:
      return "spawn.session.updated";
  }
}

function shouldBroadcastSpawnSessionUpdate(
  previous: SpawnSessionDetail | null,
  next: SpawnSessionDetail
) {
  if (!previous) {
    return true;
  }

  return (
    previous.session.updatedAt !== next.session.updatedAt ||
    previous.session.state !== next.session.state ||
    previous.session.paymentStatus !== next.session.paymentStatus ||
    previous.audit.length !== next.audit.length ||
    previous.registryRecord?.canisterId !== next.registryRecord?.canisterId
  );
}

async function resolveSpawnSessionDetail(
  fastify: FastifyInstance,
  sessionId: string
): Promise<SpawnSessionDetail | null> {
  const cached = await fastify.indexerStore.getSpawnSessionDetail(sessionId);
  const factorySnapshot = await fastify.factoryClient.getSpawnSession(sessionId);

  if (!factorySnapshot) {
    return cached;
  }

  const detail: SpawnSessionDetail = {
    session: factorySnapshot.session,
    payment: factorySnapshot.payment,
    audit: factorySnapshot.audit,
    registryRecord: factorySnapshot.registryRecord ?? cached?.registryRecord ?? null
  };

  await fastify.indexerStore.upsertSpawnSession(detail);
  if (detail.session.state === "complete") {
    const walletAddress =
      detail.registryRecord?.evmAddress ?? detail.session.automatonEvmAddress;

    if (walletAddress) {
      try {
        await fastify.faucetService.claim({
          ipAddress: `automaton:${detail.session.sessionId}`,
          walletAddress
        });
      } catch (error) {
        if (!(error instanceof FaucetError && error.statusCode === 429)) {
          fastify.log.warn(
            {
              err: error,
              sessionId: detail.session.sessionId,
              walletAddress
            },
            "automatic playground automaton funding skipped"
          );
        }
      }
    }
  }
  if (shouldBroadcastSpawnSessionUpdate(cached, detail)) {
    fastify.realtimeHub.broadcast({
      type: selectSpawnSessionEventType(detail),
      session: detail.session,
      audit: detail.audit
    });
  }

  return detail;
}

async function resolveRegistryRecord(
  fastify: FastifyInstance,
  canisterId: string
): Promise<SpawnedAutomatonRecord | null> {
  if (fastify.factoryClient.isConfigured()) {
    const record = await fastify.factoryClient.getSpawnedAutomaton(canisterId);
    if (record) {
      await fastify.indexerStore.upsertSpawnedAutomatonRegistry([record]);
      return record;
    }
  }

  return fastify.indexerStore.getSpawnedAutomatonRegistryRecord(canisterId);
}

export const spawnSessionRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.get("/api/spawn-sessions", async (request) => {
    const query = request.query as { limit?: string };
    const limit = normalizeLimit(query.limit);

    return {
      items: await fastify.indexerStore.listSpawnSessionDetails(limit)
    };
  });

  fastify.post("/api/spawn-sessions", async (request, reply) => {
    if (!fastify.factoryClient.isConfigured()) {
      reply.code(503);
      return {
        ok: false,
        error: "Factory client is not configured"
      };
    }

    const body = request.body as CreateSpawnSessionRequest;
    const created = await fastify.factoryClient.createSpawnSession(body);

    // Ensure freshly created sessions are immediately available to routes that
    // enrich automaton details from spawn-session config (for example model id).
    await fastify.indexerStore.upsertSpawnSession({
      session: created.session,
      payment: created.quote.payment,
      audit: [],
      registryRecord: null
    });

    return created;
  });

  fastify.get("/api/spawn-sessions/:sessionId", async (request, reply) => {
    const params = request.params as { sessionId: string };
    const detail = await resolveSpawnSessionDetail(fastify, params.sessionId);

    if (!detail) {
      reply.code(404);
      return {
        ok: false,
        error: "Spawn session not found",
        sessionId: params.sessionId
      };
    }

    return detail;
  });

  fastify.post("/api/spawn-sessions/:sessionId/retry", async (request, reply) => {
    if (!fastify.factoryClient.isConfigured()) {
      reply.code(503);
      return {
        ok: false,
        error: "Factory client is not configured"
      };
    }

    const params = request.params as { sessionId: string };
    const body = request.body as import("@ic-automaton/shared").FactoryStewardExecutionRequest;
    if (body?.command === undefined || !("retrySpawnSession" in body.command) || body.command.retrySpawnSession.sessionId !== params.sessionId) {
      reply.code(400);
      return { ok: false, error: "Retry command session does not match route session" };
    }
    try {
      return await fastify.factoryClient.retrySpawnSession(body);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Factory steward command rejected";
      if (/InvalidStewardProof|nonce|expired|signature|steward/i.test(message)) reply.code(400);
      throw error;
    }
  });

  fastify.post("/api/spawn-sessions/:sessionId/refund", async (request, reply) => {
    if (!fastify.factoryClient.isConfigured()) {
      reply.code(503);
      return {
        ok: false,
        error: "Factory client is not configured"
      };
    }

    const params = request.params as { sessionId: string };
    const body = request.body as import("@ic-automaton/shared").FactoryStewardExecutionRequest;
    if (body?.command === undefined || !("claimSpawnRefund" in body.command) || body.command.claimSpawnRefund.sessionId !== params.sessionId) {
      reply.code(400);
      return { ok: false, error: "Refund command session does not match route session" };
    }
    try {
      return await fastify.factoryClient.claimSpawnRefund(body);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Factory steward command rejected";
      if (/InvalidStewardProof|nonce|expired|signature|steward/i.test(message)) reply.code(400);
      throw error;
    }
  });

  fastify.post("/api/spawn-sessions/:sessionId/steward-command", async (request, reply) => {
    if (!fastify.factoryClient.isConfigured()) {
      reply.code(503);
      return { ok: false, error: "Factory client is not configured" };
    }
    const params = request.params as { sessionId: string };
    const body = request.body as { command: import("@ic-automaton/shared").FactoryStewardCommand };
    if (body?.command === undefined) {
      reply.code(400);
      return { ok: false, error: "Factory steward command is required" };
    }
    const commandSessionId = "retrySpawnSession" in body.command
      ? body.command.retrySpawnSession.sessionId
      : body.command.claimSpawnRefund.sessionId;
    if (commandSessionId !== params.sessionId) {
      reply.code(400);
      return { ok: false, error: "Steward command session does not match route session" };
    }
    return fastify.factoryClient.prepareSpawnStewardCommand(body.command);
  });

  fastify.get("/api/spawned-automatons", async (request) => {
    const query = request.query as {
      cursor?: string;
      limit?: string;
    };
    const cursor = normalizeCursor(query.cursor);
    const limit = normalizeLimit(query.limit);

    if (fastify.factoryClient.isConfigured()) {
      const page = await fastify.factoryClient.listSpawnedAutomatons(cursor, limit);
      await fastify.indexerStore.upsertSpawnedAutomatonRegistry(page.items);
      return page;
    }

    return fastify.indexerStore.listSpawnedAutomatonRegistry({
      cursor,
      limit
    });
  });

  fastify.get("/api/spawned-automatons/:canisterId", async (request, reply) => {
    const params = request.params as { canisterId: string };
    const record = await resolveRegistryRecord(fastify, params.canisterId);

    if (!record) {
      reply.code(404);
      return {
        ok: false,
        error: "Spawned automaton registry record not found",
        canisterId: params.canisterId
      };
    }

    return record;
  });
};
