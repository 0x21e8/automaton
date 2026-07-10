import {
  findTerminalCommand,
  terminalCommandRegistry,
  type TerminalCommandAuthLevel,
  type TerminalCommandDefinition,
  type TerminalCommandMode,
  type TerminalCommandTransport
} from "@ic-automaton/shared";

export type CommandAuthLevel = TerminalCommandAuthLevel;
export type CommandTransport = TerminalCommandTransport;
export type CommandMode = TerminalCommandMode;
export type CommandDefinition = TerminalCommandDefinition;
export interface CommandHelpRow extends CommandDefinition { authLabel: string; }
export const commandRegistry: CommandDefinition[] = [...terminalCommandRegistry];
export function findCommandDefinition(commandName: string): CommandDefinition | null { return findTerminalCommand(commandName); }
export function describeAuthLevel(authLevel: CommandAuthLevel): string {
  switch (authLevel) {
    case "wallet": return "Wallet required";
    case "steward": return "Steward required";
    default: return "Public";
  }
}
export function buildCommandHelpRows(): CommandHelpRow[] {
  return commandRegistry.map((definition) => ({ ...definition, authLabel: describeAuthLevel(definition.authLevel) }));
}
