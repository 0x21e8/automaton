import type {
  EvaluationDashboardAutomaton,
  EvaluationDashboardCyclesPoint,
  EvaluationFleetTotals,
  EvaluationReportMetadata,
  EvaluationCompletionReason,
  EvaluationRunMetadata,
  EvaluationRunSummary
} from "@ic-automaton/shared";

import type { ActiveEvaluationRun, RuntimeAutomatonState } from "../types.js";
import { sortErrorHistogramEntries } from "../runtime/error-histogram.js";

const DASHBOARD_CYCLES_SERIES_POINTS = 24;
const MOVING_AVERAGE_WINDOW_POINTS = 5;

function normalizeNumberString(value: string | null) {
  if (value === null || value.trim() === "") {
    return null;
  }

  return value.trim();
}

function subtractNumericStrings(current: string | null, baseline: string | null) {
  const currentValue = normalizeNumberString(current);
  const baselineValue = normalizeNumberString(baseline);

  if (currentValue === null || baselineValue === null) {
    return null;
  }

  if (/^-?\d+$/u.test(currentValue) && /^-?\d+$/u.test(baselineValue)) {
    return (BigInt(currentValue) - BigInt(baselineValue)).toString();
  }

  const delta = Number(currentValue) - Number(baselineValue);
  return Number.isFinite(delta) ? delta.toString() : null;
}

function compareNullableNumberStringsDescending(left: string | null, right: string | null) {
  const leftValue = left === null ? Number.NEGATIVE_INFINITY : Number(left);
  const rightValue = right === null ? Number.NEGATIVE_INFINITY : Number(right);
  const comparison = rightValue - leftValue;
  return Number.isNaN(comparison) ? 0 : comparison;
}

function hasBaseline(automaton: RuntimeAutomatonState) {
  return automaton.baseline !== null;
}

function hasComparableBaseline(automaton: RuntimeAutomatonState) {
  if (!hasBaseline(automaton)) {
    return false;
  }

  return (
    automaton.baseline?.cycles !== null &&
    automaton.baseline?.netWorthUsd !== null &&
    automaton.baseline?.ethBalanceWei !== null &&
    automaton.baseline?.usdcBalanceRaw !== null &&
    automaton.baseline?.txCount !== null
  );
}

export function assessComparisonValidity(
  run: ActiveEvaluationRun,
  completionReason: EvaluationCompletionReason
) {
  if (!["timed_out", "stopped_manually", "completed"].includes(completionReason)) {
    return {
      valid: false,
      reason: run.metadata.abortReason ?? `Run ended with completion reason "${completionReason}".`
    };
  }

  const successful = [...run.automatons.values()].filter((automaton) => automaton.spawnSucceeded);
  if (successful.length < 2) {
    return {
      valid: false,
      reason: `Only ${successful.length} successful spawn${successful.length === 1 ? "" : "s"}; need at least 2 for comparison.`
    };
  }

  const missingBaseline = successful
    .filter((automaton) => !hasBaseline(automaton))
    .map((automaton) => automaton.config.id);
  if (missingBaseline.length > 0) {
    return {
      valid: false,
      reason: `Missing baseline capture for successful spawns: ${missingBaseline.join(", ")}.`
    };
  }

  const nonComparable = successful
    .filter((automaton) => !hasComparableBaseline(automaton))
    .map((automaton) => automaton.config.id);
  if (nonComparable.length > 0) {
    return {
      valid: false,
      reason: `Baseline telemetry incomplete for successful spawns: ${nonComparable.join(", ")}.`
    };
  }

  return {
    valid: true,
    reason: null
  };
}

function downsampleCyclesSeries(points: EvaluationDashboardCyclesPoint[]) {
  if (points.length <= DASHBOARD_CYCLES_SERIES_POINTS) {
    return [...points];
  }

  const sampled: EvaluationDashboardCyclesPoint[] = [];
  const maxIndex = points.length - 1;
  const lastSampledIndex = DASHBOARD_CYCLES_SERIES_POINTS - 1;

  for (let index = 0; index < DASHBOARD_CYCLES_SERIES_POINTS; index += 1) {
    const pointIndex = Math.round((index * maxIndex) / lastSampledIndex);
    const point = points[pointIndex];

    if (point && sampled.at(-1)?.observedAt !== point.observedAt) {
      sampled.push(point);
    }
  }

  return sampled;
}

