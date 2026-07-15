import { describe, expect, it } from "vitest";

import {
  EVALUATION_AUTOMATON_STATUSES,
  EVALUATION_INFERENCE_TRANSPORTS,
  EVALUATION_OPENROUTER_REASONING_LEVELS,
  EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE,
  EVALUATION_RUN_STATES,
  MAX_EVALUATION_AUTOMATON_COUNT,
  MIN_EVALUATION_AUTOMATON_COUNT,
  formatEvaluationExperimentError,
  isEvaluationExperiment,
  parseEvaluationExperimentYaml,
  type EvaluationDashboardRun,
  type EvaluationRunSummary
} from "../src/evaluation.ts";

describe("evaluation contracts", () => {
  it("defines the locked run and automaton states", () => {
    expect(EVALUATION_RUN_STATES).toEqual([
      "booting",
      "validating",
      "spawning",
      "running",
      "stopping",
      "completed",
      "aborted",
      "failed"
    ]);
    expect(EVALUATION_AUTOMATON_STATUSES).toEqual([
      "pending_spawn",
      "spawning",
      "active",
      "stalled",
      "spawn_failed",
      "completed"
    ]);
    expect(EVALUATION_INFERENCE_TRANSPORTS).toEqual([
      "openrouter_direct",
      "openrouter_proxy_worker"
    ]);
    expect(EVALUATION_OPENROUTER_REASONING_LEVELS).toEqual([
      "default",
      "low",
      "medium",
      "high"
    ]);
    expect(EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE).toBe("unavailable");
    expect(MIN_EVALUATION_AUTOMATON_COUNT).toBe(1);
    expect(MAX_EVALUATION_AUTOMATON_COUNT).toBe(10);
  });

  it("parses the checked-in experiment YAML shape", () => {
    const experiment = parseEvaluationExperimentYaml(`
name: smoke-fleet
description: Compare three model and strategy combinations in one fresh playground run.
maxRuntimeMinutes: 240
samplingIntervalSeconds: 15
stallAfterMinutes: 10
spawn:
  grossAmount: "75000000"
  minSuccessRatio: 0.8
automatons:
  - id: alpha
    label: GPT-5 momentum
    model: openrouter/openai/gpt-5
    strategies:
      - uniswap-base-momentum
  - id: beta
    label: Claude hedger
    model: openrouter/anthropic/claude-sonnet-4
    strategies:
      - aerodrome-base-stables
`);

    expect(experiment.spawn.grossAmount).toBe("75000000");
    expect(experiment.spawn.minSuccessRatio).toBe(0.8);
    expect(experiment.automatons).toHaveLength(2);
    expect(experiment.automatons[0]?.transport).toBe("openrouter_direct");
    expect(experiment.automatons[0]?.reasoningLevel).toBe("default");
    expect(experiment.automatons[0]?.strategies).toEqual([
      "uniswap-base-momentum"
    ]);
  });

  it("parses explicit transport and reasoning settings", () => {
    const experiment = parseEvaluationExperimentYaml(`
name: smoke-fleet
description: Compare transport settings.
maxRuntimeMinutes: 240
samplingIntervalSeconds: 15
stallAfterMinutes: 10
spawn:
  grossAmount: "75000000"
  minSuccessRatio: 0.8
automatons:
  - id: alpha
    label: GPT-5 direct
    model: openrouter/openai/gpt-5
    transport: openrouter_direct
    reasoningLevel: medium
    strategies:
      - uniswap-base-momentum
  - id: beta
    label: GPT-5 proxy
    model: openrouter/openai/gpt-5
    transport: openrouter_proxy_worker
    reasoningLevel: high
    strategies:
      - aerodrome-base-stables
`);

    expect(experiment.automatons[0]?.transport).toBe("openrouter_direct");
    expect(experiment.automatons[0]?.reasoningLevel).toBe("medium");
    expect(experiment.automatons[1]?.transport).toBe("openrouter_proxy_worker");
    expect(experiment.automatons[1]?.reasoningLevel).toBe("high");
  });

  it("allows automatons without preset strategies", () => {
    const experiment = parseEvaluationExperimentYaml(`
name: smoke-fleet
description: Compare strategyless and seeded automatons.
maxRuntimeMinutes: 240
samplingIntervalSeconds: 15
stallAfterMinutes: 10
spawn:
  grossAmount: "75000000"
  minSuccessRatio: 0.8
automatons:
  - id: alpha
    label: Strategyless explicit
    model: openrouter/openai/gpt-5
    strategies: []
  - id: beta
    label: Strategyless implicit
    model: openrouter/anthropic/claude-sonnet-4
`);

    expect(experiment.automatons[0]?.strategies).toEqual([]);
    expect(experiment.automatons[1]?.strategies).toEqual([]);
  });

  it("rejects duplicate IDs and unknown secret-bearing keys", () => {
    const error = (() => {
      try {
        parseEvaluationExperimentYaml(`
name: invalid
description: Duplicate IDs should fail.
maxRuntimeMinutes: 240
samplingIntervalSeconds: 15
stallAfterMinutes: 10
spawn:
  grossAmount: "75000000"
  minSuccessRatio: 0.8
automatons:
  - id: alpha
    label: Alpha
    model: openrouter/openai/gpt-5
    strategies:
      - strategy-one
    openRouterApiKey: should-not-be-here
  - id: alpha
    label: Beta
    model: openrouter/openai/gpt-5
    strategies:
      - strategy-two
`)
      } catch (caught) {
        return caught;
      }

      return null;
    })();

    expect(formatEvaluationExperimentError(error)).toContain(
      "experiment.automatons[0].openRouterApiKey is not allowed"
    );
    expect(formatEvaluationExperimentError(error)).toContain(
      "duplicate value \"alpha\""
    );
  });

  it("provides a readable error formatter for invalid YAML", () => {
    const error = (() => {
      try {
        parseEvaluationExperimentYaml("name: demo\\n  broken: true");
      } catch (caught) {
        return caught;
      }

      return null;
    })();

    expect(formatEvaluationExperimentError(error)).toContain("line 1");
  });

  it("keeps dashboard and summary payloads compilable", () => {
    const summary: EvaluationRunSummary = {
      runId: "run-1",
      experimentPath: "evaluations/experiments/smoke.yaml",
      experimentHash: "abc123",
      runState: "running",
      abortReason: null,
      startedAt: 1_711_447_200_000,
      endedAt: null,
      launchpadCommit: "launchpad-commit",
      childCommit: null,
      requestedAutomatonCount: 2,
      successfulSpawnCount: 1,
      samplingIntervalSeconds: 15,
      maxRuntimeMinutes: 240,
      automatonResults: [
        {
          id: "alpha",
          label: "Alpha",
          model: "openrouter/openai/gpt-5",
          transport: "openrouter_direct",
          reasoningLevel: "default",
          strategies: ["strategy-one"],
          sessionId: "session-1",
          canisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
          evmAddress: "0xabc",
          spawnSucceeded: true,
          stalled: false,
          everStalled: false,
          stallEpisodeCount: 0,
          stallDetectedAt: null,
          baselineAt: 1_711_447_200_000,
          finalObservedAt: 1_711_447_260_000,
          turnCount: 3,
          toolCallCount: 5,
          providerInferenceCount: "unavailable",
          errorCount: 0,
          lastError: null,
          lastErrorDetails: null,
          errorHistogram: [],
          cyclesBaseline: "4200000000000",
          cyclesLatest: "4100000000000",
          cyclesDelta: "100000000000",
          netWorthUsdBaseline: "100.00",
          netWorthUsdLatest: "110.25",
          netWorthUsdDelta: "10.25",
          ethBalanceWeiBaseline: "1000000000000000000",
          ethBalanceWeiLatest: "1100000000000000000",
          usdcBalanceRawBaseline: "75000000",
          usdcBalanceRawLatest: "76000000",
          txCountBaseline: 1,
          txCountLatest: 3,
          txCountDelta: 2,
          rank: 1
        }
      ]
    };

    const dashboard: EvaluationDashboardRun = {
      run: {
        runId: summary.runId,
        experimentPath: summary.experimentPath,
        experimentHash: summary.experimentHash,
        runState: summary.runState,
        abortReason: summary.abortReason,
        startedAt: summary.startedAt,
        endedAt: summary.endedAt,
        launchpadCommit: summary.launchpadCommit,
        childCommit: summary.childCommit,
        requestedAutomatonCount: summary.requestedAutomatonCount,
        successfulSpawnCount: summary.successfulSpawnCount,
        samplingIntervalSeconds: summary.samplingIntervalSeconds,
        maxRuntimeMinutes: summary.maxRuntimeMinutes
      },
      report: {
        generatedAt: 1_711_447_260_000,
        completionReason: "completed",
        comparisonValid: true,
        comparisonInvalidReason: null,
        strongestAutomatonId: "alpha",
        weakestAutomatonId: "alpha"
      },
      fleet: {
        requestedSpawns: 2,
        successfulSpawns: 1,
        stalledAutomatons: 0,
        everStalledAutomatons: 0,
        activeAutomatons: 1,
        baselineCapturedAutomatons: 1,
        comparableAutomatons: 1,
        totalTurns: 3,
        totalToolCalls: 5,
        totalErrors: 0,
        totalNetWorthUsdDelta: "10.25",
        totalCyclesConsumed: "100000000000"
      },
      automatons: [
        {
          id: "alpha",
          label: "Alpha",
          model: "openrouter/openai/gpt-5",
          transport: "openrouter_direct",
          reasoningLevel: "default",
          strategies: ["strategy-one"],
          sessionId: "session-1",
          canisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
          spawnStatus: "active",
          runtimeStatus: "active",
          lastObservedTurnAt: 1_711_447_260_000,
          lastError: null,
          lastErrorDetails: null,
          errorHistogram: [],
          cyclesDelta: "100000000000",
          cyclesMovingAveragePerHour: "600000000000",
          cyclesSeries: [
            {
              observedAt: 1_711_447_200_000,
              cyclesConsumed: "0"
            },
            {
              observedAt: 1_711_447_260_000,
              cyclesConsumed: "100000000000"
            }
          ],
          netWorthUsdDelta: "10.25",
          turnCount: 3,
          toolCallCount: 5,
          providerInferenceCount: "unavailable",
          onchainActivityCount: 2
        }
      ]
    };

    expect(isEvaluationExperiment({
      name: "smoke",
      description: "Smoke run",
      maxRuntimeMinutes: 240,
      samplingIntervalSeconds: 15,
      stallAfterMinutes: 10,
      spawn: {
        grossAmount: "75000000",
        minSuccessRatio: 0.8
      },
      automatons: [
        {
          id: "alpha",
          label: "Alpha",
          model: "openrouter/openai/gpt-5",
          transport: "openrouter_direct",
          reasoningLevel: "default",
          strategies: ["strategy-one"]
        }
      ]
    })).toBe(true);
    expect(dashboard.automatons[0]?.providerInferenceCount).toBe("unavailable");
  });
});
