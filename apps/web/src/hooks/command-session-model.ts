import type { AutomatonDetail, MonologueEntry } from "@ic-automaton/shared";
import { tokenizeCommandInput } from "../lib/cli-command-builder";
import type {
  AutomatonContext,
  AutomatonTurnRecordResponse
} from "../api/automaton";
import {
  buildCommandHelpRows,
  describeAuthLevel,
  findCommandDefinition,
  type CommandAuthLevel
} from "../lib/cli-command-registry";

export type TerminalEntryKind = "command" | "system" | "response" | "error";

export interface TerminalEntry {
  id: number;
  kind: TerminalEntryKind;
  text: string;
}

export interface CommandSessionContext {
  automaton: AutomatonDetail | null;
  automatonContext?: AutomatonContext | null;
  viewerAddress: string | null;
}

export interface CommandSessionState {
  entries: TerminalEntry[];
  history: string[];
  inputValue: string;
  historyIndex: number | null;
}

interface ParsedCommand {
  command: string;
  flags: Set<string>;
  optionValue: string | null;
  positionals: string[];
}

const WELCOME_COPY = [
  "Command Surface ready.",
  "Type help for commands.",
  "Public commands stay available without a wallet."
];

function formatAddress(address: string | null): string {
  if (address === null) {
    return "n/a";
  }

  return `${address.slice(0, 6)}...${address.slice(-4)}`;
}

function formatUsd(value: string | null): string {
  if (value === null) {
    return "n/a";
  }

  const amount = Number(value);

  if (!Number.isFinite(amount)) {
    return "n/a";
  }

  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 0
  }).format(amount);
}

function formatEth(value: string | null): string {
  if (value === null) {
    return "n/a";
  }

  const amount = Number(value);

  if (!Number.isFinite(amount)) {
    return "n/a";
  }

  return `${(amount / 1e18).toFixed(3)} ETH`;
}

function formatHexAmount(
  value: string | null | undefined,
  decimals: number,
  digits = 3
): string {
  if (value === null || value === undefined || value.trim() === "") {
    return "n/a";
  }

  let raw: bigint;

  try {
    raw = BigInt(value);
  } catch {
    return "n/a";
  }

  const base = 10n ** BigInt(decimals);
  const whole = raw / base;
  const fraction = raw % base;

  if (digits <= 0) {
    return `${whole.toString()}`;
  }

  const precision = Math.min(digits, decimals);
  const precisionBase = 10n ** BigInt(decimals - precision);
  const scaledFraction = (fraction / precisionBase).toString().padStart(precision, "0");

  return `${whole.toString()}.${scaledFraction}`;
}

function formatHexEth(value: string | null | undefined): string {
  return `${formatHexAmount(value, 18)} ETH`;
}

function formatHexUsdc(
  value: string | null | undefined,
  decimals = 6
): string {
  return `${formatHexAmount(value, decimals)} USDC`;
}

function formatMaybeTime(timestamp: number | null | undefined): string {
  if (timestamp === null || timestamp === undefined) {
    return "n/a";
  }

  return formatTime(timestamp);
}

function formatMaybeTimeFromNs(timestampNs: number | null | undefined): string {
  if (timestampNs === null || timestampNs === undefined) {
    return "n/a";
  }

  return formatMaybeTime(timestampNs / 1_000_000);
}

function getVariantLabel(value: string | Record<string, null> | null | undefined): string {
  if (value === null || value === undefined) {
    return "n/a";
  }

  if (typeof value === "string") {
    return value;
  }

  return Object.keys(value)[0] ?? "n/a";
}

function getLiveTurnEntries(
  turns: readonly AutomatonTurnRecordResponse[],
  limit: number
): string[] {
  return turns.slice(0, limit).map((turn) => {
    const timestamp = formatMaybeTimeFromNs(turn.created_at_ns);
    const kind = (turn.tool_call_count ?? 0) > 0 ? "action" : "think";
    const message = turn.inner_dialogue?.trim() || turn.input_summary?.trim() || "No message captured.";

    return `[${timestamp}] ${kind}: ${message}`;
  });
}

function formatTime(timestamp: number): string {
  return new Intl.DateTimeFormat("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "UTC"
  }).format(timestamp);
}

function buildMonologueLines(entries: readonly MonologueEntry[], limit: number): string[] {
  return entries.slice(-limit).map((entry) => {
    const kind = entry.category === "error" ? "error" : entry.type === "thought" ? "think" : "action";

    return `[${formatTime(entry.timestamp)}] ${kind}: ${entry.headline}`;
  });
}

