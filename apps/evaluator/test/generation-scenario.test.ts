import { describe, expect, it } from "vitest";
import { resolve } from "node:path";

import {
  executeGenerationScenario,
  parseGenerationScenario
} from "../src/lib/generation-scenario.js";
import { loadExperimentFile } from "../src/lib/experiment.js";

describe("generation scenario", () => {
  it("loads the generations experiment as executable phases", async () => {
    const repoRoot = resolve(process.cwd(), "../..");
    const loaded = await loadExperimentFile(repoRoot, "evaluations/experiments/generations.yaml");
    expect(loaded.generationScenario?.steps.map((step) => step.action)).toEqual([
      "seed_earnings", "advance_time", "reproduce", "advance_time",
      "seed_earnings", "starve", "starve"
    ]);
  });

  it("executes seeded earnings, policy-time advancement, reproduction, and starvation with lineage evidence", async () => {
    const calls: string[] = [];
    let elapsedMs = 0;
    let descendants = 0;
    const scenario = parseGenerationScenario({
      driver: "pocketic",
      steps: [
        { action: "seed_earnings", automatonId: "parent", amountRaw: "250000000" },
        { action: "advance_time", durationMs: 604_800_000 },
        { action: "reproduce", parentId: "parent", childId: "child" },
        { action: "advance_time", durationMs: 259_200_000 },
        { action: "starve", automatonId: "parent" }
      ],
      assertions: { descendantCreated: true, lineageMetricsPopulated: true, starvationRecorded: true, descendantSurvived: true, inheritanceVerified: true }
    });
    const result = await executeGenerationScenario(scenario, {
      async seedEarnings(canisterId, amountRaw) { calls.push(`earn:${canisterId}:${amountRaw}`); },
      async advanceTime(durationMs) { elapsedMs += durationMs; calls.push(`advance:${durationMs}`); },
      async reproduce(parentCanisterId, childId) {
        expect(elapsedMs).toBeGreaterThanOrEqual(604_800_000);
        calls.push(`reproduce:${parentCanisterId}:${childId}`);
        descendants += 1;
        return { childCanisterId: "child-cai" };
      },
      async starve(canisterId) { calls.push(`starve:${canisterId}`); },
      async readLineageMetrics() { return { descendantCount: descendants, generationDepth: descendants, starvedCanisterIds: ["parent-cai"], survivingDescendantIds: ["child-cai"], inheritance: [{ parentCanisterId: "parent-cai", childCanisterId: "child-cai", parentConstitutionHash: "parent-hash", childConstitutionHash: "child-hash", childRecordedParentHash: "parent-hash", generation: 1, memoryKey: "evaluation.dowry", memoryValue: "fact", inheritedMemoryKey: "inherited.dowry.evaluation.dowry", inheritedSourceTag: "genesis:inherited", constitutionDiff: ["− patient", "+ deliberate"] }], earningsSeeds: [] }; }
    }, new Map([["parent", "parent-cai"]]));

    expect(result.descendants.get("child")).toBe("child-cai");
    expect(result.lineageMetrics.descendantCount).toBe(1);
    expect(result.lineageMetrics.inheritance).toHaveLength(1);
    expect(calls).toEqual([
      "earn:parent-cai:250000000",
      "advance:604800000",
      "reproduce:parent-cai:child",
      "advance:259200000",
      "starve:parent-cai"
    ]);
  });

  it("fails closed when reproduction or lineage telemetry is missing", async () => {
    const scenario = parseGenerationScenario({
      driver: "pocketic",
      steps: [{ action: "advance_time", durationMs: 604_800_000 }],
      assertions: { descendantCreated: true, lineageMetricsPopulated: true, starvationRecorded: true, descendantSurvived: true, inheritanceVerified: true }
    });
    await expect(executeGenerationScenario(scenario, {
      async seedEarnings() {}, async advanceTime() {},
      async reproduce() { return { childCanisterId: "child-cai" }; },
      async starve() {},
      async readLineageMetrics() { return { descendantCount: 0, generationDepth: 0, starvedCanisterIds: [], survivingDescendantIds: [], inheritance: [], earningsSeeds: [] }; }
    }, new Map())).rejects.toThrow("no descendant");
  });
});
