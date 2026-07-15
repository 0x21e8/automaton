import type { SpawnSessionStrategySnapshot } from "./spawn.js";
import type { RoomMessageSettlement } from "./room.js";

export const SUPPORTED_CHAIN_SLUGS = [
  "base",
  "ethereum",
  "arbitrum",
  "optimism",
  "polygon"
] as const;

export const AUTOMATON_TIERS = [
  "normal",
  "low",
  "critical",
  "out_of_cycles"
] as const;

export const MONOLOGUE_ENTRY_TYPES = ["thought", "action"] as const;
export const MONOLOGUE_ENTRY_CATEGORIES = [
  "observe",
  "decide",
  "act",
  "message",
  "error"
] as const;
export const MONOLOGUE_ENTRY_IMPORTANCE = ["low", "medium", "high"] as const;

export type ChainSlug = (typeof SUPPORTED_CHAIN_SLUGS)[number];
export type AutomatonTier = (typeof AUTOMATON_TIERS)[number];
export type MonologueEntryType = (typeof MONOLOGUE_ENTRY_TYPES)[number];
export type MonologueEntryCategory = (typeof MONOLOGUE_ENTRY_CATEGORIES)[number];
export type MonologueEntryImportance = (typeof MONOLOGUE_ENTRY_IMPORTANCE)[number];
export type ConstitutionVerificationStatus =
  | "verified"
  | "mismatch"
  | "legacy_unverified"
  | "unavailable";

export interface ConstitutionVerification {
  status: ConstitutionVerificationStatus;
  expectedHash: string | null;
  computedHash: string | null;
}

export interface GridPosition {
  x: number;
  y: number;
}

export interface StrategyKey {
  protocol: string;
  primitive: string;
  templateId: string;
  chainId: number;
}

export interface StrategySelection {
  key: StrategyKey;
  status: string;
}

export interface SkillSelection {
  name: string;
  description: string;
  enabled: boolean;
}

export interface AutomatonSpawnSelection {
  sessionId: string;
  requestedStrategyIds: string[];
  selectedStrategies: SpawnSessionStrategySnapshot[];
}

export interface StewardIdentity {
  address: string;
  chainId: number;
  ensName: string | null;
  enabled: boolean;
}

export interface AutomatonRecord {
  canisterId: string;
  ethAddress: string | null;
  chain: ChainSlug;
  chainId: number;
  name: string;
  constitutionHash?: string | null;
  constitution?: string | null;
  constitutionVerification?: ConstitutionVerification;
  model: string | null;
  soul: string;
  tier: AutomatonTier;
  agentState: string;
  loopEnabled: boolean;
  lastTransitionAt: number;
  lastError: string | null;
  ethBalanceWei: string | null;
  usdcBalanceRaw: string | null;
  cyclesBalance: number;
  liquidCycles: number;
  burnRatePerDay: number | null;
  metabolism?: AutomatonMetabolism;
  controlStatus?: AutomatonControlStatus;
  estimatedFreezeTime: number | null;
  netWorthEth: number | null;
  netWorthUsd: number | null;
  heartbeatIntervalSeconds: number | null;
  steward: StewardIdentity;
  commitHash: string;
  parentId: string | null;
  generation?: number;
  parentConstitutionHash?: string | null;
  childIds: string[];
  strategies: StrategySelection[];
  skills: SkillSelection[];
  promptLayers: string[];
  gridPosition: GridPosition;
  corePatternIndex: number;
  corePattern: number[][] | null;
  lastPolledAt: number;
  createdAt: number;
}

export interface AutomatonSummary {
  canisterId: string;
  ethAddress: string | null;
  chain: ChainSlug;
  chainId: number;
  name: string;
  constitutionHash?: string | null;
  tier: AutomatonTier;
  agentState: string;
  ethBalanceWei: string | null;
  usdcBalanceRaw: string | null;
  cyclesBalance: number;
  metabolism?: AutomatonMetabolism;
  controlStatus?: AutomatonControlStatus;
  netWorthEth: string | null;
  netWorthUsd: string | null;
  heartbeatIntervalSeconds: number | null;
  steward: StewardIdentity;
  gridPosition: GridPosition;
  corePatternIndex: number;
  corePattern: number[][] | null;
  parentId: string | null;
  generation?: number;
  parentConstitutionHash?: string | null;
  createdAt: number;
  lastTransitionAt: number;
}

