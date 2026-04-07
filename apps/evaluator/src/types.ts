import type {
  EvaluationAutomatonConfig,
  EvaluationAutomatonEvidenceSample,
  EvaluationErrorHistogramEntry,
  EvaluationErrorHistogramSource,
  EvaluationAutomatonStatus,
  EvaluationCompletionReason,
  EvaluationDashboardCyclesPoint,
  EvaluationDashboardRun,
  EvaluationExperiment,
  EvaluationReportMetadata,
  EvaluationRunEvent,
  EvaluationRunMetadata,
  EvaluationRunSummary
} from "@ic-automaton/shared";
import type { FastifyBaseLogger } from "fastify";

import type { EvaluatorConfig } from "./config.js";
import type { IndexerClientLike } from "./lib/indexer-client.js";
import type { AutomatonClientLike, AutomatonRuntimeEvidence } from "./lib/automaton-client.js";
import type { EvmClientLike } from "./lib/evm-client.js";
import type { ArtifactStore } from "./lib/files.js";
import type { PlaygroundProcessManagerLike } from "./lib/processes.js";
import type { EvaluationEventHub } from "./ws/events.js";
import type { RunController } from "./runtime/run-controller.js";

export interface EvaluatorRuntimeEnv {
  stewardAddress: string;
  openRouterApiKey: string;
  braveSearchApiKey: string | null;
  inferenceProxyWorkerBaseUrl: string | null;
  inferenceProxyTrustedCallbackPrincipal: string | null;
  localEvmForkUrl: string;
  automatonRepoPath: string;
}

export interface ExperimentFile {
  path: string;
  hash: string;
  source: string;
  absolutePath: string;
  parsed: EvaluationExperiment;
}

export interface PlaygroundRuntime {
  indexerBaseUrl: string;
  paymentRpcUrl: string;
  chainId: number;
  usdcAddress: string;
}

export interface EvaluationArtifacts {
  runDirectory: string;
  manifestPath: string;
  eventsPath: string;
  samplesDirectory: string;
  summaryPath: string;
  reportPath: string;
}

export interface StoredManifest {
  run: EvaluationRunMetadata;
  completionReason: EvaluationCompletionReason | null;
  comparisonValid: boolean;
}

export interface EvaluationTimelineEvent extends EvaluationRunEvent {
  payload?: unknown;
}

export interface RuntimeBaseline {
  observedAt: number;
  cycles: string | null;
  netWorthUsd: string | null;
  ethBalanceWei: string | null;
  usdcBalanceRaw: string | null;
  txCount: number | null;
}

export interface RuntimeAutomatonState {
  config: EvaluationAutomatonConfig;
  sessionId: string | null;
  canisterId: string | null;
  evmAddress: string | null;
  spawnStatus: EvaluationAutomatonStatus;
  runtimeStatus: EvaluationAutomatonStatus;
  spawnSucceeded: boolean;
  stalled: boolean;
  everStalled: boolean;
  stallEpisodeCount: number;
  stallDetectedAt: number | null;
  baseline: RuntimeBaseline | null;
  finalObservedAt: number | null;
  lastObservedTurnAt: number | null;
  lastError: string | null;
  turnCount: number;
  toolCallCount: number;
  providerInferenceCount: number | "unavailable";
  errorCount: number;
  onchainActivityCount: number;
  cyclesLatest: string | null;
  netWorthUsdLatest: string | null;
  ethBalanceWeiLatest: string | null;
  usdcBalanceRawLatest: string | null;
  txCountLatest: number | null;
  childCommit: string | null;
  lastProgressAt: number | null;
  latestSample: EvaluationAutomatonEvidenceSample | null;
  cyclesConsumedSeries: EvaluationDashboardCyclesPoint[];
  seenTurnIds: Set<string>;
  errorHistogram: Map<string, EvaluationErrorHistogramEntry>;
  lastObservedErrorBySource: Record<EvaluationErrorHistogramSource, string | null>;
}

export interface ActiveEvaluationRun {
  experiment: ExperimentFile;
  metadata: EvaluationRunMetadata;
  report: EvaluationReportMetadata | null;
  completionReason: EvaluationCompletionReason | null;
  comparisonValid: boolean;
  stopRequested: boolean;
  artifacts: EvaluationArtifacts;
  automatons: Map<string, RuntimeAutomatonState>;
}

export interface RunControllerDependencies {
  logger: FastifyBaseLogger;
  config: EvaluatorConfig;
  env: EvaluatorRuntimeEnv;
  artifacts: ArtifactStore;
  processes: PlaygroundProcessManagerLike;
  indexerClient: IndexerClientLike;
  automatonClient: AutomatonClientLike;
  evmClient: EvmClientLike;
  events: EvaluationEventHub;
  loadPlaygroundHelpers?: () => Promise<{
    claimPlaygroundFaucet(indexerBaseUrl: string, walletAddress: string): Promise<unknown>;
    createEphemeralWallet(rootDir: string): {
      address: string;
      privateKey: string;
    };
    submitSpawnPayment(options: {
      rootDir: string;
      rpcUrl: string;
      expectedChainId: number;
      usdcAddress: string;
      payment: {
        paymentAddress: string;
        grossAmount: string;
        claimId: string;
      };
      wallet: {
        address: string;
        privateKey: string;
      };
      sessionDetail: unknown;
    }): Promise<unknown>;
    waitForSessionCompletion(
      indexerBaseUrl: string,
      sessionId: string,
      options?: {
        pollTimeoutMs?: number;
        pollIntervalMs?: number;
      }
    ): Promise<{
      registryRecord?: {
        canisterId?: string | null;
        evmAddress?: string | null;
        versionCommit?: string | null;
      } | null;
    }>;
  }>;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
}

export interface FinalizedRun {
  dashboard: EvaluationDashboardRun;
  report: EvaluationReportMetadata;
  summary: EvaluationRunSummary;
}

export interface SampleContext {
  now: number;
  automaton: RuntimeAutomatonState;
  stallAfterMs: number;
  indexerClient: IndexerClientLike;
  automatonClient: AutomatonClientLike;
  evmClient: EvmClientLike;
  config: EvaluatorConfig;
}

export interface SampleResult {
  evidence: AutomatonRuntimeEvidence;
  sample: EvaluationAutomatonEvidenceSample;
}

declare module "fastify" {
  interface FastifyInstance {
    evaluatorConfig: EvaluatorConfig;
    runController: RunController;
    realtimeHub: EvaluationEventHub;
  }
}
