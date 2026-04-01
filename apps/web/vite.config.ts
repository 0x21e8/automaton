/// <reference types="vitest/config" />

import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const indexerBaseUrl = env.VITE_INDEXER_BASE_URL?.trim() || "http://127.0.0.1:3001";

  return {
    plugins: [react()],
    server: {
      proxy: {
        "/api": {
          target: indexerBaseUrl,
          changeOrigin: true
        },
        "/ws": {
          target: indexerBaseUrl,
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
