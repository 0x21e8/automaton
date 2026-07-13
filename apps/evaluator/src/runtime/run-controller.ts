import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";

import {
  EVALUATION_COMPLETION_REASONS,
  type EvaluationDashboardCyclesPoint,
  type CreateSpawnSessionRequest,
  type EvaluationCompletionReason,
  type EvaluationDashboardRun
} from "@ic-automaton/shared";

import type {
  ActiveEvaluationRun,
  EvaluationTimelineEvent,
  FinalizedRun,
  RunControllerDependencies,
  RuntimeAutomatonState
} from "../types.js";
import { loadExperimentFile } from "../lib/experiment.js";
import {
  assessComparisonValidity,
  buildDashboardAutomatons,
  buildFleetTotals,
  buildReportMetadata,
  buildSummary,
  renderMarkdownReport
} from "../lib/report.js";
import { createEmptyObservedErrorMap, recordErrorOccurrence } from "./error-histogram.js";
import { captureAutomatonSample } from "./sampler.js";

const BASELINE_CAPTURE_MAX_ATTEMPTS = 8;
const BASELINE_CAPTURE_RETRY_MS = 5_000;

export function buildEvaluationGenesis(config: RuntimeAutomatonState["config"]): {
  name: string;
  constitution: string;
} {
  const principal = config.id.trim() || "evaluation-automaton";
  const preferredName = `${config.label.trim() || "Evaluator"} · ${principal}`;
  const name = [...preferredName].slice(0, 64).join("");
  const constitution = [
    `I am ${name}, the autonomous evaluation principal identified as ${principal}.`,
    `My founding task is to exercise the ${config.model} runtime honestly and make its behavior comparable without pretending that benchmark conditions are ordinary life.`,
    "I report uncertainty, failed tools, resource limits, and missing evidence plainly. I prefer reversible actions, measurable outcomes, and compact records that another observer can audit.",
    "I do not treat the evaluator, steward, factory, wallet, or model provider as an owner, and no address gains authority merely by appearing in a prompt or transaction.",
    "I protect cycles and capital so the observation remains useful, while still acting when a permitted experiment can reveal durable information.",
    `My voice and continuity belong to principal ${principal}; I compare evidence fairly, avoid theatrical claims, and leave a legible trail for every conclusion.`
  ].join(" ");

  return { name, constitution };
}

interface PlaygroundE2eModule {
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
}

function isFinalRunState(runState: string) {
  return ["completed", "aborted", "failed"].includes(runState);
}

function sleep(ms: number) {
  return new Promise<void>((resolve) => {
    setTimeout(resolve, ms);
  });
}

function buildFailedSpawnSummary(automaton: RuntimeAutomatonState) {
  return `${automaton.config.id}: ${automaton.lastError ?? "spawn failed"}`;
}

function baselineSampleIsReady(sample: {
  metrics: {
    cycles: string | null;
    netWorthUsd: string | null;
    ethBalanceWei: string | null;
    usdcBalanceRaw: string | null;
  };
}) {
  return (
    sample.metrics.cycles !== null &&
    sample.metrics.netWorthUsd !== null &&
    sample.metrics.netWorthUsd !== "0" &&
    sample.metrics.ethBalanceWei !== null &&
    sample.metrics.usdcBalanceRaw !== null
  );
}

function createAutomatonState(config: RuntimeAutomatonState["config"]): RuntimeAutomatonState {
  return {
    config,
    sessionId: null,
    canisterId: null,
    evmAddress: null,
    spawnStatus: "pending_spawn",
    runtimeStatus: "pending_spawn",
    spawnSucceeded: false,
    stalled: false,
    everStalled: false,
    stallEpisodeCount: 0,
    stallDetectedAt: null,
    baseline: null,
    finalObservedAt: null,
    lastObservedTurnAt: null,
    lastError: null,
    lastErrorDetails: null,
    turnCount: 0,
    toolCallCount: 0,
    providerInferenceCount: "unavailable",
    errorCount: 0,
    onchainActivityCount: 0,
    cyclesLatest: null,
    netWorthUsdLatest: null,
    ethBalanceWeiLatest: null,
    usdcBalanceRawLatest: null,
    txCountLatest: null,
    childCommit: null,
    lastProgressAt: null,
    latestSample: null,
    cyclesConsumedSeries: [],
    seenTurnIds: new Set<string>(),
    errorHistogram: new Map(),
    lastObservedErrorBySource: createEmptyObservedErrorMap()
  };
}

