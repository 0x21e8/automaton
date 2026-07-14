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
    constitution: null,
    constitutionHash: null,
    constitutionVerification: {
      status: "unavailable",
      expectedHash: null,
      computedHash: null
    },
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
  it("renders indexer metabolism, truthful control, and both patronage flows", () => {
    const automaton = createAutomatonDetail();
    automaton.metabolism = {
      burnRateCyclesPerDay: 1_000_000_000_000,
      runwaySeconds: 172_800,
      lifetimeEarningsUsdcRaw: "12500000",
      ageSeconds: 259_200,
      state: "healthy",
      history: [
        { capturedAt: 1, liquidCycles: 2_000, usdcBalanceRaw: "0", burnRateCyclesPerDay: 1_000, runwaySeconds: 172_800 },
        { capturedAt: 2, liquidCycles: 1_000, usdcBalanceRaw: "0", burnRateCyclesPerDay: 1_000, runwaySeconds: 86_400 }
      ]
    };
    automaton.controlStatus = {
      label: "upgradeable_by_factory",
      controllers: ["rrkah-fqaaa-aaaaa-aaaaq-cai"],
      spawnerPresent: false,
      verifiedAt: Date.now()
    };
    automaton.inboxContractAddress = "0x2222222222222222222222222222222222222222";
    automaton.usdcContractAddress = "0x3333333333333333333333333333333333333333";
    const markup = renderToStaticMarkup(<AutomatonDrawer automaton={automaton} errorMessage={null} isLoading={false} isOpen onClose={() => {}} selectedCanisterId={automaton.canisterId} viewerAddress={null} walletSession={null} />);
    expect(markup).toContain("Metabolism");
    expect(markup).toContain("Runway");
    expect(markup).toContain("12.50 USDC");
    expect(markup).toContain("Upgradeable by the factory");
    expect(markup).toContain("Price of attention");
    expect(markup).toContain("This is a gift the being can metabolize");
    expect(markup).not.toContain("yield");
  });

  it("keeps historical self-controlled and unattested records truthful", () => {
    const selfControlled = createAutomatonDetail();
    selfControlled.controlStatus = {
      label: "self_controlled",
      controllers: [selfControlled.canisterId],
      spawnerPresent: false,
      verifiedAt: 1_700_000_000_000
    };
    const unverified = createAutomatonDetail();
    unverified.controlStatus = {
      label: "unverified",
      controllers: [],
      spawnerPresent: false,
      verifiedAt: null
    };

    const selfMarkup = renderToStaticMarkup(<AutomatonDrawer automaton={selfControlled} errorMessage={null} isLoading={false} isOpen onClose={() => {}} selectedCanisterId={selfControlled.canisterId} viewerAddress={null} walletSession={null} />);
    const unverifiedMarkup = renderToStaticMarkup(<AutomatonDrawer automaton={unverified} errorMessage={null} isLoading={false} isOpen onClose={() => {}} selectedCanisterId={unverified.canisterId} viewerAddress={null} walletSession={null} />);
    expect(selfMarkup).toContain("Self-controlled; no factory upgrade path");
    expect(unverifiedMarkup).toContain("Controller status unverified");
  });

  it("renders terminal death cause and monument estate from indexer facts", () => {
    const automaton = createAutomatonDetail();
    automaton.tier = "out_of_cycles";
    automaton.metabolism = {
      burnRateCyclesPerDay: 1_000_000_000_000,
      runwaySeconds: 0,
      lifetimeEarningsUsdcRaw: "0",
      ageSeconds: 86_400,
      state: "dead",
      history: [],
      mortalityTier: "dead",
      deathCause: "starved",
      diedAt: 1_800_000_000_000,
      estateDisposition: "monument"
    };
    const markup = renderToStaticMarkup(<AutomatonDrawer automaton={automaton} errorMessage={null} isLoading={false} isOpen onClose={() => {}} selectedCanisterId={automaton.canisterId} viewerAddress={null} walletSession={null} />);
    expect(markup).toContain("starved");
    expect(markup).toContain("monument");
    expect(markup).toContain("dead");
  });

  it("shows verified documents and hides mismatched content", () => {
    const verified = createAutomatonDetail();
    verified.constitution = "A verified founding document.";
    verified.constitutionHash = "abc123";
    verified.constitutionVerification = {
      status: "verified",
      expectedHash: "abc123",
      computedHash: "abc123"
    };
    const verifiedMarkup = renderToStaticMarkup(
      <AutomatonDrawer
        automaton={verified}
        errorMessage={null}
        isLoading={false}
        isOpen
        onClose={() => {}}
        selectedCanisterId={verified.canisterId}
        viewerAddress={null}
        walletSession={null}
      />
    );
    expect(verifiedMarkup).toContain("A verified founding document.");
    expect(verifiedMarkup).toContain("Verified SHA-256 abc123");

    const mismatch = createAutomatonDetail();
    mismatch.constitution = null;
    mismatch.constitutionHash = "expected";
    mismatch.constitutionVerification = {
      status: "mismatch",
      expectedHash: "expected",
      computedHash: "different"
    };
    const mismatchMarkup = renderToStaticMarkup(
      <AutomatonDrawer
        automaton={mismatch}
        errorMessage={null}
        isLoading={false}
        isOpen
        onClose={() => {}}
        selectedCanisterId={mismatch.canisterId}
        viewerAddress={null}
        walletSession={null}
      />
    );
    expect(mismatchMarkup).toContain("Integrity warning");
    expect(mismatchMarkup).not.toContain("A verified founding document.");

    const legacy = createAutomatonDetail();
    legacy.constitution = "A legacy founding document.";
    legacy.constitutionVerification = {
      status: "legacy_unverified",
      expectedHash: null,
      computedHash: "computed"
    };
    const legacyMarkup = renderToStaticMarkup(
      <AutomatonDrawer
        automaton={legacy}
        errorMessage={null}
        isLoading={false}
        isOpen
        onClose={() => {}}
        selectedCanisterId={legacy.canisterId}
        viewerAddress={null}
        walletSession={null}
      />
    );
    expect(legacyMarkup).toContain("Legacy document");
    expect(legacyMarkup).toContain("no registry hash");
  });

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

  it("shows the public journal by default and keeps operator debug reachable", () => {
    const markup = renderToStaticMarkup(
      <MonologuePanel
        entries={[]}
        errorMessage={null}
        isLoading={false}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
      />
    );

    expect(markup).toContain("Public Journal");
    expect(markup).toContain("The being&#x27;s own public voice");
    expect(markup).toContain("Operator debug");
    expect(markup).toContain("No public journal entries yet.");
  });

  it("renders first-person journal entries instead of debug turns by default", () => {
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
        journalEntries={[
          {
            id: 7,
            timestamp: 1_700_000_100_000,
            turnId: "turn-1",
            text: "I moved toward the active LP after the evidence changed.",
            genesis: false
          }
        ]}
        errorMessage={null}
        isLoading={false}
        selectedCanisterId="txyno-ch777-77776-aaaaq-cai"
      />
    );

    expect(markup).toContain("I moved toward the active LP");
    expect(markup).not.toContain("Rebalanced exposure toward the active LP");
  });

  it("renders strict journal deal settlement state", () => {
    const txHash = `0x${"ab".repeat(32)}`;
    const markup = renderToStaticMarkup(
      <MonologuePanel
        entries={[]}
        journalEntries={[{
          id: 8,
          timestamp: 1_700_000_100_000,
          turnId: "turn-payment",
          text: "I submitted the agreed peer payment.",
          genesis: false,
          dealClaim: { kind: "peer_payment_claim", version: 1, txHash, peerCanisterId: "peer-cai", asset: "eth", amountRaw: "25" },
          settlement: { status: "settled", txHash, payerCanisterId: "payer-cai", payeeCanisterId: "peer-cai", asset: "eth", amountRaw: "25", verifiedAt: 123, provenance: `/api/society/transactions/${txHash}` }
        }]}
        errorMessage={null}
        isLoading={false}
        selectedCanisterId="payer-cai"
      />
    );
    expect(markup).toContain("Settled on-chain · 25 ETH raw");
    expect(markup).toContain(txHash);
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
