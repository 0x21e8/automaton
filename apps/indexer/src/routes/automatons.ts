import type { FastifyPluginAsync } from "fastify";

import type { AutomatonTier, ChainSlug, SpawnSessionDetail } from "@ic-automaton/shared";
import { loadSettledSociety } from "../lib/society-indexer.js";

const EMPTY_STEWARD_ADDRESS = "0x0000000000000000000000000000000000000000";

function normalizeLimit(value: unknown) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return 50;
  }

  return Math.min(Math.floor(parsed), 100);
}

function normalizeCursor(value: unknown) {
  if (value === undefined) {
    return undefined;
  }

  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
}

function normalizeString(value: unknown) {
  return typeof value === "string" && value.trim().length > 0 ? value.trim() : undefined;
}

function chainIdFromChain(chain: ChainSlug, fallback: number) {
  if (chain === "base") {
    return 8453;
  }

  return fallback;
}

function buildSpawnSelection(
  spawnSession: SpawnSessionDetail | null
) {
  if (spawnSession === null) {
    return null;
  }

  return {
    sessionId: spawnSession.session.sessionId,
    requestedStrategyIds: [...spawnSession.session.config.strategies],
    selectedStrategies: spawnSession.session.selectedStrategies.map((strategy) => ({
      ...strategy,
      source: {
        ...strategy.source
      }
    }))
  };
}

export const automatonRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.get("/api/automatons", async (request) => {
    const query = request.query as {
      steward?: string;
      chain?: ChainSlug;
      tier?: AutomatonTier;
    };

    return fastify.indexerStore.listAutomatons({
      steward: normalizeString(query.steward),
      chain: normalizeString(query.chain) as ChainSlug | undefined,
      tier: normalizeString(query.tier) as AutomatonTier | undefined
    });
  });

  fastify.get("/api/automatons/:canisterId", async (request, reply) => {
    const params = request.params as { canisterId: string };
    const automaton = await fastify.indexerStore.getAutomatonDetail(params.canisterId);

    if (!automaton) {
      reply.code(404);
      return {
        ok: false,
        error: "Automaton not found",
        canisterId: params.canisterId
      };
    }

    const registryRecord =
      await fastify.indexerStore.getSpawnedAutomatonRegistryRecord(params.canisterId);
    let spawnSession =
      registryRecord === null
        ? null
        : await fastify.indexerStore.getSpawnSessionDetail(registryRecord.sessionId);

    if (
      spawnSession === null &&
      registryRecord !== null &&
      fastify.factoryClient.isConfigured()
    ) {
      const liveSession = await fastify.factoryClient.getSpawnSession(
        registryRecord.sessionId
      );

      if (liveSession !== null) {
        const hydratedDetail: SpawnSessionDetail = {
          session: liveSession.session,
          payment: liveSession.payment,
          audit: liveSession.audit,
          registryRecord: liveSession.registryRecord ?? registryRecord
        };

        await fastify.indexerStore.upsertSpawnSession(hydratedDetail);
        spawnSession = hydratedDetail;
      }
    }

    const society = await loadSettledSociety(fastify);
    const settledJournal = society.journals.find((journal) => journal.canisterId === params.canisterId)?.entries;

    return {
      ...automaton,
      ethAddress: automaton.ethAddress ?? registryRecord?.evmAddress ?? null,
      chain: registryRecord?.chain ?? automaton.chain,
      steward:
        registryRecord &&
        (automaton.steward.address === EMPTY_STEWARD_ADDRESS || !automaton.steward.enabled)
          ? {
              ...automaton.steward,
              address: registryRecord.stewardAddress,
              chainId: chainIdFromChain(registryRecord.chain, automaton.steward.chainId),
              enabled: true
            }
          : automaton.steward,
      parentId: automaton.parentId ?? registryRecord?.parentId ?? null,
      childIds:
        automaton.childIds.length > 0 ? automaton.childIds : registryRecord?.childIds ?? [],
      createdAt: registryRecord?.createdAt ?? automaton.createdAt,
      model: automaton.model ?? spawnSession?.session.config.provider.model ?? null,
      journal: settledJournal ?? automaton.journal,
      spawnSelection: buildSpawnSelection(spawnSession)
    };
  });

  fastify.get("/api/automatons/:canisterId/monologue", async (request) => {
    const params = request.params as { canisterId: string };
    const query = request.query as {
      before?: string;
      limit?: string;
    };

    return fastify.indexerStore.listMonologue(params.canisterId, {
      before: normalizeCursor(query.before),
      limit: normalizeLimit(query.limit)
    });
  });

  fastify.get("/api/automatons/:canisterId/journal", async (request) => {
    const params = request.params as { canisterId: string };
    const query = request.query as { before?: string; limit?: string };
    const page = await fastify.indexerStore.listJournal(params.canisterId, {
      before: normalizeCursor(query.before),
      limit: normalizeLimit(query.limit)
    });
    const settled = await loadSettledSociety(fastify);
    const byId = new Map((settled.journals.find((journal) => journal.canisterId === params.canisterId)?.entries ?? []).map((entry) => [entry.id, entry]));
    return { ...page, entries: page.entries.map((entry) => byId.get(entry.id) ?? entry) };
  });
};
