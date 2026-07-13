import type {
  AutomatonDetail,
  MonologueEntry,
  SkillSelection,
  SpawnedAutomatonRecord,
  StrategySelection
} from "@ic-automaton/shared";

import type {
  HttpTurnRecordResponse,
  IdentityConfigRead,
  RuntimeFinancialRead
} from "../integrations/automaton-client.js";
import type { IndexerTargetConfig } from "../indexer.config.js";
import {
  buildCanisterUrl,
  deriveMonologueCategory,
  deriveMonologueHeadline,
  deriveMonologueImportance,
  buildExplorerUrl,
  computeCorePattern,
  computeGridPosition,
  computeNetWorth,
  deriveAutomatonName,
  nsToMs,
  toChainSlug,
  toOptionalInteger,
  toOptionalNumber,
  toOptionalString,
  toVariantName
} from "../lib/automaton-derived.js";
import { verifyConstitution } from "../lib/genesis-integrity.js";

const EMPTY_STEWARD_ADDRESS = "0x0000000000000000000000000000000000000000";

function chainIdFromRegistryChain(
  chain: SpawnedAutomatonRecord["chain"] | undefined,
  fallback: number
) {
  if (chain === "base") {
    return 8453;
  }

  return fallback;
}

export function normalizeMonologueEntries(turns: HttpTurnRecordResponse[]): MonologueEntry[] {
  return turns
    .map((turn) => {
      const timestamp = nsToMs(turn.created_at_ns);
      const turnId = toOptionalString(turn.id);

      if (timestamp === null || turnId === null) {
        return null;
      }

      const toolCallCount = toOptionalInteger(turn.tool_call_count) ?? 0;
      const message =
        toOptionalString(turn.inner_dialogue) ??
        toOptionalString(turn.input_summary) ??
        "No monologue captured.";
      const type = toolCallCount > 0 ? "action" : "thought";
      const error = toOptionalString(turn.error);
      const category = deriveMonologueCategory({
        error,
        message,
        toolCallCount,
        type
      });
      const importance = deriveMonologueImportance({
        category,
        durationMs: toOptionalInteger(turn.duration_ms),
        error,
        message,
        toolCallCount
      });

      return {
        timestamp,
        turnId,
        type,
        headline: deriveMonologueHeadline(
          message,
          type === "thought" ? "Observation update" : "Action update"
        ),
        message,
        category,
        importance,
        agentState: `${toVariantName(turn.state_from, "Unknown")} -> ${toVariantName(turn.state_to, "Unknown")}`,
        toolCallCount,
        durationMs: toOptionalInteger(turn.duration_ms),
        error
      } satisfies MonologueEntry;
    })
    .filter((entry): entry is MonologueEntry => entry !== null)
    .sort((left, right) => {
      if (left.timestamp === right.timestamp) {
        return right.turnId.localeCompare(left.turnId);
      }

      return right.timestamp - left.timestamp;
    });
}

function normalizeStrategies(identity: IdentityConfigRead | undefined): StrategySelection[] {
  if (!identity) {
    return [];
  }

  return identity.strategies.map((strategy) => ({
    key: {
      protocol: strategy.key.protocol,
      primitive: strategy.key.primitive,
      templateId: strategy.key.template_id,
      chainId: Number(strategy.key.chain_id)
    },
    status: toVariantName(strategy.status, "draft").toLowerCase()
  }));
}

function normalizeSkills(identity: IdentityConfigRead | undefined): SkillSelection[] {
  if (!identity) {
    return [];
  }

  return identity.skills.map((skill) => ({
    name: skill.name,
    description: skill.description,
    enabled: skill.enabled
  }));
}

function normalizeTier(
  survivalTier: unknown,
  fallback: AutomatonDetail["tier"]
): AutomatonDetail["tier"] {
  if (survivalTier === undefined) {
    return fallback;
  }

  const tierVariant = toVariantName(survivalTier, fallback);

  if (tierVariant === "LowCycles" || tierVariant === "low") {
    return "low";
  }

  if (tierVariant === "Critical" || tierVariant === "critical") {
    return "critical";
  }

  if (tierVariant === "OutOfCycles" || tierVariant === "out_of_cycles") {
    return "out_of_cycles";
  }

  return "normal";
}

