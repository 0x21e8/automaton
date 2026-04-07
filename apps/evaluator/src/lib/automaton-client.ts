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
  snapshot: HttpSnapshotResponse;
  walletBalance: HttpWalletBalanceResponse;
  recentTurns: HttpTurnRecordResponse[];
}

export class AutomatonClient {
  constructor(
    private readonly host: string,
    private readonly port: number,
    private readonly fetchImpl: typeof fetch = fetch
  ) {}

  private async requestJson<T>(canisterId: string, path: string): Promise<T> {
    const response = await this.fetchImpl(buildCanisterApiUrl(this.host, this.port, canisterId, path), {
      headers: {
        accept: "application/json"
      }
    });

    if (!response.ok) {
      throw new Error(
        `Canister HTTP request failed for ${canisterId} ${path}: ${response.status} ${response.statusText}`
      );
    }

    return (await response.json()) as T;
  }

  async readEvidence(canisterId: string): Promise<AutomatonRuntimeEvidence> {
    const [buildInfo, evmConfig, snapshot, walletBalance] = await Promise.all([
      this.requestJson<HttpBuildInfoResponse>(canisterId, "/api/build-info"),
      this.requestJson<HttpEvmConfigResponse>(canisterId, "/api/evm/config"),
      this.requestJson<HttpSnapshotResponse>(canisterId, "/api/snapshot"),
      this.requestJson<HttpWalletBalanceResponse>(canisterId, "/api/wallet/balance")
    ]);

    return {
      buildInfo,
      evmConfig,
      snapshot,
      walletBalance,
      recentTurns: snapshot.recent_turns ?? []
    };
  }
}

export interface AutomatonClientLike {
  readEvidence(canisterId: string): Promise<AutomatonRuntimeEvidence>;
}
