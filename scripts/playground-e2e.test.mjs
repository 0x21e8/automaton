import assert from "node:assert/strict";
import test from "node:test";

import {
  createDefaultProviderSecrets,
  createDefaultSpawnConfig,
  createDefaultSpawnSessionRequest
} from "./lib/playground-e2e.mjs";

test("default playground spawn request matches the current CreateSpawnSessionRequest shape", () => {
  const config = createDefaultSpawnConfig();
  const providerSecrets = createDefaultProviderSecrets();
  const request = createDefaultSpawnSessionRequest({
    stewardAddress: "0x0000000000000000000000000000000000000001",
    grossAmount: "50000000"
  });

  assert.deepEqual(config.provider, {
    model: null,
    inferenceTransport: "openrouter_direct",
    openRouterReasoningLevel: "default"
  });
  assert.deepEqual(providerSecrets, {
    openRouterApiKey: null,
    braveSearchApiKey: null
  });
  assert.deepEqual(request, {
    stewardAddress: "0x0000000000000000000000000000000000000001",
    asset: "usdc",
    grossAmount: "50000000",
    config: {
      chain: "base",
      risk: 5,
      strategies: [],
      skills: [],
      provider: {
        model: null,
        inferenceTransport: "openrouter_direct",
        openRouterReasoningLevel: "default"
      }
    },
    providerSecrets: {
      openRouterApiKey: null,
      braveSearchApiKey: null
    },
    parentId: null
  });
  assert.equal("providerSecrets" in request.config, false);
  assert.equal("openRouterApiKey" in request.config.provider, false);
});