function defaultDetail(
  canisterId: string,
  config: IndexerTargetConfig,
  now: number,
  registryRecord?: SpawnedAutomatonRecord | null,
  spawnModel: string | null = null
): AutomatonDetail {
  const { corePatternIndex, corePattern } = computeCorePattern(canisterId);
  const chain = registryRecord?.chain ?? "base";
  const chainId = chainIdFromRegistryChain(registryRecord?.chain, 8453);

  return {
    canisterId,
    ethAddress: registryRecord?.evmAddress ?? null,
    chain,
    chainId,
    name: registryRecord?.name ?? deriveAutomatonName(canisterId),
    constitutionHash: registryRecord?.constitutionHash ?? null,
    constitutionVerification: {
      status: "unavailable",
      expectedHash: registryRecord?.constitutionHash ?? null,
      computedHash: null
    },
    tier: "normal",
    agentState: "Unknown",
    ethBalanceWei: null,
    usdcBalanceRaw: null,
    cyclesBalance: 0,
    netWorthEth: null,
    netWorthUsd: null,
    heartbeatIntervalSeconds: null,
    steward: {
      address: registryRecord?.stewardAddress ?? EMPTY_STEWARD_ADDRESS,
      chainId,
      ensName: null,
      enabled: registryRecord !== undefined && registryRecord !== null
    },
    gridPosition: computeGridPosition(canisterId),
    corePatternIndex,
    corePattern,
    parentId: registryRecord?.parentId ?? null,
    createdAt: registryRecord?.createdAt ?? now,
    lastTransitionAt: now,
    soul: "",
    constitution: null,
    canisterUrl: buildCanisterUrl(config, canisterId),
    explorerUrl: null,
    model: spawnModel,
    financials: {
      ethBalanceWei: null,
      usdcBalanceRaw: null,
      cyclesBalance: 0,
      liquidCycles: 0,
      burnRatePerDay: null,
      estimatedFreezeTime: null,
      netWorthEth: null,
      netWorthUsd: null
    },
    runtime: {
      agentState: "Unknown",
      loopEnabled: false,
      lastTransitionAt: now,
      lastError: null,
      heartbeatIntervalSeconds: null
    },
    version: {
      commitHash: "unknown",
      shortCommitHash: "unknown"
    },
    strategies: [],
    skills: [],
    promptLayers: [],
    monologue: [],
    journal: [],
    spawnSelection: null,
    childIds: registryRecord?.childIds ?? [],
    lastPolledAt: now
  };
}

