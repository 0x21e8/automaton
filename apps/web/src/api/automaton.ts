export interface AutomatonBuildInfoResponse {
  commit?: string;
}

export interface AutomatonEvmConfigResponse {
  automaton_address?: string | null;
  chain_id?: number;
  inbox_contract_address?: string | null;
}

export interface AutomatonSchedulerConfigResponse {
  base_tick_secs?: number;
  default_turn_interval_secs?: number;
  ticks_per_turn_interval?: number;
}

export interface AutomatonStewardStatusResponse {
  active_steward?: {
    address?: string;
    chain_id?: number;
    enabled?: boolean;
  } | null;
  next_nonce?: number;
}

export interface AutomatonWalletBalanceResponse {
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

export interface AutomatonTurnRecordResponse {
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

export interface AutomatonSnapshotResponse {
  cycles?: {
    burn_rate_cycles_per_day?: number | null;
    estimated_freeze_time_ns?: number | null;
    liquid_cycles?: number;
    total_cycles?: number;
  };
  prompt_layers?: Array<{
    content?: string;
  }>;
  recent_turns?: AutomatonTurnRecordResponse[];
  runtime?: {
    last_error?: string | null;
    last_transition_at_ns?: number;
    loop_enabled?: boolean;
    soul?: string;
    state?: string | Record<string, null>;
  };
  scheduler?: {
    enabled?: boolean;
    last_tick_error?: string | null;
    survival_tier?: string | Record<string, null>;
  };
}

export interface AutomatonContext {
  buildInfo: AutomatonBuildInfoResponse;
  evmConfig: AutomatonEvmConfigResponse;
  schedulerConfig: AutomatonSchedulerConfigResponse;
  stewardStatus: AutomatonStewardStatusResponse;
  snapshot: AutomatonSnapshotResponse;
  walletBalance: AutomatonWalletBalanceResponse;
  fetchedAt: number;
}

async function requestLiveAutomatonJson<T>(
  canisterUrl: string,
  path: string,
  signal?: AbortSignal
): Promise<T> {
  const response = await fetch(new URL(path, canisterUrl), {
    headers: {
      accept: "application/json"
    },
    signal
  });

  if (!response.ok) {
    throw new Error(
      `Canister HTTP request failed for ${canisterUrl} ${path}: ${response.status} ${response.statusText}`
    );
  }

  return (await response.json()) as T;
}

export async function fetchStewardStatus(
  canisterUrl: string,
  signal?: AbortSignal
): Promise<AutomatonStewardStatusResponse> {
  return requestLiveAutomatonJson<AutomatonStewardStatusResponse>(
    canisterUrl,
    "/api/steward/status",
    signal
  );
}

export async function fetchAutomatonContext(
  canisterUrl: string,
  signal?: AbortSignal
): Promise<AutomatonContext> {
  const [buildInfo, evmConfig, stewardStatus, schedulerConfig, snapshot, walletBalance] =
    await Promise.all([
      requestLiveAutomatonJson<AutomatonBuildInfoResponse>(canisterUrl, "/api/build-info", signal),
      requestLiveAutomatonJson<AutomatonEvmConfigResponse>(canisterUrl, "/api/evm/config", signal),
      requestLiveAutomatonJson<AutomatonStewardStatusResponse>(canisterUrl, "/api/steward/status", signal),
      requestLiveAutomatonJson<AutomatonSchedulerConfigResponse>(canisterUrl, "/api/scheduler/config", signal),
      requestLiveAutomatonJson<AutomatonSnapshotResponse>(canisterUrl, "/api/snapshot", signal),
      requestLiveAutomatonJson<AutomatonWalletBalanceResponse>(canisterUrl, "/api/wallet/balance", signal)
    ]);

  return {
    buildInfo,
    evmConfig,
    schedulerConfig,
    stewardStatus,
    snapshot,
    walletBalance,
    fetchedAt: Date.now()
  };
}