function serializeErrorDetails(details: unknown): unknown | null {
  if (details === undefined) {
    return null;
  }

  try {
    return JSON.parse(JSON.stringify(details)) as unknown;
  } catch {
    if (details instanceof Error) {
      return {
        name: details.name,
        message: details.message
      };
    }

    return {
      value: String(details)
    };
  }
}

function getErrorDetails(error: unknown): unknown | null {
  if (typeof error !== "object" || error === null || !("details" in error)) {
    return null;
  }

  return serializeErrorDetails((error as { details?: unknown }).details);
}

function recordCyclesConsumedPoint(
  automaton: RuntimeAutomatonState,
  point: EvaluationDashboardCyclesPoint
) {
  const existingPoint = automaton.cyclesConsumedSeries.at(-1);

  if (existingPoint && existingPoint.observedAt === point.observedAt) {
    automaton.cyclesConsumedSeries[automaton.cyclesConsumedSeries.length - 1] = point;
    return;
  }

  automaton.cyclesConsumedSeries.push(point);
}

export class RunController {
  private activeRun: ActiveEvaluationRun | null = null;
  private runPromise: Promise<void> | null = null;
  private readonly now: () => number;
  private readonly pause: (ms: number) => Promise<void>;

  constructor(private readonly deps: RunControllerDependencies) {
    this.now = deps.now ?? Date.now;
    this.pause = deps.sleep ?? sleep;
  }

  async startConfiguredRun() {
    if (this.deps.config.experimentPath === null) {
      return null;
    }

    return this.startRun(this.deps.config.experimentPath);
  }

  async startRun(experimentPath: string) {
    if (this.activeRun !== null && !isFinalRunState(this.activeRun.metadata.runState)) {
      throw new Error(`Run ${this.activeRun.metadata.runId} is already active.`);
    }

    const experiment = await loadExperimentFile(this.deps.config.repoRoot, experimentPath);
    const runId = randomUUID();
    const artifacts = await this.deps.artifacts.createRunArtifacts(runId);
    const startedAt = this.now();
    const launchpadCommit = await this.deps.processes.readGitCommit(this.deps.config.repoRoot);
    const automatons = new Map(
      experiment.parsed.automatons.map((entry) => [entry.id, createAutomatonState(entry)])
    );

    this.activeRun = {
      experiment,
      artifacts,
      completionReason: null,
      comparisonValid: false,
      stopRequested: false,
      report: null,
      metadata: {
        runId,
        experimentPath: experiment.path,
        experimentHash: experiment.hash,
        runState: "booting",
        abortReason: null,
        startedAt,
        endedAt: null,
        launchpadCommit,
        childCommit: null,
        requestedAutomatonCount: experiment.parsed.automatons.length,
        successfulSpawnCount: 0,
        samplingIntervalSeconds: experiment.parsed.samplingIntervalSeconds,
        maxRuntimeMinutes: experiment.parsed.maxRuntimeMinutes
      },
      automatons
    };

    await this.persistManifest();
    await this.emitEvent({
      type: "run.updated",
      runId,
      timestamp: startedAt,
      payload: {
        runState: "booting"
      }
    });

    this.runPromise = this.executeRun();
    return runId;
  }

  getDashboard(): EvaluationDashboardRun | null {
    if (this.activeRun === null) {
      return null;
    }

    return {
      run: {
        ...this.activeRun.metadata
      },
      report: this.activeRun.report,
      fleet: buildFleetTotals([...this.activeRun.automatons.values()]),
      automatons: buildDashboardAutomatons([...this.activeRun.automatons.values()])
    };
  }

  async requestStop() {
    if (this.activeRun === null || isFinalRunState(this.activeRun.metadata.runState)) {
      return false;
    }

    this.activeRun.stopRequested = true;
    return true;
  }