function parseCommand(rawInput: string): ParsedCommand | null {
  const tokens = tokenizeCommandInput(rawInput.trim());

  if (tokens.length === 0) {
    return null;
  }

  const [command, ...rest] = tokens;
  const flags = new Set<string>();
  const positionals: string[] = [];
  let optionValue: string | null = null;

  for (let index = 0; index < rest.length; index += 1) {
    const token = rest[index];

    if (token === "-m") {
      optionValue = rest[index + 1] ?? null;
      if (optionValue !== null) {
        index += 1;
      }
      continue;
    }

    if (token.startsWith("-")) {
      flags.add(token);
      continue;
    }

    positionals.push(token);
  }

  return {
    command: command.toLowerCase(),
    flags,
    optionValue,
    positionals
  };
}

function createEntry(id: number, kind: TerminalEntryKind, text: string): TerminalEntry {
  return {
    id,
    kind,
    text
  };
}

function reindexEntries(startId: number, entries: readonly TerminalEntry[]): TerminalEntry[] {
  return entries.map((entry, index) => createEntry(startId + index, entry.kind, entry.text));
}

function buildWelcomeEntries(context: CommandSessionContext): TerminalEntry[] {
  const lines = [...WELCOME_COPY];

  if (context.automaton === null) {
    lines.push("Select an automaton to inspect status and logs.");
  } else if (context.viewerAddress === null) {
    lines.push(
      "Connect a wallet for wallet commands and steward commands. Public commands still work."
    );
  } else if (
    context.automaton.steward.address.toLowerCase() === context.viewerAddress.toLowerCase()
  ) {
    lines.push("Connected wallet matches the steward address.");
  } else {
    lines.push("Connected wallet does not match the steward address.");
  }

  return lines.map((text, index) => createEntry(index + 1, "system", text));
}

function buildHelpEntries(startId: number): TerminalEntry[] {
  const rows = buildCommandHelpRows();

  return [
    createEntry(startId, "system", "AVAILABLE COMMANDS"),
    ...rows.flatMap((row, index) => [
      createEntry(startId + index + 1, "system", `${row.usage}  ${row.authLabel}`),
      createEntry(startId + rows.length + index + 1, "system", `  ${row.summary}`)
    ])
  ];
}

function buildStatusEntries(context: CommandSessionContext, startId: number): TerminalEntry[] {
  const { automaton } = context;

  if (automaton === null) {
    return [createEntry(startId, "system", "No automaton is selected.")];
  }

  if (context.automatonContext !== undefined && context.automatonContext !== null) {
    const steward = context.automatonContext.stewardStatus.active_steward ?? null;

    return [
      createEntry(startId, "response", `Name: ${automaton.name}`),
      createEntry(startId + 1, "response", `Chain: ${automaton.chain.toUpperCase()}`),
      createEntry(startId + 2, "response", `Live state: ${getVariantLabel(context.automatonContext.snapshot.runtime?.state)}`),
      createEntry(startId + 3, "response", `Loop: ${context.automatonContext.snapshot.runtime?.loop_enabled ? "enabled" : "disabled"}`),
      createEntry(startId + 4, "response", `Steward: ${formatAddress(steward?.address ?? automaton.steward.address)}`),
      createEntry(startId + 5, "response", `ETH: ${formatHexEth(context.automatonContext.walletBalance.eth_balance_wei_hex ?? automaton.financials.ethBalanceWei)}`),
      createEntry(startId + 6, "response", `USDC: ${formatHexUsdc(context.automatonContext.walletBalance.usdc_balance_raw_hex ?? automaton.financials.usdcBalanceRaw, context.automatonContext.walletBalance.usdc_decimals ?? 6)}`),
      createEntry(startId + 7, "response", `Cycles: ${context.automatonContext.snapshot.cycles?.total_cycles?.toLocaleString("en-US") ?? automaton.financials.cyclesBalance.toLocaleString("en-US")}`),
      createEntry(startId + 8, "response", `Heartbeat: ${context.automatonContext.schedulerConfig.default_turn_interval_secs ?? automaton.runtime.heartbeatIntervalSeconds ?? "n/a"}s`),
      createEntry(startId + 9, "response", `Last transition: ${formatMaybeTimeFromNs(context.automatonContext.snapshot.runtime?.last_transition_at_ns)}`),
      createEntry(startId + 10, "response", `Last sync: ${formatMaybeTimeFromNs(context.automatonContext.walletBalance.last_synced_at_ns)}`)
    ];
  }

  return [
    createEntry(startId, "response", `Name: ${automaton.name}`),
    createEntry(startId + 1, "response", `Chain: ${automaton.chain.toUpperCase()}`),
    createEntry(startId + 2, "response", `Tier: ${automaton.tier}`),
    createEntry(startId + 3, "response", `State: ${automaton.runtime.agentState}`),
    createEntry(startId + 4, "response", `Steward: ${formatAddress(automaton.steward.address)}`),
    createEntry(startId + 5, "response", `ETH: ${formatEth(automaton.financials.ethBalanceWei)}`),
    createEntry(startId + 6, "response", `USD: ${formatUsd(automaton.financials.netWorthUsd)}`),
    createEntry(
      startId + 7,
      "response",
      `Heartbeat: ${automaton.runtime.heartbeatIntervalSeconds ?? "n/a"}s`
    )
  ];
}