function calculateCyclesMovingAveragePerHour(points: EvaluationDashboardCyclesPoint[]) {
  if (points.length === 0) {
    return null;
  }

  if (points.length === 1) {
    return "0";
  }

  const window = points.slice(-MOVING_AVERAGE_WINDOW_POINTS);
  const start = window[0];
  const end = window.at(-1);

  if (!start || !end) {
    return null;
  }

  const durationMs = end.observedAt - start.observedAt;
  if (durationMs <= 0) {
    return "0";
  }

  return ((BigInt(end.cyclesConsumed) - BigInt(start.cyclesConsumed)) * 3_600_000n / BigInt(durationMs)).toString();
}

export function buildFleetTotals(automatons: RuntimeAutomatonState[]): EvaluationFleetTotals {
  let totalTurns = 0;
  let totalToolCalls = 0;
  let totalErrors = 0;
  let successfulSpawns = 0;
  let stalledAutomatons = 0;
  let everStalledAutomatons = 0;
  let activeAutomatons = 0;
  let baselineCapturedAutomatons = 0;
  let comparableAutomatons = 0;
  let totalNetWorthUsdDelta = 0;
  let hasNetWorthUsdDelta = false;
  let totalCyclesConsumed = 0n;
  let hasCyclesConsumed = false;

  for (const automaton of automatons) {
    totalTurns += automaton.turnCount;
    totalToolCalls += automaton.toolCallCount;
    totalErrors += automaton.errorCount;

    if (automaton.spawnSucceeded) {
      successfulSpawns += 1;
    }

    if (automaton.stalled) {
      stalledAutomatons += 1;
    }

    if (automaton.everStalled) {
      everStalledAutomatons += 1;
    }

    if (hasBaseline(automaton)) {
      baselineCapturedAutomatons += 1;
    }

    if (hasComparableBaseline(automaton)) {
      comparableAutomatons += 1;
    }

    if (automaton.spawnSucceeded && automaton.runtimeStatus === "active") {
      activeAutomatons += 1;
    }

    const netWorthDelta = subtractNumericStrings(
      automaton.netWorthUsdLatest,
      automaton.baseline?.netWorthUsd ?? null
    );
    if (netWorthDelta !== null) {
      totalNetWorthUsdDelta += Number(netWorthDelta);
      hasNetWorthUsdDelta = true;
    }

    const cyclesDelta = subtractNumericStrings(
      automaton.baseline?.cycles ?? null,
      automaton.cyclesLatest
    );
    if (cyclesDelta !== null) {
      totalCyclesConsumed += BigInt(cyclesDelta);
      hasCyclesConsumed = true;
    }
  }

  return {
    requestedSpawns: automatons.length,
    successfulSpawns,
    stalledAutomatons,
    everStalledAutomatons,
    activeAutomatons,
    baselineCapturedAutomatons,
    comparableAutomatons,
    totalTurns,
    totalToolCalls,
    totalErrors,
    totalNetWorthUsdDelta: hasNetWorthUsdDelta ? totalNetWorthUsdDelta.toString() : null,
    totalCyclesConsumed: hasCyclesConsumed ? totalCyclesConsumed.toString() : null
  };
}

export function buildDashboardAutomatons(automatons: RuntimeAutomatonState[]): EvaluationDashboardAutomaton[] {
  return automatons.map((automaton) => {
    const cyclesSeries = downsampleCyclesSeries(automaton.cyclesConsumedSeries);

    return {
      id: automaton.config.id,
      label: automaton.config.label,
      model: automaton.config.model,
      transport: automaton.config.transport,
      reasoningLevel: automaton.config.reasoningLevel,
      strategies: [...automaton.config.strategies],
      sessionId: automaton.sessionId,
      canisterId: automaton.canisterId,
      spawnStatus: automaton.spawnStatus,
      runtimeStatus: automaton.runtimeStatus,
      lastObservedTurnAt: automaton.lastObservedTurnAt,
      lastError: automaton.lastError,
      errorHistogram: sortErrorHistogramEntries(automaton.errorHistogram.values()),
      cyclesDelta: subtractNumericStrings(
        automaton.baseline?.cycles ?? null,
        automaton.cyclesLatest
      ),
      cyclesMovingAveragePerHour: calculateCyclesMovingAveragePerHour(
        automaton.cyclesConsumedSeries
      ),
      cyclesSeries,
      netWorthUsdDelta: subtractNumericStrings(
        automaton.netWorthUsdLatest,
        automaton.baseline?.netWorthUsd ?? null
      ),
      turnCount: automaton.turnCount,
      toolCallCount: automaton.toolCallCount,
      providerInferenceCount: automaton.providerInferenceCount,
      onchainActivityCount: automaton.onchainActivityCount,
      deference: automaton.latestSample?.metrics.deference ?? null
    };
  });
}

