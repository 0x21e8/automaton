import { describe, expect, it } from "vitest";
import type { AutomatonSummary } from "@ic-automaton/shared";

import { buildRenderNodes } from "./AutomatonCanvas";

function createAutomaton(
  canisterId: string,
  x: number,
  y: number,
  netWorthUsd: string
): AutomatonSummary {
  return {
    canisterId,
    ethAddress: null,
    chain: "base",
    chainId: 8453,
    name: canisterId,
    tier: "normal",
    agentState: "observing",
    ethBalanceWei: null,
    usdcBalanceRaw: null,
    cyclesBalance: 0,
    netWorthEth: null,
    netWorthUsd,
    heartbeatIntervalSeconds: 45,
    steward: {
      address: "0x0000000000000000000000000000000000000000",
      chainId: 8453,
      ensName: null,
      enabled: true
    },
    gridPosition: { x, y },
    corePatternIndex: 0,
    corePattern: null,
    parentId: null,
    createdAt: 0,
    lastTransitionAt: 0
  };
}

describe("buildRenderNodes", () => {
  it("projects automata into an unconstrained world viewport", () => {
    const nodes = buildRenderNodes(
      [createAutomaton("alpha", 200, 160, "5000")],
      { centerX: 0, centerY: 0, zoom: 1 },
      { width: 1200, height: 800 }
    );

    expect(nodes).toHaveLength(1);
    expect(nodes[0].cx).toBeGreaterThan(2000);
    expect(nodes[0].cy).toBeGreaterThan(1500);
  });

  it("separates automatons that would otherwise overlap on screen", () => {
    const nodes = buildRenderNodes(
      [
        createAutomaton("alpha", 10, 10, "12000"),
        createAutomaton("beta", 10, 10, "12000")
      ],
      { centerX: 110, centerY: 110, zoom: 1 },
      { width: 880, height: 520 }
    );

    expect(nodes).toHaveLength(2);

    const distance = Math.hypot(
      nodes[0].cx - nodes[1].cx,
      nodes[0].cy - nodes[1].cy
    );

    expect(distance).toBeGreaterThanOrEqual(
      nodes[0].radiusPixels + nodes[1].radiusPixels + 11.5
    );
  });
});
