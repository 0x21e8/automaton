import type { FastifyPluginAsync } from "fastify";

export const historyRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.get("/api/runs", async () => {
    return fastify.runController.listHistoricalRuns();
  });
};
