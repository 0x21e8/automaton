/// <reference types="vitest/config" />

import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const evaluatorBaseUrl = env.VITE_EVALUATOR_BASE_URL?.trim() || "http://127.0.0.1:3003";

  return {
    plugins: [react()],
    server: {
      proxy: {
        "/api": {
          target: evaluatorBaseUrl,
          changeOrigin: true
        },
        "/ws": {
          target: evaluatorBaseUrl,
          ws: true,
          changeOrigin: true
        }
      }
    },
    test: {
      environment: "node"
    }
  };
});
