import { Actor, HttpAgent, type ActorSubclass } from "@icp-sdk/core/agent";

import {
  AutomatonHttpRequestError,
  createAutomatonMetadataIdl,
  requestAutomatonJson,
  type AutomatonMetadataActor,
  type HttpBuildInfoResponse,
  type HttpEvmConfigResponse,
  type HttpSchedulerConfigResponse,
  type HttpSnapshotResponse,
  type HttpStewardStatusResponse,
  type HttpTurnRecordResponse,
  type HttpWalletBalanceResponse,
  type PromptLayerViewResponse,
  type SkillRecordResponse,
  type StrategyTemplateResponse
} from "@ic-automaton/canister-clients";

export type {
  HttpBuildInfoResponse,
  HttpEvmConfigResponse,
  HttpSchedulerConfigResponse,
  HttpSnapshotResponse,
  HttpStewardStatusResponse,
  HttpTurnRecordResponse,
  HttpWalletBalanceResponse,
  PromptLayerViewResponse,
  SkillRecordResponse,
  StrategyTemplateResponse
} from "@ic-automaton/canister-clients";

import type { IndexerTargetConfig } from "../indexer.config.js";
import { buildCanisterApiUrl } from "../lib/automaton-derived.js";

export interface IdentityConfigRead {
  genesis?: { name: string | null; constitution: string | null; contract_version: number | null };
  buildInfo: HttpBuildInfoResponse;
  canisterId: string;
  evmConfig: HttpEvmConfigResponse;
  promptLayers: PromptLayerViewResponse[];
  schedulerConfig: HttpSchedulerConfigResponse;
  skills: SkillRecordResponse[];
  stewardStatus: HttpStewardStatusResponse;
  strategies: StrategyTemplateResponse[];
}

type GenesisRead = NonNullable<IdentityConfigRead["genesis"]>;

export interface RuntimeFinancialRead {
  canisterId: string;
  snapshot: HttpSnapshotResponse;
  walletBalance: HttpWalletBalanceResponse;
}

export interface RecentTurnsRead {
  canisterId: string;
  recentTurns: HttpTurnRecordResponse[];
}

export interface AutomatonClient {
  readIdentityConfig(canisterId: string): Promise<IdentityConfigRead>;
  readRuntimeFinancial(canisterId: string): Promise<RuntimeFinancialRead>;
  readRecentTurns(canisterId: string): Promise<RecentTurnsRead>;
}

export class LiveAutomatonClient implements AutomatonClient {
  private agentPromise?: Promise<HttpAgent>;
  private readonly actorCache = new Map<string, ActorSubclass<AutomatonMetadataActor>>();

  constructor(private readonly config: IndexerTargetConfig) {}

  async readIdentityConfig(canisterId: string): Promise<IdentityConfigRead> {
    const actor = await this.getActor(canisterId);
    const [
      buildInfo,
      evmConfig,
      stewardStatus,
      schedulerConfig,
      promptLayers,
      skills,
      strategies,
      genesis
    ] = await Promise.all([
      this.requestJson<HttpBuildInfoResponse>(canisterId, "/api/build-info"),
      this.requestJson<HttpEvmConfigResponse>(canisterId, "/api/evm/config"),
      this.requestJson<HttpStewardStatusResponse>(canisterId, "/api/steward/status"),
      this.requestJson<HttpSchedulerConfigResponse>(canisterId, "/api/scheduler/config"),
      actor.get_prompt_layers(),
      actor.list_skills(),
      actor.list_strategy_templates([], 100),
      this.requestOptionalGenesis(canisterId)
    ]);

    return {
      canisterId,
      buildInfo,
      evmConfig,
      stewardStatus,
      schedulerConfig,
      promptLayers,
      skills,
      strategies,
      genesis
    };
  }

  async readRuntimeFinancial(canisterId: string): Promise<RuntimeFinancialRead> {
    const [snapshot, walletBalance] = await Promise.all([
      this.requestJson<HttpSnapshotResponse>(canisterId, "/api/snapshot"),
      this.requestJson<HttpWalletBalanceResponse>(canisterId, "/api/wallet/balance")
    ]);

    return { canisterId, snapshot, walletBalance };
  }

  async readRecentTurns(canisterId: string): Promise<RecentTurnsRead> {
    const snapshot = await this.requestJson<HttpSnapshotResponse>(canisterId, "/api/snapshot");
    return { canisterId, recentTurns: snapshot.recent_turns ?? [] };
  }

  private async getAgent() {
    this.agentPromise ??= (async () => {
      const host =
        this.config.network.target === "mainnet"
          ? "https://icp-api.io"
          : `http://${this.config.network.local.host}:${this.config.network.local.port}`;
      const agent = await HttpAgent.create({ host });

      if (this.config.network.target === "local") {
        await agent.fetchRootKey();
      }

      return agent;
    })();

    return this.agentPromise;
  }

  private async getActor(canisterId: string) {
    const cached = this.actorCache.get(canisterId);
    if (cached) return cached;

    const actor = Actor.createActor<AutomatonMetadataActor>(
      createAutomatonMetadataIdl() as unknown as Parameters<typeof Actor.createActor>[0],
      { agent: await this.getAgent(), canisterId }
    );
    this.actorCache.set(canisterId, actor);
    return actor;
  }

  private async requestJson<T>(canisterId: string, path: string): Promise<T> {
    return requestAutomatonJson<T>(buildCanisterApiUrl(this.config, canisterId, path), path);
  }

  private async requestOptionalGenesis(canisterId: string): Promise<GenesisRead | undefined> {
    try {
      return await this.requestJson<GenesisRead>(canisterId, "/api/genesis");
    } catch (error) {
      if (error instanceof AutomatonHttpRequestError && error.status === 404) {
        return undefined;
      }

      throw error;
    }
  }
}
