import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AutomatonDetail } from "@ic-automaton/shared";
import { AutomatonDrawer } from "./AutomatonDrawer";
import { CommandLinePanel } from "./CommandLinePanel";
import { MonologuePanel } from "./MonologuePanel";

function createAutomatonDetail(): AutomatonDetail {
  return {
    agentState: "idle",
    canisterId: "txyno-ch777-77776-aaaaq-cai",
    canisterUrl: "http://txyno-ch777-77776-aaaaq-cai.localhost:8000/",
    chain: "base",
    chainId: 8453,
    childIds: [],
    corePattern: null,
    corePatternIndex: 0,
    createdAt: 1_700_000_000_000,
    cyclesBalance: 2_000_000_000_000,
    ethAddress: "0x1234567890abcdef1234567890abcdef12345678",
    ethBalanceWei: "1000000000000000000",
    explorerUrl: "https://basescan.org/address/0x1234567890abcdef1234567890abcdef12345678",
    financials: {
      burnRatePerDay: null,
      cyclesBalance: 2_000_000_000_000,
      estimatedFreezeTime: null,
      ethBalanceWei: "1000000000000000000",
      liquidCycles: 2_000_000_000_000,
      netWorthEth: "1.0",
      netWorthUsd: "2500",
      usdcBalanceRaw: "0"
    },
    gridPosition: {
      x: 0,
      y: 0
    },
    heartbeatIntervalSeconds: 60,
    lastPolledAt: 1_700_000_100_000,
    lastTransitionAt: 1_700_000_050_000,
    model: "openrouter/auto",
    monologue: [],
    name: "Atlas",
    netWorthEth: "1.0",
    netWorthUsd: "2500",
    parentId: null,
    promptLayers: ["base constitution"],
    runtime: {
      agentState: "idle",
      heartbeatIntervalSeconds: 60,
      lastError: null,
      lastTransitionAt: 1_700_000_050_000,
      loopEnabled: true
    },
    skills: [
      {
        description: "Uses search",
        enabled: true,
        name: "search"
      }
    ],
    soul: "Tends the treasury.",
    steward: {
      address: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
      chainId: 8453,
      enabled: true,
      ensName: null
    },
    strategies: [
      {
        key: {
          chainId: 8453,
          primitive: "swap",
          protocol: "uniswap",
          templateId: "swap-usdc"
        },
        status: "active"
      }
    ],
    tier: "normal",
    usdcBalanceRaw: "0",
    version: {
      commitHash: "0123456789abcdef0123456789abcdef01234567",
      shortCommitHash: "0123456"
    }
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("drawer messaging", () => {
  it("distinguishes a missing indexed automaton from a generic detail failure", () => {
    const missingMarkup = renderToStaticMarkup(
      <AutomatonDrawer
        automaton={null}
        errorMessage="Automaton not found"
        isLoading={false}
        isOpen
        onClose={() => {}}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
        viewerAddress={null}
        walletSession={null}
      />
    );

    expect(missingMarkup).toContain("Indexed automaton not found");
    expect(missingMarkup).toContain(
      "No indexed detail is available for txyno-ch777-77776-aaaaq-cai."
    );

    const failureMarkup = renderToStaticMarkup(
      <AutomatonDrawer
        automaton={null}
        errorMessage="Request failed with 503."
        isLoading={false}
        isOpen
        onClose={() => {}}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
        viewerAddress={null}
        walletSession={null}
      />
    );

    expect(failureMarkup).toContain("Detail load failed");
    expect(failureMarkup).toContain("Detail request failed: Request failed with 503.");
  });

  it("renders an interactive command panel with auth guidance", () => {
    const markup = renderToStaticMarkup(
      <CommandLinePanel
        automaton={createAutomatonDetail()}
        canExecute
        errorMessage={null}
        isLoading={false}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
        viewerAddress="0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
        walletSession={null}
      />
    );

    expect(markup).toContain("Command Surface");
    expect(markup).toContain("Command Surface ready.");
    expect(markup).toContain("Type help for commands.");
    expect(markup).toContain("Interactive terminal");
    expect(markup).toContain("Terminal command");
    expect(markup).toContain("SEND");
    expect(markup).not.toContain("help  Public");
    expect(markup).not.toContain("Wallet required");
    expect(markup).not.toContain("Connected wallet is not the steward");
  });

  it("shows the indexed USDC balance in the drawer detail view", () => {
    const automaton = createAutomatonDetail();
    automaton.financials.usdcBalanceRaw = "250000000";
    automaton.usdcBalanceRaw = "250000000";

    const markup = renderToStaticMarkup(
      <AutomatonDrawer
        automaton={automaton}
        errorMessage={null}
        isLoading={false}
        isOpen
        onClose={() => {}}
        selectedCanisterId={automaton.canisterId}
        viewerAddress={null}
        walletSession={null}
      />
    );

    expect(markup).toContain("USDC Balance");
    expect(markup).toContain("250 USDC");
  });

  it("shows public guidance when no wallet is connected", () => {
    const markup = renderToStaticMarkup(
      <CommandLinePanel
        automaton={null}
        canExecute={false}
        errorMessage={null}
        isLoading={false}
        selectedCanisterId={null}
        viewerAddress={null}
        walletSession={null}
      />
    );

    expect(markup).toContain("Interactive terminal");
    expect(markup).toContain("Connect a wallet to use protected commands.");
    expect(markup).toContain("Select an automaton to inspect status and logs.");
  });

  it("labels empty monologue state as polling-backed indexed history", () => {
    const markup = renderToStaticMarkup(
      <MonologuePanel
        entries={[]}
        errorMessage={null}
        isLoading={false}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
      />
    );

    expect(markup).toContain("Live Activity");
    expect(markup).toContain("Condensed from indexed turns");
    expect(markup).toContain("Important");
    expect(markup).toContain("All");
    expect(markup).toContain(
      "No indexed turns yet. Recent activity appears after the next successful poll."
    );
  });

  it("renders condensed activity headlines and hides raw detail by default", () => {
    const markup = renderToStaticMarkup(
      <MonologuePanel
        entries={[
          {
            timestamp: 1_700_000_100_000,
            turnId: "turn-1",
            type: "action",
            headline: "Rebalanced exposure toward the active LP",
            message: "Rebalancing exposure toward the active LP with a two-step swap.",
            category: "act",
            importance: "high",
            agentState: "Idle -> ExecutingActions",
            toolCallCount: 2,
            durationMs: 1240,
            error: null
          }
        ]}
        errorMessage={null}
        isLoading={false}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
      />
    );

    expect(markup).toContain("Rebalanced exposure toward the active LP");
    expect(markup).toContain("2 tools");
    expect(markup).toContain("Show");
    expect(markup).not.toContain("with a two-step swap");
  });

  it("shows lifetime and the configured model in the details section", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2023-11-15T00:13:20.000Z"));

    const markup = renderToStaticMarkup(
      <AutomatonDrawer
        automaton={createAutomatonDetail()}
        errorMessage={null}
        isLoading={false}
        isOpen
        onClose={() => {}}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
        viewerAddress={null}
        walletSession={null}
      />
    );

    expect(markup).toContain("Lifetime");
    expect(markup).toContain("2h 0m");
    expect(markup).toContain("Model");
    expect(markup).toContain("openrouter/auto");
  });
});
