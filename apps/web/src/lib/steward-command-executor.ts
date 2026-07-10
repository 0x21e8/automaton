import type { AutomatonDetail } from "@ic-automaton/shared";

import type { AutomatonStewardStatusResponse } from "../api/automaton";
import { buildCliCommandPayload, tokenizeCommandInput } from "./cli-command-builder";
import type { TerminalEntry } from "../hooks/command-session-model";
import type { WalletTransport } from "./wallet-transport";

export interface StewardProofTemplate {
  canister_id: string;
  chain_id: number;
  address: string;
  command_hash: string;
  nonce: number;
  expires_at_ns: string;
  signature?: string;
  [key: string]: unknown;
}

interface PreparedCommand {
  proof_template: StewardProofTemplate;
  signing_payload: string;
  [key: string]: unknown;
}

interface ExecuteResponse {
  result?: string;
  [key: string]: unknown;
}

export interface StewardCommandExecutionContext {
  automaton: AutomatonDetail | null;
  canisterUrl: string;
  connectedAddress: string;
  connectedChainId: number;
  request: WalletTransport["request"];
  refreshStewardStatus: () => Promise<AutomatonStewardStatusResponse>;
  sleep?: (milliseconds: number) => Promise<void>;
}

export interface StewardCommandExecutionResult {
  entries: TerminalEntry[];
}

class HttpRequestError extends Error {
  constructor(
    message: string,
    readonly status: number | null,
    readonly transient: boolean
  ) {
    super(message);
    this.name = "HttpRequestError";
  }
}

const DEFAULT_SLEEP = (milliseconds: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, milliseconds));

function entry(id: number, kind: TerminalEntry["kind"], text: string): TerminalEntry {
  return { id, kind, text };
}

function errorEntries(rawInput: string, message: string): StewardCommandExecutionResult {
  return {
    entries: [entry(1, "command", `> ${rawInput.trim()}`), entry(2, "error", message)]
  };
}

function transientStatus(status: number): boolean {
  return status === 408 || status === 425 || status === 429 || status >= 500;
}

