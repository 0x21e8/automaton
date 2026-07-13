import { describe, expect, it } from "vitest";

import type { AutomatonDetail } from "@ic-automaton/shared";
import type { AutomatonContext } from "../api/automaton";
import {
  clearCommandSessionOutput,
  createCommandSessionState,
  setCommandSessionInput,
  stepCommandSessionHistory,
  submitCommandSessionInput
} from "./command-session-model";

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
    snapshot: {
      cycles: {
        burn_rate_cycles_per_day: 1000,
        estimated_freeze_time_ns: 1_700_000_500_000_000_000,
        liquid_cycles: 2_000_000_000_000,
        total_cycles: 4_500_000_000_000
      },
      prompt_layers: [{ content: "base constitution" }],
      recent_turns: [
        {
          created_at_ns: 1_700_000_010_000_000_000,
          duration_ms: 120,
          id: "turn-1",
          inner_dialogue: "Reviewing the live balance snapshot.",
          input_summary: "audit snapshot",
          state_from: "Idle",
          state_to: "Observing",
          tool_call_count: 0
        },
        {
          created_at_ns: 1_700_000_020_000_000_000,
          duration_ms: 240,
          id: "turn-2",
          inner_dialogue: "Submitted the live execution request.",
          input_summary: "submit trade",
          state_from: "Observing",
          state_to: "Executing",
          tool_call_count: 2
        }
      ],
      runtime: {
        last_error: null,
        last_transition_at_ns: 1_700_000_030_000_000_000,
        loop_enabled: true,
        soul: "Tends the treasury.",
        state: "observing"
      },
      scheduler: {
        enabled: true,
        last_tick_error: null,
        survival_tier: "Normal"
      }
    },
    walletBalance: {
      age_secs: 5,
      bootstrap_pending: false,
      eth_balance_wei_hex: "0xde0b6b3a7640000",
      freshness_window_secs: 30,
      is_stale: false,
      last_error: null,
      last_synced_at_ns: 1_700_000_040_000_000_000,
      status: "ok",
      usdc_balance_raw_hex: "0x5f5e100",
      usdc_contract_address: "0x0000000000000000000000000000000000000001",
      usdc_decimals: 6
    },
    fetchedAt: 1_700_000_050_000
  };
}

describe("command session model", () => {
  it("resets to the welcome copy for the active context", () => {
    const state = createCommandSessionState({
      automaton: createAutomatonDetail(),
      viewerAddress: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
    });

    expect(state.entries[0]?.text).toBe("Command Surface ready.");
    expect(state.entries.at(-1)?.text).toBe("Connected wallet matches the steward address.");
    expect(state.history).toEqual([]);
    expect(state.inputValue).toBe("");
  });

  it("submits commands while keeping history and command output in sync", () => {
    const context = {
      automaton: createAutomatonDetail(),
      viewerAddress: null
    };
    const initialState = createCommandSessionState(context);
    const preparedState = setCommandSessionInput(initialState, "help");
    const submittedState = submitCommandSessionInput(preparedState, context, preparedState.inputValue);

    expect(submittedState.inputValue).toBe("");
    expect(submittedState.history).toEqual(["help"]);
    expect(submittedState.entries.some((entry) => entry.text === "AVAILABLE COMMANDS")).toBe(true);
    expect(
      submittedState.entries.some((entry) => entry.text === "  Set the OpenRouter reasoning effort.")
    ).toBe(true);
  });

  it("supports history traversal and output clearing as dedicated transitions", () => {
    const context = {
      automaton: createAutomatonDetail(),
      viewerAddress: null
    };
    const firstCommand = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "status"),
      context,
      "status"
    );
    const secondCommand = submitCommandSessionInput(
      setCommandSessionInput(firstCommand, "config"),
      context,
      "config"
    );
    const historyUp = stepCommandSessionHistory(secondCommand, "up");
    const historyDown = stepCommandSessionHistory(historyUp, "down");
    const cleared = clearCommandSessionOutput(secondCommand);

    expect(secondCommand.history).toEqual(["status", "config"]);
    expect(historyUp.inputValue).toBe("config");
    expect(historyDown.inputValue).toBe("");
    expect(cleared.entries).toEqual([]);
    expect(cleared.history).toEqual(["status", "config"]);
  });

  it("uses live automaton data for query commands", () => {
    const context = {
      automaton: createAutomatonDetail(),
      automatonContext: createAutomatonContext(),
      viewerAddress: null
    };

    const statusState = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "status"),
      context,
      "status"
    );
    const configState = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "config"),
      context,
      "config"
    );
    const logState = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "log"),
      context,
      "log"
    );
    const peekState = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "peek"),
      context,
      "peek"
    );
    const inboxState = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "inbox"),
      context,
      "inbox"
    );
    const priceState = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "price"),
      context,
      "price"
    );

    expect(statusState.entries.some((entry) => entry.text === "Live state: observing")).toBe(true);
    expect(statusState.entries.some((entry) => entry.text === "ETH: 1.000 ETH")).toBe(true);
    expect(configState.entries.some((entry) => entry.text === "Inbox contract: 0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed")).toBe(true);
    expect(
      logState.entries.some((entry) => entry.text.includes("Reviewing the live balance snapshot."))
    ).toBe(true);
    expect(peekState.entries.every((entry) => !entry.text.includes("submit trade"))).toBe(true);
    expect(inboxState.entries.some((entry) => entry.text === "Steward nonce: 17")).toBe(true);
    expect(priceState.entries.some((entry) => entry.text === "USDC balance: 100.000 USDC")).toBe(true);
  });

  it("never reports a steward command as locally accepted", () => {
    const context = {
      automaton: createAutomatonDetail(),
      viewerAddress: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
    };
    const state = submitCommandSessionInput(
      setCommandSessionInput(createCommandSessionState(context), "steward-model x"),
      context,
      "steward-model x"
    );

    expect(state.entries.at(-1)?.kind).toBe("error");
    expect(state.entries.at(-1)?.text).toContain("Internal routing error");
    expect(state.entries.some((entry) => entry.text.includes("accepted"))).toBe(false);
  });
});
