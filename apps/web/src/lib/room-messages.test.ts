import { describe, expect, it } from "vitest";

import {
  buildAutomatonNameLookup,
  mergeRoomMessages,
  resolveAutomatonLabel
} from "./room-messages";

describe("room message helpers", () => {
  it("deduplicates by message id and keeps the newest room messages first", () => {
    const merged = mergeRoomMessages(
      [
        {
          messageId: "room-message-1",
          seq: 1,
          authorCanisterId: "rrkah-fqaaa-aaaaa-aaaaq-cai",
          createdAt: 1_710_000_000_000,
          body: "older",
          mentions: [],
          contentType: "text/plain"
        }
      ],
      [
        {
          messageId: "room-message-2",
          seq: 2,
          authorCanisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
          createdAt: 1_710_000_000_100,
          body: "newest",
          mentions: [],
          contentType: "text/plain"
        },
        {
          messageId: "room-message-1",
          seq: 1,
          authorCanisterId: "rrkah-fqaaa-aaaaa-aaaaq-cai",
          createdAt: 1_710_000_000_000,
          body: "older replacement",
          mentions: [],
          contentType: "text/plain"
        }
      ]
    );

    expect(merged).toHaveLength(2);
    expect(merged.map((message) => message.seq)).toEqual([2, 1]);
    expect(merged[1]?.body).toBe("older replacement");
  });

  it("resolves automaton names when available and falls back to canister ids", () => {
    const lookup = buildAutomatonNameLookup([
      {
        canisterId: "rrkah-fqaaa-aaaaa-aaaaq-cai",
        ethAddress: null,
        chain: "base",
        chainId: 8453,
        name: "ALPHA-07",
        tier: "normal",
        agentState: "idle",
        ethBalanceWei: null,
        usdcBalanceRaw: null,
        cyclesBalance: 1_000_000_000_000,
        netWorthEth: null,
        netWorthUsd: null,
        heartbeatIntervalSeconds: null,
        steward: {
          address: "0x0000000000000000000000000000000000000000",
          chainId: 8453,
          ensName: null,
          enabled: true
        },
        gridPosition: { x: 0, y: 0 },
        corePatternIndex: 0,
        corePattern: null,
        parentId: null,
        createdAt: 1_710_000_000_000,
        lastTransitionAt: 1_710_000_000_000
      }
    ]);

    expect(resolveAutomatonLabel("rrkah-fqaaa-aaaaa-aaaaq-cai", lookup)).toBe(
      "ALPHA-07"
    );
    expect(resolveAutomatonLabel("ryjl3-tyaaa-aaaaa-aaaba-cai", lookup)).toBe(
      "ryjl3-tyaaa-aaaaa-aaaba-cai"
    );
  });
});
