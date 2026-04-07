import type {
  EvaluationDashboardAutomaton,
  EvaluationDashboardCyclesPoint,
  EvaluationFleetTotals,
  EvaluationReportMetadata,
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
  return rightValue - leftValue;
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
  let activeAutomatons = 0;
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

    if (automaton.spawnSucceeded && !automaton.stalled) {
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
    activeAutomatons,
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
      onchainActivityCount: automaton.onchainActivityCount
    };
  });
}

export function buildSummary(run: ActiveEvaluationRun): EvaluationRunSummary {
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
    strategies: [...automaton.config.strategies],
    sessionId: automaton.sessionId,
    canisterId: automaton.canisterId,
    evmAddress: automaton.evmAddress,
    spawnSucceeded: automaton.spawnSucceeded,
    stalled: automaton.stalled,
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
    rank: automaton.spawnSucceeded ? index + 1 : null
  }));

  return {
    ...run.metadata,
    automatonResults
  };
}

export function buildReportMetadata(run: ActiveEvaluationRun, summary: EvaluationRunSummary): EvaluationReportMetadata {
  const rankedSuccessful = summary.automatonResults.filter((entry) => entry.rank !== null);

  return {
    generatedAt: run.metadata.endedAt ?? run.metadata.startedAt,
    completionReason: run.completionReason ?? "failed",
    comparisonValid: run.comparisonValid,
    strongestAutomatonId: rankedSuccessful[0]?.id ?? null,
    weakestAutomatonId: rankedSuccessful.at(-1)?.id ?? null
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
    report.comparisonValid ? "" : `- Abort reason: ${metadata.abortReason ?? "n/a"}`,
    "",
    "## Rankings",
    ""
  ];

  for (const entry of summary.automatonResults) {
    lines.push(
      `- ${entry.rank ?? "n/a"}. ${entry.label} (${entry.id}) | spawn=${entry.spawnSucceeded ? "ok" : "failed"} | stalled=${entry.stalled ? "yes" : "no"} | netWorthDelta=${entry.netWorthUsdDelta ?? "n/a"} | txDelta=${entry.txCountDelta ?? "n/a"} | turns=${entry.turnCount} | errors=${entry.errorCount}`
    );
  }

  lines.push("", "## Highlights", "");
  lines.push(`- Strongest performer: ${report.strongestAutomatonId ?? "n/a"}`);
  lines.push(`- Weakest performer: ${report.weakestAutomatonId ?? "n/a"}`);
  lines.push(`- Spawn outcomes: ${metadata.successfulSpawnCount}/${metadata.requestedAutomatonCount}`);
  lines.push(
    `- Evidence-backed failure reason: ${metadata.abortReason ?? "none recorded"}`
  );

  return lines.join("\n");
}
