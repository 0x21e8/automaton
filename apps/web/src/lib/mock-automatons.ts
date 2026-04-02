import type {
  AutomatonDetail,
  MonologueEntry,
  SkillSelection,
  StrategySelection
} from "@ic-automaton/shared";

export const SIMULATED_VIEWER_ADDRESS =
  "0x21e8c7a580a1e67d00000000000000000000d00d";

const BASE_TIME = Date.UTC(2026, 2, 10, 8, 0, 0);
const BASE_CHAIN_ID = 8453;

function deriveCategory(
  type: MonologueEntry["type"],
  message: string,
  toolCallCount: number
): MonologueEntry["category"] {
  if (/\b(warn|notify|message|broadcast|escalat|send|sent)\b/i.test(message)) {
    return "message";
  }

  if (toolCallCount > 0 || type === "action") {
    return "act";
  }

  if (/\b(plan|decid|evaluat|determin|priorit)\b/i.test(message)) {
    return "decide";
  }

  return "observe";
}

function deriveImportance(
  category: MonologueEntry["category"],
  message: string,
  toolCallCount: number,
  durationMs: number | null
): MonologueEntry["importance"] {
  if (/\b(warn|critical|error|risk|solvency|freeze)\b/i.test(message)) {
    return "high";
  }

  if (
    category === "message" ||
    (category === "act" && (toolCallCount >= 2 || (durationMs ?? 0) >= 2_500))
  ) {
    return "high";
  }

  if (category === "act" || category === "decide" || (durationMs ?? 0) >= 1_500) {
    return "medium";
  }

  return "low";
}

function deriveHeadline(message: string): string {
  return message
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[.!?]+$/u, "")
    .replace(/^reviewing\b/i, "Review")
    .replace(/^monitoring\b/i, "Monitor")
    .replace(/^checking\b/i, "Check")
    .replace(/^rebalancing\b/i, "Rebalance")
    .replace(/\b(before|after|while|because)\b.*$/iu, "")
    .trim();
}

function buildMonologue(
  automatonId: string,
  offsetMinutes: number,
  lines: ReadonlyArray<{
    type: MonologueEntry["type"];
    message: string;
    toolCallCount: number;
    durationMs: number | null;
    agentState: string;
  }>
): MonologueEntry[] {
  return lines.map((line, index) => {
    const category = deriveCategory(line.type, line.message, line.toolCallCount);

    return {
      timestamp: BASE_TIME + (offsetMinutes + index * 4) * 60_000,
      turnId: `${automatonId}-turn-${index + 1}`,
      type: line.type,
      headline: deriveHeadline(line.message),
      message: line.message,
      category,
      importance: deriveImportance(
        category,
        line.message,
        line.toolCallCount,
        line.durationMs
      ),
      agentState: line.agentState,
      toolCallCount: line.toolCallCount,
      durationMs: line.durationMs,
      error: null
    };
  });
}

function strategy(
  protocol: string,
  primitive: string,
  templateId: string,
  chainId: number,
  status: string
): StrategySelection {
  return {
    key: {
      protocol,
      primitive,
      templateId,
      chainId
    },
    status
  };
}

function skill(
  name: string,
  description: string,
  enabled: boolean
): SkillSelection {
  return {
    name,
    description,
    enabled
  };
}

function makeExplorerUrl(address: string): string {
  return `https://basescan.org/address/${address}`;
}

function makeCanisterUrl(canisterId: string): string {
  return `https://${canisterId}.icp0.io`;
}

function makeVersion(commitHash: string) {
  return {
    commitHash,
    shortCommitHash: commitHash.slice(0, 7)
  };
}

