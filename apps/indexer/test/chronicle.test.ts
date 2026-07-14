import { describe, expect, it } from "vitest";
import { buildChronicleDay } from "../src/lib/chronicle.js";
import { createAutomatonDetailFixture, createRoomMessageFixture } from "./fixtures.js";

describe("chronicle generator", () => {
  it("emits timestamped factual entries with provenance and only settled deals", () => {
    const timestamp = Date.parse("2026-07-14T12:00:00Z");
    const automaton = createAutomatonDetailFixture({ createdAt: timestamp });
    const settled = createRoomMessageFixture({ createdAt: timestamp, settlement: { status: "settled", txHash: `0x${"ab".repeat(32)}`, payerCanisterId: automaton.canisterId, payeeCanisterId: "peer-cai", asset: "eth", amountRaw: "1", verifiedAt: timestamp, provenance: `evm:tx:0x${"ab".repeat(32)}` } });
    const day = buildChronicleDay({ date: "2026-07-14", generatedAt: timestamp, automatons: [automaton], roomMessages: [settled, { ...settled, messageId: "unsettled", settlement: { ...settled.settlement!, status: "unsettled" } }], journals: [] });
    expect(day.entries.map((entry) => entry.kind)).toEqual(expect.arrayContaining(["birth", "deal"]));
    expect(day.entries.filter((entry) => entry.kind === "deal")).toHaveLength(1);
    expect(day.entries.every((entry) => entry.provenance.length > 0)).toBe(true);
  });

  it("does not project a current runway crisis into a historical day and records room activity", () => {
    const historical = Date.parse("2026-07-13T12:00:00Z");
    const generatedAt = Date.parse("2026-07-14T12:00:00Z");
    const automaton = createAutomatonDetailFixture({
      createdAt: historical - 1_000,
      metabolism: { ...createAutomatonDetailFixture().metabolism!, mortalityTier: "terminal" }
    });
    const message = createRoomMessageFixture({ seq: 0, createdAt: historical, authorCanisterId: automaton.canisterId });
    const day = buildChronicleDay({ date: "2026-07-13", generatedAt, automatons: [automaton], roomMessages: [message], journals: [] });
    expect(day.entries.some((entry) => entry.kind === "runway_crisis")).toBe(false);
    expect(day.entries).toContainEqual(expect.objectContaining({ kind: "room_activity", provenance: [{ label: "room message", href: "/api/room/messages?limit=1" }] }));
  });
});
