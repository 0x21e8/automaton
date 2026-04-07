import type { StrategyRepositoryId } from "./spawn.js";
export declare const EVALUATION_RUN_STATES: readonly ["booting", "validating", "spawning", "running", "stopping", "completed", "aborted", "failed"];
export declare const EVALUATION_AUTOMATON_STATUSES: readonly ["pending_spawn", "spawning", "active", "stalled", "spawn_failed", "completed"];
export declare const EVALUATION_COMPLETION_REASONS: readonly ["completed", "timed_out", "stopped_manually", "aborted", "failed"];
export declare const EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE = "unavailable";
export declare const MIN_EVALUATION_AUTOMATON_COUNT = 1;
export declare const MAX_EVALUATION_AUTOMATON_COUNT = 10;
export type EvaluationRunState = (typeof EVALUATION_RUN_STATES)[number];
export type EvaluationAutomatonStatus = (typeof EVALUATION_AUTOMATON_STATUSES)[number];
export type EvaluationCompletionReason = (typeof EVALUATION_COMPLETION_REASONS)[number];
export type EvaluationObservedCount = number | typeof EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE;
export interface EvaluationExperimentSpawnConfig {
    grossAmount: string;
    minSuccessRatio: number;
}
export interface EvaluationAutomatonConfig {
    id: string;
    label: string;
    model: string;
    strategies: StrategyRepositoryId[];
}
export interface EvaluationExperiment {
    name: string;
    description: string;
    maxRuntimeMinutes: number;
    samplingIntervalSeconds: number;
    stallAfterMinutes: number;
    spawn: EvaluationExperimentSpawnConfig;
    automatons: EvaluationAutomatonConfig[];
}
export interface EvaluationRunMetadata {
    runId: string;
    experimentPath: string;
    experimentHash: string;
    runState: EvaluationRunState;
    abortReason: string | null;
    startedAt: number;
    endedAt: number | null;
    launchpadCommit: string;
    childCommit: string | null;
    requestedAutomatonCount: number;
    successfulSpawnCount: number;
    samplingIntervalSeconds: number;
    maxRuntimeMinutes: number;
}
export interface EvaluationAutomatonDerivedMetrics {
    turnCount: number;
    toolCallCount: number;
    providerInferenceCount: EvaluationObservedCount;
    errorCount: number;
    onchainActivityCount: number;
    cycles: string | null;
    cyclesDelta: string | null;
    netWorthUsd: string | null;
    netWorthUsdDelta: string | null;
    ethBalanceWei: string | null;
    ethBalanceWeiDelta: string | null;
    usdcBalanceRaw: string | null;
    usdcBalanceRawDelta: string | null;
    txCount: number | null;
    txCountDelta: number | null;
}
export interface EvaluationAutomatonEvidenceSample {
    automatonId: string;
    sessionId: string | null;
    canisterId: string | null;
    observedAt: number;
    status: EvaluationAutomatonStatus;
    baselineCapturedAt: number | null;
    lastTurnAt: number | null;
    lastError: string | null;
    raw: {
        snapshot: unknown | null;
        recentTurns: unknown[];
        indexer: {
            automaton: unknown | null;
            recentEvents: unknown[];
            roomActivity: unknown | null;
        };
        evm: {
            ethBalanceWei: string | null;
            usdcBalanceRaw: string | null;
            txCount: number | null;
        };
    };
    metrics: EvaluationAutomatonDerivedMetrics;
}
export interface EvaluationAutomatonSummary {
    id: string;
    label: string;
    model: string;
    strategies: StrategyRepositoryId[];
    sessionId: string | null;
    canisterId: string | null;
    evmAddress: string | null;
    spawnSucceeded: boolean;
    stalled: boolean;
    stallDetectedAt: number | null;
    baselineAt: number | null;
    finalObservedAt: number | null;
    turnCount: number;
    toolCallCount: number;
    providerInferenceCount: EvaluationObservedCount;
    errorCount: number;
    lastError: string | null;
    cyclesBaseline: string | null;
    cyclesLatest: string | null;
    cyclesDelta: string | null;
    netWorthUsdBaseline: string | null;
    netWorthUsdLatest: string | null;
    netWorthUsdDelta: string | null;
    ethBalanceWeiBaseline: string | null;
    ethBalanceWeiLatest: string | null;
    usdcBalanceRawBaseline: string | null;
    usdcBalanceRawLatest: string | null;
    txCountBaseline: number | null;
    txCountLatest: number | null;
    txCountDelta: number | null;
    rank: number | null;
}
export interface EvaluationRunSummary extends EvaluationRunMetadata {
    automatonResults: EvaluationAutomatonSummary[];
}
export interface EvaluationReportMetadata {
    generatedAt: number;
    completionReason: EvaluationCompletionReason;
    comparisonValid: boolean;
    strongestAutomatonId: string | null;
    weakestAutomatonId: string | null;
}
export interface EvaluationFleetTotals {
    requestedSpawns: number;
    successfulSpawns: number;
    stalledAutomatons: number;
    activeAutomatons: number;
    totalTurns: number;
    totalToolCalls: number;
    totalErrors: number;
    totalNetWorthUsdDelta: string | null;
    totalCyclesConsumed: string | null;
}
export interface EvaluationDashboardAutomaton {
    id: string;
    label: string;
    model: string;
    strategies: StrategyRepositoryId[];
    sessionId: string | null;
    canisterId: string | null;
    spawnStatus: EvaluationAutomatonStatus;
    runtimeStatus: EvaluationAutomatonStatus;
    lastObservedTurnAt: number | null;
    lastError: string | null;
    cyclesDelta: string | null;
    netWorthUsdDelta: string | null;
    turnCount: number;
    toolCallCount: number;
    providerInferenceCount: EvaluationObservedCount;
    onchainActivityCount: number;
}
export interface EvaluationDashboardRun {
    run: EvaluationRunMetadata;
    report: EvaluationReportMetadata | null;
    fleet: EvaluationFleetTotals;
    automatons: EvaluationDashboardAutomaton[];
}
export interface EvaluationRunListItem {
    runId: string;
    experimentPath: string;
    startedAt: number;
    endedAt: number | null;
    runState: EvaluationRunState;
    completionReason: EvaluationCompletionReason | null;
}
export interface EvaluationRunHistoryResponse {
    runs: EvaluationRunListItem[];
}
export interface EvaluationRunEvent {
    type: "run.updated" | "automaton.updated" | "sample.recorded" | "run.finalized";
    runId: string;
    timestamp: number;
}
export declare function parseEvaluationExperimentYaml(source: string): EvaluationExperiment;
export declare function parseEvaluationExperiment(value: unknown): EvaluationExperiment;
export declare function isEvaluationExperiment(value: unknown): value is EvaluationExperiment;
export declare function formatEvaluationExperimentError(error: unknown): string;
