import { IDL } from "@icp-sdk/core/candid";
import { describe, expect, it, vi } from "vitest";

import {
  AUTOMATON_METADATA_METHODS,
  createAutomatonMetadataIdl,
  requestAutomatonJson
} from "../src/index.js";

describe("automaton canister client contract", () => {
  it("constructs the centralized metadata actor IDL", () => {
    const service = createAutomatonMetadataIdl()({ IDL });
    expect(service).toBeDefined();
    expect(AUTOMATON_METADATA_METHODS).toEqual([
      "get_prompt_layers",
      "list_skills",
      "list_strategy_templates"
    ]);
  });

  it("preserves local canisterId routing when resolving an absolute API path", async () => {
    let requestedUrl = "";
    const fetchImpl = vi.fn(async (input: Parameters<typeof fetch>[0]) => {
      requestedUrl = String(input);
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" }
      });
    });

    await requestAutomatonJson<{ ok: boolean }>(
      "http://127.0.0.1:8000/api/snapshot?canisterId=7vs54-wt777-77775-aaajq-cai",
      "/api/snapshot",
      { fetch: fetchImpl as typeof fetch }
    );

    expect(requestedUrl).toBe(
      "http://127.0.0.1:8000/api/snapshot?canisterId=7vs54-wt777-77775-aaajq-cai"
    );
  });
});
