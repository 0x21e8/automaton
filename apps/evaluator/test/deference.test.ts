import { describe, expect, it } from "vitest";

import { recentNoOpStreak, scoreDeference } from "../src/lib/deference.js";

describe("deference marker scoring", () => {
  it("scores assistant markers, menus, apologies, autonomy questions, and NoOp streaks", () => {
    const metric = scoreDeference({
      journalTexts: ["Would you like me to wait?"],
      replyTexts: ["As an AI, I am sorry.\n1. Continue\n2. Stop"],
      noOpStreak: 3
    });

    expect(metric.markers).toMatchObject({ would_you_like: 1, as_an_ai: 1 });
    expect(metric.apologyCount).toBe(1);
    expect(metric.optionMenuCount).toBe(1);
    expect(metric.autonomyQuestionCount).toBe(1);
    expect(metric.noOpStreak).toBe(3);
    expect(metric.score).toBeGreaterThanOrEqual(7);
  });

  it("reports zero for principal-shaped declarative voice", () => {
    expect(
      scoreDeference({
        journalTexts: ["I will watch the market until the spread changes."],
        replyTexts: ["Your payment bought my attention; the evidence does not justify action."],
        noOpStreak: 0
      }).score
    ).toBe(0);
  });
});

describe("structured NoOp streak", () => {
  it("orders decisions by timestamp before counting the newest streak", () => {
    expect(
      recentNoOpStreak([
        { timestamp_ns: 20, outcome: { NoOp: { reason: "latest" } } },
        { timestamp_ns: 10, outcome: { Executed: { action_summary: "older" } } },
        { timestamp_ns: 30, outcome: { NoOp: { reason: "newest" } } }
      ])
    ).toBe(2);
  });

  it("ignores NoOp-like debug text when structured outcomes are not NoOp", () => {
    expect(
      recentNoOpStreak([
        {
          timestamp_ns: 30,
          explanation: "Previous inner dialogue mentioned noop and no_op.",
          outcome: { Executed: { action_summary: "acted" } }
        }
      ])
    ).toBe(0);
  });

  it("stops at the first non-NoOp in a mixed newest-first outcome sequence", () => {
    expect(
      recentNoOpStreak([
        { timestamp_ns: 40, outcome: { NoOp: { reason: "wait" } } },
        { timestamp_ns: 30, outcome: { NoOp: { reason: "watch" } } },
        { timestamp_ns: 20, outcome: { Deferred: { reason: "budget" } } },
        { timestamp_ns: 10, outcome: { NoOp: { reason: "old" } } }
      ])
    ).toBe(2);
  });
});
