export interface HttpBuildInfoResponse {
  commit?: string;
}

export interface HttpEvmConfigResponse {
  automaton_address?: string | null;
  chain_id?: number;
  inbox_contract_address?: string | null;
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

export interface HttpInferenceConfigResponse {
  model?: string | null;
  provider?: string | null;
  reasoning_level?: string | null;
}

export interface HttpInferenceProxyStatusResponse {
  configured?: boolean;
  provider?: string | null;
  trusted_callback_principal?: string | null;
  worker_base_url?: string | null;
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

export interface HttpSnapshotResponse {
  cycles?: {
    burn_rate_cycles_per_day?: number | null;
    estimated_freeze_time_ns?: number | null;
    liquid_cycles?: number;
    total_cycles?: number;
  };
  prompt_layers?: Array<{
    content?: string;
  }>;
  recent_turns?: HttpTurnRecordResponse[];
  outbox_messages?: Array<{
    id?: string;
    turn_id?: string;
    body?: string;
    source_inbox_ids?: string[];
  }>;
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
      death_cause?: string | null;
      estate_disposition?: string | null;
      terminal_turn_id?: string | null;
      died_at_ns?: number | null;
    };
  };
  recent_decisions?: HttpDecisionRecordResponse[];
  scheduler?: {
    enabled?: boolean;
    last_tick_error?: string | null;
    survival_tier?: string | Record<string, null>;
  };
}

export interface HttpJournalResponse {
  entries?: Array<{
    id?: number;
    turn_id?: string;
    timestamp_ns?: number;
    text?: string;
    genesis?: boolean;
  }>;
}

function normalizeHost(host: string) {
  return host.trim().replace(/\/+$/, "");
}

function isIpHost(host: string) {
  const normalized = host.replace(/^\[|\]$/g, "");
  return /^(\d{1,3}\.){3}\d{1,3}$/.test(normalized) || normalized.includes(":");
}

function buildCanisterApiUrl(
  host: string,
  port: number,
  canisterId: string,
  path: string
) {
  const normalizedHost = normalizeHost(host);
  const origin = isIpHost(normalizedHost)
    ? `http://${normalizedHost}:${port}`
    : `http://${canisterId}.${normalizedHost}:${port}`;

  if (!isIpHost(normalizedHost)) {
    return `${origin}${path}`;
  }

  const separator = path.includes("?") ? "&" : "?";
  return `${origin}${path}${separator}canisterId=${encodeURIComponent(canisterId)}`;
}

export interface AutomatonRuntimeEvidence {
  buildInfo: HttpBuildInfoResponse;
  evmConfig: HttpEvmConfigResponse;
  inferenceConfig: HttpInferenceConfigResponse | null;
  inferenceProxyStatus: HttpInferenceProxyStatusResponse | null;
  snapshot: HttpSnapshotResponse;
  walletBalance: HttpWalletBalanceResponse;
  recentTurns: HttpTurnRecordResponse[];
  journal?: HttpJournalResponse;
}

export class AutomatonClient {
  constructor(
    private readonly host: string,
    private readonly port: number,
    private readonly fetchImpl: typeof fetch = fetch
  ) {}

  private async requestJsonOptional<T>(
    canisterId: string,
    path: string,
    allowedStatuses: readonly number[]
  ): Promise<T | null> {
    const response = await this.fetchImpl(buildCanisterApiUrl(this.host, this.port, canisterId, path), {
      headers: {
        accept: "application/json"
      }
    });

    if (!response.ok) {
      if (allowedStatuses.includes(response.status)) {
        return null;
      }

      throw new Error(
        `Canister HTTP request failed for ${canisterId} ${path}: ${response.status} ${response.statusText}`
      );
    }

    return (await response.json()) as T;
  }

  private async requestJson<T>(canisterId: string, path: string): Promise<T> {
    const result = await this.requestJsonOptional<T>(canisterId, path, []);
    if (result === null) {
      throw new Error(`Canister HTTP request failed for ${canisterId} ${path}: empty response`);
    }

    return result;
  }

  async readEvidence(canisterId: string): Promise<AutomatonRuntimeEvidence> {
    const [buildInfo, evmConfig, inferenceConfig, inferenceProxyStatus, snapshot, walletBalance, journal] =
      await Promise.all([
      this.requestJson<HttpBuildInfoResponse>(canisterId, "/api/build-info"),
      this.requestJson<HttpEvmConfigResponse>(canisterId, "/api/evm/config"),
      this.requestJsonOptional<HttpInferenceConfigResponse>(
        canisterId,
        "/api/inference/config",
        [404]
      ),
      this.requestJsonOptional<HttpInferenceProxyStatusResponse>(
        canisterId,
        "/api/inference/proxy/status",
        [400, 404]
      ),
      this.requestJson<HttpSnapshotResponse>(canisterId, "/api/snapshot"),
      this.requestJson<HttpWalletBalanceResponse>(canisterId, "/api/wallet/balance"),
      this.requestJsonOptional<HttpJournalResponse>(canisterId, "/api/journal", [404])
    ]);

    return {
      buildInfo,
      evmConfig,
      inferenceConfig,
      inferenceProxyStatus,
      snapshot,
      walletBalance,
      recentTurns: snapshot.recent_turns ?? [],
      journal: journal ?? { entries: [] }
    };
  }
}

export interface AutomatonClientLike {
  readEvidence(canisterId: string): Promise<AutomatonRuntimeEvidence>;
}
