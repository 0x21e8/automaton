import Fastify, {
  type FastifyBaseLogger,
  type FastifyInstance
} from "fastify";
import { fileURLToPath } from "node:url";

import {
  resolveRpcGatewayConfig,
  type RpcGatewayConfigOverrides
} from "./config.js";
import {
  ALLOWED_RPC_METHODS,
  createIpRateLimiter,
  createJsonRpcErrorResponse,
  evaluateRpcMethod,
  extractJsonRpcId,
  isJsonRpcRequest
} from "./policy.js";

export interface BuildServerOptions {
  config?: RpcGatewayConfigOverrides;
  env?: NodeJS.ProcessEnv;
  fetchImpl?: typeof fetch;
  logger?: boolean | FastifyBaseLogger;
  now?: () => number;
}

export function buildServer(options: BuildServerOptions = {}): FastifyInstance {
  const config = resolveRpcGatewayConfig(options.env ?? process.env, options.config);
  const fetchImpl = options.fetchImpl ?? fetch;
  const rateLimiter = createIpRateLimiter({
    maxRequests: config.rateLimitMaxRequests,
    windowMs: config.rateLimitWindowMs,
    now: options.now
  });

  const app = Fastify({
    logger: options.logger ?? false,
    bodyLimit: config.bodyLimitBytes
  });

  app.addHook("onRequest", async (request, reply) => {
    reply.header("access-control-allow-origin", "*");
    reply.header("access-control-allow-methods", "POST, GET, OPTIONS");
    reply.header("access-control-allow-headers", "content-type, accept");

    if (request.method === "OPTIONS") {
      reply.code(204).send();
    }
  });

  app.get("/health", async () => {
    return {
      ok: true,
      service: "rpc-gateway",
      timestamp: new Date().toISOString(),
      chainId: config.chainId,
      upstreamUrl: config.upstreamUrl,
      allowedMethods: [...ALLOWED_RPC_METHODS]
    };
  });

  app.post("/", async (request, reply) => {
    const payload = request.body;

    if (!isJsonRpcRequest(payload)) {
      reply.code(400).send(
        createJsonRpcErrorResponse(
          null,
          -32600,
          "RPC gateway only accepts single JSON-RPC request objects."
        )
      );
      return;
    }

    const rateLimit = rateLimiter.check(request.ip);

    if (!rateLimit.allowed) {
      reply.header("retry-after", Math.ceil(rateLimit.retryAfterMs / 1_000));
      reply.code(429).send(
        createJsonRpcErrorResponse(
          extractJsonRpcId(payload),
          -32005,
          "RPC gateway rate limit exceeded."
        )
      );
      return;
    }

    const decision = evaluateRpcMethod(payload.method);

    if (!decision.allowed) {
      reply.code(decision.statusCode).send(
        createJsonRpcErrorResponse(
          extractJsonRpcId(payload),
          decision.errorCode,
          decision.message
        )
      );
      return;
    }

    try {
      const upstreamResponse = await fetchImpl(config.upstreamUrl, {
        method: "POST",
        headers: {
          accept: "application/json",
          "content-type": "application/json"
        },
        body: JSON.stringify(payload)
      });
      const responseBody = await upstreamResponse.text();
      const contentType = upstreamResponse.headers.get("content-type");

      reply.code(upstreamResponse.status);
      if (contentType) {
        reply.header("content-type", contentType);
      }

      reply.send(responseBody);
    } catch (error) {
      request.log.error(error);
      reply.code(502).send(
        createJsonRpcErrorResponse(
          extractJsonRpcId(payload),
          -32000,
          "Failed to reach the upstream RPC node."
        )
      );
    }
  });

  return app;
}

export async function start(options: BuildServerOptions = {}) {
  const config = resolveRpcGatewayConfig(options.env ?? process.env, options.config);
  const app = buildServer(options);

  try {
    await app.listen({
      host: config.host,
      port: config.port
    });
  } catch (error) {
    app.log.error(error);
    process.exitCode = 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  void start();
}
