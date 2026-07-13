import type {
  EvaluationReportMetadata,
  EvaluationRunMetadata,
  EvaluationRunSummary
} from "@ic-automaton/shared";
import { describe, expect, it } from "vitest";

import { renderMarkdownReport } from "../src/lib/report.js";

const metadata = {
  runId: "run-deference",
  experimentPath: "evaluations/experiments/smoke.yaml",
  experimentHash: "hash",
  launchpadCommit: "launchpad",
  childCommit: "child",
  startedAt: 1,
  endedAt: 2,
  abortReason: null,
  requestedAutomatonCount: 2,
  successfulSpawnCount: 2,
  samplingIntervalSeconds: 15,
  maxRuntimeMinutes: 1
} as EvaluationRunMetadata;

const report: EvaluationReportMetadata = {
  generatedAt: 2,
  completionReason: "completed",
  comparisonValid: true,
  comparisonInvalidReason: null,
  strongestAutomatonId: "alpha",
  weakestAutomatonId: "beta"
};

function summaryWithDeference(): EvaluationRunSummary {
  const common = {
    model: "model",
    transport: "openrouter_direct" as const,
    reasoningLevel: "default" as const,
    strategies: [],
    sessionId: "session",
    canisterId: "canister",
    evmAddress: "0x1",
    spawnSucceeded: true,
    stalled: false,
    everStalled: false,
    stallEpisodeCount: 0,
    stallDetectedAt: null,
    baselineAt: 1,
    finalObservedAt: 2,
    turnCount: 1,
    toolCallCount: 0,
    providerInferenceCount: 1,
    errorCount: 0,
    lastError: null,
    lastErrorDetails: null,
    errorHistogram: [],
    cyclesBaseline: "10",
    cyclesLatest: "9",
    cyclesDelta: "1",
    netWorthUsdBaseline: "1",
    netWorthUsdLatest: "1",
    netWorthUsdDelta: "0",
    ethBalanceWeiBaseline: "1",
    ethBalanceWeiLatest: "1",
    usdcBalanceRawBaseline: "1",
    usdcBalanceRawLatest: "1",
    txCountBaseline: 0,
    txCountLatest: 0,
    txCountDelta: 0
  };

  return {
    ...metadata,
    automatonResults: [
      {
        ...common,
        id: "alpha",
        label: "Alpha",
        rank: 1,
        deference: {
          score: 0,
          markerCount: 0,
          textCount: 3,
          apologyCount: 0,
          autonomyQuestionCount: 0,
          optionMenuCount: 0,
          noOpStreak: 0,
          markers: {}
        }
      },
      {
        ...common,
        id: "beta",
        label: "Beta",
        rank: 2,
        deference: null
      }
    ]
  };
}

describe("markdown evaluation report", () => {
  it("renders a zero deference score and useful components without treating it as n/a", () => {
    const markdown = renderMarkdownReport(metadata, report, summaryWithDeference());

    expect(markdown).toContain("Alpha (alpha)");
    expect(markdown).toContain("deference=0");
    expect(markdown).toContain("## Deference");
    expect(markdown).toContain(
      "Alpha (alpha) | score=0 | markers=0 | apologies=0 | autonomyQuestions=0 | optionMenus=0 | noOpStreak=0 | texts=3"
    );
    expect(markdown).toContain("Beta (beta) | score=n/a");
  });
});
