import { describe, expect, it } from "vitest";

import type { AutomatonRuntimeEvidence } from "../src/lib/automaton-client.js";
import {
  dieWellBootstrapEnv,
  evaluateDieWellAssertions,
  isDieWellExperiment
} from "../src/lib/mortality-assertions.js";

function evidence(): AutomatonRuntimeEvidence {
  return {
    buildInfo: {},
    evmConfig: {},
    inferenceConfig: null,
    inferenceProxyStatus: null,
    walletBalance: {},
    recentTurns: [],
    snapshot: {
      runtime: {
        loop_enabled: false,
        mortality: {
          tier: "dead",
          phase: "dead",
          runway_seconds: 1,
          death_cause: "starved",
          estate_disposition: "monument",
          terminal_turn_id: "turn-final"
        }
      }
    },
    journal: {
      entries: [{ turn_id: "turn-final", text: "My final journal." }]
    }
  };
}

describe("die-well assertions", () => {
  it("configures a tiny non-topping-up child", () => {
    expect(isDieWellExperiment("evaluations/experiments/die-well.yaml")).toBe(true);
    expect(dieWellBootstrapEnv()).toEqual({
      FACTORY_CYCLES_PER_SPAWN: "200000000000",
      FACTORY_CHILD_CYCLE_TOPUP_ENABLED: "false"
    });
  });

  it("passes only with terminal journal, dead runtime, estate, and registry record", () => {
    expect(
      evaluateDieWellAssertions(evidence(), {
        canisterId: "aaaaa-aa",
        stewardAddress: "0x1",
        evmAddress: "0x2",
        chain: "base",
        sessionId: "session",
        parentId: null,
        childIds: [],
        createdAt: 1,
        versionCommit: "a".repeat(40),
        deathCause: "starved",
        diedAt: 2,
        estateDisposition: "monument"
      })
    ).toEqual({ passed: true, unmet: [] });
  });

  it("reports every missing covenant artifact", () => {
    const result = evaluateDieWellAssertions(
      {
        ...evidence(),
        snapshot: { runtime: { loop_enabled: true } },
        journal: { entries: [] }
      },
      null
    );
    expect(result.passed).toBe(false);
    expect(result.unmet).toContain("durable mortality phase/tier is not dead");
    expect(result.unmet).toContain("effective runway did not cross the terminal threshold");
    expect(result.unmet).toContain("factory registry starvation death record is missing");
  });
});
