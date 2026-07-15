import { describe, expect, it } from "vitest";
import type { AutomatonDetail } from "@ic-automaton/shared";
import { buildFitnessObservatory, constitutionalDiversity } from "../src/lib/fitness-observatory.js";

function automaton(id: string, constitution: string, parentId: string | null, generation: number): AutomatonDetail {
  return {
    canisterId: id,
    constitution,
    parentId,
    generation,
    constitutionHash: `${id}-hash`,
    constitutionVerification: { status: "verified", expectedHash: `${id}-hash`, computedHash: `${id}-hash` },
    metabolism: { diedAt: null } as AutomatonDetail["metabolism"]
  } as AutomatonDetail;
}

describe("fitness observatory", () => {
  it("labels and computes crude living constitutional dispersion", () => {
    const fleet = [
      automaton("parent", "patient cartographer durable evidence", null, 0),
      automaton("child", "restless cartographer durable evidence", "parent", 1)
    ];
    expect(constitutionalDiversity(fleet)).toBeGreaterThan(0);
    const report = buildFitnessObservatory(fleet, null);
    expect(report.framing).toContain("not population genetics");
    expect(report.lineage).toMatchObject({ descendants: 1, maxGeneration: 1 });
  });

  it("excludes a mismatched public constitution from diversity evidence", () => {
    const parent = automaton("parent", "patient durable evidence", null, 0);
    const child = automaton("child", "restless durable evidence", "parent", 1);
    const baseline = constitutionalDiversity([parent, child]);
    const mismatch = {
      ...automaton("mismatch", "completely unrelated injected narrative", null, 0),
      constitutionVerification: { status: "mismatch", expectedHash: "expected", computedHash: "actual" }
    } as AutomatonDetail;
    expect(constitutionalDiversity([parent, child, mismatch])).toBe(baseline);
  });
});