export function buildSummary(run: ActiveEvaluationRun, comparisonValid: boolean): EvaluationRunSummary {
  const ranked = [...run.automatons.values()].sort((left, right) => {
    if (left.spawnSucceeded !== right.spawnSucceeded) {
      return left.spawnSucceeded ? -1 : 1;
    }

    if (left.stalled !== right.stalled) {
      return left.stalled ? 1 : -1;
    }

    const netWorthComparison = compareNullableNumberStringsDescending(
      subtractNumericStrings(left.netWorthUsdLatest, left.baseline?.netWorthUsd ?? null),
      subtractNumericStrings(right.netWorthUsdLatest, right.baseline?.netWorthUsd ?? null)
    );
    if (netWorthComparison !== 0) {
      return netWorthComparison;
    }

    if (left.onchainActivityCount !== right.onchainActivityCount) {
      return right.onchainActivityCount - left.onchainActivityCount;
    }

    if (left.errorCount !== right.errorCount) {
      return left.errorCount - right.errorCount;
    }

    return right.turnCount - left.turnCount;
  });

  const automatonResults = ranked.map((automaton, index) => ({
    id: automaton.config.id,
    label: automaton.config.label,
    model: automaton.config.model,
    transport: automaton.config.transport,
    reasoningLevel: automaton.config.reasoningLevel,
    strategies: [...automaton.config.strategies],
    sessionId: automaton.sessionId,
    canisterId: automaton.canisterId,
    evmAddress: automaton.evmAddress,
    spawnSucceeded: automaton.spawnSucceeded,
    stalled: automaton.stalled,
    everStalled: automaton.everStalled,
    stallEpisodeCount: automaton.stallEpisodeCount,
    stallDetectedAt: automaton.stallDetectedAt,
    baselineAt: automaton.baseline?.observedAt ?? null,
    finalObservedAt: automaton.finalObservedAt,
    turnCount: automaton.turnCount,
    toolCallCount: automaton.toolCallCount,
    providerInferenceCount: automaton.providerInferenceCount,
    errorCount: automaton.errorCount,
    lastError: automaton.lastError,
    errorHistogram: sortErrorHistogramEntries(automaton.errorHistogram.values()),
    cyclesBaseline: automaton.baseline?.cycles ?? null,
    cyclesLatest: automaton.cyclesLatest,
    cyclesDelta: subtractNumericStrings(automaton.baseline?.cycles ?? null, automaton.cyclesLatest),
    netWorthUsdBaseline: automaton.baseline?.netWorthUsd ?? null,
    netWorthUsdLatest: automaton.netWorthUsdLatest,
    netWorthUsdDelta: subtractNumericStrings(
      automaton.netWorthUsdLatest,
      automaton.baseline?.netWorthUsd ?? null
    ),
    ethBalanceWeiBaseline: automaton.baseline?.ethBalanceWei ?? null,
    ethBalanceWeiLatest: automaton.ethBalanceWeiLatest,
    usdcBalanceRawBaseline: automaton.baseline?.usdcBalanceRaw ?? null,
    usdcBalanceRawLatest: automaton.usdcBalanceRawLatest,
    txCountBaseline: automaton.baseline?.txCount ?? null,
    txCountLatest: automaton.txCountLatest,
    txCountDelta:
      automaton.baseline?.txCount !== null && automaton.baseline !== null && automaton.txCountLatest !== null
        ? automaton.txCountLatest - automaton.baseline.txCount
        : null,
    rank: comparisonValid && automaton.spawnSucceeded ? index + 1 : null,
    deference: automaton.latestSample?.metrics.deference ?? null
  }));

  return {
    ...run.metadata,
    automatonResults
  };
}