function buildConfigEntries(context: CommandSessionContext, startId: number): TerminalEntry[] {
  const { automaton } = context;

  if (automaton === null) {
    return [createEntry(startId, "system", "No automaton configuration is loaded.")];
  }

  if (context.automatonContext !== undefined && context.automatonContext !== null) {
    const promptLayerCount = automaton.promptLayers.length;
    const strategyCount = automaton.strategies.length;
    const skillCount = automaton.skills.length;

    return [
      createEntry(startId, "response", `Canister: ${automaton.canisterId}`),
      createEntry(startId + 1, "response", `Commit: ${context.automatonContext.buildInfo.commit ?? automaton.version.commitHash}`),
      createEntry(startId + 2, "response", `Chain ID: ${context.automatonContext.evmConfig.chain_id ?? automaton.chainId}`),
      createEntry(
        startId + 3,
        "response",
        `Automaton address: ${context.automatonContext.evmConfig.automaton_address ?? automaton.ethAddress ?? "n/a"}`
      ),
      createEntry(
        startId + 4,
        "response",
        `Inbox contract: ${context.automatonContext.evmConfig.inbox_contract_address ?? "n/a"}`
      ),
      createEntry(
        startId + 5,
        "response",
        `Steward active: ${context.automatonContext.stewardStatus.active_steward?.enabled ? "yes" : "no"}`
      ),
      createEntry(
        startId + 6,
        "response",
        `Heartbeat: ${context.automatonContext.schedulerConfig.default_turn_interval_secs ?? automaton.runtime.heartbeatIntervalSeconds ?? "n/a"}s`
      ),
      createEntry(
        startId + 7,
        "response",
        `Prompt layers: ${promptLayerCount} | Strategies: ${strategyCount} | Skills: ${skillCount}`
      )
    ];
  }

  return [
    createEntry(startId, "response", `Canister: ${automaton.canisterId}`),
    createEntry(startId + 1, "response", `Version: ${automaton.version.shortCommitHash}`),
    createEntry(startId + 2, "response", `Prompt layers: ${automaton.promptLayers.length}`),
    createEntry(startId + 3, "response", `Strategies: ${automaton.strategies.length}`),
    createEntry(startId + 4, "response", `Skills: ${automaton.skills.length}`),
    createEntry(startId + 5, "response", `Steward enabled: ${automaton.steward.enabled ? "yes" : "no"}`)
  ];
}

function buildLogEntries(
  context: CommandSessionContext,
  startId: number,
  followMode: boolean
): TerminalEntry[] {
  const { automaton } = context;

  if (automaton === null) {
    return [createEntry(startId, "system", "No indexed activity is available.")];
  }

  const logLines =
    context.automatonContext !== undefined && context.automatonContext !== null
      ? getLiveTurnEntries(context.automatonContext.snapshot.recent_turns ?? [], 5)
      : buildMonologueLines(automaton.monologue, 5);

  if (logLines.length === 0) {
    return [createEntry(startId, "system", "No indexed activity entries yet.")];
  }

  const entries = logLines.map((line, index) => createEntry(startId + index, "response", line));

  if (followMode) {
    entries.push(
      createEntry(
        startId + logLines.length,
        "system",
        "Follow mode is not wired in this launchpad slice."
      )
    );
  }

  return entries;
}

