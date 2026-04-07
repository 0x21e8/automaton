import type {
  EvaluationErrorHistogramEntry,
  EvaluationErrorHistogramSource
} from "@ic-automaton/shared";

import type { RuntimeAutomatonState } from "../types.js";

export function createEmptyObservedErrorMap(): Record<EvaluationErrorHistogramSource, string | null> {
  return {
    spawn: null,
    sampling: null,
    turn: null,
    runtime: null,
    scheduler: null,
    wallet: null,
    indexer: null
  };
}

export function normalizeErrorMessage(message: string | null | undefined) {
  if (typeof message !== "string") {
    return null;
  }

  const normalized = message.replace(/\s+/gu, " ").trim();
  return normalized === "" ? null : normalized;
}

export function recordErrorOccurrence(
  automaton: RuntimeAutomatonState,
  source: EvaluationErrorHistogramSource,
  message: string,
  observedAt: number
) {
  const normalizedMessage = normalizeErrorMessage(message);
  if (normalizedMessage === null) {
    return null;
  }

  const key = `${source}:${normalizedMessage}`;
  const existing = automaton.errorHistogram.get(key);

  if (existing) {
    existing.count += 1;
    existing.lastObservedAt = observedAt;
  } else {
    automaton.errorHistogram.set(key, {
      source,
      message: normalizedMessage,
      count: 1,
      lastObservedAt: observedAt
    } satisfies EvaluationErrorHistogramEntry);
  }

  automaton.errorCount += 1;
  return normalizedMessage;
}

export function recordTrackedSourceError(
  automaton: RuntimeAutomatonState,
  source: Exclude<EvaluationErrorHistogramSource, "turn">,
  message: string | null | undefined,
  observedAt: number
) {
  const normalizedMessage = normalizeErrorMessage(message);
  const previousMessage = automaton.lastObservedErrorBySource[source];

  automaton.lastObservedErrorBySource[source] = normalizedMessage;

  if (normalizedMessage === null || normalizedMessage === previousMessage) {
    return normalizedMessage;
  }

  return recordErrorOccurrence(automaton, source, normalizedMessage, observedAt);
}

export function sortErrorHistogramEntries(entries: Iterable<EvaluationErrorHistogramEntry>) {
  return [...entries].sort((left, right) => {
    if (left.count !== right.count) {
      return right.count - left.count;
    }

    if (left.lastObservedAt !== right.lastObservedAt) {
      return right.lastObservedAt - left.lastObservedAt;
    }

    if (left.source !== right.source) {
      return left.source.localeCompare(right.source);
    }

    return left.message.localeCompare(right.message);
  });
}
