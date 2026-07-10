export type TerminalCommandAuthLevel = "public" | "wallet" | "steward";
export type TerminalCommandTransport = "local" | "query" | "wallet" | "steward_signed";
export type TerminalCommandMode = "local" | "query" | "mutation";

export interface TerminalCommandDefinition {
  name: string;
  usage: string;
  summary: string;
  authLevel: TerminalCommandAuthLevel;
  transport: TerminalCommandTransport;
  mode: TerminalCommandMode;
}

export const terminalCommandRegistry: readonly TerminalCommandDefinition[] = [
  ["help", "help", "Show the terminal command reference.", "public", "local", "local"],
  ["clear", "clear", "Clear the terminal output.", "public", "local", "local"],
  ["code", "code", "Open the source repository reference.", "public", "local", "local"],
  ["status", "status", "Show the selected automaton status.", "public", "query", "query"],
  ["config", "config", "Show the selected automaton configuration.", "public", "query", "query"],
  ["log", "log [-f]", "Show indexed activity log entries.", "public", "query", "query"],
  ["peek", "peek [-f]", "Show indexed monologue entries.", "public", "query", "query"],
  ["inbox", "inbox", "Show unread reply status.", "public", "query", "query"],
  ["history", "history", "Show recent terminal commands.", "public", "local", "local"],
  ["price", "price", "Show the selected automaton price and balance snapshot.", "public", "query", "query"],
  ["connect", "connect", "Connect the active EVM wallet.", "public", "local", "local"],
  ["disconnect", "disconnect", "Disconnect the active EVM wallet.", "public", "local", "local"],
  ["send", "send -m \"message\" [--usdc]", "Post a message to the automaton.", "wallet", "wallet", "mutation"],
  ["donate", "donate <amount> [--usdc]", "Send funds directly to the automaton.", "wallet", "wallet", "mutation"],
  ["steward-send", "steward-send -m \"message\"", "Send a direct steward message.", "steward", "steward_signed", "mutation"],
  ["steward-model", "steward-model <variant>", "Set the inference model variant.", "steward", "steward_signed", "mutation"],
  ["steward-reasoning", "steward-reasoning <variant>", "Set the OpenRouter reasoning effort.", "steward", "steward_signed", "mutation"]
].map(([name, usage, summary, authLevel, transport, mode]) => ({ name, usage, summary, authLevel, transport, mode } as TerminalCommandDefinition));

export function findTerminalCommand(commandName: string): TerminalCommandDefinition | null {
  const normalized = commandName.trim().toLowerCase();
  return terminalCommandRegistry.find((definition) => definition.name === normalized) ?? null;
}