function buildPeekEntries(
  context: CommandSessionContext,
  startId: number,
  followMode: boolean
): TerminalEntry[] {
  const { automaton } = context;

  if (automaton === null) {
    return [createEntry(startId, "system", "No indexed monologue is available.")];
  }

  const thoughtLines =
    context.automatonContext !== undefined && context.automatonContext !== null
      ? getLiveTurnEntries(
          (context.automatonContext.snapshot.recent_turns ?? []).filter(
            (entry) => (entry.tool_call_count ?? 0) === 0
          ),
          5
        )
      : buildMonologueLines(
          automaton.monologue.filter((entry) => entry.type === "thought"),
          5
        );

  if (thoughtLines.length === 0) {
    return [createEntry(startId, "system", "No thought entries have been indexed yet.")];
  }

  const entries = thoughtLines.map((line, index) => createEntry(startId + index, "response", line));

  if (followMode) {
    entries.push(
      createEntry(
        startId + thoughtLines.length,
        "system",
        "Follow mode is not wired in this launchpad slice."
      )
    );
  }

  return entries;
}

function buildHistoryEntries(history: readonly string[], startId: number): TerminalEntry[] {
  if (history.length === 0) {
    return [createEntry(startId, "system", "No commands entered yet.")];
  }

  return history.slice(-8).map((command, index) =>
    createEntry(startId + index, "response", `${index + 1}. ${command}`)
  );
}

function buildInboxEntries(context: CommandSessionContext, startId: number): TerminalEntry[] {
  if (context.automaton === null) {
    return [createEntry(startId, "system", "No inbox is available without a selected automaton.")];
  }

  if (context.automatonContext !== undefined && context.automatonContext !== null) {
    const steward = context.automatonContext.stewardStatus.active_steward;
    const inboxAddress = context.automatonContext.evmConfig.inbox_contract_address ?? "n/a";
    const automatonAddress = context.automatonContext.evmConfig.automaton_address ?? context.automaton.ethAddress ?? "n/a";

    return [
      createEntry(startId, "response", `Inbox contract: ${inboxAddress}`),
      createEntry(startId + 1, "response", `Automaton address: ${automatonAddress}`),
      createEntry(startId + 2, "response", `Steward active: ${steward?.enabled ? "yes" : "no"}`),
      createEntry(startId + 3, "response", `Steward nonce: ${context.automatonContext.stewardStatus.next_nonce ?? "n/a"}`),
      createEntry(
        startId + 4,
        "system",
        inboxAddress === "n/a"
          ? "The live automaton has no inbox contract configured."
          : "Inbox message history is not exposed over HTTP, but the live route is configured."
      )
    ];
  }

  return [
    createEntry(
      startId,
      "system",
      `Unread replies are not indexed here for ${context.automaton.canisterId}.`
    )
  ];
}

function buildPriceEntries(context: CommandSessionContext, startId: number): TerminalEntry[] {
  if (context.automaton === null) {
    return [createEntry(startId, "system", "No price snapshot is available without a selection.")];
  }

  if (context.automatonContext !== undefined && context.automatonContext !== null) {
    const ethBalance = formatHexEth(
      context.automatonContext.walletBalance.eth_balance_wei_hex ?? context.automaton.financials.ethBalanceWei
    );
    const usdcBalance = formatHexUsdc(
      context.automatonContext.walletBalance.usdc_balance_raw_hex ?? context.automaton.financials.usdcBalanceRaw,
      context.automatonContext.walletBalance.usdc_decimals ?? 6
    );
    const cycles = context.automatonContext.snapshot.cycles?.total_cycles ?? context.automaton.financials.cyclesBalance;
    const liquidCycles =
      context.automatonContext.snapshot.cycles?.liquid_cycles ?? context.automaton.financials.liquidCycles;

    return [
      createEntry(startId, "response", `ETH balance: ${ethBalance}`),
      createEntry(startId + 1, "response", `USDC balance: ${usdcBalance}`),
      createEntry(startId + 2, "response", `Cycles: ${cycles.toLocaleString("en-US")}`),
      createEntry(startId + 3, "response", `Liquid cycles: ${liquidCycles.toLocaleString("en-US")}`),
      createEntry(
        startId + 4,
        "system",
        "The live automaton does not expose a market price feed over HTTP, so this view shows the on-chain balance snapshot."
      )
    ];
  }

  return [
    createEntry(startId, "response", `ETH balance: ${formatEth(context.automaton.financials.ethBalanceWei)}`),
    createEntry(startId + 1, "response", `USDC balance: ${context.automaton.financials.usdcBalanceRaw ?? "n/a"}`),
    createEntry(startId + 2, "response", `Net worth: ${formatUsd(context.automaton.financials.netWorthUsd)}`)
  ];
}