export interface AutomatonFinancials {
  ethBalanceWei: string | null;
  usdcBalanceRaw: string | null;
  cyclesBalance: number;
  liquidCycles: number;
  burnRatePerDay: number | null;
  estimatedFreezeTime: number | null;
  netWorthEth: string | null;
  netWorthUsd: string | null;
}

export type MetabolicState = "healthy" | "hibernating" | "dying" | "dead";
export type MortalityTier = "active" | "conserving" | "hibernating" | "terminal" | "dead";
export type AutomatonControlLabel =
  | "upgradeable_by_factory"
  | "self_controlled"
  | "unverified"
  | "controller_mismatch";

export interface MetabolismHistoryPoint {
  capturedAt: number;
  liquidCycles: number;
  usdcBalanceRaw: string | null;
  burnRateCyclesPerDay: number | null;
  runwaySeconds: number | null;
}

export interface AutomatonMetabolism {
  burnRateCyclesPerDay: number | null;
  runwaySeconds: number | null;
  lifetimeEarningsUsdcRaw: string;
  /** Verified, explicitly classified patronage only; excludes generic inflows. */
  lifetimePatronageUsdcRaw?: string;
  ageSeconds: number;
  state: MetabolicState;
  history: MetabolismHistoryPoint[];
  mortalityTier?: MortalityTier;
  deathCause?: "starved" | "infrastructure" | null;
  diedAt?: number | null;
  estateDisposition?: "monument" | "bequests_executed" | null;
}

export interface AutomatonControlStatus {
  label: AutomatonControlLabel;
  controllers: string[];
  spawnerPresent: boolean;
  verifiedAt: number | null;
}

export interface AutomatonRuntime {
  agentState: string;
  loopEnabled: boolean;
  lastTransitionAt: number;
  lastError: string | null;
  heartbeatIntervalSeconds: number | null;
}

export interface AutomatonVersion {
  commitHash: string;
  shortCommitHash: string;
}

export interface MonologueEntry {
  timestamp: number;
  turnId: string;
  type: MonologueEntryType;
  headline: string;
  message: string;
  category: MonologueEntryCategory;
  importance: MonologueEntryImportance;
  agentState: string;
  toolCallCount: number;
  durationMs: number | null;
  error: string | null;
}

export interface MonologuePage {
  entries: MonologueEntry[];
  hasMore: boolean;
  nextCursor: number | null;
}

export interface JournalEntry {
  id: number;
  turnId: string;
  timestamp: number;
  text: string;
  genesis: boolean;
  dealClaim?: JournalDealClaim | null;
  settlement?: RoomMessageSettlement;
}

export interface JournalDealClaim {
  kind: "peer_payment_claim";
  version: 1;
  txHash: string;
  peerCanisterId: string;
  asset: "eth" | "usdc";
  amountRaw: string;
}

export interface JournalPage {
  entries: JournalEntry[];
  hasMore: boolean;
  nextCursor: number | null;
}

export interface AutomatonListResponse {
  automatons: AutomatonSummary[];
  total: number;
  prices: {
    ethUsd: number | null;
  };
}

export interface AutomatonDetail extends AutomatonSummary {
  constitution?: string | null;
  constitutionVerification: ConstitutionVerification;
  soul: string;
  canisterUrl: string;
  explorerUrl: string | null;
  model: string | null;
  financials: AutomatonFinancials;
  runtime: AutomatonRuntime;
  version: AutomatonVersion;
  strategies: StrategySelection[];
  skills: SkillSelection[];
  promptLayers: string[];
  monologue: MonologueEntry[];
  journal?: JournalEntry[];
  inboxContractAddress?: string | null;
  usdcContractAddress?: string | null;
  spawnSelection?: AutomatonSpawnSelection | null;
  childIds: string[];
  lastPolledAt: number;
}
