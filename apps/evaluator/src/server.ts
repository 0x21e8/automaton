import websocket from "@fastify/websocket";
import Fastify, {
  type FastifyBaseLogger,
  type FastifyInstance
} from "fastify";
import { fileURLToPath } from "node:url";

import { resolveEvaluatorConfig, type EvaluatorConfigOverrides } from "./config.js";
import { AutomatonClient, type AutomatonClientLike } from "./lib/automaton-client.js";
import { loadEvaluatorEnv, loadRepoEnv } from "./lib/env.js";
import { EvmClient, type EvmClientLike } from "./lib/evm-client.js";
import { ArtifactStore } from "./lib/files.js";
import { IndexerClient, type IndexerClientLike } from "./lib/indexer-client.js";
import { PlaygroundProcessManager, type PlaygroundProcessManagerLike } from "./lib/processes.js";
import { historyRoutes } from "./routes/history.js";
import { runRoutes } from "./routes/run.js";
import "./types.js";
import { RunController } from "./runtime/run-controller.js";
import { dieWellBootstrapEnv, isDieWellExperiment } from "./lib/mortality-assertions.js";
import { EvaluationEventHub, evaluatorRealtimeRoutes } from "./ws/events.js";

export interface BuildServerOptions {
  config?: EvaluatorConfigOverrides;
  env?: NodeJS.ProcessEnv;
  logger?: boolean | FastifyBaseLogger;
  runtimeEnv?: ReturnType<typeof loadEvaluatorEnv>;
  artifactStore?: ArtifactStore;
  processManager?: PlaygroundProcessManagerLike;
  indexerClient?: IndexerClientLike;
  automatonClient?: AutomatonClientLike;
  evmClient?: EvmClientLike;
  realtimeHub?: EvaluationEventHub;
  runController?: RunController;
}

function parseCliArgs(argv: string[]) {
  let experimentPath: string | null = null;

  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--experiment") {
      experimentPath = argv[index + 1] ?? null;
      index += 1;
    }
  }

  return {
    experimentPath
  };
}

export function buildServer(options: BuildServerOptions = {}): FastifyInstance {
  const baseEnv = options.env ?? process.env;
  const initialConfig = resolveEvaluatorConfig(baseEnv, options.config);
  const mergedEnv = loadRepoEnv(initialConfig.repoRoot, baseEnv);
  const app = Fastify({
    logger: options.logger ?? false
  });
  const config = resolveEvaluatorConfig(mergedEnv, options.config);
  if (isDieWellExperiment(config.experimentPath)) {
    Object.assign(mergedEnv, dieWellBootstrapEnv());
  }
  const runtimeEnv = options.runtimeEnv ?? loadEvaluatorEnv(config.repoRoot, mergedEnv);
  const artifactStore = options.artifactStore ?? new ArtifactStore(config.artifactsRoot);
  const processManager =
    options.processManager ?? new PlaygroundProcessManager(config.repoRoot, mergedEnv);
  const indexerClient = options.indexerClient ?? new IndexerClient(config.indexerBaseUrl);
  const automatonClient =
    options.automatonClient ??
    new AutomatonClient(config.localReplicaHost, config.localReplicaPort);
  const evmClient = options.evmClient ?? new EvmClient(config.rpcGatewayUrl);
  const realtimeHub = options.realtimeHub ?? new EvaluationEventHub(config.websocketPath);
  const runController =
    options.runController ??
    new RunController({
      logger: app.log,
      config,
      env: runtimeEnv,
      artifacts: artifactStore,
      processes: processManager,
      indexerClient,
      automatonClient,
      evmClient,
      events: realtimeHub
    });

  app.decorate("evaluatorConfig", config);
  app.decorate("runController", runController);
  app.decorate("realtimeHub", realtimeHub);

  app.register(websocket);

  app.addHook("onRequest", async (request, reply) => {
    const origin = request.headers.origin;

    if (typeof origin === "string" && origin.trim() !== "") {
      reply.header("access-control-allow-origin", origin);
      reply.header("access-control-allow-methods", "GET,POST,OPTIONS");
      reply.header("access-control-allow-headers", "accept,content-type");
      reply.header("vary", "Origin");
    }

    if (request.method === "OPTIONS") {
      reply.code(204).send();
    }
  });

  app.addHook("onReady", async () => {
    void app.runController.startConfiguredRun().catch((error) => {
      app.log.error({ err: error }, "failed to start configured evaluation run");
    });
  });

  app.addHook("onClose", async () => {
    await Promise.all([app.runController.close(), app.realtimeHub.close()]);
  });

  app.get("/health", async () => {
    return {
      ok: true,
      service: "evaluator",
      run: app.runController.getDashboard()?.run ?? null,
      realtime: app.realtimeHub.getSnapshot()
    };
  });

  app.register(runRoutes);
  app.register(historyRoutes);
  app.register(evaluatorRealtimeRoutes);

  return app;
}

export async function start(options: BuildServerOptions = {}) {
  const app = buildServer(options);

  try {
    await app.listen({
      host: app.evaluatorConfig.host,
      port: app.evaluatorConfig.port
    });
  } catch (error) {
    app.log.error(error);
    process.exitCode = 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const cli = parseCliArgs(process.argv.slice(2));
  void start({
    config: {
      experimentPath: cli.experimentPath
    }
  });
}
