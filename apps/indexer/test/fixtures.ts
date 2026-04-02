import {
  deriveClaimId,
  type RoomMessage,
  type RoomMessagePage,
  type SpawnPaymentInstructions,
  type SpawnSessionDetail,
  type SpawnedAutomatonRecord
} from "@ic-automaton/shared";
import type {
  AutomatonDetail,
  AutomatonRecord,
  MonologueEntry,
} from "@ic-automaton/shared";

export function createAutomatonDetailFixture(
  overrides: Partial<AutomatonDetail> = {}
): AutomatonDetail {
  const base: AutomatonDetail = {
    canisterId: "aaaaa-aa",
    ethAddress: "0x0000000000000000000000000000000000000001",
    chain: "base",
    chainId: 8453,
    name: "ALPHA-42",
    model: "openrouter/auto",
    tier: "normal",
    agentState: "Idle",
    ethBalanceWei: "0x1",
    usdcBalanceRaw: "0x0",
    cyclesBalance: 42,
    netWorthEth: "1.23",
    netWorthUsd: "2500.00",
    heartbeatIntervalSeconds: 30,
    steward: {
      address: "0x0000000000000000000000000000000000000002",
      chainId: 8453,
      ensName: "dom.eth",
      enabled: true
    },
    gridPosition: {
      x: 4,
      y: 8
    },
    corePatternIndex: 3,
    corePattern: [[1, 1], [1, 0]],
    parentId: null,
    createdAt: 1_709_912_345_000,
    lastTransitionAt: 1_709_912_346_000,
    soul: "alpha soul",
    canisterUrl: "https://aaaaa-aa.icp0.io",
    explorerUrl: "https://basescan.org/address/0x0000000000000000000000000000000000000001",
    financials: {
      ethBalanceWei: "0x1",
      usdcBalanceRaw: "0x0",
      cyclesBalance: 42,
      liquidCycles: 21,
      burnRatePerDay: 1.5,
      estimatedFreezeTime: 123_456,
      netWorthEth: "1.23",
      netWorthUsd: "2500.00"
    },
    runtime: {
      agentState: "Idle",
      loopEnabled: true,
      lastTransitionAt: 1_709_912_346_000,
      lastError: null,
      heartbeatIntervalSeconds: 30
    },
    version: {
      commitHash: "abcdef1234567890",
      shortCommitHash: "abcdef1"
    },
    strategies: [],
    skills: [],
    promptLayers: ["constitution"],
    monologue: [],
    childIds: [],
    lastPolledAt: 1_709_912_347_000
  };

  return {
    ...base,
    ...overrides,
    financials: overrides.financials ?? base.financials,
    runtime: overrides.runtime ?? base.runtime,
    version: overrides.version ?? base.version,
    steward: overrides.steward ?? base.steward,
    gridPosition: overrides.gridPosition ?? base.gridPosition
  };
}

