import {
  requestAutomatonJson,
  type HttpBuildInfoResponse,
  type HttpEvmConfigResponse,
  type HttpSchedulerConfigResponse,
  type HttpSnapshotResponse,
  type HttpStewardStatusResponse,
  type HttpWalletBalanceResponse
} from "@ic-automaton/canister-clients";

export type {
  HttpBuildInfoResponse as AutomatonBuildInfoResponse,
  HttpEvmConfigResponse as AutomatonEvmConfigResponse,
  HttpSchedulerConfigResponse as AutomatonSchedulerConfigResponse,
  HttpSnapshotResponse as AutomatonSnapshotResponse,
  HttpStewardStatusResponse as AutomatonStewardStatusResponse,
  HttpTurnRecordResponse as AutomatonTurnRecordResponse,
  HttpWalletBalanceResponse as AutomatonWalletBalanceResponse
} from "@ic-automaton/canister-clients";

export interface AutomatonContext {
  buildInfo: HttpBuildInfoResponse;
  evmConfig: HttpEvmConfigResponse;
  schedulerConfig: HttpSchedulerConfigResponse;
  stewardStatus: HttpStewardStatusResponse;
  snapshot: HttpSnapshotResponse;
  walletBalance: HttpWalletBalanceResponse;
  fetchedAt: number;
}

async function requestLiveAutomatonJson<T>(
  canisterUrl: string,
  path: string,
  signal?: AbortSignal
): Promise<T> {
  return requestAutomatonJson<T>(canisterUrl, path, { signal });
}

export async function fetchStewardStatus(
  canisterUrl: string,
  signal?: AbortSignal
): Promise<HttpStewardStatusResponse> {
  return requestLiveAutomatonJson<HttpStewardStatusResponse>(
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
      requestLiveAutomatonJson<HttpBuildInfoResponse>(canisterUrl, "/api/build-info", signal),
      requestLiveAutomatonJson<HttpEvmConfigResponse>(canisterUrl, "/api/evm/config", signal),
      requestLiveAutomatonJson<HttpStewardStatusResponse>(
        canisterUrl,
        "/api/steward/status",
        signal
      ),
      requestLiveAutomatonJson<HttpSchedulerConfigResponse>(
        canisterUrl,
        "/api/scheduler/config",
        signal
      ),
      requestLiveAutomatonJson<HttpSnapshotResponse>(canisterUrl, "/api/snapshot", signal),
      requestLiveAutomatonJson<HttpWalletBalanceResponse>(
        canisterUrl,
        "/api/wallet/balance",
        signal
      )
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