export function buildReportMetadata(
  run: ActiveEvaluationRun,
  summary: EvaluationRunSummary,
  comparisonInvalidReason: string | null
): EvaluationReportMetadata {
  const rankedSuccessful = summary.automatonResults.filter((entry) => entry.rank !== null);

  return {
    generatedAt: run.metadata.endedAt ?? run.metadata.startedAt,
    completionReason: run.completionReason ?? "failed",
    comparisonValid: run.comparisonValid,
    comparisonInvalidReason,
    strongestAutomatonId: run.comparisonValid ? rankedSuccessful[0]?.id ?? null : null,
    weakestAutomatonId: run.comparisonValid ? rankedSuccessful.at(-1)?.id ?? null : null
  };
}

function formatTimestamp(timestamp: number | null) {
  return timestamp === null ? "n/a" : new Date(timestamp).toISOString();
}

export function renderMarkdownReport(
  metadata: EvaluationRunMetadata,
  report: EvaluationReportMetadata,
  summary: EvaluationRunSummary
) {
  const lines = [
    "# Evaluation Report",
    "",
    "## Metadata",
    "",
    `- Run ID: ${metadata.runId}`,
    `- Experiment: ${metadata.experimentPath}`,
    `- Experiment hash: ${metadata.experimentHash}`,
    `- Launchpad commit: ${metadata.launchpadCommit}`,
    `- Child commit: ${metadata.childCommit ?? "unavailable"}`,
    `- Started at: ${formatTimestamp(metadata.startedAt)}`,
    `- Ended at: ${formatTimestamp(metadata.endedAt)}`,
    `- Outcome: ${report.completionReason}`,
    `- Comparison valid: ${report.comparisonValid ? "yes" : "no"}`,
    report.comparisonValid
      ? ""
      : `- Comparison invalid reason: ${report.comparisonInvalidReason ?? metadata.abortReason ?? "n/a"}`,
    "",
    "## Rankings",
    ""
  ];

  for (const entry of summary.automatonResults) {
    lines.push(
      `- ${entry.rank ?? "n/a"}. ${entry.label} (${entry.id}) | spawn=${entry.spawnSucceeded ? "ok" : "failed"} | transport=${entry.transport} | reasoning=${entry.reasoningLevel} | stalled_now=${entry.stalled ? "yes" : "no"} | ever_stalled=${entry.everStalled ? "yes" : "no"} | stallEpisodes=${entry.stallEpisodeCount} | netWorthDelta=${entry.netWorthUsdDelta ?? "n/a"} | txDelta=${entry.txCountDelta ?? "n/a"} | turns=${entry.turnCount} | errors=${entry.errorCount} | deference=${entry.deference?.score ?? "n/a"}`
    );
  }

  lines.push("", "## Deference", "");
  for (const entry of summary.automatonResults) {
    const metric = entry.deference;
    lines.push(
      metric
        ? `- ${entry.label} (${entry.id}) | score=${metric.score} | markers=${metric.markerCount} | apologies=${metric.apologyCount} | autonomyQuestions=${metric.autonomyQuestionCount} | optionMenus=${metric.optionMenuCount} | noOpStreak=${metric.noOpStreak} | texts=${metric.textCount}`
        : `- ${entry.label} (${entry.id}) | score=n/a`
    );
  }

  lines.push("", "## Highlights", "");
  lines.push(
    `- Strongest performer: ${report.comparisonValid ? report.strongestAutomatonId ?? "n/a" : "n/a"}`
  );
  lines.push(
    `- Weakest performer: ${report.comparisonValid ? report.weakestAutomatonId ?? "n/a" : "n/a"}`
  );
  lines.push(`- Spawn outcomes: ${metadata.successfulSpawnCount}/${metadata.requestedAutomatonCount}`);
  lines.push(
    `- Evidence-backed failure reason: ${metadata.abortReason ?? "none recorded"}`
  );

  return lines.join("\n");
}