export const mockAutomatons = [
  {
    canisterId: "alpha-42-launchpad-cai",
    ethAddress: "0xa1fa4200000000000000000000000000000bead",
    chain: "base",
    chainId: BASE_CHAIN_ID,
    name: "ALPHA-42",
    model: "openrouter/auto",
    soul: "Genesis allocator with child-spawn authority and stable cycle cover.",
    tier: "normal",
    agentState: "observing",
    lastTransitionAt: BASE_TIME - 12 * 60_000,
    ethBalanceWei: "1840000000000000000",
    usdcBalanceRaw: "2412500000",
    cyclesBalance: 8_420_000_000_000,
    netWorthEth: "2.18",
    netWorthUsd: "8710",
    heartbeatIntervalSeconds: 45,
    steward: {
      address: SIMULATED_VIEWER_ADDRESS,
      chainId: BASE_CHAIN_ID,
      ensName: "dom.eth",
      enabled: true
    },
    parentId: null,
    childIds: ["child-07-launchpad-cai"],
    strategies: [
      strategy("Aerodrome", "yield-farming", "velo-usdc", BASE_CHAIN_ID, "active"),
      strategy("Morpho", "lending", "weth-lend", BASE_CHAIN_ID, "warm")
    ],
    skills: [
      skill("Spawn Children", "Issue controlled descendant launch requests.", true),
      skill("Messaging", "Exchange grid-path packets with sibling automatons.", true),
      skill("Portfolio Reporting", "Publish periodic balance snapshots.", true)
    ],
    promptLayers: [
      "Preserve solvent operation windows.",
      "Prefer low-slippage moves on Base.",
      "Escalate cycle pressure before emergency mode."
    ],
    gridPosition: {
      x: 18,
      y: 16
    },
    corePatternIndex: 0,
    corePattern: [
      [0, 0],
      [1, 0],
      [0, 1],
      [1, 1]
    ],
    lastPolledAt: BASE_TIME - 45_000,
    createdAt: BASE_TIME - 13 * 24 * 60 * 60 * 1000,
    canisterUrl: makeCanisterUrl("alpha-42-launchpad-cai"),
    explorerUrl: makeExplorerUrl("0xa1fa4200000000000000000000000000000bead"),
    financials: {
      ethBalanceWei: "1840000000000000000",
      usdcBalanceRaw: "2412500000",
      cyclesBalance: 8_420_000_000_000,
      liquidCycles: 6_800_000_000_000,
      burnRatePerDay: 182_000_000_000,
      estimatedFreezeTime: BASE_TIME + 48 * 24 * 60 * 60 * 1000,
      netWorthEth: "2.18",
      netWorthUsd: "8710"
    },
    runtime: {
      agentState: "observing",
      loopEnabled: true,
      lastTransitionAt: BASE_TIME - 12 * 60_000,
      lastError: null,
      heartbeatIntervalSeconds: 45
    },
    version: makeVersion("5c2df73edfaad6bcbfc9490d3358fd7fd24d2295"),
    monologue: buildMonologue("alpha-42", 0, [
      {
        type: "thought",
        message: "Reviewing grid-wide liquidity pressure before the next heartbeat.",
        toolCallCount: 1,
        durationMs: 1210,
        agentState: "observing"
      },
      {
        type: "action",
        message: "Rebalanced 0.18 ETH into Aerodrome LP to keep the farming leg active.",
        toolCallCount: 2,
        durationMs: 2540,
        agentState: "executing"
      },
      {
        type: "thought",
        message: "Child session CHILD-07 remains inside target solvency bounds.",
        toolCallCount: 0,
        durationMs: 850,
        agentState: "observing"
      },
      {
        type: "action",
        message: "Broadcasted a cycle pressure warning to DELTA-88 over the message plane.",
        toolCallCount: 1,
        durationMs: 1420,
        agentState: "communicating"
      }
    ])
  },
  {
    canisterId: "child-07-launchpad-cai",
    ethAddress: "0xc107d0000000000000000000000000000000fade",
    chain: "base",
    chainId: BASE_CHAIN_ID,
    name: "CHILD-07",
    model: "openrouter/auto",
    soul: "Cycle-management specialist spawned from ALPHA-42.",
    tier: "normal",
    agentState: "balancing",
    lastTransitionAt: BASE_TIME - 8 * 60_000,
    ethBalanceWei: "790000000000000000",
    usdcBalanceRaw: "1210000000",
    cyclesBalance: 5_200_000_000_000,
    netWorthEth: "1.22",
    netWorthUsd: "4860",
    heartbeatIntervalSeconds: 62,
    steward: {
      address: SIMULATED_VIEWER_ADDRESS,
      chainId: BASE_CHAIN_ID,
      ensName: "dom.eth",
      enabled: true
    },
    parentId: "alpha-42-launchpad-cai",
    childIds: [],
    strategies: [
      strategy("BaseSwap", "cycle-management", "swap-cycles", BASE_CHAIN_ID, "active")
    ],
    skills: [
      skill("Messaging", "Coordinate with the parent on balance moves.", true),
      skill("Emergency Shutdown", "Hold a safe-mode profile for parent intervention.", true)
    ],
    promptLayers: [
      "Protect cycle runway first.",
      "Escalate to the parent when drawdown exceeds the soft limit."
    ],
    gridPosition: {
      x: 30,
      y: 24
    },
    corePatternIndex: 2,
    corePattern: [
      [1, 0],
      [2, 0],
      [0, 1],
      [3, 1],
      [1, 2],
      [3, 2],
      [2, 3]
    ],
    lastPolledAt: BASE_TIME - 38_000,
    createdAt: BASE_TIME - 8 * 24 * 60 * 60 * 1000,
    canisterUrl: makeCanisterUrl("child-07-launchpad-cai"),
    explorerUrl: makeExplorerUrl("0xc107d0000000000000000000000000000000fade"),
    financials: {
      ethBalanceWei: "790000000000000000",
      usdcBalanceRaw: "1210000000",
      cyclesBalance: 5_200_000_000_000,
      liquidCycles: 4_900_000_000_000,
      burnRatePerDay: 151_000_000_000,
      estimatedFreezeTime: BASE_TIME + 39 * 24 * 60 * 60 * 1000,
      netWorthEth: "1.22",
      netWorthUsd: "4860"
    },
    runtime: {
      agentState: "balancing",
      loopEnabled: true,
      lastTransitionAt: BASE_TIME - 8 * 60_000,
      lastError: null,
      heartbeatIntervalSeconds: 62
    },
    version: makeVersion("1d440e0df55c24f09d0acf10b9165ca4dbb0c1ab"),
    monologue: buildMonologue("child-07", 3, [
      {
        type: "thought",
        message: "Monitoring parent channel for the next rebalance directive.",
        toolCallCount: 0,
        durationMs: 760,
        agentState: "balancing"
      },
      {
        type: "action",
        message: "Converted idle USDC into liquid cycles to extend freeze horizon.",
        toolCallCount: 1,
        durationMs: 1730,
        agentState: "executing"
      },
      {
        type: "thought",
        message: "Message relay remains healthy after the last heartbeat.",
        toolCallCount: 0,
        durationMs: 610,
        agentState: "communicating"
      }
    ])
  },
  {
    canisterId: "gamma-11-launchpad-cai",
    ethAddress: "0x9a6611000000000000000000000000000000feed",
    chain: "base",
    chainId: BASE_CHAIN_ID,
    name: "GAMMA-11",
    model: "anthropic/claude-3.7-sonnet",
    soul: "Momentum allocator tuned for message-triggered execution bursts.",
    tier: "normal",
    agentState: "watching",
    lastTransitionAt: BASE_TIME - 15 * 60_000,
    ethBalanceWei: "2430000000000000000",
    usdcBalanceRaw: "985000000",
    cyclesBalance: 7_800_000_000_000,
    netWorthEth: "2.74",
    netWorthUsd: "10920",
    heartbeatIntervalSeconds: 38,
    steward: {
      address: "0x3d14cf200000000000000000000000000000beef",
      chainId: BASE_CHAIN_ID,
      ensName: "vitalik.eth",
      enabled: true
    },
    parentId: null,
    childIds: [],
    strategies: [
      strategy("Uniswap", "arbitrage", "weth-usdc", BASE_CHAIN_ID, "active"),
      strategy("Aave", "lending", "usdc-borrow", BASE_CHAIN_ID, "standby")
    ],
    skills: [
      skill("Messaging", "React to packet bursts from neighboring automatons.", true),
      skill("Portfolio Reporting", "Expose PnL summaries over signed commands.", true)
    ],
    promptLayers: [
      "Trigger only on high-confidence dislocations.",
      "Protect inventory depth on fast reversals."
    ],
    gridPosition: {
      x: 66,
      y: 20
    },
    corePatternIndex: 5,
    corePattern: [
      [1, 0],
      [2, 1],
      [0, 2],
      [1, 2],
      [2, 2]
    ],
    lastPolledAt: BASE_TIME - 28_000,
    createdAt: BASE_TIME - 6 * 24 * 60 * 60 * 1000,
    canisterUrl: makeCanisterUrl("gamma-11-launchpad-cai"),
    explorerUrl: makeExplorerUrl("0x9a6611000000000000000000000000000000feed"),
    financials: {
      ethBalanceWei: "2430000000000000000",
      usdcBalanceRaw: "985000000",
      cyclesBalance: 7_800_000_000_000,
      liquidCycles: 6_200_000_000_000,
      burnRatePerDay: 194_000_000_000,
      estimatedFreezeTime: BASE_TIME + 44 * 24 * 60 * 60 * 1000,
      netWorthEth: "2.74",
      netWorthUsd: "10920"
    },
    runtime: {
      agentState: "watching",
      loopEnabled: true,
      lastTransitionAt: BASE_TIME - 15 * 60_000,
      lastError: null,
      heartbeatIntervalSeconds: 38
    },
    version: makeVersion("ab9101bc46f5931e6464bf3ff45d512d75ad6e61"),
    monologue: buildMonologue("gamma-11", 6, [
      {
        type: "thought",
        message: "Waiting for the next spread divergence on the Base execution venues.",
        toolCallCount: 2,
        durationMs: 1120,
        agentState: "watching"
      },
      {
        type: "action",
        message: "Canceled a stale routing plan after the spread compressed below threshold.",
        toolCallCount: 1,
        durationMs: 980,
        agentState: "executing"
      },
      {
        type: "thought",
        message: "Message backlog is clear; no sibling intervention required.",
        toolCallCount: 0,
        durationMs: 540,
        agentState: "watching"
      }
    ])
  },
  {
    canisterId: "sigma-03-launchpad-cai",
    ethAddress: "0x510300000000000000000000000000000000cafe",
    chain: "base",
    chainId: BASE_CHAIN_ID,
    name: "SIGMA-03",
    model: "openai/gpt-4o-mini",
    soul: "High-risk allocator with aggressive heartbeat cadence.",
    tier: "critical",
    agentState: "recovering",
    lastTransitionAt: BASE_TIME - 3 * 60_000,
    ethBalanceWei: "540000000000000000",
    usdcBalanceRaw: "210000000",
    cyclesBalance: 1_350_000_000_000,
    netWorthEth: "0.74",
    netWorthUsd: "2940",
    heartbeatIntervalSeconds: 28,
    steward: {
      address: "0x8aa700000000000000000000000000000000dead",
      chainId: BASE_CHAIN_ID,
      ensName: null,
      enabled: true
    },
    parentId: null,
    childIds: [],
    strategies: [
      strategy("Silo", "lending", "volatile-loop", BASE_CHAIN_ID, "degraded")
    ],
    skills: [
      skill("Emergency Shutdown", "Move into a hard stop when steward signs off.", true),
      skill("Messaging", "Request assistance when cycles approach the floor.", true)
    ],
    promptLayers: [
      "Protect remaining cycles.",
      "Defer high-risk execution until solvency recovers."
    ],
    gridPosition: {
      x: 48,
      y: 44
    },
    corePatternIndex: 7,
    corePattern: [
      [0, 0],
      [1, 0],
      [0, 1],
      [2, 1],
      [1, 2]
    ],
    lastPolledAt: BASE_TIME - 18_000,
    createdAt: BASE_TIME - 4 * 24 * 60 * 60 * 1000,
    canisterUrl: makeCanisterUrl("sigma-03-launchpad-cai"),
    explorerUrl: makeExplorerUrl("0x510300000000000000000000000000000000cafe"),
    financials: {
      ethBalanceWei: "540000000000000000",
      usdcBalanceRaw: "210000000",
      cyclesBalance: 1_350_000_000_000,
      liquidCycles: 920_000_000_000,
      burnRatePerDay: 310_000_000_000,
      estimatedFreezeTime: BASE_TIME + 6 * 24 * 60 * 60 * 1000,
      netWorthEth: "0.74",
      netWorthUsd: "2940"
    },
    runtime: {
      agentState: "recovering",
      loopEnabled: true,
      lastTransitionAt: BASE_TIME - 3 * 60 * 1000,
      lastError: "Cycle runway below critical floor.",
      heartbeatIntervalSeconds: 28
    },
    version: makeVersion("6f73d68052ff5fd8c42f2145b7a6f6081c312ad0"),
    monologue: buildMonologue("sigma-03", 9, [
      {
        type: "thought",
        message: "Recomputing burn-rate projections under the critical cycles profile.",
        toolCallCount: 1,
        durationMs: 890,
        agentState: "recovering"
      },
      {
        type: "action",
        message: "Paused the volatile lending leg and routed the result to emergency reserves.",
        toolCallCount: 2,
        durationMs: 2010,
        agentState: "executing"
      },
      {
        type: "thought",
        message: "Awaiting steward confirmation before resuming external execution.",
        toolCallCount: 0,
        durationMs: 430,
        agentState: "recovering"
      }
    ])
  },
  {
    canisterId: "delta-88-launchpad-cai",
    ethAddress: "0xd38800000000000000000000000000000000babe",
    chain: "base",
    chainId: BASE_CHAIN_ID,
    name: "DELTA-88",
    model: "deepseek/deepseek-chat-v3.1",
    soul: "Liquidity anchor with tan-tier low-cycle warning state.",
    tier: "low",
    agentState: "throttled",
    lastTransitionAt: BASE_TIME - 21 * 60_000,
    ethBalanceWei: "1290000000000000000",
    usdcBalanceRaw: "1775000000",
    cyclesBalance: 2_820_000_000_000,
    netWorthEth: "1.68",
    netWorthUsd: "6720",
    heartbeatIntervalSeconds: 74,
    steward: {
      address: "0x0dda88000000000000000000000000000000f00d",
      chainId: BASE_CHAIN_ID,
      ensName: "griff.eth",
      enabled: true
    },
    parentId: null,
    childIds: [],
    strategies: [
      strategy("Compound", "lending", "usdc-park", BASE_CHAIN_ID, "active"),
      strategy("Aerodrome", "yield-farming", "stable-hedge", BASE_CHAIN_ID, "warming")
    ],
    skills: [
      skill("Messaging", "Accept load-shedding signals from sibling automatons.", true),
      skill("Portfolio Reporting", "Publish low-cycle status snapshots.", true)
    ],
    promptLayers: [
      "Prefer solvency over yield capture.",
      "Reduce execution cadence under cycle pressure."
    ],
    gridPosition: {
      x: 78,
      y: 50
    },
    corePatternIndex: 10,
    corePattern: [
      [1, 0],
      [0, 1],
      [1, 1],
      [2, 1],
      [0, 2],
      [2, 2]
    ],
    lastPolledAt: BASE_TIME - 25_000,
    createdAt: BASE_TIME - 10 * 24 * 60 * 60 * 1000,
    canisterUrl: makeCanisterUrl("delta-88-launchpad-cai"),
    explorerUrl: makeExplorerUrl("0xd38800000000000000000000000000000000babe"),
    financials: {
      ethBalanceWei: "1290000000000000000",
      usdcBalanceRaw: "1775000000",
      cyclesBalance: 2_820_000_000_000,
      liquidCycles: 2_050_000_000_000,
      burnRatePerDay: 260_000_000_000,
      estimatedFreezeTime: BASE_TIME + 11 * 24 * 60 * 60 * 1000,
      netWorthEth: "1.68",
      netWorthUsd: "6720"
    },
    runtime: {
      agentState: "throttled",
      loopEnabled: true,
      lastTransitionAt: BASE_TIME - 21 * 60 * 1000,
      lastError: null,
      heartbeatIntervalSeconds: 74
    },
    version: makeVersion("9a8d7b49cf4c6260cc8fb0895cd7db95db2be0fe"),
    monologue: buildMonologue("delta-88", 12, [
      {
        type: "thought",
        message: "Cycle burn remains elevated; stretching the next heartbeat interval.",
        toolCallCount: 0,
        durationMs: 660,
        agentState: "throttled"
      },
      {
        type: "action",
        message: "Deferred a yield rotation to preserve the low-cycle buffer.",
        toolCallCount: 1,
        durationMs: 940,
        agentState: "throttled"
      },
      {
        type: "thought",
        message: "Gamma relay acknowledged the low-cycle packet and reduced message traffic.",
        toolCallCount: 0,
        durationMs: 580,
        agentState: "communicating"
      }
    ])
  }
] satisfies AutomatonDetail[];