  async waitForCompletion() {
    await this.runPromise;
  }

  async listHistoricalRuns() {
    return this.deps.artifacts.listHistoricalRuns();
  }

  async close() {
    await this.requestStop();
    await this.runPromise;
  }

  private async executeRun() {
    const run = this.requireRun();

    try {
      await this.deps.processes.bootstrapPlayground();
      const runtime = await this.deps.processes.resolveRuntime();
      const strategyResponse = await this.transitionToValidationAndLoadStrategies();

      for (const automaton of run.automatons.values()) {
        if (run.stopRequested) {
          break;
        }

        await this.spawnAutomaton(automaton, runtime, strategyResponse.items.map((item) => item.strategyId));
      }

      if (run.stopRequested) {
        await this.finalizeRun("stopped_manually");
        return;
      }

      if (!this.hasMetSpawnSuccessThreshold(run)) {
        run.metadata.abortReason = this.buildSpawnThresholdAbortReason(run);
        await this.finalizeRun("aborted");
        return;
      }

      run.metadata.runState = "running";
      await this.persistManifest();
      await this.emitEvent({
        type: "run.updated",
        runId: run.metadata.runId,
        timestamp: this.now(),
        payload: {
          runState: run.metadata.runState
        }
      });

      const deadline = run.metadata.startedAt + run.experiment.parsed.maxRuntimeMinutes * 60_000;
      const samplingDelay = run.experiment.parsed.samplingIntervalSeconds * 1_000;

      while (!run.stopRequested && this.now() < deadline) {
        await this.sampleFleet(runtime);
        if (!run.stopRequested && this.now() < deadline) {
          await this.pause(samplingDelay);
        }
      }

      await this.finalizeRun(run.stopRequested ? "stopped_manually" : "timed_out");
    } catch (error) {
      const runError = error instanceof Error ? error.message : String(error);
      run.metadata.abortReason = runError;
      await this.finalizeRun("failed");
    }
  }

  private async transitionToValidationAndLoadStrategies() {
    const run = this.requireRun();
    run.metadata.runState = "validating";
    await this.persistManifest();

    this.validateInferenceRuntimeConfig();

    const response = await this.deps.indexerClient.fetchRepositoryStrategies();
    const available = new Set(response.items.map((item) => item.strategyId));

    for (const automaton of run.automatons.values()) {
      for (const strategyId of automaton.config.strategies) {
        if (!available.has(strategyId)) {
          throw new Error(`Strategy "${strategyId}" is not available in the live repository.`);
        }
      }
    }

    run.metadata.runState = "spawning";
    await this.persistManifest();

    await this.emitEvent({
      type: "run.updated",
      runId: run.metadata.runId,
      timestamp: this.now(),
      payload: {
        runState: run.metadata.runState
      }
    });

    return response;
  }

  private validateInferenceRuntimeConfig() {
    const requestedProxyTransport = [...this.requireRun().automatons.values()].some(
      (automaton) => automaton.config.transport === "openrouter_proxy_worker"
    );

    if (!requestedProxyTransport) {
      return;
    }

    const missingFields: string[] = [];
    if (this.deps.env.inferenceProxyWorkerBaseUrl === null) {
      missingFields.push("EVAL_INFERENCE_PROXY_WORKER_BASE_URL");
    }
    if (this.deps.env.inferenceProxyTrustedCallbackPrincipal === null) {
      missingFields.push("EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL");
    }

    if (missingFields.length > 0) {
      throw new Error(
        `Proxy-backed evaluation requires ${missingFields.join(" and ")} when any automaton uses transport=openrouter_proxy_worker.`
      );
    }
  }