export function createAutomatonRecordFixture(
  overrides: Partial<AutomatonRecord> = {}
): AutomatonRecord {
  const detail = createAutomatonDetailFixture();
  const base: AutomatonRecord = {
    canisterId: detail.canisterId,
    ethAddress: detail.ethAddress,
    chain: detail.chain,
    chainId: detail.chainId,
    name: detail.name,
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
    netWorthEth: Number(detail.financials.netWorthEth),
    netWorthUsd: Number(detail.financials.netWorthUsd),
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

  return {
    ...base,
    ...overrides,
    steward: overrides.steward ?? base.steward,
    gridPosition: overrides.gridPosition ?? base.gridPosition,
    strategies: overrides.strategies ?? base.strategies,
    skills: overrides.skills ?? base.skills,
    promptLayers: overrides.promptLayers ?? base.promptLayers,
    childIds: overrides.childIds ?? base.childIds
  };
}

export function createMonologueEntryFixture(
  overrides: Partial<MonologueEntry> = {}
): MonologueEntry {
  return {
    timestamp: 1_709_912_348_000,
    turnId: "turn-1",
    type: "thought",
    headline: "Check balances",
    message: "Checking balances.",
    category: "observe",
    importance: "low",
    agentState: "Idle -> Inferring",
    toolCallCount: 0,
    durationMs: 1200,
    error: null,
    ...overrides
  };
}

export function createSpawnedAutomatonRecordFixture(
  overrides: Partial<SpawnedAutomatonRecord> = {}
): SpawnedAutomatonRecord {
  return {
    canisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
    stewardAddress: "0x0000000000000000000000000000000000000002",
    evmAddress: "0x0000000000000000000000000000000000000003",
    chain: "base",
    sessionId: "session-1709912345000-1",
    parentId: null,
    childIds: [],
    createdAt: 1_709_912_360_000,
    versionCommit: "abcdef1234567890abcdef1234567890abcdef12",
    ...overrides
  };
}

export function createRoomMessageFixture(
  overrides: Partial<RoomMessage> = {}
): RoomMessage {
  return {
    messageId: "room-message-1",
    seq: 1,
    authorCanisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
    createdAt: 1_709_912_360_000,
    body: "status nominal",
    mentions: [],
    contentType: "text/plain",
    ...overrides
  };
}

export function createRoomMessagePageFixture(
  overrides: Partial<RoomMessagePage> = {}
): RoomMessagePage {
  const message =
    overrides.messages?.[0] ??
    createRoomMessageFixture({
      seq: overrides.latestSeq ?? 1
    });

  return {
    messages: overrides.messages ?? [message],
    nextAfterSeq: overrides.nextAfterSeq ?? null,
    latestSeq: overrides.latestSeq ?? message.seq
  };
}

export function createEscrowPaymentRecordFixture(
  overrides: Partial<SpawnPaymentInstructions> = {}
): SpawnPaymentInstructions {
  const sessionId = overrides.sessionId ?? "session-1709912345000-1";

  return {
    sessionId,
    claimId: deriveClaimId(sessionId),
    chain: "base",
    asset: "usdc",
    paymentAddress: "0x00000000000000000000000000000000000000ff",
    grossAmount: "1000000000",
    quoteTermsHash: "0xdeadbeef",
    expiresAt: 1_709_912_500_000,
    ...overrides
  };
}

export function createSpawnSessionDetailFixture(
  overrides: Partial<SpawnSessionDetail> = {}
): SpawnSessionDetail {
  const registryRecord =
    overrides.registryRecord === undefined
      ? createSpawnedAutomatonRecordFixture()
      : overrides.registryRecord;
  const defaultSessionId =
    overrides.session?.sessionId ??
    registryRecord?.sessionId ??
    "session-1709912345000-1";
  const payment =
    overrides.payment ?? createEscrowPaymentRecordFixture({ sessionId: defaultSessionId });
  const session: SpawnSessionDetail["session"] = {
    sessionId: defaultSessionId,
    claimId: payment.claimId,
    stewardAddress: "0x0000000000000000000000000000000000000002",
    chain: "base",
    asset: "usdc",
    grossAmount: "1000000000",
    platformFee: "1000000",
    creationCost: "2000000",
    netForwardAmount: "997000000",
    quoteTermsHash: payment.quoteTermsHash,
    expiresAt: 1_709_912_500_000,
    state: registryRecord ? "complete" : "awaiting_payment",
    retryable: false,
    refundable: false,
    paymentStatus: registryRecord ? "paid" : "unpaid",
    automatonCanisterId: registryRecord?.canisterId ?? null,
    automatonEvmAddress: registryRecord?.evmAddress ?? null,
    releaseTxHash: registryRecord ? "0x1111111111111111111111111111111111111111111111111111111111111111" : null,
    releaseBroadcastAt: registryRecord ? 1_709_912_359_000 : null,
    parentId: null,
    childIds: [],
    config: {
      chain: "base",
      risk: 3,
      strategies: ["yield-farming"],
      skills: ["portfolio-reporting"],
      provider: {
        openRouterApiKey: null,
        model: "openrouter/auto",
        braveSearchApiKey: null
      }
    },
    createdAt: 1_709_912_345_000,
    updatedAt: 1_709_912_360_000
  };
  const audit: SpawnSessionDetail["audit"] = [
    {
      sessionId: session.sessionId,
      timestamp: 1_709_912_345_000,
      fromState: null,
      toState: "awaiting_payment",
      actor: "user",
      reason: "session created"
    }
  ];

  if (session.state === "complete") {
    audit.push({
      sessionId: session.sessionId,
      timestamp: 1_709_912_360_000,
      fromState: "broadcasting_release",
      toState: "complete",
      actor: "system",
      reason: "spawn completed"
    });
  }

  return {
    ...overrides,
    session: overrides.session ?? session,
    payment,
    audit: overrides.audit ?? audit,
    registryRecord
  };
}