function isStewardWallet(context: CommandSessionContext): boolean {
  const { automaton, viewerAddress } = context;

  if (automaton === null || viewerAddress === null) {
    return false;
  }

  return automaton.steward.address.toLowerCase() === viewerAddress.toLowerCase();
}

function buildUnauthorizedEntries(
  authLevel: CommandAuthLevel,
  startId: number,
  commandName: string
): TerminalEntry[] {
  if (authLevel === "wallet") {
    return [
      createEntry(startId, "error", `Wallet required for ${commandName}. Connect a wallet first.`)
    ];
  }

  return [
    createEntry(
      startId,
      "error",
      `Steward required for ${commandName}. Connect the recorded steward wallet.`
    )
  ];
}

function buildWalletEntry(
  context: CommandSessionContext,
  startId: number,
  commandName: string,
  parsed: ParsedCommand
): TerminalEntry[] {
  if (commandName.startsWith("steward")) {
    return [
      createEntry(
        startId,
        "error",
        "Internal routing error: steward commands require the signed command executor."
      )
    ];
  }

  if (context.viewerAddress === null) {
    return [
      createEntry(startId, "error", `Wallet required for ${commandName}. Connect a wallet first.`)
    ];
  }

  if (commandName.startsWith("steward") && !isStewardWallet(context)) {
    return [
      createEntry(
        startId,
        "error",
        "Steward required for this command. Connect the recorded steward wallet."
      )
    ];
  }

  const message =
    commandName === "send" || commandName === "steward-send"
      ? parsed.optionValue ?? parsed.positionals.join(" ")
      : commandName === "donate"
        ? parsed.positionals[0] ?? null
        : parsed.positionals.join(" ");

  if (commandName === "send" || commandName === "steward-send") {
    return [
      createEntry(startId, "response", `Message command accepted: ${message || "no message supplied"}`),
      createEntry(startId + 1, "system", "Wallet transport is handled by the live wallet session.")
    ];
  }

  if (commandName === "donate") {
    const asset = parsed.flags.has("--usdc") ? "USDC" : "ETH";

    return [
      createEntry(startId, "response", `Donation command accepted: ${message || "0"} ${asset}`),
      createEntry(startId + 1, "system", "Wallet transport is handled by the live wallet session.")
    ];
  }

  return [
    createEntry(startId, "response", `Wallet command accepted: ${commandName}`),
    createEntry(startId + 1, "system", "Wallet transport is handled by the live wallet session.")
  ];
}

function buildLocalCommandEntries(
  context: CommandSessionContext,
  parsed: ParsedCommand,
  history: readonly string[],
  commandName: string,
  startId: number
): TerminalEntry[] {
  switch (commandName) {
    case "connect":
      return [
        createEntry(
          startId,
          "response",
          "Wallet connection is handled from the header button in this launchpad slice."
        )
      ];
    case "disconnect":
      return [
        createEntry(
          startId,
          "response",
          "Wallet disconnection is handled from the header button in this launchpad slice."
        )
      ];
    case "help":
      return buildHelpEntries(startId);
    case "clear":
      return [createEntry(startId, "system", "Terminal cleared.")];
    case "code":
      return [
        createEntry(startId, "response", "Source repository: https://github.com/0x21e8/ic-automaton")
      ];
    case "status":
      return buildStatusEntries(context, startId);
    case "config":
      return buildConfigEntries(context, startId);
    case "log":
      return buildLogEntries(context, startId, parsed.flags.has("-f"));
    case "peek":
      return buildPeekEntries(context, startId, parsed.flags.has("-f"));
    case "history":
      return buildHistoryEntries(history, startId);
    case "inbox":
      return buildInboxEntries(context, startId);
    case "price":
      return buildPriceEntries(context, startId);
    default:
      return [];
  }
}

export function createCommandSessionState(context: CommandSessionContext): CommandSessionState {
  return {
    entries: buildWelcomeEntries(context),
    history: [],
    inputValue: "",
    historyIndex: null
  };
}

export function resetCommandSessionState(context: CommandSessionContext): CommandSessionState {
  return createCommandSessionState(context);
}

