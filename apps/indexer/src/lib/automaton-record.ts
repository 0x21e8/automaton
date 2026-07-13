import type {
  AutomatonDetail,
  AutomatonRecord
} from "@ic-automaton/shared";

const AUTOMATON_RECORD_KEYS = [
  "canisterId",
  "ethAddress",
  "chain",
  "chainId",
  "name",
  "constitutionHash",
  "constitution",
  "constitutionVerification",
  "model",
  "soul",
  "tier",
  "agentState",
  "loopEnabled",
  "lastTransitionAt",
  "lastError",
  "ethBalanceWei",
  "usdcBalanceRaw",
  "cyclesBalance",
  "liquidCycles",
  "burnRatePerDay",
  "estimatedFreezeTime",
  "netWorthEth",
  "netWorthUsd",
  "heartbeatIntervalSeconds",
  "steward",
  "commitHash",
  "parentId",
  "childIds",
  "strategies",
  "skills",
  "promptLayers",
  "gridPosition",
  "corePatternIndex",
  "corePattern",
  "createdAt"
] as const satisfies readonly (keyof AutomatonRecord)[];

export function toAutomatonRecord(detail: AutomatonDetail): AutomatonRecord {
  return {
    canisterId: detail.canisterId,
    ethAddress: detail.ethAddress,
    chain: detail.chain,
    chainId: detail.chainId,
    name: detail.name,
    constitutionHash: detail.constitutionHash,
    constitution: detail.constitution,
    constitutionVerification: detail.constitutionVerification,
    model: detail.model,
    soul: detail.soul,
    tier: detail.tier,
    agentState: detail.runtime.agentState,
    loopEnabled: detail.runtime.loopEnabled,
    lastTransitionAt: detail.runtime.lastTransitionAt,
    lastError: detail.runtime.lastError,
    ethBalanceWei: detail.financials.ethBalanceWei,
    usdcBalanceRaw: detail.financials.usdcBalanceRaw,
    cyclesBalance: detail.financials.cyclesBalance,
    liquidCycles: detail.financials.liquidCycles,
    burnRatePerDay: detail.financials.burnRatePerDay,
    estimatedFreezeTime: detail.financials.estimatedFreezeTime,
    netWorthEth:
      detail.financials.netWorthEth === null ? null : Number(detail.financials.netWorthEth),
    netWorthUsd:
      detail.financials.netWorthUsd === null ? null : Number(detail.financials.netWorthUsd),
    heartbeatIntervalSeconds: detail.runtime.heartbeatIntervalSeconds,
    steward: detail.steward,
    commitHash: detail.version.commitHash,
    parentId: detail.parentId,
    childIds: detail.childIds,
    strategies: detail.strategies,
    skills: detail.skills,
    promptLayers: detail.promptLayers,
    gridPosition: detail.gridPosition,
    corePatternIndex: detail.corePatternIndex,
    corePattern: detail.corePattern,
    lastPolledAt: detail.lastPolledAt,
    createdAt: detail.createdAt
  };
}

export function diffAutomatonRecord(
  previousDetail: AutomatonDetail | null | undefined,
  nextDetail: AutomatonDetail
): Partial<AutomatonRecord> | null {
  const nextRecord = toAutomatonRecord(nextDetail);

  if (!previousDetail) {
    return nextRecord;
  }

  const previousRecord = toAutomatonRecord(previousDetail);
  const changes: Partial<AutomatonRecord> = {};

  for (const key of AUTOMATON_RECORD_KEYS) {
    if (JSON.stringify(previousRecord[key]) === JSON.stringify(nextRecord[key])) {
      continue;
    }

    Object.assign(changes, {
      [key]: nextRecord[key]
    });
  }

  return Object.keys(changes).length > 0 ? changes : null;
}
