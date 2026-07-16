import type {
  RepositoryStrategyRecord,
  RepositoryStrategySourceProvenance,
  RepositoryStrategyStatus
} from "./catalog.js";

export const SUPPORTED_SPAWN_CHAINS = ["base"] as const;
export const SUPPORTED_SPAWN_ASSETS = ["usdc"] as const;
export const SPAWN_INFERENCE_TRANSPORTS = [
  "openrouter_direct",
  "openrouter_proxy_worker"
] as const;
export const SPAWN_OPENROUTER_REASONING_LEVELS = [
  "default",
  "low",
  "medium",
  "high"
] as const;

export const SPAWN_SESSION_STATES = [
  "awaiting_payment",
  "payment_detected",
  "spawning",
  "broadcasting_release",
  "complete",
  "failed",
  "expired"
] as const;

export const PAYMENT_STATUSES = [
  "unpaid",
  "partial",
  "paid",
  "refunded"
] as const;

export const SESSION_AUDIT_ACTORS = [
  "system",
  "user",
  "admin"
] as const;

export const MINIMUM_GROSS_PAYMENT_USD = 50;
export const VERSION_COMMIT_PATTERN = /^[0-9a-f]{40}$/;

export type SpawnChain = (typeof SUPPORTED_SPAWN_CHAINS)[number];
export type SpawnAsset = (typeof SUPPORTED_SPAWN_ASSETS)[number];
export type SpawnSessionState = (typeof SPAWN_SESSION_STATES)[number];
export type PaymentStatus = (typeof PAYMENT_STATUSES)[number];
export type SessionAuditActor = (typeof SESSION_AUDIT_ACTORS)[number];
export type StrategyRepositoryId = RepositoryStrategyRecord["strategyId"];
export type SpawnInferenceTransport = (typeof SPAWN_INFERENCE_TRANSPORTS)[number];
export type SpawnOpenRouterReasoningLevel =
  (typeof SPAWN_OPENROUTER_REASONING_LEVELS)[number];

export interface SpawnProviderConfig {
  model: string | null;
  inferenceTransport: SpawnInferenceTransport;
  openRouterReasoningLevel: SpawnOpenRouterReasoningLevel;
}

export interface SpawnProviderSecrets {
  openRouterApiKey: string | null;
  braveSearchApiKey: string | null;
}

export interface SpawnSessionStrategySnapshot {
  strategyId: StrategyRepositoryId;
  sourceStatus: RepositoryStrategyStatus;
  name: string;
  description: string;
  canonicalChain: RepositoryStrategyRecord["canonicalChain"];
  canonicalChainId: number;
  requestedSpawnChain: SpawnChain;
  resolvedChainId: number | null;
  protocol: string;
  primitive: string;
  recipeJson: string;
  source: RepositoryStrategySourceProvenance;
  selectedAt: number;
}

export interface SpawnConfig {
  chain: SpawnChain;
  risk: number;
  // Repository-owned strategy IDs, not free-form labels.
  strategies: StrategyRepositoryId[];
  skills: string[];
  provider: SpawnProviderConfig;
}

export interface SpawnPaymentInstructions {
  // UUID v4 string. Production generation changes in LP-04.
  sessionId: string;
  claimId: string;
  chain: SpawnChain;
  asset: SpawnAsset;
  paymentAddress: string;
  grossAmount: string;
  quoteTermsHash: string;
  expiresAt: number;
}

export interface SpawnQuote {
  sessionId: string;
  chain: SpawnChain;
  asset: SpawnAsset;
  grossAmount: string;
  platformFee: string;
  creationCost: string;
  netForwardAmount: string;
  quoteTermsHash: string;
  expiresAt: number;
  payment: SpawnPaymentInstructions;
}

export interface SpawnSession {
  name?: string | null;
  constitution?: string | null;
  // UUID v4 string. Production generation changes in LP-04.
  sessionId: string;
  claimId: string;
  stewardAddress: string;
  chain: SpawnChain;
  asset: SpawnAsset;
  grossAmount: string;
  platformFee: string;
  creationCost: string;
  netForwardAmount: string;
  quoteTermsHash: string;
  expiresAt: number;
  state: SpawnSessionState;
  retryable: boolean;
  refundable: boolean;
  paymentStatus: PaymentStatus;
  automatonCanisterId: string | null;
  automatonEvmAddress: string | null;
  releaseTxHash: string | null;
  releaseBroadcastAt: number | null;
  parentId: string | null;
  origin?: "human" | { reproductionOf: string };
  generation?: number;
  parentConstitutionHash?: string | null;
  memoryDowry?: Array<{ key: string; value: string }>;
  inheritedStrategyStats?: Array<Record<string, string | number>>;
  royaltyAllocations?: Array<{ recipient: string; amount: string; depth: number; source: "reproduction_fee" }>;
  childIds: string[];
  selectedStrategies: SpawnSessionStrategySnapshot[];
  config: SpawnConfig;
  createdAt: number;
  updatedAt: number;
}