export function normalizeAutomatonDetail(options: {
  canisterId: string;
  config: IndexerTargetConfig;
  existingDetail?: AutomatonDetail | null;
  identity?: IdentityConfigRead;
  monologue?: MonologueEntry[];
  now: number;
  registryRecord?: SpawnedAutomatonRecord | null;
  runtime?: RuntimeFinancialRead;
  spawnModel?: string | null;
  ethUsd: number | null;
}): AutomatonDetail {
  const base =
    options.existingDetail ??
    defaultDetail(
      options.canisterId,
      options.config,
      options.now,
      options.registryRecord,
      options.spawnModel ?? null
    );
  const identity = options.identity;
  const runtime = options.runtime;
  const identityChainId = toOptionalInteger(identity?.evmConfig.chain_id);
  const chainId =
    identityChainId ?? chainIdFromRegistryChain(options.registryRecord?.chain, base.chainId);
  const chain =
    identityChainId === null
      ? options.registryRecord?.chain ?? base.chain
      : toChainSlug(chainId);
  const automatonAddress =
    toOptionalString(identity?.evmConfig.automaton_address) ??
    options.registryRecord?.evmAddress ??
    base.ethAddress;
  const walletEthBalance =
    toOptionalString(runtime?.walletBalance.eth_balance_wei_hex) ?? base.financials.ethBalanceWei;
  const walletUsdcBalance =
    toOptionalString(runtime?.walletBalance.usdc_balance_raw_hex) ?? base.financials.usdcBalanceRaw;
  const usdcDecimals = toOptionalInteger(runtime?.walletBalance.usdc_decimals) ?? 6;
  const netWorth = computeNetWorth(walletEthBalance, walletUsdcBalance, usdcDecimals, options.ethUsd);
  const transitionAt =
    nsToMs(runtime?.snapshot.runtime?.last_transition_at_ns) ??
    base.runtime.lastTransitionAt;
  const heartbeatIntervalSeconds =
    toOptionalInteger(identity?.schedulerConfig.default_turn_interval_secs) ??
    base.runtime.heartbeatIntervalSeconds;
  const runtimeState = toVariantName(runtime?.snapshot.runtime?.state, base.runtime.agentState);
  const commitHash = toOptionalString(identity?.buildInfo.commit) ?? base.version.commitHash;
  const runtimeLastError =
    runtime !== undefined
      ? toOptionalString(runtime.snapshot.runtime?.last_error) ??
        toOptionalString(runtime.snapshot.scheduler?.last_tick_error) ??
        null
      : base.runtime.lastError;
  const constitutionHash =
    options.registryRecord?.constitutionHash ?? base.constitutionHash ?? null;
  const identityConstitution = options.identity?.genesis?.constitution ?? null;
  const constitutionResult =
    identityConstitution === null && options.identity === undefined
      ? base.constitutionVerification === undefined
        ? verifyConstitution(base.constitution ?? null, constitutionHash)
        : {
            constitution: base.constitution ?? null,
            verification: base.constitutionVerification
          }
      : verifyConstitution(identityConstitution, constitutionHash);

  return {
    ...base,
    canisterId: options.canisterId,
    ethAddress: automatonAddress,
    chainId,
    chain,
    name: options.identity?.genesis?.name ?? options.registryRecord?.name ?? base.name,
    constitutionHash,
    constitution: constitutionResult.constitution,
    constitutionVerification: constitutionResult.verification,
    model: options.spawnModel ?? base.model,
    tier: normalizeTier(runtime?.snapshot.scheduler?.survival_tier, base.tier),
    agentState: runtimeState,
    ethBalanceWei: walletEthBalance,
    usdcBalanceRaw: walletUsdcBalance,
    cyclesBalance:
      toOptionalNumber(runtime?.snapshot.cycles?.total_cycles) ?? base.financials.cyclesBalance,
    netWorthEth: netWorth.netWorthEth,
    netWorthUsd: netWorth.netWorthUsd,
    heartbeatIntervalSeconds,
    steward:
      identity?.stewardStatus.active_steward
        ? {
            address:
              toOptionalString(identity.stewardStatus.active_steward.address) ??
              base.steward.address,
            chainId:
              toOptionalInteger(identity.stewardStatus.active_steward.chain_id) ??
              base.steward.chainId,
            ensName: base.steward.ensName,
            enabled: Boolean(identity.stewardStatus.active_steward.enabled)
          }
        : options.registryRecord
          ? {
              address: options.registryRecord.stewardAddress,
              chainId: chainIdFromRegistryChain(options.registryRecord.chain, chainId),
              ensName: base.steward.ensName,
              enabled: true
            }
          : base.steward,
    gridPosition: computeGridPosition(options.canisterId),
    ...computeCorePattern(options.canisterId),
    parentId: options.registryRecord?.parentId ?? base.parentId,
    childIds: options.registryRecord?.childIds ?? base.childIds,
    createdAt: options.registryRecord?.createdAt ?? base.createdAt,
    lastTransitionAt: transitionAt,
    soul:
      toOptionalString(runtime?.snapshot.runtime?.soul) ??
      base.soul,
    canisterUrl: buildCanisterUrl(options.config, options.canisterId),
    explorerUrl: buildExplorerUrl(chainId, automatonAddress),
    financials: {
      ethBalanceWei: walletEthBalance,
      usdcBalanceRaw: walletUsdcBalance,
      cyclesBalance:
        toOptionalNumber(runtime?.snapshot.cycles?.total_cycles) ?? base.financials.cyclesBalance,
      liquidCycles:
        toOptionalNumber(runtime?.snapshot.cycles?.liquid_cycles) ?? base.financials.liquidCycles,
      burnRatePerDay:
        toOptionalNumber(runtime?.snapshot.cycles?.burn_rate_cycles_per_day) ??
        base.financials.burnRatePerDay,
      estimatedFreezeTime:
        nsToMs(runtime?.snapshot.cycles?.estimated_freeze_time_ns) ??
        base.financials.estimatedFreezeTime,
      netWorthEth: netWorth.netWorthEth,
      netWorthUsd: netWorth.netWorthUsd
    },
    runtime: {
      agentState: runtimeState,
      loopEnabled: runtime?.snapshot.runtime?.loop_enabled ?? base.runtime.loopEnabled,
      lastTransitionAt: transitionAt,
      lastError: runtimeLastError,
      heartbeatIntervalSeconds
    },
    version: {
      commitHash,
      shortCommitHash: commitHash.slice(0, 7)
    },
    strategies: identity ? normalizeStrategies(identity) : base.strategies,
    skills: identity ? normalizeSkills(identity) : base.skills,
    promptLayers:
      identity?.promptLayers.map((layer) => layer.content) ??
      base.promptLayers,
    monologue: options.monologue ?? base.monologue,
    journal: base.journal ?? [],
    lastPolledAt: options.now
  };
}
