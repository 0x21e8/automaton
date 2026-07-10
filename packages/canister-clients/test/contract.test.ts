import { IDL } from "@icp-sdk/core/candid";
import { describe, expect, it } from "vitest";

import { AUTOMATON_METADATA_METHODS, createAutomatonMetadataIdl } from "../src/index.js";

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
});