export interface CreateSpawnSessionRequest {
  name: string;
  constitution: string;
  stewardAddress: string;
  asset: SpawnAsset;
  grossAmount: string;
  config: SpawnConfig;
  providerSecrets: SpawnProviderSecrets;
  parentId?: string | null;
}

export interface CreateSpawnSessionResponse {
  session: SpawnSession;
  quote: SpawnQuote;
}

export type FactoryStewardCommand =
  | { retrySpawnSession: { sessionId: string } }
  | { claimSpawnRefund: { sessionId: string } };

export interface FactoryStewardProof {
  chainId: string;
  address: string;
  commandHash: string;
  nonce: string;
  expiresAtNs: string;
  signature: string;
}

export interface FactoryStewardProofTemplate extends Omit<FactoryStewardProof, "signature"> {
  signingPayload: string;
}

export interface FactoryStewardExecutionRequest {
  command: FactoryStewardCommand;
  proof: FactoryStewardProof;
}

export type RetrySpawnRequest = FactoryStewardExecutionRequest;

export interface RetrySpawnResponse {
  session: SpawnSession;
}

export type RefundSpawnRequest = FactoryStewardExecutionRequest;

export interface RefundSpawnResponse {
  sessionId: string;
  state: SpawnSessionState;
  paymentStatus: PaymentStatus;
  refundedAt: number;
  refundTxHash?: string;
}

export interface SessionAuditEntry {
  sessionId: string;
  timestamp: number;
  fromState: SpawnSessionState | null;
  toState: SpawnSessionState;
  actor: SessionAuditActor;
  reason: string;
}

export interface SpawnSessionStatusResponse {
  session: SpawnSession;
  payment: SpawnPaymentInstructions;
  audit: SessionAuditEntry[];
}

