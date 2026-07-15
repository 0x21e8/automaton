import type { RepositoryStrategyRecord, RepositoryStrategySourceProvenance, RepositoryStrategyStatus } from "./catalog.js";
export declare const SUPPORTED_SPAWN_CHAINS: readonly ["base"];
export declare const SUPPORTED_SPAWN_ASSETS: readonly ["usdc"];
export declare const SPAWN_SESSION_STATES: readonly ["awaiting_payment", "payment_detected", "spawning", "broadcasting_release", "complete", "failed", "expired"];
export declare const PAYMENT_STATUSES: readonly ["unpaid", "partial", "paid", "refunded"];
export declare const SESSION_AUDIT_ACTORS: readonly ["system", "user", "admin"];
export declare const MINIMUM_GROSS_PAYMENT_USD = 50;
export declare const VERSION_COMMIT_PATTERN: RegExp;
export type SpawnChain = (typeof SUPPORTED_SPAWN_CHAINS)[number];
export type SpawnAsset = (typeof SUPPORTED_SPAWN_ASSETS)[number];
export type SpawnSessionState = (typeof SPAWN_SESSION_STATES)[number];
export type PaymentStatus = (typeof PAYMENT_STATUSES)[number];
export type SessionAuditActor = (typeof SESSION_AUDIT_ACTORS)[number];
export type StrategyRepositoryId = RepositoryStrategyRecord["strategyId"];
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
    strategies: StrategyRepositoryId[];
    skills: string[];
    provider: SpawnProviderConfig;
}
export interface SpawnPaymentInstructions {
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
    childIds: string[];
    config: SpawnConfig;
    createdAt: number;
    updatedAt: number;
}
export interface CreateSpawnSessionRequest {
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
export interface RetrySpawnRequest {
    sessionId: string;
}
export interface RetrySpawnResponse {
    session: SpawnSession;
}
export interface RefundSpawnRequest {
    sessionId: string;
}
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
    canisterId: string;
    stewardAddress: string;
    evmAddress: string;
    chain: SpawnChain;
    sessionId: string;
    parentId: string | null;
    generation?: number;
    parentConstitutionHash?: string | null;
    childIds: string[];
    createdAt: number;
    versionCommit: string;
}
export interface SpawnedAutomatonRegistryPage {
    items: SpawnedAutomatonRecord[];
    nextCursor: string | null;
}
export interface SpawnSessionDetail extends SpawnSessionStatusResponse {
    registryRecord: SpawnedAutomatonRecord | null;
}
export declare function deriveClaimId(sessionId: string): string;
//# sourceMappingURL=spawn.d.ts.map
