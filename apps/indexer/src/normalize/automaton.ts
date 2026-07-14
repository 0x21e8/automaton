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
import {
  computeCanonicalRunwaySeconds,
  computeWindowedBurnRate,
  deriveMetabolicState,
  positiveUsdcInflowRaw
} from "../lib/metabolism.js";

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
    metabolism: {
      burnRateCyclesPerDay: null,
      runwaySeconds: null,
      lifetimeEarningsUsdcRaw: "0",
      ageSeconds: Math.max(0, Math.floor((now - (registryRecord?.createdAt ?? now)) / 1_000)),
      state: "healthy",
      history: []
    },
    controlStatus: {
      label: "unverified",
      controllers: [],
      spawnerPresent: false,
      verifiedAt: null
    },
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
    inboxContractAddress: null,
    usdcContractAddress: null,
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
  const capturedAt = options.now;
  const liquidCycles =
    toOptionalNumber(runtime?.snapshot.cycles?.liquid_cycles) ?? base.financials.liquidCycles;
  const observedSample = {
    capturedAt,
    liquidCycles,
    usdcBalanceRaw: walletUsdcBalance
  };
  const priorHistory = base.metabolism?.history ?? [];
  const lastHistorySample = priorHistory.at(-1);
  const shouldAppendRuntimeSample = runtime !== undefined &&
    (lastHistorySample === undefined || capturedAt - lastHistorySample.capturedAt >= 1_000);
  const historyInputs = [...priorHistory, ...(shouldAppendRuntimeSample ? [observedSample] : [])]
    .filter((sample) => capturedAt - sample.capturedAt <= 24 * 60 * 60 * 1_000)
    .slice(-96);
  const windowedBurn = computeWindowedBurnRate(historyInputs);
  const childBurn = toOptionalNumber(runtime?.snapshot.cycles?.burn_rate_cycles_per_day);
  const burnRateCyclesPerDay = runtime === undefined
    ? base.metabolism?.burnRateCyclesPerDay ?? base.financials.burnRatePerDay
    : windowedBurn === null || windowedBurn === 0
      ? childBurn ?? windowedBurn ?? base.financials.burnRatePerDay
      : windowedBurn;
  const runwaySeconds = runtime === undefined
    ? base.metabolism?.runwaySeconds ?? null
    : computeCanonicalRunwaySeconds({
        burnRateCyclesPerDay,
        liquidCycles,
        usdcBalanceRaw: walletUsdcBalance,
        usdcDecimals,
        usdPerTrillionCycles: toOptionalNumber(
          runtime.snapshot.cycles?.usd_per_trillion_cycles
        ) ?? undefined
      });
  const previousUsdc = priorHistory.at(-1)?.usdcBalanceRaw ?? walletUsdcBalance;
  const lifetimeEarningsUsdcRaw = runtime === undefined
    ? base.metabolism?.lifetimeEarningsUsdcRaw ?? "0"
    : (
        BigInt(base.metabolism?.lifetimeEarningsUsdcRaw ?? "0") +
        BigInt(positiveUsdcInflowRaw(previousUsdc, walletUsdcBalance))
      ).toString();
  const cyclesBalance =
    toOptionalNumber(runtime?.snapshot.cycles?.total_cycles) ?? base.financials.cyclesBalance;
  const tier = normalizeTier(runtime?.snapshot.scheduler?.survival_tier, base.tier);
  const nextHistory = !shouldAppendRuntimeSample ? priorHistory : [...priorHistory, {
    capturedAt,
    liquidCycles,
    usdcBalanceRaw: walletUsdcBalance,
    burnRateCyclesPerDay,
    runwaySeconds
  }];
  const history = nextHistory.filter((point, index, points) =>
    capturedAt - point.capturedAt <= 7 * 24 * 60 * 60 * 1_000 &&
    (index === points.length - 1 || point.capturedAt !== points[index + 1]?.capturedAt)
  ).slice(-96);
  const attestedControllers = options.registryRecord?.controllers;
  const attestedControlLabel = options.registryRecord?.controlStatus;
  const attestedAt = options.registryRecord?.controlVerifiedAt;
  const hasKnownControlLabel = attestedControlLabel === "upgradeable_by_factory" ||
    attestedControlLabel === "self_controlled" ||
    attestedControlLabel === "controller_mismatch";
  const hasCoherentControllers = attestedControllers !== undefined &&
    attestedControllers.length > 0 &&
    (attestedControlLabel === "self_controlled"
      ? attestedControllers.length === 1 && attestedControllers[0] === options.canisterId
      : attestedControlLabel === "upgradeable_by_factory"
        ? attestedControllers.length === 1 && attestedControllers[0] !== options.canisterId
        : true);
  const hasControlAttestation = attestedControllers !== undefined &&
    hasCoherentControllers &&
    hasKnownControlLabel &&
    attestedAt !== undefined &&
    Number.isFinite(attestedAt) &&
    attestedAt >= 0;
  const registryControllers = hasControlAttestation ? [...attestedControllers] : [];
  const controlLabel = hasControlAttestation ? attestedControlLabel : "unverified";

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
    tier,
    agentState: runtimeState,
    ethBalanceWei: walletEthBalance,
    usdcBalanceRaw: walletUsdcBalance,
    cyclesBalance,
    metabolism: {
      burnRateCyclesPerDay,
      runwaySeconds,
      lifetimeEarningsUsdcRaw,
      ageSeconds: Math.max(0, Math.floor((capturedAt - (options.registryRecord?.createdAt ?? base.createdAt)) / 1_000)),
      state: deriveMetabolicState({ runwaySeconds, tier, cyclesBalance }),
      history
    },
    controlStatus: {
      label: controlLabel,
      controllers: [...registryControllers],
      // No factory-principal field is currently part of the attestation, so do
      // not infer this by comparing ICP principals with an EVM steward address.
      spawnerPresent: false,
      verifiedAt: hasControlAttestation ? attestedAt : null
    },
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
      cyclesBalance,
      liquidCycles,
      burnRatePerDay: burnRateCyclesPerDay,
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
    inboxContractAddress:
      toOptionalString(identity?.evmConfig.inbox_contract_address) ?? base.inboxContractAddress,
    usdcContractAddress:
      toOptionalString(identity?.evmConfig.usdc_address) ?? base.usdcContractAddress,
    lastPolledAt: options.now
  };
}
