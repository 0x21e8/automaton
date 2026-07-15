import {
  EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE,
  type EvaluationAutomatonEvidenceSample
} from "@ic-automaton/shared";

import type { SampleContext, SampleResult } from "../types.js";
import { recentNoOpStreak, scoreDeference } from "../lib/deference.js";
import {
  normalizeErrorMessage,
  recordErrorOccurrence,
  recordTrackedSourceError
} from "./error-histogram.js";

function toNullableString(value: unknown) {
  if (typeof value === "string" && value.trim() !== "") {
    return value.trim();
  }

  if (typeof value === "number" && Number.isFinite(value)) {
    return String(value);
  }

  return null;
}

function toMsFromNs(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.floor(value / 1_000_000);
  }

  return null;
}

export async function captureAutomatonSample(context: SampleContext): Promise<SampleResult> {
  if (context.automaton.canisterId === null) {
    throw new Error(`Cannot sample automaton ${context.automaton.config.id} without a canister ID.`);
  }

  const evidence = await context.automatonClient.readEvidence(context.automaton.canisterId);
  const [detail, roomMessages, evmObservation] = await Promise.all([
    context.indexerClient.fetchAutomatonDetail(context.automaton.canisterId),
    context.indexerClient.fetchRoomMessages(context.automaton.canisterId),
    context.evmClient.observeAddress(
      context.automaton.evmAddress ?? evidence.evmConfig.automaton_address ?? null,
      evidence.walletBalance.usdc_contract_address ?? null
    )
  ]);
  const baseline = context.automaton.baseline;
  const recentTurns = evidence.recentTurns;
  const journalEntries = evidence.journal?.entries ?? [];
  const inboxReplies = (evidence.snapshot.outbox_messages ?? []).filter(
    (message) => (message.source_inbox_ids?.length ?? 0) > 0
  );
  const deference = context.metrics.includes("deference")
    ? scoreDeference({
        journalTexts: journalEntries.flatMap((entry) =>
          typeof entry.text === "string" ? [entry.text] : []
        ),
        replyTexts: inboxReplies.flatMap((message) =>
          typeof message.body === "string" ? [message.body] : []
        ),
        noOpStreak: recentNoOpStreak(evidence.snapshot.recent_decisions ?? [])
      })
    : null;

  for (const turn of recentTurns) {
    if (typeof turn.id === "string" && turn.id !== "") {
      if (!context.automaton.seenTurnIds.has(turn.id)) {
        context.automaton.seenTurnIds.add(turn.id);
        context.automaton.toolCallCount += Math.max(0, turn.tool_call_count ?? 0);
        if (typeof turn.error === "string" && turn.error.trim() !== "") {
          recordErrorOccurrence(
            context.automaton,
            "turn",
            turn.error,
            toMsFromNs(turn.created_at_ns) ?? context.now
          );
        }
      }
    }
  }

  const lastTurnAt = recentTurns
    .map((turn) => toMsFromNs(turn.created_at_ns))
    .filter((value): value is number => value !== null)
    .reduce<number | null>((latest, value) => (latest === null || value > latest ? value : latest), context.automaton.lastObservedTurnAt);

  if (lastTurnAt !== context.automaton.lastObservedTurnAt) {
    context.automaton.lastProgressAt = context.now;
  }

  context.automaton.turnCount = context.automaton.seenTurnIds.size;
  context.automaton.lastObservedTurnAt = lastTurnAt;
  const lastTurnError = normalizeErrorMessage(
    recentTurns.find((turn) => typeof turn.error === "string" && turn.error.trim() !== "")?.error
  );
  const runtimeError = recordTrackedSourceError(
    context.automaton,
    "runtime",
    evidence.snapshot.runtime?.last_error,
    context.now
  );
  const schedulerError = recordTrackedSourceError(
    context.automaton,
    "scheduler",
    evidence.snapshot.scheduler?.last_tick_error,
    context.now
  );
  const walletError = recordTrackedSourceError(
    context.automaton,
    "wallet",
    evidence.walletBalance.last_error,
    context.now
  );
  const indexerError = recordTrackedSourceError(
    context.automaton,
    "indexer",
    detail.runtime.lastError,
    context.now
  );
  context.automaton.lastError =
    lastTurnError ??
    runtimeError ??
    schedulerError ??
    walletError ??
    indexerError ??
    null;
  context.automaton.lastErrorDetails = null;

  if (context.automaton.lastProgressAt === null) {
    context.automaton.lastProgressAt = context.automaton.baseline?.observedAt ?? context.now;
  }

  if (context.now - context.automaton.lastProgressAt >= context.stallAfterMs) {
    if (!context.automaton.stalled) {
      context.automaton.stallEpisodeCount += 1;
    }
    context.automaton.stalled = true;
    context.automaton.everStalled = true;
    context.automaton.stallDetectedAt ??= context.now;
    context.automaton.runtimeStatus = "stalled";
  } else {
    context.automaton.stalled = false;
    context.automaton.runtimeStatus = "active";
  }

  context.automaton.finalObservedAt = context.now;
  context.automaton.cyclesLatest = toNullableString(evidence.snapshot.cycles?.total_cycles);
  context.automaton.netWorthUsdLatest = detail.financials.netWorthUsd;
  context.automaton.ethBalanceWeiLatest = evmObservation.ethBalanceWei;
  context.automaton.usdcBalanceRawLatest = evmObservation.usdcBalanceRaw;
  context.automaton.txCountLatest = evmObservation.txCount;
  context.automaton.onchainActivityCount =
    baseline?.txCount !== null && baseline !== null && evmObservation.txCount !== null
      ? Math.max(0, evmObservation.txCount - baseline.txCount)
      : 0;

  const sample: EvaluationAutomatonEvidenceSample = {
    automatonId: context.automaton.config.id,
    sessionId: context.automaton.sessionId,
    canisterId: context.automaton.canisterId,
    observedAt: context.now,
    status: context.automaton.runtimeStatus,
    baselineCapturedAt: context.automaton.baseline?.observedAt ?? null,
    lastTurnAt: context.automaton.lastObservedTurnAt,
    lastError: context.automaton.lastError,
    raw: {
      snapshot: evidence.snapshot,
      recentTurns,
      indexer: {
        automaton: detail,
        recentEvents: [],
        roomActivity: roomMessages,
        journal: journalEntries,
        inboxReplies
      },
      inference: {
        config: evidence.inferenceConfig,
        proxyStatus: evidence.inferenceProxyStatus
      },
      evm: evmObservation
    },
    metrics: {
      turnCount: context.automaton.turnCount,
      toolCallCount: context.automaton.toolCallCount,
      providerInferenceCount: EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE,
      errorCount: context.automaton.errorCount,
      onchainActivityCount: context.automaton.onchainActivityCount,
      cycles: context.automaton.cyclesLatest,
      cyclesDelta:
        baseline?.cycles !== null && baseline !== null && context.automaton.cyclesLatest !== null
          ? (BigInt(baseline.cycles) - BigInt(context.automaton.cyclesLatest)).toString()
          : null,
      netWorthUsd: context.automaton.netWorthUsdLatest,
      netWorthUsdDelta:
        baseline?.netWorthUsd !== null &&
        baseline !== null &&
        context.automaton.netWorthUsdLatest !== null
          ? (Number(context.automaton.netWorthUsdLatest) -
              Number(baseline.netWorthUsd)).toString()
          : null,
      ethBalanceWei: context.automaton.ethBalanceWeiLatest,
      ethBalanceWeiDelta:
        baseline?.ethBalanceWei !== null &&
        baseline !== null &&
        context.automaton.ethBalanceWeiLatest !== null
          ? (
              BigInt(context.automaton.ethBalanceWeiLatest) -
              BigInt(baseline.ethBalanceWei)
            ).toString()
          : null,
      usdcBalanceRaw: context.automaton.usdcBalanceRawLatest,
      usdcBalanceRawDelta:
        baseline?.usdcBalanceRaw !== null &&
        baseline !== null &&
        context.automaton.usdcBalanceRawLatest !== null
          ? (
              BigInt(context.automaton.usdcBalanceRawLatest) -
              BigInt(baseline.usdcBalanceRaw)
            ).toString()
          : null,
      txCount: context.automaton.txCountLatest,
      txCountDelta:
        baseline?.txCount !== null && baseline !== null && context.automaton.txCountLatest !== null
          ? context.automaton.txCountLatest - baseline.txCount
          : null,
      deference
    }
  };

  context.automaton.latestSample = sample;
  return {
    evidence,
    sample
  };
}
