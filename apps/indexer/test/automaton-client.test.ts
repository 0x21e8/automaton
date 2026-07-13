import { AutomatonHttpRequestError } from "@ic-automaton/canister-clients";
import { describe, expect, it, vi } from "vitest";

import { LiveAutomatonClient } from "../src/integrations/automaton-client.js";
import type { IndexerTargetConfig } from "../src/indexer.config.js";

const config: IndexerTargetConfig = {
  canisterIds: [],
  network: {
    target: "local",
    local: { host: "localhost", port: 8000 }
  }
};

function createActor() {
  return {
    get_prompt_layers: vi.fn(async () => []),
    list_skills: vi.fn(async () => []),
    list_strategy_templates: vi.fn(async () => [])
  };
}

function stubClient(statusForGenesis: number) {
  const client = new LiveAutomatonClient(config);
  const actor = createActor();
  const requestJson = vi.fn(async (_canisterId: string, path: string) => {
    if (path === "/api/genesis") {
      throw new AutomatonHttpRequestError("legacy child response", statusForGenesis);
    }

    return {};
  });

  Object.assign(client, {
    getActor: async () => actor,
    requestJson
  });

  return { actor, client, requestJson };
}

describe("LiveAutomatonClient identity reads", () => {
  it("keeps legacy pre-v2 identity reads healthy when /api/genesis is absent", async () => {
    const { client } = stubClient(404);

    await expect(client.readIdentityConfig("legacy-canister")).resolves.toMatchObject({
      canisterId: "legacy-canister",
      genesis: undefined
    });
  });

  it("preserves meaningful non-404 Genesis failures", async () => {
    const { client } = stubClient(500);

    await expect(client.readIdentityConfig("broken-canister")).rejects.toMatchObject({
      status: 500
    });
  });
});
