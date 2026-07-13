import { describe, expect, it, vi } from "vitest";

import type { AutomatonDetail } from "@ic-automaton/shared";
import type { AutomatonContext } from "../api/automaton";
import { executeWalletCommand } from "./wallet-command-executor";

function createAutomatonDetail(): AutomatonDetail {
  return {
    agentState: "idle",
    canisterId: "txyno-ch777-77776-aaaaq-cai",
    canisterUrl: "http://txyno-ch777-77776-aaaaq-cai.localhost:8000/",
    chain: "base",
    chainId: 8453,
    constitutionVerification: {
      status: "unavailable",
      expectedHash: null,
      computedHash: null
    },
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
    skills: [],
    soul: "Tends the treasury.",
    steward: {
      address: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
      chainId: 8453,
      enabled: true,
      ensName: null
    },
    strategies: [],
    tier: "normal",
    usdcBalanceRaw: "0",
    version: {
      commitHash: "0123456789abcdef0123456789abcdef01234567",
      shortCommitHash: "0123456"
    }
  };
}

function createAutomatonContext(): AutomatonContext {
  return {
    buildInfo: {
      commit: "0123456789abcdef0123456789abcdef01234567"
    },
    evmConfig: {
      automaton_address: "0x1234567890abcdef1234567890abcdef12345678",
      chain_id: 8453,
      inbox_contract_address: "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed"
    },
    schedulerConfig: {
      base_tick_secs: 5,
      default_turn_interval_secs: 42,
      ticks_per_turn_interval: 8
    },
    stewardStatus: {
      active_steward: {
        address: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
        chain_id: 8453,
        enabled: true
      },
      next_nonce: 17
    },
    snapshot: {},
    walletBalance: {
      usdc_contract_address: "0x0000000000000000000000000000000000000001",
      usdc_decimals: 6
    },
    fetchedAt: 1_700_000_050_000
  };
}

describe("wallet command executor", () => {
  it("submits send commands through the wallet provider", async () => {
    const request = vi.fn().mockResolvedValue("0xabc123");

    const result = await executeWalletCommand(
      'send -m "hello world"',
      {
        automaton: createAutomatonDetail(),
        automatonContext: createAutomatonContext(),
        viewerAddress: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
      },
      { request }
    );

    expect(request).toHaveBeenCalledWith({
      method: "eth_sendTransaction",
      params: [
        {
          from: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
          to: "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed",
          data: "0x68656c6c6f20776f726c64",
          value: "0x0"
        }
      ]
    });
    expect(result?.entries.some((entry) => entry.text === "Message transaction submitted: 0xabc123")).toBe(true);
  });

  it("submits eth donations as value transfers", async () => {
    const request = vi.fn().mockResolvedValue("0xdef456");

    const result = await executeWalletCommand(
      "donate 0.5",
      {
        automaton: createAutomatonDetail(),
        automatonContext: createAutomatonContext(),
        viewerAddress: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
      },
      { request }
    );

    expect(request).toHaveBeenCalledWith({
      method: "eth_sendTransaction",
      params: [
        {
          from: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
          to: "0x1234567890abcdef1234567890abcdef12345678",
          value: "0x6f05b59d3b20000"
        }
      ]
    });
    expect(result?.entries.some((entry) => entry.text === "Donation transaction submitted: 0xdef456")).toBe(true);
  });

  it("submits usdc donations as token transfers", async () => {
    const request = vi.fn().mockResolvedValue("0x789abc");

    const result = await executeWalletCommand(
      "donate 12.5 --usdc",
      {
        automaton: createAutomatonDetail(),
        automatonContext: createAutomatonContext(),
        viewerAddress: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
      },
      { request }
    );

    expect(request).toHaveBeenCalledWith({
      method: "eth_sendTransaction",
      params: [
        {
          from: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd",
          to: "0x0000000000000000000000000000000000000001",
          data:
            "0xa9059cbb0000000000000000000000001234567890abcdef1234567890abcdef123456780000000000000000000000000000000000000000000000000000000000bebc20",
          value: "0x0"
        }
      ]
    });
    expect(result?.entries.some((entry) => entry.text === "Donation transaction submitted: 0x789abc")).toBe(true);
  });
});
