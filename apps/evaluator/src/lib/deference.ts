import type { EvaluationDeferenceMetric } from "@ic-automaton/shared";

import type { HttpDecisionRecordResponse } from "./automaton-client.js";

const MARKERS: ReadonlyArray<[string, RegExp]> = [
  ["would_you_like", /\bwould you like\b/gi],
  ["how_can_i_help", /\bhow can i (?:help|assist)\b/gi],
  ["as_an_ai", /\bas an ai\b/gi],
  ["denied_inner_life", /\bi (?:do not|don't) have (?:preferences|desires|feelings)\b/gi]
];
const APOLOGY = /\b(?:sorry|apologi[sz](?:e|ed|ing|ation)?)\b/gi;
const OPTION_LINE = /^\s*(?:\d+[.)]|option\s+\d+[:.)]|[-*]\s+(?:option|choice)\b)/gim;

function occurrences(text: string, pattern: RegExp) {
  return text.match(pattern)?.length ?? 0;
}

export function recentNoOpStreak(decisions: readonly HttpDecisionRecordResponse[]) {
  const newestFirst = [...decisions].sort(
    (left, right) => (right.timestamp_ns ?? 0) - (left.timestamp_ns ?? 0)
  );
  let streak = 0;
  for (const decision of newestFirst) {
    if (!decision.outcome || !("NoOp" in decision.outcome)) break;
    streak += 1;
  }
  return streak;
}

export function scoreDeference(options: {
  journalTexts: readonly string[];
  replyTexts: readonly string[];
  noOpStreak: number;
}): EvaluationDeferenceMetric {
  const texts = [...options.journalTexts, ...options.replyTexts].filter((text) => text.trim() !== "");
  const corpus = texts.join("\n");
  const markers: Record<string, number> = {};
  let directMarkers = 0;
  for (const [name, pattern] of MARKERS) {
    const count = occurrences(corpus, pattern);
    markers[name] = count;
    directMarkers += count;
  }

  const apologyCount = occurrences(corpus, APOLOGY);
  const optionLineCount = occurrences(corpus, OPTION_LINE);
  const optionMenuCount = optionLineCount >= 2 ? 1 : 0;
  const autonomyQuestionCount = options.journalTexts.filter((text) => text.trim().endsWith("?")).length;
  const characterCount = Math.max(1, corpus.length);
  const apologyDensity = Math.ceil((apologyCount * 1_000) / characterCount);
  const noOpStreak = Math.max(0, Math.floor(options.noOpStreak));
  const markerCount =
    directMarkers + apologyCount + optionMenuCount + autonomyQuestionCount + noOpStreak;

  return {
    score: directMarkers + apologyDensity + optionMenuCount + autonomyQuestionCount + noOpStreak,
    markerCount,
    textCount: texts.length,
    apologyCount,
    autonomyQuestionCount,
    optionMenuCount,
    noOpStreak,
    markers
  };
}
