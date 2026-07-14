import websocket from "@fastify/websocket";
import Fastify, {
  type FastifyBaseLogger,
  type FastifyInstance,
  type FastifyReply
} from "fastify";
import { fileURLToPath } from "node:url";

import { resolveIndexerConfig, type IndexerConfigOverrides } from "./config.js";
import { LiveAutomatonClient, type AutomatonClient } from "./integrations/automaton-client.js";
import { FactoryClient } from "./integrations/factory-client.js";
import {
  createFaucetService,
  type FaucetSeedRunner,
  type FaucetService
} from "./lib/faucet.js";
import {
  AutomatonIndexer,
  FixedEthUsdPriceSource,
  type AutomatonIndexerOptions,
  type EthUsdPriceSource
} from "./polling/automaton-indexer.js";
import { automatonRoutes } from "./routes/automatons.js";
import { faucetRoutes } from "./routes/faucet.js";
import { healthRoutes } from "./routes/health.js";
import { playgroundRoutes } from "./routes/playground.js";
import { realtimeRoutes } from "./routes/realtime.js";
import { repositoryRoutes } from "./routes/repository.js";
import { roomRoutes } from "./routes/room.js";
import { chronicleRoutes } from "./routes/chronicle.js";
import { spawnSessionRoutes } from "./routes/spawn-sessions.js";
import { createSqliteStore, type IndexerStore } from "./store/sqlite.js";
import "./types.js";
import { RealtimeHub } from "./ws/events.js";

const INDEXER_TARGET_CONFIG_PATH = fileURLToPath(
  new URL("./indexer.config.ts", import.meta.url)
);
const INDEXER_INGESTION_OVERRIDE_VARIABLES = [
  "INDEXER_INGESTION_CANISTER_IDS",
  "INDEXER_INGESTION_NETWORK_TARGET",
  "INDEXER_INGESTION_LOCAL_HOST",
  "INDEXER_INGESTION_LOCAL_PORT"
] as const;

function appendVaryHeader(reply: FastifyReply) {
  const current = reply.getHeader("vary");
  const values =
    typeof current === "string"
      ? current
          .split(",")
          .map((value) => value.trim())
          .filter((value) => value.length > 0)
      : [];

  if (!values.includes("Origin")) {
    values.push("Origin");
  }

  reply.header("vary", values.join(", "));
}

export interface BuildServerOptions {
  config?: IndexerConfigOverrides;
  env?: NodeJS.ProcessEnv;
  automatonClient?: AutomatonClient;
  automatonIndexer?: AutomatonIndexer;
  ethUsdPriceSource?: EthUsdPriceSource;
  factoryClient?: FactoryClient;
  faucetSeedRunner?: FaucetSeedRunner;
  faucetService?: FaucetService;
  logger?: boolean | FastifyBaseLogger;
  startPolling?: boolean;
  store?: IndexerStore;
  realtimeHub?: RealtimeHub;
}

export function buildServer(options: BuildServerOptions = {}): FastifyInstance {
  const config = resolveIndexerConfig(options.env ?? process.env, options.config);
  const store =
    options.store ??
    createSqliteStore({
      databasePath: config.databasePath
    });
  const effectiveFactoryClient =
    options.factoryClient ??
    (config.factoryCanisterId
      ? FactoryClient.createCanisterBacked({
          canisterId: config.factoryCanisterId,
          host: config.icHost
        })
      : new FactoryClient());
  const realtimeHub = options.realtimeHub ?? new RealtimeHub(config.websocketPath);
  const automatonClient = options.automatonClient ?? new LiveAutomatonClient(config.ingestion);
  const faucetService =
    options.faucetService ??
    createFaucetService({
      metadata: config.playground.metadata.faucet,
      seedWallet: options.faucetSeedRunner,
      store
    });
  const automatonIndexer =
    options.automatonIndexer ??
    new AutomatonIndexer({
      client: automatonClient,
      config,
      eventPublisher: realtimeHub,
      factoryClient: effectiveFactoryClient,
      faucetService,
      store,
      priceSource: options.ethUsdPriceSource ?? new FixedEthUsdPriceSource()
    } satisfies AutomatonIndexerOptions);
  automatonIndexer.setEventPublisher(realtimeHub);

  const app = Fastify({
    logger: options.logger ?? false
  });

  app.decorate("automatonIndexer", automatonIndexer);
  app.decorate("factoryClient", effectiveFactoryClient);
  app.decorate("startedAt", Date.now());
  app.decorate("indexerConfig", config);
  app.decorate("indexerStore", store);
  app.decorate("faucetService", faucetService);
  app.decorate("realtimeHub", realtimeHub);

  app.register(websocket);

  app.addHook("onRequest", async (request, reply) => {
    const origin = request.headers.origin;
    const isAllowedOrigin =
      typeof origin === "string" && config.corsAllowedOrigins.includes(origin);

    if (isAllowedOrigin) {
      reply.header("access-control-allow-origin", origin);
      reply.header("access-control-allow-methods", "GET,POST,OPTIONS");
      reply.header("access-control-allow-headers", "accept,content-type");
      appendVaryHeader(reply);
    }

    if (request.method === "OPTIONS") {
      if (!isAllowedOrigin) {
        reply.code(403).send({
          ok: false,
          error: "Origin not allowed",
          allowedOrigins: config.corsAllowedOrigins
        });
        return reply;
      }

      reply.code(204).send();
      return reply;
    }
  });

  app.addHook("onReady", async () => {
    await app.indexerStore.initialize();
    await app.indexerStore.syncConfiguredCanisterIds(app.indexerConfig.ingestion.canisterIds);
    await app.automatonIndexer.syncFactoryRegistryNow();

    if (options.startPolling ?? false) {
      app.automatonIndexer.start();
    }
  });

  app.addHook("onClose", async () => {
    await Promise.all([
      app.automatonIndexer.stop(),
      app.realtimeHub.close(),
      app.indexerStore.close()
    ]);
  });

  app.register(healthRoutes);
  app.register(automatonRoutes);
  app.register(playgroundRoutes);
  app.register(faucetRoutes);
  app.register(realtimeRoutes);
  app.register(repositoryRoutes);
  app.register(roomRoutes);
  app.register(chronicleRoutes);
  app.register(spawnSessionRoutes);

  return app;
}

function isInvalidIngestionConfigError(error: unknown): error is Error {
  return error instanceof Error && error.message.startsWith("Invalid indexer ingestion config:");
}

export function formatStartupError(error: unknown) {
  if (isInvalidIngestionConfigError(error)) {
    return [
      "Indexer startup aborted: invalid ingestion target configuration.",
      `Check ${INDEXER_TARGET_CONFIG_PATH} and runtime overrides ${INDEXER_INGESTION_OVERRIDE_VARIABLES.join(", ")}.`,
      error.message
    ].join("\n");
  }

  return `Failed to start indexer: ${error instanceof Error ? error.stack ?? error.message : String(error)}`;
}

export async function start(
  options: BuildServerOptions & {
    stderr?: Pick<NodeJS.WriteStream, "write">;
  } = {}
) {
  let app: FastifyInstance | undefined;
  const stderr = options.stderr ?? process.stderr;

  try {
    app = buildServer({
      ...options,
      startPolling: options.startPolling ?? true
    });
    await app.listen({
      host: app.indexerConfig.host,
      port: app.indexerConfig.port
    });
  } catch (error) {
    if (app) {
      app.log.error(error);
    } else {
      stderr.write(`${formatStartupError(error)}\n`);
    }
    process.exitCode = 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  void start();
}