async function requestJson<T>(
  canisterUrl: string,
  path: string,
  body: Record<string, unknown>
): Promise<T> {
  let response: Response;
  try {
    response = await fetch(new URL(path, canisterUrl), {
      method: "POST",
      headers: {
        accept: "application/json",
        "content-type": "application/json"
      },
      body: JSON.stringify(body)
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : "Network request failed.";
    throw new HttpRequestError(message, null, true);
  }

  if (!response.ok) {
    throw new HttpRequestError(
      `Steward command request failed: ${response.status} ${response.statusText}`,
      response.status,
      transientStatus(response.status)
    );
  }

  return (await response.json()) as T;
}

function encodeUtf8Hex(payload: string): string {
  return `0x${Array.from(new TextEncoder().encode(payload), (byte) =>
    byte.toString(16).padStart(2, "0")
  ).join("")}`;
}

function parseCommand(rawInput: string): { command: string; args: string[]; message: string | null } | null {
  const payload = buildCliCommandPayload(rawInput, "steward-command");
  if (payload === null) {
    return null;
  }

  const tokens = tokenizeCommandInput(rawInput.trim());
  const [command, ...rest] = tokens;
  const messageIndex = rest.indexOf("-m");
  const message = messageIndex >= 0 ? rest[messageIndex + 1] ?? "" : null;
  const args = messageIndex >= 0 ? rest.filter((_, index) => index !== messageIndex && index !== messageIndex + 1) : rest;

  return { command: command.toLowerCase(), args, message };
}

function validatePrepared(
  prepared: PreparedCommand,
  context: StewardCommandExecutionContext
): string | null {
  const proof = prepared?.proof_template;
  if (
    proof === null ||
    typeof proof !== "object" ||
    typeof prepared?.signing_payload !== "string" ||
    prepared.signing_payload.trim() === ""
  ) {
    return "Steward command preparation payload is incomplete.";
  }
  if (proof.address.toLowerCase() !== context.connectedAddress.toLowerCase()) {
    return "Prepared steward address does not match the connected wallet.";
  }
  if (proof.chain_id !== context.connectedChainId) {
    return "Prepared steward chain does not match the connected wallet.";
  }
  if (!Number.isFinite(proof.nonce)) {
    return "Prepared steward nonce is invalid.";
  }
  return null;
}

function commandMapping(
  parsed: { command: string; args: string[]; message: string | null },
  connectedAddress: string
): {
  preparePath: string;
  prepareBody: Record<string, unknown>;
  executePath: string;
  buildExecuteBody: (prepared: PreparedCommand, signature: string) => Record<string, unknown>;
} | { error: string } {
  if (parsed.command === "steward-send") {
    const message = parsed.message ?? parsed.args.join(" ");
    if (message.trim() === "") {
      return { error: 'Usage: steward-send -m "message"' };
    }
    return {
      preparePath: "/api/steward/direct-message/prepare",
      prepareBody: { sender: connectedAddress, message },
      executePath: "/api/steward/direct-message/execute",
      buildExecuteBody: (prepared, signature) => ({
        sender: String(prepared.sender ?? connectedAddress),
        message: String(prepared.message ?? message),
        proof: { ...prepared.proof_template, signature }
      })
    };
  }

  if (parsed.command === "steward-model") {
    const model = parsed.args.join(" ").trim();
    if (model === "") {
      return { error: "Usage: steward-model <model>" };
    }
    return {
      preparePath: "/api/steward/model/prepare",
      prepareBody: { model },
      executePath: "/api/steward/model/execute",
      buildExecuteBody: (prepared, signature) => ({
        model: String(prepared.model ?? model),
        proof: { ...prepared.proof_template, signature }
      })
    };
  }

  if (parsed.command === "steward-reasoning") {
    const variant = parsed.args[0]?.trim().toLowerCase() ?? "";
    if (!["default", "low", "medium", "high"].includes(variant)) {
      return { error: "Usage: steward-reasoning <default|low|medium|high>" };
    }
    return {
      preparePath: "/api/steward/reasoning/prepare",
      prepareBody: { variant },
      executePath: "/api/steward/reasoning/execute",
      buildExecuteBody: (prepared, signature) => ({
        variant: String(prepared.variant ?? variant),
        proof: { ...prepared.proof_template, signature }
      })
    };
  }

  return { error: `Unsupported steward command: ${parsed.command}` };
}

export async function executeStewardCommand(
  rawInput: string,
  context: StewardCommandExecutionContext
): Promise<StewardCommandExecutionResult> {
  const parsed = parseCommand(rawInput);
  if (parsed === null) {
    return errorEntries(rawInput, "Invalid steward command.");
  }
  const mapping = commandMapping(parsed, context.connectedAddress);
  if ("error" in mapping) {
    return errorEntries(rawInput, mapping.error);
  }

  let prepared: PreparedCommand;
  try {
    prepared = await requestJson<PreparedCommand>(context.canisterUrl, mapping.preparePath, mapping.prepareBody);
  } catch (error) {
    return errorEntries(rawInput, error instanceof Error ? error.message : "Steward command preparation failed.");
  }

  const validationError = validatePrepared(prepared, context);
  if (validationError !== null) {
    return errorEntries(rawInput, validationError);
  }

  let signature: string;
  try {
    signature = await context.request<string>({
      method: "personal_sign",
      params: [encodeUtf8Hex(prepared.signing_payload), context.connectedAddress]
    });
  } catch (error) {
    return errorEntries(rawInput, error instanceof Error ? error.message : "Wallet signature rejected.");
  }

  const executeBody = mapping.buildExecuteBody(prepared, signature);
  let response: ExecuteResponse | null = null;
  let executeError: unknown = null;
  for (let attempt = 1; attempt <= 2; attempt += 1) {
    try {
      response = await requestJson<ExecuteResponse>(context.canisterUrl, mapping.executePath, executeBody);
      break;
    } catch (error) {
      executeError = error;
      if (attempt === 1 && error instanceof HttpRequestError && error.transient) {
        await (context.sleep ?? DEFAULT_SLEEP)(900);
        continue;
      }
      break;
    }
  }

  if (response === null && executeError instanceof HttpRequestError && executeError.transient) {
    try {
      const status = await context.refreshStewardStatus();
      if ((status.next_nonce ?? Number.NaN) >= prepared.proof_template.nonce + 1) {
        response = { result: "Steward command likely applied; response was lost and nonce advanced." };
      }
    } catch {
      // Preserve the original execute error when reconciliation is unavailable.
    }
  }

  if (response === null) {
    return errorEntries(
      rawInput,
      executeError instanceof Error ? executeError.message : "Steward command execution failed."
    );
  }

  return {
    entries: [
      entry(1, "command", `> ${rawInput.trim()}`),
      entry(2, "response", response.result ?? "Steward command applied.")
    ]
  };
}
