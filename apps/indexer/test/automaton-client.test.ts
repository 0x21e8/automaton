import { AutomatonHttpRequestError } from "@ic-automaton/canister-clients";
import { afterEach, describe, expect, it, vi } from "vitest";

import { LiveAutomatonClient } from "../src/integrations/automaton-client.js";
import type { IndexerTargetConfig } from "../src/indexer.config.js";

const config: IndexerTargetConfig = {
  canisterIds: [],
  network: {
    target: "local",
    local: { host: "localhost", port: 8000 }
  }
};

afterEach(() => {
  vi.unstubAllGlobals();
});

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

function stubJournalClient(status: number) {
  const client = new LiveAutomatonClient(config);
  Object.assign(client, {
    requestJson: async () => {
      throw new AutomatonHttpRequestError("journal response", status);
    }
  });
  return client;
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

describe("LiveAutomatonClient journal reads", () => {
  it("treats a missing legacy journal endpoint as an empty journal", async () => {
    await expect(stubJournalClient(404).readJournal("legacy-canister")).resolves.toEqual({
      canisterId: "legacy-canister",
      entries: []
    });
  });

  it("preserves non-404 journal failures", async () => {
    await expect(stubJournalClient(500).readJournal("broken-canister")).rejects.toMatchObject({
      status: 500
    });
  });
});

describe("LiveAutomatonClient local HTTP routing", () => {
  it("keeps the canisterId query on every runtime endpoint for an IP replica host", async () => {
    const requestedUrls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: Parameters<typeof fetch>[0]) => {
        requestedUrls.push(String(input));
        return new Response("{}", {
          status: 200,
          headers: { "content-type": "application/json" }
        });
      })
    );
    const client = new LiveAutomatonClient({
      canisterIds: [],
      network: {
        target: "local",
        local: { host: "127.0.0.1", port: 8000 }
      }
    });

    await client.readRuntimeFinancial("7vs54-wt777-77775-aaajq-cai");

    expect(requestedUrls.sort()).toEqual(
      ["/api/snapshot", "/api/wallet/balance"].map(
        (path) =>
          `http://127.0.0.1:8000${path}?canisterId=7vs54-wt777-77775-aaajq-cai`
      )
    );
  });
});
