import type { FastifyPluginAsync } from "fastify";
import { buildChronicleDay } from "../lib/chronicle.js";
import { loadSettledSociety } from "../lib/society-indexer.js";

export const chronicleRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.get("/api/chronicle", async (request) => {
    const query = request.query as { date?: string };
    const date = /^\d{4}-\d{2}-\d{2}$/.test(query.date ?? "") ? query.date! : new Date().toISOString().slice(0, 10);
    const generatedAt = Date.now();
    const automatons = (await fastify.indexerStore.listAutomatons()).automatons;
    const society = await loadSettledSociety(fastify);
    const journalPayments = society.paymentEvents.filter((message) => message.messageId.startsWith("journal:"));
    return { days: [buildChronicleDay({ date, generatedAt, automatons, roomMessages: [...society.roomMessages, ...journalPayments], journals: society.journals })], nextBefore: null };
  });
};