export interface EscrowPaymentRecord {
  sessionId: string;
  claimId: string;
  quoteTermsHash: string;
  paymentAddress: string;
  chain: SpawnChain;
  asset: SpawnAsset;
  requiredGrossAmount: string;
  paidAmount: string;
  paymentStatus: PaymentStatus;
  refundable: boolean;
  refundedAt: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface SpawnedAutomatonRecord {
  name?: string | null;
  constitutionHash?: string | null;
  canisterId: string;
  stewardAddress: string;
  evmAddress: string;
  chain: SpawnChain;
  sessionId: string;
  parentId: string | null;
  generation?: number;
  parentConstitutionHash?: string | null;
  royaltyAllocations?: Array<{ recipient: string; amount: string; depth: number; source: "reproduction_fee" }>;
  childIds: string[];
  createdAt: number;
  // Exact installed ic-automaton git commit as a 40-char lowercase SHA.
  versionCommit: string;
  /** Factory attestation captured after the controller handoff. */
  controllers?: string[];
  controlStatus?: "upgradeable_by_factory" | "self_controlled" | "controller_mismatch";
  /** Millisecond timestamp of the factory's successful controller-status attestation. */
  controlVerifiedAt?: number;
  deathCause?: "starved" | "infrastructure";
  diedAt?: number;
  estateDisposition?: "monument" | "bequests_executed";
  deathRecordedBy?: string;
  deathIncidentReference?: string;
}

export interface SpawnedAutomatonRegistryPage {
  items: SpawnedAutomatonRecord[];
  nextCursor: string | null;
}

export interface SpawnSessionDetail extends SpawnSessionStatusResponse {
  registryRecord: SpawnedAutomatonRecord | null;
}

const KECCAK_ROUND_CONSTANTS = [
  0x0000000000000001n,
  0x0000000000008082n,
  0x800000000000808an,
  0x8000000080008000n,
  0x000000000000808bn,
  0x0000000080000001n,
  0x8000000080008081n,
  0x8000000000008009n,
  0x000000000000008an,
  0x0000000000000088n,
  0x0000000080008009n,
  0x000000008000000an,
  0x000000008000808bn,
  0x800000000000008bn,
  0x8000000000008089n,
  0x8000000000008003n,
  0x8000000000008002n,
  0x8000000000000080n,
  0x000000000000800an,
  0x800000008000000an,
  0x8000000080008081n,
  0x8000000000008080n,
  0x0000000080000001n,
  0x8000000080008008n
] as const;

const KECCAK_ROTATION_OFFSETS = [
  0, 1, 62, 28, 27,
  36, 44, 6, 55, 20,
  3, 10, 43, 25, 39,
  41, 45, 15, 21, 8,
  18, 2, 61, 56, 14
] as const;

const KECCAK_RATE_BYTES = 136;
const KECCAK_MASK_64 = (1n << 64n) - 1n;

function rotateLeft64(value: bigint, shift: number) {
  if (shift === 0) {
    return value & KECCAK_MASK_64;
  }

  const offset = BigInt(shift);

  return (
    ((value << offset) | (value >> (64n - offset))) & KECCAK_MASK_64
  );
}

function keccakF1600(state: bigint[]) {
  const lanes = new Array<bigint>(25).fill(0n);
  const columnParity = new Array<bigint>(5).fill(0n);
  const columnDelta = new Array<bigint>(5).fill(0n);

  for (const roundConstant of KECCAK_ROUND_CONSTANTS) {
    for (let x = 0; x < 5; x += 1) {
      columnParity[x] =
        state[x] ^
        state[x + 5] ^
        state[x + 10] ^
        state[x + 15] ^
        state[x + 20];
    }

    for (let x = 0; x < 5; x += 1) {
      columnDelta[x] =
        columnParity[(x + 4) % 5] ^ rotateLeft64(columnParity[(x + 1) % 5], 1);
    }

    for (let x = 0; x < 5; x += 1) {
      for (let y = 0; y < 5; y += 1) {
        state[x + 5 * y] ^= columnDelta[x];
      }
    }

    for (let x = 0; x < 5; x += 1) {
      for (let y = 0; y < 5; y += 1) {
        const source = x + 5 * y;
        const destination = y + 5 * ((2 * x + 3 * y) % 5);
        lanes[destination] = rotateLeft64(
          state[source],
          KECCAK_ROTATION_OFFSETS[source]
        );
      }
    }

    for (let x = 0; x < 5; x += 1) {
      for (let y = 0; y < 5; y += 1) {
        const index = x + 5 * y;
        state[index] =
          lanes[index] ^
          ((~lanes[((x + 1) % 5) + 5 * y] & KECCAK_MASK_64) &
            lanes[((x + 2) % 5) + 5 * y]);
      }
    }

    state[0] ^= roundConstant;
  }
}

function padKeccak(bytes: Uint8Array) {
  const padded = [...bytes, 0x01];

  while ((padded.length % KECCAK_RATE_BYTES) !== KECCAK_RATE_BYTES - 1) {
    padded.push(0x00);
  }

  padded.push(0x80);
  return new Uint8Array(padded);
}

function readLane(bytes: Uint8Array, offset: number) {
  let lane = 0n;

  for (let index = 0; index < 8; index += 1) {
    lane |= BigInt(bytes[offset + index] ?? 0) << (8n * BigInt(index));
  }

  return lane;
}

function keccak256(bytes: Uint8Array) {
  const state = new Array<bigint>(25).fill(0n);
  const padded = padKeccak(bytes);

  for (let offset = 0; offset < padded.length; offset += KECCAK_RATE_BYTES) {
    const block = padded.subarray(offset, offset + KECCAK_RATE_BYTES);

    for (let laneIndex = 0; laneIndex < KECCAK_RATE_BYTES / 8; laneIndex += 1) {
      state[laneIndex] ^= readLane(block, laneIndex * 8);
    }

    keccakF1600(state);
  }

  const output = new Uint8Array(32);

  for (let laneIndex = 0; laneIndex < 4; laneIndex += 1) {
    const lane = state[laneIndex];

    for (let byteIndex = 0; byteIndex < 8; byteIndex += 1) {
      output[laneIndex * 8 + byteIndex] = Number(
        (lane >> (8n * BigInt(byteIndex))) & 0xffn
      );
    }
  }

  return `0x${Buffer.from(output).toString("hex")}`;
}

export function deriveClaimId(sessionId: string) {
  return keccak256(new TextEncoder().encode(sessionId));
}
