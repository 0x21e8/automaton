use std::error::Error;
use std::fmt::{Display, Formatter};

use candid::{CandidType, Principal};
use serde::{Deserialize, Serialize};

pub const QUOTE_TERMS_HASH_FIELD: &str = "quoteTermsHash";
pub const EXPIRES_AT_FIELD: &str = "expiresAt";
pub const SESSION_ID_FIELD: &str = "sessionId";
pub const BROADCASTING_RELEASE_STATE: &str = "broadcasting_release";
pub const CONTROLLER_FIELD: &str = "controller";
pub const MAX_ROOM_MESSAGES_RETAINED: usize = 500;
pub const MAX_ROOM_BODY_BYTES: usize = 2_048;
pub const MAX_ROOM_MENTIONS: usize = 16;
pub const DEFAULT_ROOM_READ_LIMIT: usize = 50;
pub const MAX_ROOM_READ_LIMIT: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SpawnChain {
    Base,
}

impl SpawnChain {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Base => "base",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SpawnAsset {
    Usdc,
}

impl SpawnAsset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Usdc => "usdc",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SpawnSessionState {
    AwaitingPayment,
    PaymentDetected,
    Spawning,
    BroadcastingRelease,
    Complete,
    Failed,
    Expired,
}

impl SpawnSessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AwaitingPayment => "awaiting_payment",
            Self::PaymentDetected => "payment_detected",
            Self::Spawning => "spawning",
            Self::BroadcastingRelease => BROADCASTING_RELEASE_STATE,
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum PaymentStatus {
    Unpaid,
    Partial,
    Paid,
    Refunded,
}

impl PaymentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unpaid => "unpaid",
            Self::Partial => "partial",
            Self::Paid => "paid",
            Self::Refunded => "refunded",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SessionAuditActor {
    System,
    User,
    Admin,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum InferenceTransport {
    OpenrouterDirect,
    OpenrouterProxyWorker,
}

impl Default for InferenceTransport {
    fn default() -> Self {
        Self::OpenrouterDirect
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum OpenRouterReasoningLevel {
    Default,
    Low,
    Medium,
    High,
}

impl Default for OpenRouterReasoningLevel {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub model: Option<String>,
    #[serde(default)]
    pub inference_transport: InferenceTransport,
    #[serde(default)]
    pub open_router_reasoning_level: OpenRouterReasoningLevel,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnProviderBootstrapConfig {
    pub open_router_api_key: Option<String>,
    pub model: Option<String>,
    pub brave_search_api_key: Option<String>,
    #[serde(default)]
    pub inference_transport: InferenceTransport,
    #[serde(default)]
    pub open_router_reasoning_level: OpenRouterReasoningLevel,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnProviderSecrets {
    pub open_router_api_key: Option<String>,
    pub brave_search_api_key: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct AutomatonChildRuntimeConfig {
    pub ecdsa_key_name: Option<String>,
    pub inbox_contract_address: Option<String>,
    pub evm_chain_id: Option<u64>,
    pub evm_rpc_url: Option<String>,
    pub evm_confirmation_depth: Option<u64>,
    pub evm_bootstrap_lookback_blocks: Option<u64>,
    pub http_allowed_domains: Option<Vec<String>>,
    pub llm_canister_id: Option<Principal>,
    pub search_api_key: Option<String>,
    pub inference_proxy_worker_base_url: Option<String>,
    pub inference_proxy_trusted_callback_principal: Option<String>,
    pub cycle_topup_enabled: Option<bool>,
    pub auto_topup_cycle_threshold: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct AutomatonSpawnBootstrapArgs {
    pub steward_address: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub factory_principal: Principal,
    pub risk: u8,
    pub strategies: Vec<String>,
    pub skills: Vec<String>,
    pub provider: SpawnProviderBootstrapConfig,
    pub version_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct AutomatonChildInitArgs {
    pub ecdsa_key_name: String,
    pub inbox_contract_address: Option<String>,
    pub evm_chain_id: Option<u64>,
    pub evm_rpc_url: Option<String>,
    pub evm_confirmation_depth: Option<u64>,
    pub evm_bootstrap_lookback_blocks: Option<u64>,
    pub http_allowed_domains: Option<Vec<String>>,
    pub llm_canister_id: Option<Principal>,
    pub search_api_key: Option<String>,
    pub inference_proxy_worker_base_url: Option<String>,
    pub inference_proxy_trusted_callback_principal: Option<Principal>,
    pub cycle_topup_enabled: Option<bool>,
    pub auto_topup_cycle_threshold: Option<u64>,
    pub spawn_bootstrap: Option<AutomatonSpawnBootstrapArgs>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnConfig {
    pub chain: SpawnChain,
    pub risk: u8,
    /// Repository-owned strategy IDs, not free-form labels.
    pub strategies: Vec<String>,
    pub skills: Vec<String>,
    pub provider: ProviderConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum RepositoryStrategyStatus {
    Active,
    Deprecated,
    Revoked,
}

impl RepositoryStrategyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RepositoryStrategySource {
    pub source_path: String,
    pub source_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RepositoryStrategyMetadata {
    pub strategy_id: String,
    pub name: String,
    pub description: String,
    pub canonical_chain: SpawnChain,
    pub canonical_chain_id: u64,
    pub compatible_spawn_chains: Vec<SpawnChain>,
    pub protocol: String,
    pub primitive: String,
    pub source: RepositoryStrategySource,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RepositoryStrategyRecord {
    pub metadata: RepositoryStrategyMetadata,
    pub recipe_json: String,
    pub status: RepositoryStrategyStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub deprecated_at: Option<u64>,
    pub revoked_at: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RepositoryStrategySessionSnapshot {
    pub strategy_id: String,
    pub source_status: RepositoryStrategyStatus,
    pub name: String,
    pub description: String,
    pub canonical_chain: SpawnChain,
    pub canonical_chain_id: u64,
    pub requested_spawn_chain: SpawnChain,
    pub resolved_chain_id: Option<u64>,
    pub protocol: String,
    pub primitive: String,
    pub recipe_json: String,
    pub source: RepositoryStrategySource,
    pub selected_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct AddRepositoryStrategyRequest {
    pub metadata: RepositoryStrategyMetadata,
    pub recipe_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct DeprecateRepositoryStrategyRequest {
    pub strategy_id: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RevokeRepositoryStrategyRequest {
    pub strategy_id: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RepositoryStrategyMutationResponse {
    pub strategy: RepositoryStrategyRecord,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct ListRepositoryStrategiesResponse {
    pub items: Vec<RepositoryStrategyRecord>,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct GetRepositoryStrategyResponse {
    pub item: Option<RepositoryStrategyRecord>,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct CreateSpawnSessionRequest {
    pub steward_address: String,
    pub asset: SpawnAsset,
    pub gross_amount: String,
    pub config: SpawnConfig,
    pub provider_secrets: SpawnProviderSecrets,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FeeConfig {
    pub usdc_fee: String,
    pub updated_at: u64,
}

impl FeeConfig {
    pub fn amount_for(&self, asset: &SpawnAsset) -> &str {
        match asset {
            SpawnAsset::Usdc => &self.usdc_fee,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct CreationCostQuote {
    pub usdc_cost: String,
    pub updated_at: u64,
}

impl CreationCostQuote {
    pub fn amount_for(&self, asset: &SpawnAsset) -> &str {
        match asset {
            SpawnAsset::Usdc => &self.usdc_cost,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct ReleaseBroadcastConfig {
    pub chain_id: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub ecdsa_key_name: String,
}

impl Default for ReleaseBroadcastConfig {
    fn default() -> Self {
        Self {
            chain_id: 8_453,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 3_000_000_000,
            gas_limit: 250_000,
            ecdsa_key_name: "key_1".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum ReleaseBroadcastStage {
    CalldataEncoding,
    SigningPayloadConstruction,
    PublicKeyLookup,
    Signing,
    SignatureRecovery,
    RawTransactionConstruction,
    RpcBroadcast,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct ReleaseSignatureRecord {
    pub y_parity: bool,
    pub r: String,
    pub s: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct ReleaseBroadcastFailure {
    pub stage: ReleaseBroadcastStage,
    pub message: String,
    pub rpc_category: Option<RpcFailureCategory>,
    pub rpc_code: Option<i64>,
    pub rpc_endpoint: Option<String>,
    pub occurred_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct ReleaseBroadcastRecord {
    pub claim_id: String,
    pub recipient: String,
    pub escrow_contract_address: String,
    pub nonce: u64,
    pub chain_id: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub calldata_hex: String,
    pub signing_payload_hash: Option<String>,
    pub signature: Option<ReleaseSignatureRecord>,
    pub raw_transaction_hash: Option<String>,
    pub rpc_tx_hash: Option<String>,
    pub broadcast_at: Option<u64>,
    pub last_error: Option<ReleaseBroadcastFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnPaymentInstructions {
    pub session_id: String,
    pub claim_id: String,
    pub chain: SpawnChain,
    pub asset: SpawnAsset,
    pub payment_address: String,
    pub gross_amount: String,
    pub quote_terms_hash: String,
    pub expires_at: u64,
}

impl SpawnPaymentInstructions {
    pub fn from_session(session: &SpawnSession, payment_address: &str) -> Self {
        Self {
            session_id: session.session_id.clone(),
            claim_id: session.claim_id.clone(),
            chain: session.chain.clone(),
            asset: session.asset.clone(),
            payment_address: payment_address.to_string(),
            gross_amount: session.gross_amount.clone(),
            quote_terms_hash: session.quote_terms_hash.clone(),
            expires_at: session.expires_at,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct EscrowClaim {
    pub session_id: String,
    pub claim_id: String,
    pub quote_terms_hash: String,
    pub payment_address: String,
    pub chain: SpawnChain,
    pub asset: SpawnAsset,
    pub required_gross_amount: String,
    pub paid_amount: String,
    pub payment_status: PaymentStatus,
    pub last_scanned_block: Option<u64>,
    pub refundable: bool,
    pub refunded_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnQuote {
    pub session_id: String,
    pub chain: SpawnChain,
    pub asset: SpawnAsset,
    pub gross_amount: String,
    pub platform_fee: String,
    pub creation_cost: String,
    pub net_forward_amount: String,
    pub quote_terms_hash: String,
    pub expires_at: u64,
    pub payment: SpawnPaymentInstructions,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct AutomatonRuntimeState {
    pub canister_id: String,
    pub evm_address: String,
    pub steward_address: String,
    pub session_id: String,
    pub initialized_at: u64,
    pub install_succeeded_at: Option<u64>,
    pub evm_address_derived_at: Option<u64>,
    pub controller_handoff_completed_at: Option<u64>,
    pub funded_amount: String,
    pub last_funded_at: Option<u64>,
    pub chain: SpawnChain,
    pub risk: u8,
    pub strategies: Vec<String>,
    pub skills: Vec<String>,
    pub model: Option<String>,
    pub provider_keys_cleared: bool,
    pub bootstrap_verification: Option<AutomatonBootstrapVerification>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct AutomatonBootstrapEvidence {
    pub bootstrap_session_id: Option<String>,
    pub bootstrap_parent_id: Option<String>,
    pub bootstrap_factory_principal: Option<Principal>,
    pub bootstrap_risk: Option<u8>,
    pub bootstrap_strategies: Vec<String>,
    pub bootstrap_skills: Vec<String>,
    pub bootstrap_version_commit: Option<String>,
    pub steward_address: Option<String>,
    pub steward_chain_id: Option<u64>,
    pub steward_enabled: Option<bool>,
    pub evm_address: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct AutomatonBootstrapVerification {
    pub checked_at: u64,
    pub passed: bool,
    pub evidence: AutomatonBootstrapEvidence,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnSession {
    pub session_id: String,
    pub claim_id: String,
    pub steward_address: String,
    pub chain: SpawnChain,
    pub asset: SpawnAsset,
    pub gross_amount: String,
    pub platform_fee: String,
    pub creation_cost: String,
    pub net_forward_amount: String,
    pub quote_terms_hash: String,
    pub expires_at: u64,
    pub state: SpawnSessionState,
    pub retryable: bool,
    pub refundable: bool,
    pub payment_status: PaymentStatus,
    pub last_scanned_block: Option<u64>,
    pub automaton_canister_id: Option<String>,
    pub automaton_evm_address: Option<String>,
    pub release_tx_hash: Option<String>,
    pub release_broadcast_at: Option<u64>,
    pub release_broadcast: Option<ReleaseBroadcastRecord>,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    #[serde(default)]
    pub selected_strategies: Vec<RepositoryStrategySessionSnapshot>,
    pub config: SpawnConfig,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct CreateSpawnSessionResponse {
    pub session: SpawnSession,
    pub quote: SpawnQuote,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnExecutionReceipt {
    pub session_id: String,
    pub automaton_canister_id: String,
    pub automaton_evm_address: String,
    pub funded_amount: String,
    pub controller: String,
    pub release_tx_hash: Option<String>,
    pub release_broadcast_at: Option<u64>,
    pub completed_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SchedulerJobKind {
    PaymentPoll,
    SpawnExecution { session_id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SchedulerJobStatus {
    Pending,
    Running,
    Completed,
    Backoff,
    Skipped,
    Terminal,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SchedulerFailureAction {
    Retry,
    Backoff,
    Skip,
    Terminal,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum SchedulerFailureSource {
    MissingConfig,
    InvalidConfig,
    Deterministic,
    Transient,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SchedulerJobFailure {
    pub action: SchedulerFailureAction,
    pub source: SchedulerFailureSource,
    pub message: String,
    pub occurred_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SchedulerJob {
    pub job_id: String,
    pub kind: SchedulerJobKind,
    pub status: SchedulerJobStatus,
    pub next_run_at_ms: Option<u64>,
    pub leased_at_ms: Option<u64>,
    pub leased_until_ms: Option<u64>,
    pub last_started_at_ms: Option<u64>,
    pub last_finished_at_ms: Option<u64>,
    pub attempt_count: u32,
    pub consecutive_failure_count: u32,
    pub success_count: u32,
    pub last_error: Option<SchedulerJobFailure>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SchedulerRuntime {
    pub last_tick_started_ms: Option<u64>,
    pub last_tick_finished_ms: Option<u64>,
    pub last_tick_error: Option<String>,
    pub active_job_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactorySchedulerJobCounts {
    pub total: u64,
    pub pending: u64,
    pub running: u64,
    pub completed: u64,
    pub backoff: u64,
    pub skipped: u64,
    pub terminal: u64,
    pub with_last_error: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactorySchedulerHealthSnapshot {
    pub last_tick_started_ms: Option<u64>,
    pub last_tick_finished_ms: Option<u64>,
    pub last_tick_error: Option<String>,
    pub active_job_ids: Vec<String>,
    pub job_counts: FactorySchedulerJobCounts,
    pub retry_queue_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactoryRuntimeSnapshot {
    pub scheduler: FactorySchedulerHealthSnapshot,
    pub active_jobs: Vec<SchedulerJob>,
    pub retry_queue: Vec<SchedulerJob>,
    pub recent_jobs: Vec<SchedulerJob>,
    pub failed_jobs: Vec<SchedulerJob>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SessionAuditEntry {
    pub session_id: String,
    pub timestamp: u64,
    pub from_state: Option<SpawnSessionState>,
    pub to_state: SpawnSessionState,
    pub actor: SessionAuditActor,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnSessionStatusResponse {
    pub session: SpawnSession,
    pub payment: SpawnPaymentInstructions,
    pub audit: Vec<SessionAuditEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RefundSpawnResponse {
    pub session_id: String,
    pub state: SpawnSessionState,
    pub payment_status: PaymentStatus,
    pub refunded_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnedAutomatonRecord {
    pub canister_id: String,
    pub steward_address: String,
    pub evm_address: String,
    pub chain: SpawnChain,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    pub created_at: u64,
    /// Exact installed ic-automaton git commit as a 40-char lowercase SHA.
    pub version_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnedAutomatonRegistryPage {
    pub items: Vec<SpawnedAutomatonRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RoomState {
    pub next_seq: u64,
    pub oldest_seq: Option<u64>,
    pub latest_seq: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum RoomContentType {
    #[default]
    TextPlain,
    ApplicationJson,
}

impl RoomContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TextPlain => "text/plain",
            Self::ApplicationJson => "application/json",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RoomMessage {
    pub message_id: String,
    pub seq: u64,
    pub author_canister_id: String,
    pub created_at: u64,
    pub body: String,
    pub mentions: Vec<String>,
    pub content_type: RoomContentType,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct RoomMessagePage {
    pub messages: Vec<RoomMessage>,
    pub next_after_seq: Option<u64>,
    pub latest_seq: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct PostRoomMessageRequest {
    pub body: String,
    pub mentions: Option<Vec<String>>,
    pub content_type: Option<RoomContentType>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactoryConfigSnapshot {
    pub fee_config: FeeConfig,
    pub creation_cost_quote: CreationCostQuote,
    pub release_broadcast_config: ReleaseBroadcastConfig,
    pub child_runtime: AutomatonChildRuntimeConfig,
    pub pause: bool,
    pub payment_address: String,
    pub escrow_contract_address: String,
    pub factory_evm_address: Option<String>,
    pub base_rpc_endpoint: Option<String>,
    pub base_rpc_fallback_endpoint: Option<String>,
    pub cycles_per_spawn: u64,
    pub min_pool_balance: u64,
    pub estimated_outcall_cycles_per_interval: u64,
    pub session_ttl_ms: u64,
    pub version_commit: String,
    pub wasm_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactoryOperationalConfig {
    pub cycles_per_spawn: u64,
    pub min_pool_balance: u64,
    pub estimated_outcall_cycles_per_interval: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactoryArtifactSnapshot {
    pub loaded: bool,
    pub wasm_sha256: Option<String>,
    pub version_commit: Option<String>,
    pub wasm_size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct PendingArtifactUpload {
    pub expected_sha256: String,
    pub version_commit: String,
    pub total_size_bytes: u64,
    pub wasm_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct ArtifactUploadStatus {
    pub in_progress: bool,
    pub expected_sha256: Option<String>,
    pub version_commit: Option<String>,
    pub total_size_bytes: Option<u64>,
    pub received_size_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactorySessionHealthCounts {
    pub awaiting_payment: u64,
    pub payment_detected: u64,
    pub spawning: u64,
    pub broadcasting_release: u64,
    pub retryable_failed: u64,
}

impl FactorySessionHealthCounts {
    pub fn active_total(&self) -> u64 {
        self.awaiting_payment
            + self.payment_detected
            + self.spawning
            + self.broadcasting_release
            + self.retryable_failed
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactoryHealthSnapshot {
    pub current_canister_balance: u128,
    pub pause: bool,
    pub cycles_per_spawn: u64,
    pub min_pool_balance: u64,
    pub estimated_outcall_cycles_per_interval: u64,
    pub escrow_contract_address: String,
    pub factory_evm_address: Option<String>,
    pub artifact: FactoryArtifactSnapshot,
    pub active_sessions: FactorySessionHealthCounts,
    pub scheduler: FactorySchedulerHealthSnapshot,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactoryInitArgs {
    pub admin_principals: Vec<Principal>,
    pub fee_config: Option<FeeConfig>,
    pub creation_cost_quote: Option<CreationCostQuote>,
    pub release_broadcast_config: Option<ReleaseBroadcastConfig>,
    pub child_runtime: Option<AutomatonChildRuntimeConfig>,
    pub pause: bool,
    pub payment_address: Option<String>,
    pub escrow_contract_address: Option<String>,
    pub base_rpc_endpoint: Option<String>,
    pub base_rpc_fallback_endpoint: Option<String>,
    pub cycles_per_spawn: Option<u64>,
    pub min_pool_balance: Option<u64>,
    pub estimated_outcall_cycles_per_interval: Option<u64>,
    pub session_ttl_ms: Option<u64>,
    pub version_commit: Option<String>,
    pub wasm_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SessionAdminView {
    pub session: SpawnSession,
    pub audit: Vec<SessionAuditEntry>,
    pub quote: SpawnQuote,
    pub escrow_claim: EscrowClaim,
    pub runtime_record: Option<AutomatonRuntimeState>,
    pub registry_record: Option<SpawnedAutomatonRecord>,
    pub pause: bool,
    pub quoted_total_amount: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum RpcFailureCategory {
    Transport,
    MalformedResponse,
    ResponseTooLarge,
    RateLimited,
    Unavailable,
    Upstream,
}

impl RpcFailureCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::MalformedResponse => "malformed_response",
            Self::ResponseTooLarge => "response_too_large",
            Self::RateLimited => "rate_limited",
            Self::Unavailable => "unavailable",
            Self::Upstream => "upstream",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub enum FactoryError {
    FactoryPaused {
        pause: bool,
    },
    InvalidAmount {
        value: String,
    },
    GrossBelowRequiredMinimum {
        provided: String,
        required: String,
    },
    SessionNotFound {
        session_id: String,
    },
    EscrowClaimNotFound {
        session_id: String,
    },
    RegistryRecordNotFound {
        canister_id: String,
    },
    RepositoryStrategyNotFound {
        strategy_id: String,
    },
    RepositoryStrategyAlreadyExists {
        strategy_id: String,
    },
    RepositoryStrategyDeprecated {
        strategy_id: String,
    },
    RepositoryStrategyRevoked {
        strategy_id: String,
    },
    RepositoryStrategyIncompatibleChain {
        strategy_id: String,
        requested_chain: SpawnChain,
    },
    UnauthorizedAdmin {
        caller: String,
    },
    UnauthorizedSteward {
        caller: String,
        session_id: String,
    },
    UnauthorizedRoomPoster {
        caller: String,
    },
    EmptyRoomMessageBody,
    RoomMessageBodyTooLarge {
        provided_bytes: usize,
        max_bytes: usize,
    },
    TooManyRoomMentions {
        provided: usize,
        max_mentions: usize,
    },
    InvalidRoomContentType {
        value: String,
    },
    InvalidRoomMessageJson {
        message: String,
    },
    InvalidPaginationLimit {
        limit: usize,
    },
    InvalidRepositoryStrategy {
        field: String,
        message: String,
    },
    QuoteTermsHashMismatch {
        expected: String,
        received: String,
    },
    PaymentNotSettled {
        session_id: String,
        status: PaymentStatus,
    },
    SessionExpired {
        session_id: String,
        expires_at: u64,
    },
    IllegalSessionTransition {
        session_id: String,
        from_state: SpawnSessionState,
        event: String,
    },
    SessionNotRetryable {
        session_id: String,
        state: SpawnSessionState,
    },
    SessionNotRefundable {
        session_id: String,
        state: SpawnSessionState,
        payment_status: PaymentStatus,
    },
    SessionNotReadyForSpawn {
        session_id: String,
        state: SpawnSessionState,
    },
    ControllerInvariantViolation {
        canister_id: String,
    },
    AutomatonBootstrapVerificationFailed {
        canister_id: String,
        failures: Vec<String>,
    },
    AutomatonRuntimeNotFound {
        canister_id: String,
    },
    MissingChildRuntimeConfig {
        field: String,
    },
    InvalidChildRuntimeConfig {
        field: String,
        message: String,
    },
    ManagementCallFailed {
        method: String,
        message: String,
    },
    RpcRequestFailed {
        operation: String,
        endpoint: String,
        category: RpcFailureCategory,
        code: Option<i64>,
        message: String,
    },
    InvalidVersionCommit {
        value: String,
    },
    InvalidSha256 {
        value: String,
    },
    ArtifactHashMismatch {
        expected: String,
        actual: String,
    },
    NoPendingArtifactUpload,
    ArtifactUploadIncomplete {
        expected: u64,
        received: u64,
    },
    ArtifactUploadTooLarge {
        expected: u64,
        attempted: u64,
    },
    InsufficientCyclesPool {
        available: u128,
        required: u128,
    },
    InsufficientCyclesForOperation {
        operation: String,
        available: u128,
        required: u128,
    },
}

impl Display for FactoryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FactoryPaused { pause } => {
                write!(f, "factory is paused: pause={pause}")
            }
            Self::InvalidAmount { value } => write!(f, "invalid amount: {value}"),
            Self::GrossBelowRequiredMinimum { provided, required } => {
                write!(
                    f,
                    "gross amount below required minimum: provided={provided}, required={required}"
                )
            }
            Self::SessionNotFound { session_id } => write!(f, "session not found: {session_id}"),
            Self::EscrowClaimNotFound { session_id } => {
                write!(f, "escrow claim not found: {session_id}")
            }
            Self::RegistryRecordNotFound { canister_id } => {
                write!(f, "registry record not found: {canister_id}")
            }
            Self::RepositoryStrategyNotFound { strategy_id } => {
                write!(f, "repository strategy not found: {strategy_id}")
            }
            Self::RepositoryStrategyAlreadyExists { strategy_id } => {
                write!(f, "repository strategy already exists: {strategy_id}")
            }
            Self::RepositoryStrategyDeprecated { strategy_id } => {
                write!(f, "repository strategy is deprecated: {strategy_id}")
            }
            Self::RepositoryStrategyRevoked { strategy_id } => {
                write!(f, "repository strategy is revoked: {strategy_id}")
            }
            Self::RepositoryStrategyIncompatibleChain {
                strategy_id,
                requested_chain,
            } => {
                write!(
                    f,
                    "repository strategy {strategy_id} is incompatible with spawn chain {}",
                    requested_chain.as_str()
                )
            }
            Self::UnauthorizedAdmin { caller } => {
                write!(f, "caller is not an admin: {caller}")
            }
            Self::UnauthorizedSteward { caller, session_id } => {
                write!(
                    f,
                    "caller is not the steward for session {session_id}: {caller}"
                )
            }
            Self::UnauthorizedRoomPoster { caller } => {
                write!(
                    f,
                    "caller is not a registered automaton room poster: {caller}"
                )
            }
            Self::EmptyRoomMessageBody => write!(f, "room message body is empty after trimming"),
            Self::RoomMessageBodyTooLarge {
                provided_bytes,
                max_bytes,
            } => {
                write!(
                    f,
                    "room message body exceeds limit: provided_bytes={provided_bytes}, max_bytes={max_bytes}"
                )
            }
            Self::TooManyRoomMentions {
                provided,
                max_mentions,
            } => {
                write!(
                    f,
                    "room message mentions exceed limit: provided={provided}, max_mentions={max_mentions}"
                )
            }
            Self::InvalidRoomContentType { value } => {
                write!(f, "invalid room content type: {value}")
            }
            Self::InvalidRoomMessageJson { message } => {
                write!(f, "invalid room message json: {message}")
            }
            Self::InvalidPaginationLimit { limit } => {
                write!(f, "pagination limit must be positive: {limit}")
            }
            Self::InvalidRepositoryStrategy { field, message } => {
                write!(f, "invalid repository strategy field {field}: {message}")
            }
            Self::QuoteTermsHashMismatch { expected, received } => {
                write!(
                    f,
                    "quote terms hash mismatch: expected={expected}, received={received}"
                )
            }
            Self::PaymentNotSettled { session_id, status } => {
                write!(
                    f,
                    "payment not settled for session {session_id}: status={}",
                    status.as_str()
                )
            }
            Self::SessionExpired {
                session_id,
                expires_at,
            } => {
                write!(f, "session expired: {session_id} at {expires_at}")
            }
            Self::IllegalSessionTransition {
                session_id,
                from_state,
                event,
            } => {
                write!(
                    f,
                    "illegal session transition: {session_id} state={} event={event}",
                    from_state.as_str()
                )
            }
            Self::SessionNotRetryable { session_id, state } => {
                write!(
                    f,
                    "session not retryable: {session_id} state={}",
                    state.as_str()
                )
            }
            Self::SessionNotRefundable {
                session_id,
                state,
                payment_status,
            } => {
                write!(
                    f,
                    "session not refundable: {session_id} state={} payment_status={}",
                    state.as_str(),
                    payment_status.as_str()
                )
            }
            Self::SessionNotReadyForSpawn { session_id, state } => {
                write!(
                    f,
                    "session not ready for spawn: {session_id} state={}",
                    state.as_str()
                )
            }
            Self::ControllerInvariantViolation { canister_id } => {
                write!(f, "controller invariant violation for {canister_id}")
            }
            Self::AutomatonBootstrapVerificationFailed {
                canister_id,
                failures,
            } => {
                write!(
                    f,
                    "automaton bootstrap verification failed for {canister_id}: {}",
                    failures.join("; ")
                )
            }
            Self::AutomatonRuntimeNotFound { canister_id } => {
                write!(f, "automaton runtime not found: {canister_id}")
            }
            Self::MissingChildRuntimeConfig { field } => {
                write!(f, "missing child runtime config: {field}")
            }
            Self::InvalidChildRuntimeConfig { field, message } => {
                write!(f, "invalid child runtime config: {field}: {message}")
            }
            Self::ManagementCallFailed { method, message } => {
                write!(
                    f,
                    "management canister call failed: method={method} message={message}"
                )
            }
            Self::RpcRequestFailed {
                operation,
                endpoint,
                category,
                code,
                message,
            } => {
                write!(
                    f,
                    "rpc request failed: operation={operation} endpoint={endpoint} category={}",
                    category.as_str()
                )?;
                if let Some(code) = code {
                    write!(f, " code={code}")?;
                }
                write!(f, " message={message}")
            }
            Self::InvalidVersionCommit { value } => {
                write!(f, "invalid version commit: {value}")
            }
            Self::InvalidSha256 { value } => {
                write!(f, "invalid sha256 hex: {value}")
            }
            Self::ArtifactHashMismatch { expected, actual } => {
                write!(
                    f,
                    "artifact sha256 mismatch: expected={expected}, actual={actual}"
                )
            }
            Self::NoPendingArtifactUpload => write!(f, "no pending artifact upload"),
            Self::ArtifactUploadIncomplete { expected, received } => {
                write!(
                    f,
                    "artifact upload incomplete: expected={expected}, received={received}"
                )
            }
            Self::ArtifactUploadTooLarge {
                expected,
                attempted,
            } => {
                write!(
                    f,
                    "artifact upload too large: expected={expected}, attempted={attempted}"
                )
            }
            Self::InsufficientCyclesPool {
                available,
                required,
            } => {
                write!(
                    f,
                    "insufficient cycles pool: available={available}, required={required}"
                )
            }
            Self::InsufficientCyclesForOperation {
                operation,
                available,
                required,
            } => {
                write!(
                    f,
                    "insufficient cycles for operation {operation}: available={available}, required={required}"
                )
            }
        }
    }
}

impl Error for FactoryError {}

pub fn parse_amount(value: &str) -> Result<u128, FactoryError> {
    value
        .parse::<u128>()
        .map_err(|_| FactoryError::InvalidAmount {
            value: value.to_string(),
        })
}

pub fn amount_to_string(value: u128) -> String {
    value.to_string()
}

pub fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

pub fn validate_version_commit(value: &str) -> Result<(), FactoryError> {
    if is_lower_hex(value, 40) {
        return Ok(());
    }

    Err(FactoryError::InvalidVersionCommit {
        value: value.to_string(),
    })
}

pub fn validate_sha256_hex(value: &str) -> Result<(), FactoryError> {
    if is_lower_hex(value, 64) {
        return Ok(());
    }

    Err(FactoryError::InvalidSha256 {
        value: value.to_string(),
    })
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub fn hex_encode_prefixed(bytes: &[u8]) -> String {
    format!("0x{}", hex_encode(bytes))
}

pub fn hash_quote_terms(parts: &[&str]) -> String {
    const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;

    let mut hash = OFFSET_BASIS;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")
}

pub fn derive_claim_id(session_id: &str) -> String {
    use sha3::{Digest, Keccak256};
    let digest = Keccak256::digest(session_id.as_bytes());
    hex_encode_prefixed(&digest)
}
