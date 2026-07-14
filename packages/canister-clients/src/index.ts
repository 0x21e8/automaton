import { IDL } from "@icp-sdk/core/candid";

import type { ActorMethod } from "@icp-sdk/core/agent";

export const AUTOMATON_METADATA_METHODS = [
  "get_prompt_layers",
  "list_skills",
  "list_strategy_templates"
] as const;

export interface PromptLayerViewResponse {
  content: string;
  is_mutable: boolean;
  layer_id: number;
  updated_at_ns: [] | [bigint];
  updated_by_turn: [] | [string];
  version: [] | [number];
}

export interface SkillRecordResponse {
  allowed_canister_calls: Array<{
    call_type: { Query?: null; Update?: null };
    canister_id: string;
    method: string;
  }>;
  description: string;
  enabled: boolean;
  instructions: string;
  mutable: boolean;
  name: string;
}

export interface StrategyTemplateResponse {
  actions: Array<{
    action_id: string;
    call_sequence: Array<{
      inputs: unknown[];
      name: string;
      outputs: unknown[];
      role: string;
      selector_hex: string;
      state_mutability: string;
    }>;
    postconditions: string[];
    preconditions: string[];
    risk_checks: string[];
  }>;
  constraints_json: string;
  contract_roles: Array<{
    address: string;
    codehash: [] | [string];
    role: string;
    source_ref: string;
  }>;
  created_at_ns: bigint;
  key: {
    chain_id: bigint;
    primitive: string;
    protocol: string;
    template_id: string;
  };
  status: {
    Active?: null;
    Deprecated?: null;
    Draft?: null;
    Revoked?: null;
  };
  updated_at_ns: bigint;
}

export interface AutomatonMetadataActor {
  get_prompt_layers: ActorMethod<[], PromptLayerViewResponse[]>;
  list_skills: ActorMethod<[], SkillRecordResponse[]>;
  list_strategy_templates: ActorMethod<[[] | [unknown], number], StrategyTemplateResponse[]>;
}

export interface HttpBuildInfoResponse {
  commit?: string;
}

export interface HttpEvmConfigResponse {
  automaton_address?: string | null;
  chain_id?: number;
  inbox_contract_address?: string | null;
  usdc_address?: string | null;
}

export interface HttpSchedulerConfigResponse {
  base_tick_secs?: number;
  default_turn_interval_secs?: number;
  ticks_per_turn_interval?: number;
}

export interface HttpStewardStatusResponse {
  active_steward?: {
    address?: string;
    chain_id?: number;
    enabled?: boolean;
  } | null;
  next_nonce?: number;
}

export interface HttpWalletBalanceResponse {
  age_secs?: number | null;
  bootstrap_pending?: boolean;
  eth_balance_wei_hex?: string | null;
  freshness_window_secs?: number;
  is_stale?: boolean;
  last_error?: string | null;
  last_synced_at_ns?: number | null;
  status?: string | Record<string, null>;
  usdc_balance_raw_hex?: string | null;
  usdc_contract_address?: string | null;
  usdc_decimals?: number;
}

export interface HttpTurnRecordResponse {
  created_at_ns?: number;
  duration_ms?: number | null;
  error?: string | null;
  id?: string;
  inner_dialogue?: string | null;
  input_summary?: string;
  state_from?: string | Record<string, null>;
  state_to?: string | Record<string, null>;
  tool_call_count?: number;
}

export type HttpDecisionOutcomeResponse =
  | { Executed: { action_summary?: string } }
  | { Simulated: { action_summary?: string } }
  | { NoOp: { reason?: string } }
  | { Deferred: { reason?: string } }
  | { Escalated: { gap?: unknown } };

export interface HttpDecisionRecordResponse {
  turn_id?: string;
  timestamp_ns?: number;
  trigger?: string | Record<string, null>;
  outcome?: HttpDecisionOutcomeResponse;
  policy_version?: number;
  candidates_summary?: string;
  explanation?: string;
}

export interface HttpJournalEntryResponse {
  id: number;
  turn_id: string;
  timestamp_ns: number;
  text: string;
  genesis?: boolean;
  deal_claim?: {
    kind: string;
    version: number;
    tx_hash: string;
    peer_canister_id: string;
    asset: string;
    amount_raw: string;
  } | null;
}

export interface HttpJournalResponse {
  entries?: HttpJournalEntryResponse[];
}

