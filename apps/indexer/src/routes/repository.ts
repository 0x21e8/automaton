import type { FastifyPluginAsync } from "fastify";

export const repositoryRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.get("/api/repository/strategies", async (_request, reply) => {
    if (!fastify.factoryClient.isConfigured()) {
      reply.code(503);
      return {
        ok: false,
        error: "Factory client is not configured"
      };
    }

    return fastify.factoryClient.listRepositoryStrategies();
  });

  fastify.get("/api/repository/strategies/:strategyId", async (request, reply) => {
    if (!fastify.factoryClient.isConfigured()) {
      reply.code(503);
      return {
        ok: false,
        error: "Factory client is not configured"
      };
    }

    const params = request.params as { strategyId: string };
    return fastify.factoryClient.getRepositoryStrategy(params.strategyId);
  });
};
