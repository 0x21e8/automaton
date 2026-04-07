import type { FastifyPluginAsync } from "fastify";

export const runRoutes: FastifyPluginAsync = async (fastify) => {
  fastify.get("/api/run", async (_request, reply) => {
    const dashboard = fastify.runController.getDashboard();

    if (dashboard === null) {
      reply.code(404);
      return {
        ok: false,
        error: "No evaluation run is available."
      };
    }

    return dashboard;
  });

  fastify.post("/api/run/stop", async (_request) => {
    const accepted = await fastify.runController.requestStop();
    return {
      ok: true,
      accepted,
      run: fastify.runController.getDashboard()?.run ?? null
    };
  });
};