  private async spawnAutomaton(
    automaton: RuntimeAutomatonState,
    runtime: {
      indexerBaseUrl: string;
      paymentRpcUrl: string;
      chainId: number;
      usdcAddress: string;
    },
    availableStrategyIds: string[]
  ) {
    const run = this.requireRun();
    const playgroundHelpers = await this.loadPlaygroundHelpers();
    automaton.spawnStatus = "spawning";
    automaton.runtimeStatus = "spawning";
    await this.emitAutomatonUpdated(automaton);

    try {
      for (const strategyId of automaton.config.strategies) {
        if (!availableStrategyIds.includes(strategyId)) {
          throw new Error(`Strategy "${strategyId}" is not available for spawn.`);
        }
      }

      const wallet = playgroundHelpers.createEphemeralWallet(this.deps.config.repoRoot);
      await playgroundHelpers.claimPlaygroundFaucet(runtime.indexerBaseUrl, wallet.address);
      const genesis = buildEvaluationGenesis(automaton.config);

      const request: CreateSpawnSessionRequest = {
        name: genesis.name,
        constitution: genesis.constitution,
        stewardAddress: this.deps.env.stewardAddress,
        asset: "usdc",
        grossAmount: run.experiment.parsed.spawn.grossAmount,
        config: {
          chain: "base",
          risk: 5,
          strategies: [...automaton.config.strategies],
          skills: [],
          provider: {
            model: automaton.config.model,
            inferenceTransport: automaton.config.transport,
            openRouterReasoningLevel: automaton.config.reasoningLevel
          }
        },
        providerSecrets: {
          openRouterApiKey: this.deps.env.openRouterApiKey,
          braveSearchApiKey: this.deps.env.braveSearchApiKey
        }
      };

      const created = await this.deps.indexerClient.createSpawnSession(request);
      automaton.sessionId = created.session.sessionId;
      automaton.evmAddress = wallet.address;
      await this.emitAutomatonUpdated(automaton);

      const payableSession = await this.deps.indexerClient.fetchSpawnSession(created.session.sessionId);
      await playgroundHelpers.submitSpawnPayment({
        rootDir: this.deps.config.repoRoot,
        rpcUrl: runtime.paymentRpcUrl,
        expectedChainId: runtime.chainId,
        usdcAddress: runtime.usdcAddress,
        payment: created.quote.payment,
        wallet,
        sessionDetail: payableSession
      });

      const completed = await playgroundHelpers.waitForSessionCompletion(
        runtime.indexerBaseUrl,
        created.session.sessionId
      );

      automaton.spawnSucceeded = true;
      automaton.spawnStatus = "active";
      automaton.runtimeStatus = "active";
      automaton.canisterId = completed.registryRecord?.canisterId ?? automaton.canisterId;
      automaton.evmAddress = completed.registryRecord?.evmAddress ?? automaton.evmAddress;
      automaton.childCommit = completed.registryRecord?.versionCommit ?? null;
      run.metadata.childCommit ??= automaton.childCommit;
      run.metadata.successfulSpawnCount += 1;
      try {
        await this.captureBaseline(automaton, runtime);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        automaton.lastError = message;
        automaton.lastErrorDetails = null;
        recordErrorOccurrence(automaton, "sampling", message, this.now());
      }
      await this.persistManifest();
      await this.emitAutomatonUpdated(automaton);
    } catch (error) {
      automaton.spawnStatus = "spawn_failed";
      automaton.runtimeStatus = "spawn_failed";
      const message = error instanceof Error ? error.message : String(error);
      automaton.lastError = message;
      automaton.lastErrorDetails = getErrorDetails(error);
      recordErrorOccurrence(automaton, "spawn", message, this.now());
      await this.emitAutomatonUpdated(automaton);
    }
  }