export interface HttpSnapshotResponse {
  cycles?: {
    burn_rate_cycles_per_day?: number | null;
    estimated_freeze_time_ns?: number | null;
    liquid_cycles?: number;
    total_cycles?: number;
    usd_per_trillion_cycles?: number;
  };
  recent_decisions?: HttpDecisionRecordResponse[];
  prompt_layers?: Array<{
    content?: string;
  }>;
  recent_turns?: HttpTurnRecordResponse[];
  runtime?: {
    last_error?: string | null;
    last_transition_at_ns?: number;
    loop_enabled?: boolean;
    soul?: string;
    state?: string | Record<string, null>;
    mortality?: {
      tier?: string | Record<string, null>;
      phase?: string | Record<string, null>;
      runway_seconds?: number | null;
      died_at_ns?: number | null;
      death_cause?: string | null;
      estate_disposition?: string | null;
      terminal_bequest_count?: number;
    };
  };
  scheduler?: {
    enabled?: boolean;
    last_tick_error?: string | null;
    survival_tier?: string | Record<string, null>;
  };
}

export function createAutomatonMetadataIdl() {
  return ({ IDL: candid }: { IDL: typeof IDL }) => {
    const CanisterCallType = candid.Variant({
      Query: candid.Null,
      Update: candid.Null
    });
    const CanisterCallPermission = candid.Record({
      canister_id: candid.Text,
      method: candid.Text,
      call_type: CanisterCallType
    });
    const SkillRecord = candid.Record({
      name: candid.Text,
      description: candid.Text,
      instructions: candid.Text,
      enabled: candid.Bool,
      mutable: candid.Bool,
      allowed_canister_calls: candid.Vec(CanisterCallPermission)
    });
    const StrategyTemplateKey = candid.Record({
      protocol: candid.Text,
      primitive: candid.Text,
      chain_id: candid.Nat64,
      template_id: candid.Text
    });
    const TemplateStatus = candid.Variant({
      Draft: candid.Null,
      Active: candid.Null,
      Deprecated: candid.Null,
      Revoked: candid.Null
    });
    const ContractRoleBinding = candid.Record({
      role: candid.Text,
      address: candid.Text,
      source_ref: candid.Text,
      codehash: candid.Opt(candid.Text)
    });
    const AbiTypeSpec = candid.Rec();
    const AbiFunctionSpec = candid.Record({
      role: candid.Text,
      name: candid.Text,
      selector_hex: candid.Text,
      inputs: candid.Vec(AbiTypeSpec),
      outputs: candid.Vec(AbiTypeSpec),
      state_mutability: candid.Text
    });
    AbiTypeSpec.fill(
      candid.Record({
        name: candid.Text,
        kind: candid.Text,
        components: candid.Vec(AbiTypeSpec)
      })
    );
    const ActionSpec = candid.Record({
      action_id: candid.Text,
      call_sequence: candid.Vec(AbiFunctionSpec),
      preconditions: candid.Vec(candid.Text),
      postconditions: candid.Vec(candid.Text),
      risk_checks: candid.Vec(candid.Text)
    });
    const StrategyTemplate = candid.Record({
      key: StrategyTemplateKey,
      status: TemplateStatus,
      contract_roles: candid.Vec(ContractRoleBinding),
      actions: candid.Vec(ActionSpec),
      constraints_json: candid.Text,
      created_at_ns: candid.Nat64,
      updated_at_ns: candid.Nat64
    });
    const PromptLayerView = candid.Record({
      layer_id: candid.Nat8,
      is_mutable: candid.Bool,
      content: candid.Text,
      updated_at_ns: candid.Opt(candid.Nat64),
      updated_by_turn: candid.Opt(candid.Text),
      version: candid.Opt(candid.Nat32)
    });

    return candid.Service({
      get_prompt_layers: candid.Func([], [candid.Vec(PromptLayerView)], ["query"]),
      list_skills: candid.Func([], [candid.Vec(SkillRecord)], ["query"]),
      list_strategy_templates: candid.Func(
        [candid.Opt(StrategyTemplateKey), candid.Nat32],
        [candid.Vec(StrategyTemplate)],
        ["query"]
      )
    });
  };
}

export class AutomatonHttpRequestError extends Error {
  constructor(
    message: string,
    public readonly status: number
  ) {
    super(message);
    this.name = "AutomatonHttpRequestError";
  }
}

export async function requestAutomatonJson<T>(
  canisterUrl: string,
  path: string,
  options?: { fetch?: typeof fetch; signal?: AbortSignal }
): Promise<T> {
  const baseUrl = new URL(canisterUrl);
  const requestUrl = new URL(path, baseUrl);
  for (const [name, value] of baseUrl.searchParams) {
    if (!requestUrl.searchParams.has(name)) {
      requestUrl.searchParams.append(name, value);
    }
  }
  const response = await (options?.fetch ?? fetch)(requestUrl, {
    headers: { accept: "application/json" },
    signal: options?.signal
  });

  if (!response.ok) {
    throw new AutomatonHttpRequestError(
      `Canister HTTP request failed for ${canisterUrl} ${path}: ${response.status} ${response.statusText}`,
      response.status
    );
  }

  return (await response.json()) as T;
}