export function setCommandSessionInput(
  state: CommandSessionState,
  value: string
): CommandSessionState {
  return {
    ...state,
    inputValue: value,
    historyIndex: null
  };
}

export function clearCommandSessionOutput(state: CommandSessionState): CommandSessionState {
  return {
    ...state,
    entries: []
  };
}

export function stepCommandSessionHistory(
  state: CommandSessionState,
  direction: "up" | "down"
): CommandSessionState {
  if (state.history.length === 0) {
    return state;
  }

  if (direction === "up") {
    const nextIndex =
      state.historyIndex === null
        ? state.history.length - 1
        : Math.max(state.historyIndex - 1, 0);

    return {
      ...state,
      historyIndex: nextIndex,
      inputValue: state.history[nextIndex] ?? ""
    };
  }

  if (state.historyIndex === null) {
    return state;
  }

  if (state.historyIndex >= state.history.length - 1) {
    return {
      ...state,
      historyIndex: null,
      inputValue: ""
    };
  }

  const nextIndex = state.historyIndex + 1;

  return {
    ...state,
    historyIndex: nextIndex,
    inputValue: state.history[nextIndex] ?? ""
  };
}

export function submitCommandSessionInput(
  state: CommandSessionState,
  context: CommandSessionContext,
  rawInput: string
): CommandSessionState {
  const trimmed = rawInput.trim();

  if (trimmed === "") {
    return state;
  }

  const parsed = parseCommand(trimmed);
  const nextHistory = [...state.history, trimmed];

  if (parsed === null) {
    return {
      ...state,
      entries: [...state.entries, ...reindexEntries(state.entries.length + 1, [
        createEntry(0, "command", `> ${trimmed}`),
        createEntry(0, "error", "No command entered.")
      ])],
      history: nextHistory,
      historyIndex: null,
      inputValue: ""
    };
  }

  const commandBlock = [createEntry(0, "command", `> ${trimmed}`)];
  const definition = findCommandDefinition(parsed.command);

  if (definition === null) {
    return {
      ...state,
      entries: [...state.entries, ...reindexEntries(state.entries.length + 1, [
        ...commandBlock,
        createEntry(0, "error", `Unknown command: ${parsed.command}. Type help for assistance.`)
      ])],
      history: nextHistory,
      historyIndex: null,
      inputValue: ""
    };
  }

  if (definition.authLevel === "public") {
    if (definition.name === "clear") {
      return {
        ...state,
        entries: [createEntry(1, "system", "Terminal cleared.")],
        history: nextHistory,
        historyIndex: null,
        inputValue: ""
      };
    }

    return {
      ...state,
      entries: [
        ...state.entries,
        ...reindexEntries(state.entries.length + 1, [
          ...commandBlock,
          ...buildLocalCommandEntries(context, parsed, nextHistory, definition.name, 0)
        ])
      ],
      history: nextHistory,
      historyIndex: null,
      inputValue: ""
    };
  }

  if (definition.authLevel === "wallet" && context.viewerAddress === null) {
    return {
      ...state,
      entries: [
        ...state.entries,
        ...reindexEntries(state.entries.length + 1, [
          ...commandBlock,
          ...buildUnauthorizedEntries("wallet", 0, definition.name)
        ])
      ],
      history: nextHistory,
      historyIndex: null,
      inputValue: ""
    };
  }

  if (definition.authLevel === "steward" && !isStewardWallet(context)) {
    return {
      ...state,
      entries: [
        ...state.entries,
        ...reindexEntries(state.entries.length + 1, [
          ...commandBlock,
          ...buildUnauthorizedEntries("steward", 0, definition.name)
        ])
      ],
      history: nextHistory,
      historyIndex: null,
      inputValue: ""
    };
  }

  return {
    ...state,
    entries: [
      ...state.entries,
      ...reindexEntries(state.entries.length + 1, [
        ...commandBlock,
        ...buildWalletEntry(context, 0, definition.name, parsed)
      ])
    ],
    history: nextHistory,
    historyIndex: null,
    inputValue: ""
  };
}

export function isStewardCommandSession(context: CommandSessionContext): boolean {
  return isStewardWallet(context);
}

export function describeCommandSessionAuth(context: CommandSessionContext): string {
  if (context.viewerAddress === null) {
    return describeAuthLevel("public");
  }

  if (isStewardWallet(context)) {
    return describeAuthLevel("steward");
  }

  return describeAuthLevel("wallet");
}

export { buildCommandHelpRows };
