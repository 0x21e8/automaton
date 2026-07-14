import type { JournalEntry, RoomMessage, SpawnedAutomatonRecord } from "@ic-automaton/shared";
import type { FastifyInstance } from "fastify";
import { createJsonRpcTransactionVerifier, settleSocietyClaims } from "./society.js";

async function loadRegistry(fastify: FastifyInstance) {
  const records: SpawnedAutomatonRecord[] = [];
  let cursor: string | undefined;
  do {
    const page = await fastify.indexerStore.listSpawnedAutomatonRegistry({ cursor, limit: 100 });
    records.push(...page.items);
    cursor = page.nextCursor ?? undefined;
  } while (cursor && records.length < 5_000);
  return records;
}

async function loadRoomHistory(fastify: FastifyInstance) {
  const messages: RoomMessage[] = [];
  let afterSeq: number | undefined;
  do {
    const page = await fastify.indexerStore.listRoomMessages({ afterSeq, limit: 100, scope: "all" });
    messages.push(...page.messages);
    afterSeq = page.nextAfterSeq ?? undefined;
  } while (afterSeq !== undefined && messages.length < 5_000);
  return messages;
}

async function loadJournals(fastify: FastifyInstance) {
  const automatons = (await fastify.indexerStore.listAutomatons()).automatons;
  return Promise.all(automatons.map(async ({ canisterId }) => {
    const entries: JournalEntry[] = [];
    let before: number | undefined;
    do {
      const page = await fastify.indexerStore.listJournal(canisterId, { before, limit: 100 });
      entries.push(...page.entries);
      before = page.nextCursor ?? undefined;
    } while (before !== undefined && entries.length < 500);
    return { canisterId, entries };
  }));
}

export async function loadSettledSociety(fastify: FastifyInstance) {
  const [registry, roomMessages, journals] = await Promise.all([loadRegistry(fastify), loadRoomHistory(fastify), loadJournals(fastify)]);
  const verifier = createJsonRpcTransactionVerifier(fastify.indexerConfig.playground.metadata.chain.publicRpcUrl);
  return settleSocietyClaims(
    roomMessages,
    journals,
    registry,
    verifier,
    Date.now(),
    new Set(fastify.indexerConfig.society.trustedInboxAddresses),
    new Set(fastify.indexerConfig.society.trustedUsdcAddresses)
  );
}