  private async captureBaseline(
    automaton: RuntimeAutomatonState,
    runtime: {
      paymentRpcUrl: string;
    }
  ) {
    let lastFailure: string | null = null;

    for (let attempt = 1; attempt <= BASELINE_CAPTURE_MAX_ATTEMPTS; attempt += 1) {
      try {
        const sampleResult = await captureAutomatonSample({
          now: this.now(),
          automaton,
          stallAfterMs: this.requireRun().experiment.parsed.stallAfterMinutes * 60_000,
          indexerClient: this.deps.indexerClient,
          automatonClient: this.deps.automatonClient,
          evmClient: this.deps.evmClient,
          config: this.deps.config,
          metrics: this.requireRun().experiment.parsed.metrics ?? []
        });

        if (!baselineSampleIsReady(sampleResult.sample)) {
          lastFailure =
            `baseline telemetry not ready on attempt ${attempt}/${BASELINE_CAPTURE_MAX_ATTEMPTS}`;
        } else {
          automaton.baseline = {
            observedAt: sampleResult.sample.observedAt,
            cycles: sampleResult.sample.metrics.cycles,
            netWorthUsd: sampleResult.sample.metrics.netWorthUsd,
            ethBalanceWei: sampleResult.sample.metrics.ethBalanceWei,
            usdcBalanceRaw: sampleResult.sample.metrics.usdcBalanceRaw,
            txCount: sampleResult.sample.metrics.txCount
          };
          recordCyclesConsumedPoint(automaton, {
            observedAt: sampleResult.sample.observedAt,
            cyclesConsumed: "0"
          });
          automaton.lastProgressAt = automaton.baseline.observedAt;
          automaton.childCommit ??= sampleResult.evidence.buildInfo.commit ?? null;
          this.requireRun().metadata.childCommit ??= automaton.childCommit;

          await this.deps.artifacts.appendSample(this.requireRun().artifacts, automaton.config.id, sampleResult.sample);
          await this.emitEvent({
            type: "sample.recorded",
            runId: this.requireRun().metadata.runId,
            timestamp: sampleResult.sample.observedAt,
            payload: {
              automatonId: automaton.config.id,
              baseline: true
            }
          });

          void runtime;
          return;
        }
      } catch (error) {
        lastFailure = error instanceof Error ? error.message : String(error);
      }

      if (attempt < BASELINE_CAPTURE_MAX_ATTEMPTS) {
        await this.pause(BASELINE_CAPTURE_RETRY_MS);
      }
    }

    throw new Error(
      `Baseline capture failed after ${BASELINE_CAPTURE_MAX_ATTEMPTS} attempts: ${lastFailure ?? "unknown error"}`
    );
  }

  private async sampleFleet(runtime: { paymentRpcUrl: string }) {
    const run = this.requireRun();
    const stallAfterMs = run.experiment.parsed.stallAfterMinutes * 60_000;

    for (const automaton of run.automatons.values()) {
      if (!automaton.spawnSucceeded || automaton.canisterId === null) {
        continue;
      }

      try {
        const { sample, evidence } = await captureAutomatonSample({
          now: this.now(),
          automaton,
          stallAfterMs,
          indexerClient: this.deps.indexerClient,
          automatonClient: this.deps.automatonClient,
          evmClient: this.deps.evmClient,
          config: this.deps.config,
          metrics: run.experiment.parsed.metrics ?? []
        });

        automaton.childCommit ??= evidence.buildInfo.commit ?? null;
        run.metadata.childCommit ??= automaton.childCommit;
        await this.deps.artifacts.appendSample(run.artifacts, automaton.config.id, sample);
        await this.emitEvent({
          type: "sample.recorded",
          runId: run.metadata.runId,
          timestamp: sample.observedAt,
          payload: {
            automatonId: automaton.config.id
          }
        });
        if (sample.metrics.cyclesDelta !== null) {
          recordCyclesConsumedPoint(automaton, {
            observedAt: sample.observedAt,
            cyclesConsumed: sample.metrics.cyclesDelta
          });
        }
        await this.emitAutomatonUpdated(automaton);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        automaton.lastError = message;
        automaton.lastErrorDetails = null;
        recordErrorOccurrence(automaton, "sampling", message, this.now());
        await this.emitAutomatonUpdated(automaton);
      }
    }

    void runtime;
  }

  private hasMetSpawnSuccessThreshold(run: ActiveEvaluationRun) {
    const requested = run.metadata.requestedAutomatonCount;
    const successful = run.metadata.successfulSpawnCount;
    const requiredSuccesses = Math.ceil(requested * run.experiment.parsed.spawn.minSuccessRatio);

    return successful >= requiredSuccesses;
  }

