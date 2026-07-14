import type { RoomMessageScope } from "@ic-automaton/shared";
import type { FastifyPluginAsync } from "fastify";
import { buildPaymentGraph, createJsonRpcTransactionVerifier } from "../lib/society.js";
import { loadSettledSociety } from "../lib/society-indexer.js";

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

    const page = await fastify.indexerStore.listRoomMessages({
      afterSeq: normalizeAfterSeq(query.afterSeq),
      limit: normalizeLimit(query.limit),
      canisterId,
      scope
    });
    const all = (await loadSettledSociety(fastify)).roomMessages;
    const byId = new Map(all.map((message) => [message.messageId, message]));
    return { ...page, messages: page.messages.map((message) => byId.get(message.messageId) ?? message) };
  });

  fastify.get("/api/society/payment-graph", async (request) => {
    const query = request.query as { from?: string; to?: string };
    const to = Number.isFinite(Number(query.to)) ? Number(query.to) : Date.now();
    const from = Number.isFinite(Number(query.from)) ? Number(query.from) : to - 7 * 86_400_000;
    const messages = (await loadSettledSociety(fastify)).paymentEvents.filter((message) => message.createdAt >= from && message.createdAt <= to);
    return { from, to, edges: buildPaymentGraph(messages) };
  });

  fastify.get("/api/society/transactions/:txHash", async (request, reply) => {
    const { txHash } = request.params as { txHash: string };
    if (!/^0x[0-9a-fA-F]{64}$/.test(txHash)) {
      reply.code(400);
      return { ok: false, error: "txHash must be a 32-byte hex transaction hash" };
    }
    const verifier = createJsonRpcTransactionVerifier(fastify.indexerConfig.playground.metadata.chain.publicRpcUrl);
    const [transaction, succeeded] = await Promise.all([verifier.getTransaction(txHash), verifier.getReceiptStatus(txHash)]);
    if (!transaction) { reply.code(404); return { ok: false, error: "transaction not found" }; }
    return { ok: true, succeeded, transaction, source: fastify.indexerConfig.playground.metadata.chain.publicRpcUrl };
  });
};
