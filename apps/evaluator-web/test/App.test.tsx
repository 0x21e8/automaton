import type { EvaluationDashboardRun } from "@ic-automaton/shared";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import App from "../src/App";
import type { DisplayEvent } from "../src/components/RecentEvents";

function createDashboard(): EvaluationDashboardRun {
  return {
    run: {
      runId: "run-123",
      experimentPath: "evaluations/experiments/smoke.yaml",
      experimentHash: "abc123",
      runState: "running",
      abortReason: null,
      startedAt: 1_712_345_678_000,
      endedAt: null,
      launchpadCommit: "0123456789abcdef0123456789abcdef01234567",
      childCommit: "fedcba9876543210fedcba9876543210fedcba98",
      requestedAutomatonCount: 2,
      successfulSpawnCount: 1,
      samplingIntervalSeconds: 15,
      maxRuntimeMinutes: 240
    },
    report: {
      generatedAt: 1_712_345_679_000,
      completionReason: "stopped_manually",
      comparisonValid: false,
      comparisonInvalidReason: "Missing baseline capture for successful spawns: alpha.",
      strongestAutomatonId: null,
      weakestAutomatonId: null
    },
    fleet: {
      requestedSpawns: 2,
      successfulSpawns: 1,
      stalledAutomatons: 1,
      everStalledAutomatons: 1,
      activeAutomatons: 0,
      baselineCapturedAutomatons: 0,
      comparableAutomatons: 0,
      totalTurns: 7,
      totalToolCalls: 11,
      totalErrors: 1,
      totalNetWorthUsdDelta: "12.5",
      totalCyclesConsumed: "5000"
    },
    automatons: [
      {
        id: "alpha",
        label: "GPT-5 momentum",
        model: "openrouter/openai/gpt-5",
        transport: "openrouter_direct",
        reasoningLevel: "default",
        strategies: ["uniswap-base-momentum"],
        sessionId: "session-1",
        canisterId: "aaaaa-aa",
        spawnStatus: "active",
        runtimeStatus: "active",
        lastObservedTurnAt: 1_712_345_678_500,
        lastError: null,
        lastErrorDetails: null,
        errorHistogram: [],
        cyclesDelta: "2500",
        cyclesMovingAveragePerHour: "6000",
        cyclesSeries: [
          {
            observedAt: 1_712_345_670_000,
            cyclesConsumed: "0"
          },
          {
            observedAt: 1_712_345_678_500,
            cyclesConsumed: "2500"
          }
        ],
        netWorthUsdDelta: "14.25",
        turnCount: 4,
        toolCallCount: 6,
        providerInferenceCount: "unavailable",
        onchainActivityCount: 2
      },
      {
        id: "beta",
        label: "Claude hedger",
        model: "openrouter/anthropic/claude-sonnet-4",
        transport: "openrouter_proxy_worker",
        reasoningLevel: "medium",
        strategies: ["aerodrome-base-stables"],
        sessionId: "session-2",
        canisterId: "bbbbb-bb",
        spawnStatus: "active",
        runtimeStatus: "stalled",
        lastObservedTurnAt: 1_712_345_678_250,
        lastError: "No new turns for 10m",
        lastErrorDetails: null,
        errorHistogram: [
          {
            source: "scheduler",
            message: "autonomy inference error: openrouter returned status 429",
            count: 3,
            lastObservedAt: 1_712_345_678_250
          },
          {
            source: "turn",
            message: "No new turns for 10m",
            count: 1,
            lastObservedAt: 1_712_345_678_250
          }
        ],
        cyclesDelta: "2500",
        cyclesMovingAveragePerHour: "3000",
        cyclesSeries: [
          {
            observedAt: 1_712_345_670_000,
            cyclesConsumed: "0"
          },
          {
            observedAt: 1_712_345_678_250,
            cyclesConsumed: "2500"
          }
        ],
        netWorthUsdDelta: "-1.75",
        turnCount: 3,
        toolCallCount: 5,
        providerInferenceCount: 2,
        onchainActivityCount: 0
      }
    ]
  };
}

describe("App", () => {
  it("renders operator controls, fleet metrics, automaton signals, and recent events", () => {
    const events: DisplayEvent[] = [
      {
        id: "evt-1",
        type: "sample.recorded",
        runId: "run-123",
        timestamp: 1_712_345_679_100,
        summary: "Sample recorded for alpha",
        payload: {
          automatonId: "alpha"
        }
      }
    ];

    const markup = renderToStaticMarkup(
      <App initialDashboard={createDashboard()} initialEvents={events} />
    );

    expect(markup).toContain("Automaton Evaluation Console");
    expect(markup).toContain("invalid for comparison");
    expect(markup).toContain("Fleet Summary");
    expect(markup).toContain("Requested spawns");
    expect(markup).toContain("Automaton Fleet");
    expect(markup).toContain("Sampling is active. Evidence and derived counters update every 15 seconds.");
    expect(markup).toContain("GPT-5 momentum");
    expect(markup).toContain("Claude hedger");
    expect(markup).toContain("unavailable");
    expect(markup).toContain("Cycles / h MA");
    expect(markup).toContain("Onchain activity");
    expect(markup).toContain("Error histogram");
    expect(markup).toContain("scheduler");
    expect(markup).toContain("x3");
    expect(markup).toContain("Recent Events");
    expect(markup).toContain("Sample recorded for alpha");
    expect(markup).toContain("Stop Run");
  });
});