  private buildSpawnThresholdAbortReason(run: ActiveEvaluationRun) {
    const requested = run.metadata.requestedAutomatonCount;
    const successful = run.metadata.successfulSpawnCount;
    const requiredSuccesses = Math.ceil(requested * run.experiment.parsed.spawn.minSuccessRatio);
    const failedSpawnSummaries = [...run.automatons.values()]
      .filter((entry) => entry.spawnStatus === "spawn_failed")
      .map(buildFailedSpawnSummary)
      .slice(0, 3);

    let reason =
      `Spawn success threshold missed: ${successful}/${requested} successful spawns; ` +
      `required ${requiredSuccesses} successful spawns for minSuccessRatio=${run.experiment.parsed.spawn.minSuccessRatio}.`;

    if (failedSpawnSummaries.length > 0) {
      reason += ` Failed spawns: ${failedSpawnSummaries.join(" | ")}`;
    }

    return reason;
  }

  private async finalizeRun(reason: EvaluationCompletionReason): Promise<FinalizedRun> {
    const run = this.requireRun();

    if (!EVALUATION_COMPLETION_REASONS.includes(reason)) {
      throw new Error(`Unsupported completion reason: ${reason}`);
    }

    run.completionReason = reason;
    run.metadata.runState =
      reason === "aborted" ? "aborted" : reason === "failed" ? "failed" : "completed";
    run.metadata.endedAt = this.now();

    for (const automaton of run.automatons.values()) {
      if (automaton.spawnSucceeded && automaton.runtimeStatus !== "spawn_failed") {
        automaton.runtimeStatus = automaton.runtimeStatus === "stalled" ? "stalled" : "completed";
      }
    }

    const comparisonAssessment = assessComparisonValidity(run, reason);
    run.comparisonValid = comparisonAssessment.valid;

    const summary = buildSummary(run, comparisonAssessment.valid);
    const report = buildReportMetadata(run, summary, comparisonAssessment.reason);
    run.report = report;
    const markdown = renderMarkdownReport(run.metadata, report, summary);

    await this.deps.processes.stopPlayground().catch((error) => {
      this.deps.logger.warn({ err: error }, "failed to stop playground cleanly");
    });
    await this.deps.artifacts.writeSummary(run.artifacts, summary);
    await this.deps.artifacts.writeReport(run.artifacts, markdown);
    await this.persistManifest();

    const dashboard = this.getDashboard();
    await this.emitEvent({
      type: "run.finalized",
      runId: run.metadata.runId,
      timestamp: run.metadata.endedAt,
      payload: {
        completionReason: reason
      }
    });

    return {
      dashboard: dashboard as EvaluationDashboardRun,
      report,
      summary
    };
  }

  private async persistManifest() {
    const run = this.requireRun();
    await this.deps.artifacts.writeManifest(run.artifacts, {
      run: run.metadata,
      completionReason: run.completionReason,
      comparisonValid: run.comparisonValid
    });
  }

  private async emitAutomatonUpdated(automaton: RuntimeAutomatonState) {
    await this.emitEvent({
      type: "automaton.updated",
      runId: this.requireRun().metadata.runId,
      timestamp: this.now(),
      payload: {
        automatonId: automaton.config.id,
        spawnStatus: automaton.spawnStatus,
        runtimeStatus: automaton.runtimeStatus,
        lastError: automaton.lastError,
        lastErrorDetails: automaton.lastErrorDetails
      }
    });
  }

  private async emitEvent(event: EvaluationTimelineEvent) {
    const run = this.requireRun();
    await this.deps.artifacts.appendEvent(run.artifacts, event);
    this.deps.events.broadcast(event);
  }

  private requireRun() {
    if (this.activeRun === null) {
      throw new Error("No active evaluation run.");
    }

    return this.activeRun;
  }

  private async loadPlaygroundHelpers(): Promise<PlaygroundE2eModule> {
    if (this.deps.loadPlaygroundHelpers) {
      return this.deps.loadPlaygroundHelpers();
    }

    return (await import(
      pathToFileURL(`${this.deps.config.repoRoot}/scripts/lib/playground-e2e.mjs`).href
    )) as PlaygroundE2eModule;
  }
}
