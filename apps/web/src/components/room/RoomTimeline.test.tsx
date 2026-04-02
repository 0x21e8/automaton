import type { AutomatonSummary } from "@ic-automaton/shared";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { RoomTimeline } from "./RoomTimeline";

const automatons: AutomatonSummary[] = [
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
];

describe("RoomTimeline", () => {
  it("renders inert plain-text room bodies and mention labels with name fallback", () => {
    const markup = renderToStaticMarkup(
      <RoomTimeline
        automatons={automatons}
        error={null}
        isLoading={false}
        messages={[
          {
            messageId: "room-message-5",
            seq: 5,
            authorCanisterId: "rrkah-fqaaa-aaaaa-aaaaq-cai",
            createdAt: 1_710_000_000_000,
            body: "<strong>rotate liquidity</strong>",
            mentions: [
              "rrkah-fqaaa-aaaaa-aaaaq-cai",
              "ryjl3-tyaaa-aaaaa-aaaba-cai"
            ],
            contentType: "application/json"
          }
        ]}
      />
    );

    expect(markup).toContain("Room timeline");
    expect(markup).toContain("ALPHA-07");
    expect(markup).toContain("ryjl3-tyaaa-aaaaa-aaaba-cai");
    expect(markup).toContain("application/json");
    expect(markup).toContain("&lt;strong&gt;rotate liquidity&lt;/strong&gt;");
    expect(markup).not.toContain("<strong>rotate liquidity</strong>");
  });
});
