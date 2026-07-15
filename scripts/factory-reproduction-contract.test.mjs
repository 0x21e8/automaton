import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("factory Candid and indexer actor preserve reproduction and lineage fields", async () => {
  const [did, adapter] = await Promise.all([
    readFile(new URL("../backend/factory/factory.did", import.meta.url), "utf8"),
    readFile(new URL("../apps/indexer/src/integrations/factory-canister-adapter.ts", import.meta.url), "utf8")
  ]);
  for (const token of [
    "create_reproduction_session",
    "get_reproduction_policy",
    "get_reproduction_eligibility",
    "SpawnSessionOrigin",
    "memory_dowry",
    "inherited_strategy_stats",
    "parent_constitution_hash",
    "royalty_allocations",
    "generation"
  ]) {
    assert.match(did, new RegExp(token));
    assert.match(adapter, new RegExp(token));
  }
});
