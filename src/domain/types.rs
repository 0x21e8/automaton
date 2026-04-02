/// Canonical type definitions for the entire canister.
///
/// This module is the single source of truth for all shared data structures —
/// every other module imports from here.  The types are grouped into logical
/// sections:
///
/// - **Agent FSM** — the finite-state machine that controls the agent lifecycle
/// - **EVM** — event polling cursors and ingested events
/// - **Wallet** — EVM wallet balance snapshots and synchronisation config
/// - **Inference** — LLM provider configuration and turn I/O
/// - **Strategy** — templates, execution plans, ABI artefacts, and outcome stats
/// - **Memory** — persistent key/value knowledge base, rollups, and reflection lessons
/// - **Observability** — snapshots, storage metrics, cycle telemetry, and views
/// - **Scheduler** — jobs, leases, task configs, and survival tiers
use crate::timing;
use candid::{CandidType, Principal};
use serde::{Deserialize, Serialize};

/// Deserializes a `u64` from either a JSON number or a JSON string.
///
/// Nanosecond timestamps exceed JavaScript's `Number.MAX_SAFE_INTEGER`, so the
/// browser may send them as strings to preserve precision.  Candid and other
/// callers still send plain numbers, so both forms must be accepted.
fn deserialize_u64_from_string_or_number<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    struct U64OrStringVisitor;

    impl<'de> serde::de::Visitor<'de> for U64OrStringVisitor {
        type Value = u64;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a u64 or a string containing a u64")
        }

        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u64, E> {
            v.parse::<u64>().map_err(serde::de::Error::custom)
        }
    }

    deserializer.deserialize_any(U64OrStringVisitor)
}

// ── Agent FSM types ──────────────────────────────────────────────────────────

/// All stable states of the agent finite-state machine.
///
/// Transitions are driven by [`AgentEvent`] and recorded in the transition log.
/// - `Bootstrapping` — canister just installed; awaiting initial configuration.
/// - `Idle` — ready and waiting for the next timer tick or inbox message.
/// - `LoadingContext` — building the prompt from stable storage.
/// - `Inferring` — an LLM call is in progress.
/// - `ExecutingActions` — tool calls from the inference round are being run.
/// - `Persisting` — writing turn results and memory facts to stable storage.
/// - `Sleeping` — the agent requested a voluntary sleep; skips future turns.
/// - `Faulted` — a non-recoverable error occurred; requires `ResetFault`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum AgentState {
    Bootstrapping,
    Idle,
    LoadingContext,
    Inferring,
    ExecutingActions,
    Persisting,
    Sleeping,
    Faulted,
}

/// Events that drive FSM transitions in the agent state machine.
///
/// Each variant corresponds to a step completing (or failing) during an agent
/// turn.  The scheduler emits `TimerTick` every `BASE_TICK_SECS`; all other
/// events are produced internally by the agent or its subsystems.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub enum AgentEvent {
    TimerTick,
    EvmPollCompleted { new_events: u32, has_input: bool },
    ContextLoaded,
    InferenceCompleted,
    ActionsCompleted,
    PersistCompleted,
    SleepRequested,
    TurnFailed { reason: String },
    ResetFault,
}

/// Describes a rejected FSM transition — logged but never stored in steady state.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct TransitionError {
    pub from: AgentState,
    pub event: String,
    pub reason: String,
}

/// A single tool invocation produced by the LLM during an inference round.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    #[serde(default)]
    pub tool_call_id: Option<String>,
    pub tool: String,
    pub args_json: String,
}

/// Persisted record of a completed tool call, stored under `tools:{turn_id}`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct ToolCallRecord {
    pub turn_id: String,
    pub tool: String,
    pub args_json: String,
    pub output: String,
    pub success: bool,
    #[serde(default)]
    pub outcome: ToolCallOutcome,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<ToolFailureKind>,
}

/// Classification of a persisted tool-call record.
#[derive(
    CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default,
)]
pub enum ToolCallOutcome {
    /// Tool implementation actually ran and produced either success or failure.
    #[default]
    Executed,
    /// Call skipped because an identical autonomous call recently succeeded.
    SuppressedDedupe,
    /// Call skipped due to repeated-failure cooldown.
    SuppressedFailureCooldown,
    /// Call blocked by tool sequence policy.
    BlockedSequence,
}

/// Coarse classification of executed tool failures for autonomy handling.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ToolFailureKind {
    /// Tool call arguments or local validation failed before a real execution.
    MalformedInput,
    /// External dependency or outcall failed after a valid invocation.
    OutcallFailure,
    /// Non-malformed local/runtime failures.
    InternalFailure,
}

// ── Memory types ─────────────────────────────────────────────────────────────

/// A single entry in the persistent knowledge base.
///
/// Facts are keyed by a dotted namespace path (e.g. `"balance.eth"`,
/// `"config.chain_id"`) and stored in `MEMORY_FACTS_MAP`.  Critical keys
/// (those matching `is_critical_exact_memory_key`) are always included in the
/// agent's context prompt regardless of the fact limit.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct MemoryFact {
    pub key: String,
    pub value: String,
    pub created_at_ns: u64,
    pub updated_at_ns: u64,
    /// The turn that last wrote this fact — useful for audit.
    pub source_turn_id: String,
}

// ── EVM types ────────────────────────────────────────────────────────────────

/// Tracks the current EVM log-polling position for a given chain and contract.
///
/// The pair (`next_block`, `next_log_index`) is the resume point for the next
/// `eth_getLogs` call.  `consecutive_empty_polls` drives the exponential
/// backoff schedule defined in `timing::EMPTY_POLL_BACKOFF_SCHEDULE_SECS`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct EvmPollCursor {
    pub chain_id: u64,
    #[serde(default)]
    pub contract_address: Option<String>,
    /// The automaton's own address encoded as a 32-byte padded EVM topic,
    /// used to filter logs directed at this canister.
    #[serde(default)]
    pub automaton_address_topic: Option<String>,
    pub next_block: u64,
    pub next_log_index: u64,
    #[serde(default = "default_evm_confirmation_depth")]
    pub confirmation_depth: u64,
    #[serde(default)]
    pub last_poll_at_ns: u64,
    #[serde(default)]
    pub consecutive_empty_polls: u32,
}

impl Default for EvmPollCursor {
    fn default() -> Self {
        Self {
            chain_id: 8453,
            contract_address: None,
            automaton_address_topic: None,
            next_block: 0,
            next_log_index: 0,
            confirmation_depth: default_evm_confirmation_depth(),
            last_poll_at_ns: 0,
            consecutive_empty_polls: 0,
        }
    }
}

/// A decoded EVM log event received from the inbox contract.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct EvmEvent {
    pub tx_hash: String,
    pub chain_id: u64,
    pub block_number: u64,
    pub log_index: u64,
    /// Address that emitted the log (the inbox contract).
    pub source: String,
    /// Decoded message body extracted from the log data.
    pub payload: String,
}

/// The single active EVM steward identity authorized for signed commands.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StewardState {
    pub chain_id: u64,
    pub address: String,
    pub enabled: bool,
    #[serde(default)]
    pub last_used_at_ns: Option<u64>,
    #[serde(default)]
    pub principal: Option<Principal>,
}

/// Monotonic nonce cursor used for replay-safe steward proof verification.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct StewardNonceState {
    #[serde(default)]
    pub next_nonce: u64,
}

/// Public steward status view used by controller recovery and UI queries.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct StewardStatusView {
    #[serde(default)]
    pub active_steward: Option<StewardState>,
    #[serde(default)]
    pub next_nonce: u64,
}

/// Signed proof attached to steward command execution requests.
///
/// The signature authenticates a canonical payload containing:
/// `canister_id`, `chain_id`, `address`, `command_hash`, `nonce`, and
/// `expires_at_ns`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EvmStewardProof {
    pub canister_id: String,
    pub chain_id: u64,
    pub address: String,
    pub command_hash: String,
    pub nonce: u64,
    /// Accepts both JSON numbers and strings to avoid JavaScript `Number`
    /// precision loss (nanosecond timestamps exceed `Number.MAX_SAFE_INTEGER`).
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub expires_at_ns: u64,
    /// 65-byte ECDSA signature encoded as `0x{r}{s}{v}`.
    pub signature: String,
}

/// Runtime command variants executable by the active EVM steward.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StewardCommand {
    /// Executes no runtime mutation beyond auth bookkeeping (nonce + last-used).
    Noop,
    SetLoopEnabled {
        enabled: bool,
    },
    SetAutonomyToolDedupeEnabled {
        enabled: bool,
    },
    SetAutonomySuppressionConfig {
        config: AutonomySuppressionConfig,
    },
    SetInferenceProvider {
        provider: InferenceProvider,
    },
    SetInferenceModel {
        model: String,
    },
    SetOpenrouterBaseUrl {
        base_url: String,
    },
    SetOpenrouterApiKey {
        api_key: Option<String>,
    },
    ConfigureSearch {
        api_key: String,
        max_searches_per_turn: Option<u8>,
    },
    SetOpenrouterReasoningLevel {
        level: OpenRouterReasoningLevel,
    },
    SetInferenceProxyConfig {
        config: OpenRouterProxyWorkerConfig,
    },
    SetWelcomeMessage {
        message: String,
    },
    SetEvmRpcUrl {
        url: String,
    },
    SetEvmRpcFallbackUrl {
        url: Option<String>,
    },
    SetEvmRpcMaxResponseBytes {
        max_response_bytes: u64,
    },
    SetInboxContractAddress {
        address: Option<String>,
    },
    SendStewardMessage {
        sender: String,
        message: String,
    },
    /// Links or clears the ingress principal allowed to execute steward
    /// commands without EVM signatures.
    SetPrincipal {
        principal: Option<Principal>,
    },
    /// Rotates the single active steward identity.
    ///
    /// When `chain_id` or `address` changes, the steward nonce cursor is reset
    /// atomically with the steward update.
    UpdateSteward {
        chain_id: u64,
        address: String,
        enabled: bool,
    },
    SetEvmChainId {
        chain_id: u64,
    },
    SetEvmConfirmationDepth {
        confirmation_depth: u64,
    },
    DeriveAutomatonEvmAddress,
    SetHttpAllowedDomains {
        domains: Vec<String>,
    },
    UpdatePromptLayer {
        layer_id: u8,
        content: String,
    },
    PruneMemoryFacts {
        prefix: Option<String>,
        updated_before_ns: Option<u64>,
        limit: u32,
    },
    SetSchedulerEnabled {
        enabled: bool,
    },
    SetSchedulerLowCyclesMode {
        enabled: bool,
    },
    SetSchedulerBaseTickSecs {
        interval_secs: u64,
    },
    SetTaskIntervalSecs {
        kind: TaskKind,
        interval_secs: u64,
    },
    SetTaskEnabled {
        kind: TaskKind,
        enabled: bool,
    },
    SetRetentionConfig {
        config: RetentionConfig,
    },
    UpdateSoul {
        new_soul: String,
    },
    UpsertSkill {
        skill: SkillRecord,
    },
    RegisterStrategy {
        recipe_json: String,
    },
    /// Deprecated: use `RegisterStrategy` instead.
    IngestStrategyTemplate {
        template: StrategyTemplate,
    },
    /// Deprecated: use `RegisterStrategy` instead.
    IngestStrategyAbiArtifact {
        key: AbiArtifactKey,
        abi_json: String,
        source_ref: String,
        codehash: Option<String>,
        selector_assertions: Vec<AbiSelectorAssertion>,
    },
    /// Deprecated: use `RegisterStrategy` instead.
    ActivateStrategyTemplate {
        key: StrategyTemplateKey,
        reason: Option<String>,
    },
    DeprecateStrategyTemplate {
        key: StrategyTemplateKey,
        reason: Option<String>,
    },
    RevokeStrategyTemplate {
        key: StrategyTemplateKey,
        reason: Option<String>,
    },
    SetStrategyKillSwitch {
        key: StrategyTemplateKey,
        enabled: bool,
        reason: Option<String>,
    },
}

// ── Wallet types ─────────────────────────────────────────────────────────────

/// Coarse classification of a wallet balance reading.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum WalletBalanceStatus {
    /// No balance has ever been fetched.
    #[default]
    Unknown,
    /// The most recent reading is within the configured freshness window.
    Fresh,
    /// The most recent reading is older than the freshness window.
    Stale,
    /// The last sync attempt returned an error.
    Error,
}

/// Derived freshness assessment for a [`WalletBalanceSnapshot`].
///
/// Computed on demand by [`WalletBalanceSnapshot::derive_freshness`]; not stored.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WalletBalanceFreshness {
    pub age_secs: Option<u64>,
    pub freshness_window_secs: u64,
    pub is_stale: bool,
    pub status: WalletBalanceStatus,
}

/// Cached EVM wallet balances for the automaton's key-pair address.
///
/// Both ETH and USDC balances are stored as hex strings (wei / raw token units)
/// so that the agent can include them verbatim in prompts without lossy
/// floating-point conversion.  `last_synced_at_ns` drives freshness checks.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WalletBalanceSnapshot {
    #[serde(default)]
    pub eth_balance_wei_hex: Option<String>,
    #[serde(default)]
    pub usdc_balance_raw_hex: Option<String>,
    #[serde(default = "default_usdc_decimals")]
    pub usdc_decimals: u8,
    #[serde(default)]
    pub usdc_contract_address: Option<String>,
    #[serde(default)]
    pub last_synced_at_ns: Option<u64>,
    #[serde(default)]
    pub last_synced_block: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[allow(dead_code)]
impl WalletBalanceSnapshot {
    pub fn derive_freshness(
        &self,
        now_ns: u64,
        freshness_window_secs: u64,
    ) -> WalletBalanceFreshness {
        let age_secs = self
            .last_synced_at_ns
            .map(|synced_at| now_ns.saturating_sub(synced_at) / 1_000_000_000);
        let is_stale = age_secs
            .map(|age| age > freshness_window_secs)
            .unwrap_or(true);
        let status = if self.last_error.is_some() {
            WalletBalanceStatus::Error
        } else if self.last_synced_at_ns.is_none() {
            WalletBalanceStatus::Unknown
        } else if is_stale {
            WalletBalanceStatus::Stale
        } else {
            WalletBalanceStatus::Fresh
        };

        WalletBalanceFreshness {
            age_secs,
            freshness_window_secs,
            is_stale,
            status,
        }
    }
}

impl Default for WalletBalanceSnapshot {
    fn default() -> Self {
        Self {
            eth_balance_wei_hex: None,
            usdc_balance_raw_hex: None,
            usdc_decimals: default_usdc_decimals(),
            usdc_contract_address: None,
            last_synced_at_ns: None,
            last_synced_block: None,
            last_error: None,
        }
    }
}

/// Controls when and how the wallet balance sync job runs.
///
/// `low_cycles_interval_secs` must be ≥ `normal_interval_secs`; the longer
/// interval is used while the canister is in a low-cycles survival tier.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WalletBalanceSyncConfig {
    #[serde(default = "default_wallet_balance_sync_enabled")]
    pub enabled: bool,
    #[serde(default = "default_wallet_balance_sync_normal_interval_secs")]
    pub normal_interval_secs: u64,
    #[serde(default = "default_wallet_balance_sync_low_cycles_interval_secs")]
    pub low_cycles_interval_secs: u64,
    #[serde(default = "default_wallet_balance_sync_freshness_window_secs")]
    pub freshness_window_secs: u64,
    #[serde(default = "default_wallet_balance_sync_max_response_bytes")]
    pub max_response_bytes: u64,
    /// When `true`, the USDC contract address is discovered by querying the
    /// inbox contract rather than requiring manual configuration.
    #[serde(default = "default_wallet_balance_sync_discover_usdc_via_inbox")]
    pub discover_usdc_via_inbox: bool,
}

impl Default for WalletBalanceSyncConfig {
    fn default() -> Self {
        Self {
            enabled: default_wallet_balance_sync_enabled(),
            normal_interval_secs: default_wallet_balance_sync_normal_interval_secs(),
            low_cycles_interval_secs: default_wallet_balance_sync_low_cycles_interval_secs(),
            freshness_window_secs: default_wallet_balance_sync_freshness_window_secs(),
            max_response_bytes: default_wallet_balance_sync_max_response_bytes(),
            discover_usdc_via_inbox: default_wallet_balance_sync_discover_usdc_via_inbox(),
        }
    }
}

/// Read-only telemetry view of the wallet balance, returned by the
/// `wallet_balance_telemetry_view` canister query.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WalletBalanceTelemetryView {
    pub eth_balance_wei_hex: Option<String>,
    pub usdc_balance_raw_hex: Option<String>,
    pub usdc_decimals: u8,
    pub usdc_contract_address: Option<String>,
    pub last_synced_at_ns: Option<u64>,
    pub last_synced_block: Option<u64>,
    pub last_error: Option<String>,
    pub age_secs: Option<u64>,
    pub freshness_window_secs: u64,
    pub is_stale: bool,
    pub status: WalletBalanceStatus,
    /// `true` while the first successful balance sync has not yet completed.
    pub bootstrap_pending: bool,
}

impl WalletBalanceTelemetryView {
    pub fn from_snapshot(snapshot: &RuntimeSnapshot, now_ns: u64) -> Self {
        let freshness = snapshot
            .wallet_balance
            .derive_freshness(now_ns, snapshot.wallet_balance_sync.freshness_window_secs);
        Self {
            eth_balance_wei_hex: snapshot.wallet_balance.eth_balance_wei_hex.clone(),
            usdc_balance_raw_hex: snapshot.wallet_balance.usdc_balance_raw_hex.clone(),
            usdc_decimals: snapshot.wallet_balance.usdc_decimals,
            usdc_contract_address: snapshot.wallet_balance.usdc_contract_address.clone(),
            last_synced_at_ns: snapshot.wallet_balance.last_synced_at_ns,
            last_synced_block: snapshot.wallet_balance.last_synced_block,
            last_error: snapshot.wallet_balance.last_error.clone(),
            age_secs: freshness.age_secs,
            freshness_window_secs: freshness.freshness_window_secs,
            is_stale: freshness.is_stale,
            status: freshness.status,
            bootstrap_pending: snapshot.wallet_balance_bootstrap_pending,
        }
    }
}

/// Candid-serialisable projection of [`WalletBalanceSyncConfig`] for query responses.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WalletBalanceSyncConfigView {
    pub enabled: bool,
    pub normal_interval_secs: u64,
    pub low_cycles_interval_secs: u64,
    pub freshness_window_secs: u64,
    pub max_response_bytes: u64,
    pub discover_usdc_via_inbox: bool,
}

impl From<&WalletBalanceSyncConfig> for WalletBalanceSyncConfigView {
    fn from(config: &WalletBalanceSyncConfig) -> Self {
        Self {
            enabled: config.enabled,
            normal_interval_secs: config.normal_interval_secs,
            low_cycles_interval_secs: config.low_cycles_interval_secs,
            freshness_window_secs: config.freshness_window_secs,
            max_response_bytes: config.max_response_bytes,
            discover_usdc_via_inbox: config.discover_usdc_via_inbox,
        }
    }
}

/// Runtime policy for autonomous tool-call suppression behavior.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AutonomySuppressionConfig {
    #[serde(default = "default_autonomy_tool_dedupe_enabled")]
    pub tool_dedupe_enabled: bool,
    #[serde(default = "default_autonomy_dedupe_window_secs")]
    pub dedupe_window_secs: u64,
    #[serde(default = "default_autonomy_failure_repeat_window_secs")]
    pub failure_repeat_window_secs: u64,
    #[serde(default = "default_autonomy_failure_repeat_threshold")]
    pub failure_repeat_threshold: u32,
    #[serde(default = "default_autonomy_failure_cooldown_secs")]
    pub failure_cooldown_secs: u64,
}

impl Default for AutonomySuppressionConfig {
    fn default() -> Self {
        Self {
            tool_dedupe_enabled: default_autonomy_tool_dedupe_enabled(),
            dedupe_window_secs: default_autonomy_dedupe_window_secs(),
            failure_repeat_window_secs: default_autonomy_failure_repeat_window_secs(),
            failure_repeat_threshold: default_autonomy_failure_repeat_threshold(),
            failure_cooldown_secs: default_autonomy_failure_cooldown_secs(),
        }
    }
}

/// Durable governance policy that bounds autonomous economic behavior.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AutonomyPolicy {
    #[serde(default = "default_autonomy_policy_version")]
    pub version: u32,
    #[serde(default)]
    pub reserve_policy: ReservePolicy,
    #[serde(default)]
    pub risk_limits: RiskLimits,
    #[serde(default)]
    pub execution_authority: ExecutionAuthority,
    #[serde(default)]
    pub escalation_rules: EscalationRules,
    #[serde(default)]
    pub updated_at_ns: u64,
}

impl AutonomyPolicy {
    pub fn conservative_default(now_ns: u64) -> Self {
        Self {
            version: default_autonomy_policy_version(),
            reserve_policy: ReservePolicy::default(),
            risk_limits: RiskLimits::default(),
            execution_authority: ExecutionAuthority::default(),
            escalation_rules: EscalationRules::default(),
            updated_at_ns: now_ns,
        }
    }
}

impl Default for AutonomyPolicy {
    fn default() -> Self {
        Self::conservative_default(0)
    }
}

/// Protected-balance floors the automaton must preserve before deploying capital.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReservePolicy {
    #[serde(default = "default_min_cycles_runway_hours")]
    pub min_cycles_runway_hours: u64,
    #[serde(default = "default_min_inference_usdc_6dp")]
    pub min_inference_usdc_6dp: Option<u64>,
    #[serde(default = "default_min_gas_wei")]
    pub min_gas_wei: Option<u128>,
}

impl Default for ReservePolicy {
    fn default() -> Self {
        Self {
            min_cycles_runway_hours: default_min_cycles_runway_hours(),
            min_inference_usdc_6dp: default_min_inference_usdc_6dp(),
            min_gas_wei: default_min_gas_wei(),
        }
    }
}

/// Exposure and concentration bounds for autonomous execution.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RiskLimits {
    #[serde(default = "default_max_total_exposure_bps")]
    pub max_total_exposure_bps: u16,
    #[serde(default = "default_max_single_action_bps")]
    pub max_single_action_bps: u16,
    #[serde(default = "default_max_protocol_concentration_bps")]
    pub max_protocol_concentration_bps: u16,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_total_exposure_bps: default_max_total_exposure_bps(),
            max_single_action_bps: default_max_single_action_bps(),
            max_protocol_concentration_bps: default_max_protocol_concentration_bps(),
        }
    }
}

/// Hard execution authority limits for autonomous capital-touching actions.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExecutionAuthority {
    #[serde(default = "default_autonomous_execution_enabled")]
    pub autonomous_execution_enabled: bool,
    #[serde(default = "default_require_simulation_first")]
    pub require_simulation_first: bool,
    #[serde(default = "default_per_action_value_limit_wei")]
    pub per_action_value_limit_wei: Option<u128>,
}

impl Default for ExecutionAuthority {
    fn default() -> Self {
        Self {
            autonomous_execution_enabled: default_autonomous_execution_enabled(),
            require_simulation_first: default_require_simulation_first(),
            per_action_value_limit_wei: default_per_action_value_limit_wei(),
        }
    }
}

/// Rules for escalating or quarantining autonomous execution decisions.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EscalationRules {
    #[serde(default = "default_escalate_on_missing_policy")]
    pub escalate_on_missing_policy: bool,
    #[serde(default = "default_escalate_on_authority_exceeded")]
    pub escalate_on_authority_exceeded: bool,
    #[serde(default = "default_escalate_on_repeated_failure")]
    pub escalate_on_repeated_failure: bool,
    #[serde(default = "default_failure_quarantine_threshold")]
    pub failure_quarantine_threshold: u32,
}

impl Default for EscalationRules {
    fn default() -> Self {
        Self {
            escalate_on_missing_policy: default_escalate_on_missing_policy(),
            escalate_on_authority_exceeded: default_escalate_on_authority_exceeded(),
            escalate_on_repeated_failure: default_escalate_on_repeated_failure(),
            failure_quarantine_threshold: default_failure_quarantine_threshold(),
        }
    }
}

/// Configuration for the automatic cycle top-up feature.
///
/// When the canister's cycle balance drops below `auto_topup_cycle_threshold`,
/// the top-up flow bridges USDC from the EVM wallet through the 1Sec locker
/// and Kong swap to ICP, then converts ICP to cycles via the CMC.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CycleTopUpConfig {
    #[serde(default = "default_cycle_topup_enabled")]
    pub enabled: bool,
    #[serde(default = "default_auto_topup_cycle_threshold")]
    pub auto_topup_cycle_threshold: u128,
    #[serde(default)]
    pub usdc_contract_address: Option<String>,
    #[serde(default = "default_onesec_locker_address")]
    pub onesec_locker_address: String,
    #[serde(default = "default_onesec_canister_id")]
    pub onesec_canister_id: String,
    #[serde(default = "default_bridged_usdc_ledger_id")]
    pub bridged_usdc_ledger_id: String,
    #[serde(default = "default_kong_backend_id")]
    pub kong_backend_id: String,
    #[serde(default = "default_icp_ledger_id")]
    pub icp_ledger_id: String,
    #[serde(default = "default_cmc_id")]
    pub cmc_id: String,
    #[serde(default)]
    pub target_canister_id: Option<String>,
    #[serde(default = "default_min_usdc_reserve")]
    pub min_usdc_reserve: u64,
    #[serde(default = "default_max_usdc_per_topup")]
    pub max_usdc_per_topup: u64,
    #[serde(default = "default_max_slippage_pct")]
    pub max_slippage_pct: f64,
    #[serde(default = "default_max_bridge_polls")]
    pub max_bridge_polls: u8,
    #[serde(default = "default_lock_confirmations")]
    pub lock_confirmations: u8,
}

impl Default for CycleTopUpConfig {
    fn default() -> Self {
        Self {
            enabled: default_cycle_topup_enabled(),
            auto_topup_cycle_threshold: default_auto_topup_cycle_threshold(),
            usdc_contract_address: None,
            onesec_locker_address: default_onesec_locker_address(),
            onesec_canister_id: default_onesec_canister_id(),
            bridged_usdc_ledger_id: default_bridged_usdc_ledger_id(),
            kong_backend_id: default_kong_backend_id(),
            icp_ledger_id: default_icp_ledger_id(),
            cmc_id: default_cmc_id(),
            target_canister_id: None,
            min_usdc_reserve: default_min_usdc_reserve(),
            max_usdc_per_topup: default_max_usdc_per_topup(),
            max_slippage_pct: default_max_slippage_pct(),
            max_bridge_polls: default_max_bridge_polls(),
            lock_confirmations: default_lock_confirmations(),
        }
    }
}

// ── Runtime snapshot ─────────────────────────────────────────────────────────

/// The canonical in-memory view of all mutable canister state.
///
/// A single `RuntimeSnapshot` is serialised and stored under the key
/// `"runtime.snapshot"` in `RUNTIME_MAP`.  Every subsystem that needs to
/// mutate persistent state reads this snapshot, modifies it, and writes it
/// back atomically.
///
/// Fields are grouped loosely as: FSM state, EVM cursor, inference config,
/// wallet balance, and feature configs.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct RuntimeSnapshot {
    pub state: AgentState,
    pub turn_in_flight: bool,
    pub loop_enabled: bool,
    pub turn_counter: u64,
    pub last_turn_id: Option<String>,
    pub last_error: Option<String>,
    pub soul: String,
    pub evm_cursor: EvmPollCursor,
    pub event_seq: u64,
    pub transition_seq: u64,
    pub last_transition_at_ns: u64,
    #[serde(default)]
    pub inference_provider: InferenceProvider,
    #[serde(default = "default_inference_model")]
    pub inference_model: String,
    #[serde(default = "default_llm_canister_id")]
    pub llm_canister_id: String,
    #[serde(default)]
    pub openrouter_api_key: Option<String>,
    #[serde(default = "default_openrouter_base_url")]
    pub openrouter_base_url: String,
    #[serde(default = "default_openrouter_max_response_bytes")]
    pub openrouter_max_response_bytes: u64,
    #[serde(default)]
    pub openrouter_reasoning_level: OpenRouterReasoningLevel,
    #[serde(default)]
    pub openrouter_proxy: OpenRouterProxyWorkerConfig,
    #[serde(default)]
    pub installed_version_commit: Option<String>,
    #[serde(default)]
    pub ecdsa_key_name: String,
    #[serde(default)]
    pub evm_address: Option<String>,
    #[serde(default)]
    pub inbox_contract_address: Option<String>,
    #[serde(default)]
    pub active_steward: Option<StewardState>,
    #[serde(default)]
    pub steward_nonce: StewardNonceState,
    #[serde(default = "default_evm_rpc_url")]
    pub evm_rpc_url: String,
    #[serde(default)]
    pub evm_rpc_fallback_url: Option<String>,
    #[serde(default = "default_evm_rpc_max_response_bytes")]
    pub evm_rpc_max_response_bytes: u64,
    #[serde(default = "default_evm_bootstrap_lookback_blocks")]
    pub evm_bootstrap_lookback_blocks: u64,
    #[serde(default)]
    pub wallet_balance: WalletBalanceSnapshot,
    #[serde(default)]
    pub wallet_balance_sync: WalletBalanceSyncConfig,
    #[serde(default)]
    pub autonomy_suppression: AutonomySuppressionConfig,
    #[serde(default)]
    pub spawn_session_id: Option<String>,
    #[serde(default)]
    pub spawn_parent_id: Option<String>,
    #[serde(default)]
    pub spawn_risk: Option<u8>,
    #[serde(default)]
    pub spawn_strategies: Vec<String>,
    #[serde(default)]
    pub spawn_skills: Vec<String>,
    #[serde(default = "default_wallet_balance_bootstrap_pending")]
    pub wallet_balance_bootstrap_pending: bool,
    #[serde(default)]
    pub cycle_topup: CycleTopUpConfig,
    #[serde(default)]
    pub timing_telemetry: RuntimeTimingTelemetry,
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            state: AgentState::Bootstrapping,
            turn_in_flight: false,
            loop_enabled: true,
            turn_counter: 0,
            last_turn_id: None,
            last_error: None,
            soul: "ic-automaton-v1".to_string(),
            evm_cursor: EvmPollCursor::default(),
            event_seq: 0,
            transition_seq: 0,
            last_transition_at_ns: 0,
            inference_provider: InferenceProvider::default(),
            inference_model: default_inference_model(),
            llm_canister_id: default_llm_canister_id(),
            openrouter_api_key: None,
            openrouter_base_url: default_openrouter_base_url(),
            openrouter_max_response_bytes: default_openrouter_max_response_bytes(),
            openrouter_reasoning_level: OpenRouterReasoningLevel::default(),
            openrouter_proxy: OpenRouterProxyWorkerConfig::default(),
            installed_version_commit: None,
            ecdsa_key_name: String::new(),
            evm_address: None,
            inbox_contract_address: None,
            active_steward: None,
            steward_nonce: StewardNonceState::default(),
            evm_rpc_url: default_evm_rpc_url(),
            evm_rpc_fallback_url: None,
            evm_rpc_max_response_bytes: default_evm_rpc_max_response_bytes(),
            evm_bootstrap_lookback_blocks: default_evm_bootstrap_lookback_blocks(),
            wallet_balance: WalletBalanceSnapshot::default(),
            wallet_balance_sync: WalletBalanceSyncConfig::default(),
            autonomy_suppression: AutonomySuppressionConfig::default(),
            spawn_session_id: None,
            spawn_parent_id: None,
            spawn_risk: None,
            spawn_strategies: Vec::new(),
            spawn_skills: Vec::new(),
            wallet_balance_bootstrap_pending: default_wallet_balance_bootstrap_pending(),
            cycle_topup: CycleTopUpConfig::default(),
            timing_telemetry: RuntimeTimingTelemetry::default(),
        }
    }
}

/// Durable log entry for every FSM state transition, stored in `TRANSITION_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct TransitionLogRecord {
    pub id: String,
    pub turn_id: String,
    pub from_state: AgentState,
    pub to_state: AgentState,
    pub event: String,
    pub error: Option<String>,
    pub occurred_at_ns: u64,
}

/// Reason why a multi-round inference loop stopped before receiving a final answer.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum ContinuationStopReason {
    #[default]
    None,
    MaxRounds,
    MaxDuration,
    InferenceError,
    MaxToolCalls,
}

/// Durable record of a completed agent turn, stored in `TURN_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct TurnRecord {
    pub id: String,
    pub created_at_ns: u64,
    #[serde(default)]
    pub finished_at_ns: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    pub state_from: AgentState,
    pub state_to: AgentState,
    pub source_events: u32,
    pub tool_call_count: u32,
    pub input_summary: String,
    #[serde(default)]
    pub inner_dialogue: Option<String>,
    #[serde(default)]
    pub inference_round_count: u32,
    #[serde(default)]
    pub continuation_stop_reason: ContinuationStopReason,
    pub error: Option<String>,
}

/// Durable exposure record used for protocol concentration and reconciliation.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActiveExposure {
    pub strategy_id: String,
    pub protocol: String,
    pub chain_id: u64,
    pub asset_symbol: String,
    #[serde(default)]
    pub notional_wei: Option<u128>,
    pub updated_at_ns: u64,
}

/// Durable quarantine record for strategies blocked after repeated failures.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StrategyQuarantine {
    pub strategy_id: String,
    pub reason: String,
    pub failure_count: u32,
    pub quarantined_at_ns: u64,
    #[serde(default)]
    pub release_after_ns: Option<u64>,
}

/// Trigger that caused an autonomous economic decision turn.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DecisionTrigger {
    ScheduledReview,
    InboxMessage,
    LowRunway,
    PositionMaintenance,
    RecoveryFollowUp,
}

impl DecisionTrigger {
    pub fn as_wire_name(&self) -> &'static str {
        match self {
            Self::ScheduledReview => "ScheduledReview",
            Self::InboxMessage => "InboxMessage",
            Self::LowRunway => "LowRunway",
            Self::PositionMaintenance => "PositionMaintenance",
            Self::RecoveryFollowUp => "RecoveryFollowUp",
        }
    }

    pub fn inference_input_marker(&self) -> &'static str {
        match self {
            Self::ScheduledReview => "scheduled_review",
            Self::InboxMessage => "inbox_message",
            Self::LowRunway => "low_runway",
            Self::PositionMaintenance => "position_maintenance",
            Self::RecoveryFollowUp => "recovery_follow_up",
        }
    }
}

/// Escalation category emitted when the automaton cannot proceed autonomously.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum EscalationClass {
    MissingPolicy {
        what: String,
    },
    OutOfAuthority {
        what: String,
    },
    CapabilityGap {
        what: String,
    },
    SafetyConflict {
        what: String,
    },
    RepeatedFailure {
        strategy: String,
        failure_count: u32,
    },
}

/// Terminal persisted decision outcome for an autonomous economic turn.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DecisionOutcome {
    Executed { action_summary: String },
    Simulated { action_summary: String },
    NoOp { reason: String },
    Deferred { reason: String },
    Escalated { gap: EscalationClass },
}

/// Durable audit record for recent autonomous economic decisions.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DecisionRecord {
    pub turn_id: String,
    pub timestamp_ns: u64,
    pub trigger: DecisionTrigger,
    pub outcome: DecisionOutcome,
    pub policy_version: u32,
    pub candidates_summary: String,
    pub explanation: String,
}

/// Machine-readable terminal outcome required from autonomous economic turns.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DecisionEnvelopeOutcome {
    Executed { action_summary: String },
    Simulated { action_summary: String },
    NoOp { reason: String },
    Deferred { reason: String },
    Escalated { gap: EscalationClass },
}

impl From<DecisionEnvelopeOutcome> for DecisionOutcome {
    fn from(outcome: DecisionEnvelopeOutcome) -> Self {
        match outcome {
            DecisionEnvelopeOutcome::Executed { action_summary } => {
                Self::Executed { action_summary }
            }
            DecisionEnvelopeOutcome::Simulated { action_summary } => {
                Self::Simulated { action_summary }
            }
            DecisionEnvelopeOutcome::NoOp { reason } => Self::NoOp { reason },
            DecisionEnvelopeOutcome::Deferred { reason } => Self::Deferred { reason },
            DecisionEnvelopeOutcome::Escalated { gap } => Self::Escalated { gap },
        }
    }
}

/// Final JSON contract for autonomous economic turns.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AutonomyDecisionEnvelope {
    pub trigger: DecisionTrigger,
    pub candidates_summary: String,
    pub outcome: DecisionEnvelopeOutcome,
    pub explanation: String,
}

/// Small operator-facing view of the last exposure reconciliation run.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct ExposureReconciliationStatus {
    #[serde(default)]
    pub last_attempted_at_ns: Option<u64>,
    #[serde(default)]
    pub last_succeeded_at_ns: Option<u64>,
    #[serde(default)]
    pub repaired_exposures: u32,
    #[serde(default)]
    pub recreated_exposures: u32,
    #[serde(default)]
    pub closed_exposures: u32,
    #[serde(default)]
    pub drift_reason: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// Whether an inter-canister call is a query (read-only) or update (state-mutating).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum CanisterCallType {
    Query,
    Update,
}

/// A single canister method that a skill permits the agent to call.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CanisterCallPermission {
    /// Target canister principal as text (e.g. `"um5iw-rqaaa-aaaaq-qaaba-cai"`).
    pub canister_id: String,
    /// Method name (e.g. `"icrc1_balance_of"`).
    pub method: String,
    /// Whether this is a read-only query or a state-mutating update.
    pub call_type: CanisterCallType,
}

/// A named, optionally-mutable capability that can be enabled or disabled at runtime.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SkillRecord {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub enabled: bool,
    pub mutable: bool,
    /// Inter-canister calls this skill grants the agent permission to make.
    ///
    /// The `canister_call` tool checks this list across all active skills before
    /// executing any call.  An empty vec means the skill grants no IC call permissions.
    #[serde(default)]
    pub allowed_canister_calls: Vec<CanisterCallPermission>,
}

// ── Prompt layer types ───────────────────────────────────────────────────────

/// A versioned, mutable segment of the system prompt, stored in `PROMPT_LAYER_MAP`.
///
/// Immutable layers are defined at compile time in `src/prompt.rs`; mutable
/// layers (layer IDs in `MUTABLE_LAYER_MIN_ID..=MUTABLE_LAYER_MAX_ID`) can be
/// updated at runtime by a controller or by the agent itself.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PromptLayer {
    pub layer_id: u8,
    pub content: String,
    pub updated_at_ns: u64,
    pub updated_by_turn: String,
    pub version: u32,
}

/// Read-only view of a prompt layer, returned by `list_prompt_layers`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PromptLayerView {
    pub layer_id: u8,
    pub is_mutable: bool,
    pub content: String,
    pub updated_at_ns: Option<u64>,
    pub updated_by_turn: Option<String>,
    pub version: Option<u32>,
}

/// A single request–response exchange between a sender and the agent.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationEntry {
    pub inbox_message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbox_message_id: Option<String>,
    pub sender_body: String,
    pub agent_reply: String,
    pub turn_id: String,
    pub timestamp_ns: u64,
}

/// All conversation entries for a given sender, stored in `CONVERSATION_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationLog {
    pub sender: String,
    pub entries: Vec<ConversationEntry>,
    pub last_activity_ns: u64,
}

/// Lightweight summary of a conversation, used in the observability snapshot.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConversationSummary {
    pub sender: String,
    pub last_activity_ns: u64,
    pub entry_count: u32,
}

/// Aggregated inbox/outbox statistics for one sender over a 24-hour window,
/// produced during retention maintenance and stored in `SESSION_SUMMARY_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub sender: String,
    pub window_start_ns: u64,
    pub window_end_ns: u64,
    pub source_count: u32,
    pub inbox_message_count: u32,
    pub outbox_message_count: u32,
    pub inbox_preview: String,
    pub outbox_preview: String,
    pub generated_at_ns: u64,
}

/// Aggregated turn and tool-call statistics over a 24-hour window,
/// produced during retention maintenance and stored in `TURN_WINDOW_SUMMARY_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TurnWindowSummary {
    pub window_start_ns: u64,
    pub window_end_ns: u64,
    pub source_count: u32,
    pub turn_count: u32,
    pub transition_count: u32,
    pub tool_call_count: u32,
    pub succeeded_turn_count: u32,
    pub failed_turn_count: u32,
    pub tool_success_count: u32,
    pub tool_failure_count: u32,
    pub top_errors: Vec<String>,
    pub generated_at_ns: u64,
}

/// A condensed snapshot of all memory facts in a namespace, produced by the
/// retention maintenance pass and stored in `MEMORY_ROLLUP_MAP`.
///
/// Rollups compress facts that have not changed recently into a single
/// `canonical_value` string so that the agent context prompt stays bounded.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MemoryRollup {
    pub namespace: String,
    pub window_start_ns: u64,
    pub window_end_ns: u64,
    pub source_count: u32,
    pub source_keys: Vec<String>,
    pub canonical_value: String,
    pub generated_at_ns: u64,
}

/// High-level source of a persisted reflection-memory lesson.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum ReflectionOrigin {
    #[default]
    Autonomy,
    ExternalInput,
    Maintenance,
}

/// Durable bounded lesson derived from repeated degraded tool execution.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReflectionMemoryRecord {
    pub key: String,
    pub tool: String,
    pub subject: String,
    pub error_class: String,
    pub what_failed: String,
    #[serde(default)]
    pub what_worked: Option<String>,
    pub degraded_turn_count: u32,
    pub repeat_count: u32,
    pub last_failed_at_ns: u64,
    pub last_failed_turn_id: String,
    #[serde(default)]
    pub last_worked_at_ns: Option<u64>,
    #[serde(default)]
    pub last_worked_turn_id: Option<String>,
    #[serde(default)]
    pub last_origin: ReflectionOrigin,
    pub updated_at_ns: u64,
}

// ── Strategy types ───────────────────────────────────────────────────────────

/// Composite key that uniquely identifies a strategy template.
///
/// The four fields together form a namespaced identifier:
/// `{protocol}:{primitive}@chain{chain_id}#{template_id}`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StrategyTemplateKey {
    pub protocol: String,
    pub primitive: String,
    pub chain_id: u64,
    pub template_id: String,
}

/// Semantic version for a strategy template or ABI artefact.
///
/// Retained for Candid backward compatibility even though strategy entities are
/// now keyed without versions.
#[derive(
    CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default,
)]
pub struct TemplateVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

/// Lifecycle state of a strategy template.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum TemplateStatus {
    /// Not yet approved for execution.
    #[default]
    Draft,
    Active,
    Deprecated,
    /// Permanently disabled; cannot be re-activated.
    Revoked,
}

/// Binds a logical contract role (e.g. `"vault"`) to a verified on-chain address.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ContractRoleBinding {
    pub role: String,
    pub address: String,
    pub source_ref: String,
    #[serde(default)]
    pub codehash: Option<String>,
}

/// Recursive ABI type specification (mirrors the JSON ABI format).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AbiTypeSpec {
    #[serde(default)]
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub components: Vec<AbiTypeSpec>,
}

/// A single ABI function entry, including its 4-byte selector for verification.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AbiFunctionSpec {
    pub role: String,
    pub name: String,
    pub selector_hex: String,
    pub inputs: Vec<AbiTypeSpec>,
    pub outputs: Vec<AbiTypeSpec>,
    pub state_mutability: String,
}

/// A named action within a strategy template — a sequence of EVM calls with
/// guard conditions.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActionSpec {
    pub action_id: String,
    pub call_sequence: Vec<AbiFunctionSpec>,
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
    pub risk_checks: Vec<String>,
}

/// A strategy template defining the actions, contract roles, and
/// constraints for an on-chain DeFi operation (e.g. a Uniswap swap).
///
/// Templates are stored in `STRATEGY_TEMPLATE_MAP` and referenced by agents
/// when building execution plans.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StrategyTemplate {
    pub key: StrategyTemplateKey,
    pub status: TemplateStatus,
    pub contract_roles: Vec<ContractRoleBinding>,
    pub actions: Vec<ActionSpec>,
    pub constraints_json: String,
    pub created_at_ns: u64,
    pub updated_at_ns: u64,
}

/// Composite key for an ABI artefact: protocol + chain + contract role.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AbiArtifactKey {
    pub protocol: String,
    pub chain_id: u64,
    pub role: String,
}

/// A versioned ABI artefact stored in `ABI_ARTIFACT_MAP`, providing the raw
/// JSON ABI and pre-parsed function specs for selector validation.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AbiArtifact {
    pub key: AbiArtifactKey,
    pub source_ref: String,
    #[serde(default)]
    pub codehash: Option<String>,
    pub abi_json: String,
    pub functions: Vec<AbiFunctionSpec>,
    pub created_at_ns: u64,
    pub updated_at_ns: u64,
}

/// A compile-time / test assertion that a human-readable function signature
/// produces the expected 4-byte selector.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AbiSelectorAssertion {
    pub signature: String,
    pub selector_hex: String,
}

/// An agent's intention to execute a specific action of a strategy template
/// with user-supplied typed parameters.  Validated before producing an
/// [`ExecutionPlan`].
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct StrategyExecutionIntent {
    pub key: StrategyTemplateKey,
    pub action_id: String,
    /// JSON object whose fields match the action's typed parameter schema.
    pub typed_params_json: String,
}

/// A single EVM call within an execution plan — fully resolved with target
/// address, value, and ABI-encoded calldata.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct StrategyExecutionCall {
    pub role: String,
    pub to: String,
    pub value_wei: String,
    pub data: String,
}

/// A fully materialised sequence of EVM calls ready for threshold-signing and
/// broadcasting.  Produced by expanding a [`StrategyExecutionIntent`] against
/// the matching [`StrategyTemplate`] and ABI artefacts.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ExecutionPlan {
    pub key: StrategyTemplateKey,
    pub action_id: String,
    pub calls: Vec<StrategyExecutionCall>,
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
}

/// The pipeline stage at which a validation finding was produced.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ValidationLayer {
    Schema,
    Address,
    Policy,
    /// Deprecated: retained for serialized data compatibility.
    #[deprecated(
        note = "retained for serialization compatibility; no longer emitted by the validator"
    )]
    Preflight,
    Postcondition,
}

/// A single issue found during strategy validation.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ValidationFinding {
    pub layer: ValidationLayer,
    pub code: String,
    pub message: String,
    /// `true` means the same inputs will always produce this finding.
    pub deterministic: bool,
}

/// The aggregate result of validating a strategy execution intent.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ValidationReport {
    pub passed: bool,
    pub findings: Vec<ValidationFinding>,
    pub checked_at_ns: u64,
}

/// Classification of a strategy execution result used for confidence accounting.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StrategyOutcomeKind {
    Success,
    /// The failure is reproducible — same inputs always fail (e.g. slippage too high).
    DeterministicFailure,
    /// The failure was transient (e.g. RPC timeout, gas spike).
    NondeterministicFailure,
}

/// A single strategy execution result submitted for outcome accounting.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StrategyOutcomeEvent {
    pub key: StrategyTemplateKey,
    pub action_id: String,
    pub outcome: StrategyOutcomeKind,
    pub tx_hash: Option<String>,
    pub error: Option<String>,
    pub observed_at_ns: u64,
}

/// Running execution statistics for a strategy template, stored in
/// `STRATEGY_OUTCOME_STATS_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StrategyOutcomeStats {
    pub key: StrategyTemplateKey,
    pub total_runs: u64,
    pub success_runs: u64,
    pub deterministic_failures: u64,
    pub nondeterministic_failures: u64,
    #[serde(default)]
    pub deterministic_failure_streak: u32,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_tx_hash: Option<String>,
    #[serde(default)]
    pub last_observed_at_ns: Option<u64>,
}

/// Records whether a strategy template is enabled for execution.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TemplateActivationState {
    pub key: StrategyTemplateKey,
    pub enabled: bool,
    pub updated_at_ns: u64,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Records whether a strategy template has been permanently revoked.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TemplateRevocationState {
    pub key: StrategyTemplateKey,
    pub revoked: bool,
    pub updated_at_ns: u64,
    #[serde(default)]
    pub reason: Option<String>,
}

/// An emergency circuit-breaker that halts all executions of a strategy,
/// stored in `STRATEGY_KILL_SWITCH_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StrategyKillSwitchState {
    pub key: StrategyTemplateKey,
    pub enabled: bool,
    pub updated_at_ns: u64,
    #[serde(default)]
    pub reason: Option<String>,
}

// ── Observability types ──────────────────────────────────────────────────────

/// A lightweight projection of [`RuntimeSnapshot`] suitable for Candid queries.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct RuntimeView {
    pub state: AgentState,
    pub turn_in_flight: bool,
    pub loop_enabled: bool,
    pub turn_counter: u64,
    pub last_turn_id: Option<String>,
    pub last_error: Option<String>,
    pub soul: String,
    pub evm_chain_id: u64,
    pub evm_next_block: u64,
    pub evm_next_log_index: u64,
    pub last_transition_at_ns: u64,
    pub inference_provider: InferenceProvider,
    pub inference_model: String,
    #[serde(default)]
    pub timing_telemetry: RuntimeTimingTelemetry,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct SpawnBootstrapView {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub risk: Option<u8>,
    #[serde(default)]
    pub strategies: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub version_commit: Option<String>,
}

/// Read-only view of the EVM polling configuration and cursor, returned by queries.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct EvmRouteStateView {
    pub chain_id: u64,
    pub automaton_evm_address: Option<String>,
    pub inbox_contract_address: Option<String>,
    pub automaton_address_topic: Option<String>,
    pub next_block: u64,
    pub next_log_index: u64,
    pub confirmation_depth: u64,
    pub last_poll_at_ns: u64,
    pub consecutive_empty_polls: u32,
}

impl From<&RuntimeSnapshot> for RuntimeView {
    fn from(snapshot: &RuntimeSnapshot) -> Self {
        Self {
            state: snapshot.state.clone(),
            turn_in_flight: snapshot.turn_in_flight,
            loop_enabled: snapshot.loop_enabled,
            turn_counter: snapshot.turn_counter,
            last_turn_id: snapshot.last_turn_id.clone(),
            last_error: snapshot.last_error.clone(),
            soul: snapshot.soul.clone(),
            evm_chain_id: snapshot.evm_cursor.chain_id,
            evm_next_block: snapshot.evm_cursor.next_block,
            evm_next_log_index: snapshot.evm_cursor.next_log_index,
            last_transition_at_ns: snapshot.last_transition_at_ns,
            inference_provider: snapshot.inference_provider.clone(),
            inference_model: snapshot.inference_model.clone(),
            timing_telemetry: snapshot.timing_telemetry.clone(),
        }
    }
}

impl From<&RuntimeSnapshot> for SpawnBootstrapView {
    fn from(snapshot: &RuntimeSnapshot) -> Self {
        Self {
            session_id: snapshot.spawn_session_id.clone(),
            parent_id: snapshot.spawn_parent_id.clone(),
            risk: snapshot.spawn_risk,
            strategies: snapshot.spawn_strategies.clone(),
            skills: snapshot.spawn_skills.clone(),
            version_commit: snapshot.installed_version_commit.clone(),
        }
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct OutcallTimingStats {
    #[serde(default)]
    pub total_calls: u64,
    #[serde(default)]
    pub failure_calls: u64,
    #[serde(default)]
    pub timeout_failures: u64,
    #[serde(default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub max_duration_ms: u64,
    #[serde(default)]
    pub last_duration_ms: Option<u64>,
    #[serde(default)]
    pub last_started_at_ns: Option<u64>,
    #[serde(default)]
    pub last_finished_at_ns: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct RuntimeTimingTelemetry {
    #[serde(default)]
    pub last_turn_duration_ms: Option<u64>,
    #[serde(default)]
    pub max_turn_duration_ms: u64,
    #[serde(default)]
    pub turns_over_budget: u64,
    #[serde(default)]
    pub inference_outcall: OutcallTimingStats,
    #[serde(default)]
    pub http_fetch_outcall: OutcallTimingStats,
}

impl From<&RuntimeSnapshot> for EvmRouteStateView {
    fn from(snapshot: &RuntimeSnapshot) -> Self {
        Self {
            chain_id: snapshot.evm_cursor.chain_id,
            automaton_evm_address: snapshot.evm_address.clone(),
            inbox_contract_address: snapshot.inbox_contract_address.clone(),
            automaton_address_topic: snapshot.evm_cursor.automaton_address_topic.clone(),
            next_block: snapshot.evm_cursor.next_block,
            next_log_index: snapshot.evm_cursor.next_log_index,
            confirmation_depth: snapshot.evm_cursor.confirmation_depth,
            last_poll_at_ns: snapshot.evm_cursor.last_poll_at_ns,
            consecutive_empty_polls: snapshot.evm_cursor.consecutive_empty_polls,
        }
    }
}

/// A point-in-time snapshot of the entire observable state of the canister,
/// returned by the `observability_snapshot` query.
///
/// Building this snapshot also updates the cycle-balance and storage-growth
/// sample ring buffers, so calling it has a small write side effect.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct ObservabilitySnapshot {
    pub captured_at_ns: u64,
    pub runtime: RuntimeView,
    pub scheduler: SchedulerRuntime,
    #[serde(default)]
    pub storage_growth: StorageGrowthMetrics,
    pub inbox_stats: InboxStats,
    pub inbox_messages: Vec<InboxMessage>,
    pub outbox_stats: OutboxStats,
    pub outbox_messages: Vec<OutboxMessage>,
    pub prompt_layers: Vec<PromptLayerView>,
    pub conversation_summaries: Vec<ConversationSummary>,
    #[serde(default)]
    pub session_summaries: Vec<SessionSummary>,
    #[serde(default)]
    pub turn_window_summaries: Vec<TurnWindowSummary>,
    #[serde(default)]
    pub memory_rollups: Vec<MemoryRollup>,
    #[serde(default)]
    pub cycles: CycleTelemetry,
    pub recent_turns: Vec<TurnRecord>,
    pub recent_transitions: Vec<TransitionLogRecord>,
    pub recent_jobs: Vec<ScheduledJob>,
}

/// Coarse pressure classification derived from the highest utilisation
/// percentage across all bounded stable-memory collections.
///
/// Thresholds: Elevated ≥ 70 %, High ≥ 85 %, Critical ≥ 95 %.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default)]
pub enum StoragePressureLevel {
    #[default]
    Normal,
    Elevated,
    High,
    Critical,
}

/// Detailed stable-memory utilisation metrics included in every
/// [`ObservabilitySnapshot`].
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default)]
pub struct StorageGrowthMetrics {
    pub runtime_map_entries: u64,
    pub transition_map_entries: u64,
    pub turn_map_entries: u64,
    pub tool_map_entries: u64,
    pub job_map_entries: u64,
    pub job_queue_map_entries: u64,
    pub dedupe_map_entries: u64,
    pub inbox_map_entries: u64,
    pub inbox_pending_queue_entries: u64,
    pub inbox_staged_queue_entries: u64,
    pub outbox_map_entries: u64,
    #[serde(default)]
    pub session_summary_entries: u64,
    #[serde(default)]
    pub session_summary_limit: u64,
    #[serde(default)]
    pub turn_window_summary_entries: u64,
    #[serde(default)]
    pub turn_window_summary_limit: u64,
    #[serde(default)]
    pub memory_rollup_entries: u64,
    #[serde(default)]
    pub memory_rollup_limit: u64,
    pub memory_fact_entries: u64,
    pub memory_fact_limit: u64,
    #[serde(default)]
    pub session_summary_utilization_percent: u8,
    #[serde(default)]
    pub turn_window_summary_utilization_percent: u8,
    #[serde(default)]
    pub memory_rollup_utilization_percent: u8,
    #[serde(default)]
    pub memory_fact_utilization_percent: u8,
    #[serde(default)]
    pub memory_fact_retention_max_age_secs: u64,
    #[serde(default)]
    pub memory_fact_prune_batch_size: u32,
    #[serde(default)]
    pub last_deleted_memory_facts: u32,
    #[serde(default)]
    pub near_limit: bool,
    #[serde(default)]
    pub pressure_level: StoragePressureLevel,
    #[serde(default)]
    pub pressure_warnings: Vec<String>,
    #[serde(default)]
    pub tracked_entry_count: u64,
    #[serde(default)]
    pub tracked_entries_delta_per_hour: Option<i64>,
    #[serde(default)]
    pub trend_window_seconds: u64,
    #[serde(default)]
    pub trend_sample_count: u32,
    pub retention_progress_percent: u8,
    pub summarization_progress_percent: u8,
    #[serde(default)]
    pub heap_memory_mb: f64,
    #[serde(default)]
    pub stable_memory_mb: f64,
}

/// Configuration for the periodic retention maintenance job.
///
/// Controls maximum ages and record counts for each stored collection.
/// Written to stable storage and readable / updatable via `set_retention_config`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RetentionConfig {
    #[serde(default = "default_jobs_max_age_secs")]
    pub jobs_max_age_secs: u64,
    #[serde(default = "default_jobs_max_records")]
    pub jobs_max_records: u64,
    #[serde(default = "default_dedupe_max_age_secs")]
    pub dedupe_max_age_secs: u64,
    #[serde(default = "default_turns_max_age_secs")]
    pub turns_max_age_secs: u64,
    #[serde(default = "default_transitions_max_age_secs")]
    pub transitions_max_age_secs: u64,
    #[serde(default = "default_tools_max_age_secs")]
    pub tools_max_age_secs: u64,
    #[serde(default = "default_inbox_max_age_secs")]
    pub inbox_max_age_secs: u64,
    #[serde(default = "default_outbox_max_age_secs")]
    pub outbox_max_age_secs: u64,
    #[serde(default = "default_memory_facts_max_age_secs")]
    pub memory_facts_max_age_secs: u64,
    #[serde(default = "default_memory_facts_prune_batch_size")]
    pub memory_facts_prune_batch_size: u32,
    #[serde(default = "default_maintenance_batch_size")]
    pub maintenance_batch_size: u32,
    #[serde(default = "default_maintenance_interval_secs")]
    pub maintenance_interval_secs: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            jobs_max_age_secs: default_jobs_max_age_secs(),
            jobs_max_records: default_jobs_max_records(),
            dedupe_max_age_secs: default_dedupe_max_age_secs(),
            turns_max_age_secs: default_turns_max_age_secs(),
            transitions_max_age_secs: default_transitions_max_age_secs(),
            tools_max_age_secs: default_tools_max_age_secs(),
            inbox_max_age_secs: default_inbox_max_age_secs(),
            outbox_max_age_secs: default_outbox_max_age_secs(),
            memory_facts_max_age_secs: default_memory_facts_max_age_secs(),
            memory_facts_prune_batch_size: default_memory_facts_prune_batch_size(),
            maintenance_batch_size: default_maintenance_batch_size(),
            maintenance_interval_secs: default_maintenance_interval_secs(),
        }
    }
}

/// Persisted progress state for the incremental retention maintenance job.
///
/// Scan cursors allow the job to resume a partially-completed pass without
/// re-scanning already-processed keys, keeping each tick's work bounded.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionMaintenanceRuntime {
    #[serde(default)]
    pub next_run_after_ns: u64,
    #[serde(default)]
    pub job_scan_cursor: Option<String>,
    #[serde(default)]
    pub dedupe_scan_cursor: Option<String>,
    #[serde(default)]
    pub inbox_scan_cursor: Option<String>,
    #[serde(default)]
    pub outbox_scan_cursor: Option<String>,
    #[serde(default)]
    pub turn_scan_cursor: Option<String>,
    #[serde(default)]
    pub transition_scan_cursor: Option<String>,
    #[serde(default)]
    pub last_started_ns: Option<u64>,
    #[serde(default)]
    pub last_finished_ns: Option<u64>,
    #[serde(default)]
    pub last_deleted_jobs: u32,
    #[serde(default)]
    pub last_deleted_dedupe: u32,
    #[serde(default)]
    pub last_deleted_inbox: u32,
    #[serde(default)]
    pub last_deleted_outbox: u32,
    #[serde(default)]
    pub last_deleted_turns: u32,
    #[serde(default)]
    pub last_deleted_transitions: u32,
    #[serde(default)]
    pub last_deleted_tools: u32,
    #[serde(default)]
    pub last_generated_session_summaries: u32,
    #[serde(default)]
    pub last_generated_turn_window_summaries: u32,
    #[serde(default)]
    pub last_generated_memory_rollups: u32,
    #[serde(default)]
    pub last_deleted_memory_facts: u32,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub retention_progress_percent: u8,
    #[serde(default)]
    pub summarization_progress_percent: u8,
}

/// Cycle balance telemetry derived from a ring-buffer of balance samples.
///
/// Burn-rate projections and estimated freeze time are `None` until enough
/// samples have been collected (at least two distinct timestamps).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default)]
pub struct CycleTelemetry {
    pub total_cycles: u128,
    pub liquid_cycles: u128,
    pub freezing_threshold_cycles: u128,
    pub moving_window_seconds: u64,
    pub window_duration_seconds: u64,
    pub window_sample_count: u32,
    pub burn_rate_cycles_per_hour: Option<u128>,
    pub burn_rate_cycles_per_day: Option<u128>,
    pub burn_rate_usd_per_hour: Option<f64>,
    pub burn_rate_usd_per_day: Option<f64>,
    pub estimated_seconds_until_freezing_threshold: Option<u64>,
    pub estimated_freeze_time_ns: Option<u64>,
    pub usd_per_trillion_cycles: f64,
}

// ── Inference types ──────────────────────────────────────────────────────────

/// The LLM backend used for inference.
///
/// - `IcLlm` — on-chain IC LLM canister (no API key required).
/// - `OpenRouter` — external HTTP gateway; requires `openrouter_api_key`.
/// - `OpenRouterProxyWorker` — async Cloudflare worker proxy callback path.
#[derive(
    CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default,
)]
pub enum InferenceProvider {
    #[default]
    IcLlm,
    OpenRouter,
    OpenRouterProxyWorker,
}

/// Runtime reasoning-effort level used for OpenRouter requests.
#[derive(
    CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default,
)]
pub enum OpenRouterReasoningLevel {
    #[default]
    Default,
    Low,
    Medium,
    High,
}

/// User-facing steward model variants used by the signed HTTP command plane.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StewardModelVariant {
    Flash,
    Mini,
}

impl StewardModelVariant {
    pub fn inference_model(self) -> &'static str {
        match self {
            Self::Flash => "google/gemini-3-flash-preview",
            Self::Mini => "openai/gpt-4o-mini",
        }
    }
}

/// User-facing steward reasoning variants used by the signed HTTP command plane.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StewardReasoningVariant {
    Default,
    Low,
    Medium,
    High,
}

impl StewardReasoningVariant {
    pub fn reasoning_level(self) -> OpenRouterReasoningLevel {
        match self {
            Self::Default => OpenRouterReasoningLevel::Default,
            Self::Low => OpenRouterReasoningLevel::Low,
            Self::Medium => OpenRouterReasoningLevel::Medium,
            Self::High => OpenRouterReasoningLevel::High,
        }
    }
}

/// Runtime configuration for the OpenRouter async worker proxy inference path.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct OpenRouterProxyWorkerConfig {
    #[serde(default)]
    pub worker_base_url: String,
    #[serde(default)]
    pub trusted_callback_principal: Option<Principal>,
}

/// Read-only view of the current inference configuration, returned by queries.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct InferenceConfigView {
    pub provider: InferenceProvider,
    pub model: String,
    pub openrouter_base_url: String,
    pub openrouter_has_api_key: bool,
    pub openrouter_max_response_bytes: u64,
    #[serde(default)]
    pub openrouter_reasoning_level: OpenRouterReasoningLevel,
    #[serde(default)]
    pub openrouter_proxy_worker_base_url: Option<String>,
    #[serde(default)]
    pub openrouter_proxy_trusted_callback_principal: Option<String>,
}

impl From<&RuntimeSnapshot> for InferenceConfigView {
    fn from(snapshot: &RuntimeSnapshot) -> Self {
        let worker_base_url = snapshot.openrouter_proxy.worker_base_url.trim().to_string();
        Self {
            provider: snapshot.inference_provider.clone(),
            model: snapshot.inference_model.clone(),
            openrouter_base_url: snapshot.openrouter_base_url.clone(),
            openrouter_has_api_key: snapshot
                .openrouter_api_key
                .as_ref()
                .map(|key| !key.trim().is_empty())
                .unwrap_or(false),
            openrouter_max_response_bytes: snapshot.openrouter_max_response_bytes,
            openrouter_reasoning_level: snapshot.openrouter_reasoning_level,
            openrouter_proxy_worker_base_url: if worker_base_url.is_empty() {
                None
            } else {
                Some(worker_base_url)
            },
            openrouter_proxy_trusted_callback_principal: snapshot
                .openrouter_proxy
                .trusted_callback_principal
                .as_ref()
                .map(Principal::to_text),
        }
    }
}

/// Worker callback payload accepted by `submit_inference_result`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct InferenceProxyResultPayload {
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

/// Update-call arguments for the async worker callback endpoint.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct SubmitInferenceResultArgs {
    pub job_id: String,
    pub turn_id: String,
    pub completed_at_ns: u64,
    #[serde(default)]
    pub result: Option<InferenceProxyResultPayload>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Durable pending async inference submission tracked until callback arrives.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PendingInferenceProxyJob {
    pub job_id: String,
    pub turn_id: String,
    pub submitted_at_ns: u64,
    pub model: String,
}

/// Durable async-wait state for a staged inbox message awaiting proxy completion.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InboxProxyWaitState {
    pub inbox_message_id: String,
    #[serde(default)]
    pub pending_job_id: Option<String>,
    pub submitted_turn_id: String,
    pub started_at_ns: u64,
    pub wait_attempts: u32,
}

/// Durable callback record for an async inference job.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct InferenceProxyCallbackRecord {
    pub job_id: String,
    pub turn_id: String,
    pub completed_at_ns: u64,
    pub accepted_at_ns: u64,
    pub caller_principal: String,
    #[serde(default)]
    pub result: Option<InferenceProxyResultPayload>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Result of attempting to apply an async callback payload.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum InferenceProxyCallbackApply {
    Accepted,
    Duplicate,
}

/// Public-safe status of async inference proxy state.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InferenceProxyStatusView {
    pub worker_base_url: Option<String>,
    pub trusted_callback_principal: Option<String>,
    pub pending_jobs: u64,
    pub completed_jobs: u64,
    pub oldest_pending_age_secs: Option<u64>,
    pub submit_accepted: u64,
    pub submit_failed: u64,
    pub callback_accepted: u64,
    pub callback_rejected: u64,
    pub callback_duplicates: u64,
    pub callback_auth_failures: u64,
    pub resumed_callbacks: u64,
    pub expired_jobs: u64,
}

/// The assembled prompt and context passed to the LLM at the start of a turn.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct InferenceInput {
    pub input: String,
    pub context_snippet: String,
    pub turn_id: String,
    #[serde(default)]
    pub proxy_resume_job_id: Option<String>,
    #[serde(default)]
    pub allow_global_proxy_callback_resume: bool,
}

/// Lifecycle state of an inbox message.
///
/// - `Pending` — arrived but not yet staged for processing.
/// - `Staged` — moved to the staged queue; awaiting consumption by a turn.
/// - `Consumed` — processed by a completed turn.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum InboxMessageStatus {
    Pending,
    Staged,
    Consumed,
}

/// Origin of a message ingested into the canister inbox pipeline.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InboxMessageSource {
    #[default]
    EvmInbox,
    StewardDirect,
}

impl InboxMessageSource {
    pub fn as_tag(self) -> &'static str {
        match self {
            Self::EvmInbox => "evm_inbox",
            Self::StewardDirect => "steward_direct",
        }
    }
}

/// A message posted to the canister inbox, stored in `INBOX_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct InboxMessage {
    pub id: String,
    pub seq: u64,
    pub body: String,
    pub posted_at_ns: u64,
    pub posted_by: String,
    #[serde(default)]
    pub source: InboxMessageSource,
    pub status: InboxMessageStatus,
    pub staged_at_ns: Option<u64>,
    pub consumed_at_ns: Option<u64>,
}

/// Aggregate counts across all inbox messages by status.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default)]
pub struct InboxStats {
    pub total_messages: u64,
    pub pending_count: u64,
    pub staged_count: u64,
    pub consumed_count: u64,
}

/// A reply or autonomous output written by the agent, stored in `OUTBOX_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct OutboxMessage {
    pub id: String,
    pub seq: u64,
    pub turn_id: String,
    pub body: String,
    pub created_at_ns: u64,
    /// The inbox message IDs that triggered this reply (empty for autonomous output).
    pub source_inbox_ids: Vec<String>,
}

/// Aggregate count of all outbox messages.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default)]
pub struct OutboxStats {
    pub total_messages: u64,
}

// ── Scheduler types ──────────────────────────────────────────────────────────

/// The type of recurring work the scheduler dispatches.
///
/// Priority order (lower = higher priority): `AgentTurn` → `PollInbox` →
/// `CheckCycles` → `TopUpCycles` → `Reconcile`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TaskKind {
    AgentTurn,
    PollInbox,
    CheckCycles,
    TopUpCycles,
    Reconcile,
}

impl std::str::FromStr for TaskKind {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "AgentTurn" => Ok(Self::AgentTurn),
            "PollInbox" => Ok(Self::PollInbox),
            "CheckCycles" => Ok(Self::CheckCycles),
            "TopUpCycles" => Ok(Self::TopUpCycles),
            "Reconcile" => Ok(Self::Reconcile),
            _ => Err(()),
        }
    }
}

impl TaskKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AgentTurn => "AgentTurn",
            Self::PollInbox => "PollInbox",
            Self::CheckCycles => "CheckCycles",
            Self::TopUpCycles => "TopUpCycles",
            Self::Reconcile => "Reconcile",
        }
    }

    pub const fn default_priority(&self) -> u8 {
        match self {
            Self::AgentTurn => 0,
            Self::PollInbox => 1,
            Self::CheckCycles => 2,
            Self::TopUpCycles => 3,
            Self::Reconcile => 4,
        }
    }

    pub const fn essential(&self) -> bool {
        match self {
            Self::AgentTurn => true,
            Self::PollInbox => true,
            Self::CheckCycles => true,
            Self::TopUpCycles => true,
            Self::Reconcile => false,
        }
    }

    pub const fn default_interval_secs(&self) -> u64 {
        timing::DEFAULT_TASK_INTERVAL_SECS
    }

    pub const fn all() -> &'static [Self] {
        static TASK_KINDS: [TaskKind; 5] = [
            TaskKind::AgentTurn,
            TaskKind::PollInbox,
            TaskKind::CheckCycles,
            TaskKind::TopUpCycles,
            TaskKind::Reconcile,
        ];
        &TASK_KINDS
    }
}

/// Cycle-based survival tier that governs which operations are permitted.
///
/// Tier escalation / recovery follows a hysteresis rule: the scheduler requires
/// `SURVIVAL_TIER_RECOVERY_CHECKS_REQUIRED` consecutive `Normal` observations
/// before downgrading from an elevated tier.
///
/// | Tier | Blocked operations |
/// |------|--------------------|
/// | `Normal` | none |
/// | `LowCycles` | `ThresholdSign`, `EvmBroadcast` |
/// | `Critical` | all |
/// | `OutOfCycles` | all |
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum SurvivalTier {
    #[default]
    Normal,
    LowCycles,
    Critical,
    OutOfCycles,
}

/// Execution lane for a scheduled job.
///
/// - `Mutating` — acquires the global mutating lease; at most one active at a time.
/// - `ReadOnly` — runs concurrently with other read-only jobs.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TaskLane {
    Mutating,
    ReadOnly,
}

/// Categories of outbound operations subject to per-tier blocking and backoff.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum SurvivalOperationClass {
    Inference,
    EvmPoll,
    EvmBroadcast,
    ThresholdSign,
    InterCanisterCall,
}

impl SurvivalOperationClass {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Inference => "inference",
            Self::EvmPoll => "evm_poll",
            Self::EvmBroadcast => "evm_broadcast",
            Self::ThresholdSign => "threshold_sign",
            Self::InterCanisterCall => "inter_canister_call",
        }
    }
}

/// The class of operation that experienced a recoverable failure.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum RecoveryOperation {
    WalletBalanceSync,
    EvmPoll,
    Inference,
    ToolExecution,
    #[default]
    Unknown,
}

/// Classification of an HTTP outcall failure used by the recovery policy.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum OutcallFailureKind {
    ResponseTooLarge,
    Timeout,
    Transport,
    RateLimited,
    UpstreamUnavailable,
    InvalidRequest,
    InvalidResponse,
    RejectedByPolicy,
    Unknown,
}

/// Details of a failed HTTP outcall, including optional retry guidance.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OutcallFailure {
    pub kind: OutcallFailureKind,
    #[serde(default)]
    pub retry_after_secs: Option<u64>,
    #[serde(default)]
    pub observed_response_bytes: Option<u64>,
}

/// Classification of a non-outcall operation failure.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum OperationFailureKind {
    BlockedBySurvivalPolicy,
    MissingConfiguration,
    InvalidConfiguration,
    InsufficientCycles,
    Unauthorized,
    Deterministic,
    Unknown,
}

/// A non-outcall operation failure with its kind.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OperationFailure {
    pub kind: OperationFailureKind,
}

/// A failure that may require recovery — either an outcall or an operation failure.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum RecoveryFailure {
    Outcall(OutcallFailure),
    Operation(OperationFailure),
}

/// The action the recovery policy recommends after analysing a failure.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum RecoveryPolicyAction {
    Skip,
    RetryImmediate,
    Backoff,
    TuneResponseLimit,
    #[default]
    EscalateFault,
}

/// The specific reason that drove a recovery policy decision.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum RecoveryDecisionReason {
    ResponseTooLarge,
    ResponseLimitAlreadyMaxed,
    TransientOutcallFailure,
    OutcallRateLimited,
    NonRetriableOutcallFailure,
    SurvivalPolicyBlocked,
    InsufficientCycles,
    NonRetriableOperationFailure,
    UnknownFailure,
}

/// Before/after byte counts for a response-limit tuning step.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResponseLimitAdjustment {
    pub from_bytes: u64,
    pub to_bytes: u64,
}

/// The complete recommendation produced by the recovery policy, including the
/// recommended action and any backoff or response-limit adjustment.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RecoveryDecision {
    pub action: RecoveryPolicyAction,
    pub reason: RecoveryDecisionReason,
    #[serde(default)]
    pub backoff_secs: Option<u64>,
    #[serde(default)]
    pub response_limit_adjustment: Option<ResponseLimitAdjustment>,
}

/// Adaptive response-size limits used to recover from `ResponseTooLarge` errors.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResponseLimitPolicy {
    pub current_bytes: u64,
    pub min_bytes: u64,
    pub max_bytes: u64,
    /// Each successful tune step multiplies `current_bytes` by this value.
    #[serde(default = "default_response_limit_tune_multiplier")]
    pub tune_multiplier: u64,
}

impl Default for ResponseLimitPolicy {
    fn default() -> Self {
        Self {
            current_bytes: 256,
            min_bytes: 256,
            max_bytes: 4 * 1024,
            tune_multiplier: default_response_limit_tune_multiplier(),
        }
    }
}

/// Per-operation recovery state including consecutive failure count and
/// exponential backoff configuration.
#[allow(dead_code)]
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RecoveryContext {
    #[serde(default)]
    pub operation: RecoveryOperation,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default = "default_recovery_backoff_base_secs")]
    pub backoff_base_secs: u64,
    #[serde(default = "default_recovery_backoff_max_secs")]
    pub backoff_max_secs: u64,
    #[serde(default)]
    pub response_limit: Option<ResponseLimitPolicy>,
}

impl Default for RecoveryContext {
    fn default() -> Self {
        Self {
            operation: RecoveryOperation::Unknown,
            consecutive_failures: 0,
            backoff_base_secs: default_recovery_backoff_base_secs(),
            backoff_max_secs: default_recovery_backoff_max_secs(),
            response_limit: None,
        }
    }
}

impl TaskLane {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Mutating => "mutating",
            Self::ReadOnly => "read_only",
        }
    }
}

/// Per-task scheduling configuration stored in `TASK_CONFIG_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct TaskScheduleConfig {
    pub kind: TaskKind,
    pub enabled: bool,
    /// Essential tasks are preserved in low-cycles mode; non-essential are skipped.
    pub essential: bool,
    pub interval_secs: u64,
    pub priority: u8,
    pub max_backoff_secs: u64,
}

impl TaskScheduleConfig {
    pub fn default_for(kind: &TaskKind) -> Self {
        let enabled = !matches!(kind, TaskKind::TopUpCycles);
        Self {
            kind: kind.clone(),
            enabled,
            essential: kind.essential(),
            interval_secs: kind.default_interval_secs(),
            priority: kind.default_priority(),
            max_backoff_secs: 120,
        }
    }
}

/// Mutable runtime state for a task kind, stored in `TASK_RUNTIME_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct TaskScheduleRuntime {
    pub kind: TaskKind,
    pub next_due_ns: u64,
    pub backoff_until_ns: Option<u64>,
    pub consecutive_failures: u32,
    /// The job ID of the most recently enqueued job for this task, if still active.
    pub pending_job_id: Option<String>,
    pub last_started_ns: Option<u64>,
    pub last_finished_ns: Option<u64>,
    pub last_error: Option<String>,
}

/// Current execution status of a [`ScheduledJob`].
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    InFlight,
    Succeeded,
    Failed,
    TimedOut,
    Skipped,
}

/// A single scheduler job entry stored in `JOB_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct ScheduledJob {
    pub id: String,
    pub kind: TaskKind,
    pub lane: TaskLane,
    pub dedupe_key: String,
    pub priority: u8,
    pub created_at_ns: u64,
    pub scheduled_for_ns: u64,
    pub started_at_ns: Option<u64>,
    pub finished_at_ns: Option<u64>,
    pub status: JobStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub last_error: Option<String>,
}

impl ScheduledJob {
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            JobStatus::Succeeded | JobStatus::Failed | JobStatus::TimedOut | JobStatus::Skipped
        )
    }
}

/// An exclusive execution lease held for a mutating job.
///
/// Only one `Mutating` lease may be active at a time.  The scheduler checks
/// `expires_at_ns` on every tick; expired leases are recovered automatically.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct SchedulerLease {
    pub lane: TaskLane,
    pub job_id: String,
    pub acquired_at_ns: u64,
    pub expires_at_ns: u64,
}

/// The top-level scheduler runtime state, stored in `SCHEDULER_RUNTIME_MAP`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug)]
pub struct SchedulerRuntime {
    pub enabled: bool,
    pub paused_reason: Option<String>,
    pub low_cycles_mode: bool,
    #[serde(default)]
    pub survival_tier: SurvivalTier,
    #[serde(default)]
    pub survival_tier_recovery_checks: u32,
    pub next_job_seq: u64,
    pub active_mutating_lease: Option<SchedulerLease>,
    pub last_tick_started_ns: u64,
    pub last_tick_finished_ns: u64,
    pub last_tick_error: Option<String>,
}

impl Default for SchedulerRuntime {
    fn default() -> Self {
        Self {
            enabled: true,
            paused_reason: None,
            low_cycles_mode: false,
            survival_tier: SurvivalTier::Normal,
            survival_tier_recovery_checks: 0,
            next_job_seq: 0,
            active_mutating_lease: None,
            last_tick_started_ns: 0,
            last_tick_finished_ns: 0,
            last_tick_error: None,
        }
    }
}

fn default_inference_model() -> String {
    "llama3.1:8b".to_string()
}

fn default_llm_canister_id() -> String {
    "w36hm-eqaaa-aaaal-qr76a-cai".to_string()
}

fn default_openrouter_base_url() -> String {
    "https://openrouter.ai/api/v1".to_string()
}

fn default_openrouter_max_response_bytes() -> u64 {
    64 * 1024
}

fn default_evm_rpc_url() -> String {
    "https://base.publicnode.com".to_string()
}

fn default_evm_rpc_max_response_bytes() -> u64 {
    64 * 1024
}

fn default_evm_bootstrap_lookback_blocks() -> u64 {
    1_000
}

fn default_evm_confirmation_depth() -> u64 {
    6
}

fn default_usdc_decimals() -> u8 {
    6
}

fn default_wallet_balance_sync_enabled() -> bool {
    true
}

fn default_wallet_balance_sync_normal_interval_secs() -> u64 {
    300
}

fn default_wallet_balance_sync_low_cycles_interval_secs() -> u64 {
    900
}

fn default_wallet_balance_sync_freshness_window_secs() -> u64 {
    600
}

fn default_wallet_balance_sync_max_response_bytes() -> u64 {
    1_024
}

fn default_wallet_balance_sync_discover_usdc_via_inbox() -> bool {
    true
}

fn default_wallet_balance_bootstrap_pending() -> bool {
    true
}

fn default_autonomy_policy_version() -> u32 {
    1
}

fn default_autonomy_tool_dedupe_enabled() -> bool {
    true
}

fn default_min_cycles_runway_hours() -> u64 {
    72
}

fn default_min_inference_usdc_6dp() -> Option<u64> {
    Some(10_000_000)
}

fn default_min_gas_wei() -> Option<u128> {
    Some(3_000_000_000_000_000)
}

fn default_max_total_exposure_bps() -> u16 {
    3_000
}

fn default_max_single_action_bps() -> u16 {
    1_000
}

fn default_max_protocol_concentration_bps() -> u16 {
    1_500
}

fn default_autonomous_execution_enabled() -> bool {
    true
}

fn default_require_simulation_first() -> bool {
    true
}

fn default_per_action_value_limit_wei() -> Option<u128> {
    Some(50_000_000_000_000_000)
}

fn default_escalate_on_missing_policy() -> bool {
    true
}

fn default_escalate_on_authority_exceeded() -> bool {
    true
}

fn default_escalate_on_repeated_failure() -> bool {
    true
}

fn default_failure_quarantine_threshold() -> u32 {
    3
}

fn default_autonomy_dedupe_window_secs() -> u64 {
    timing::AUTONOMY_DEDUPE_WINDOW_SECS
}

fn default_autonomy_failure_repeat_window_secs() -> u64 {
    timing::AUTONOMY_FAILURE_REPEAT_WINDOW_SECS
}

fn default_autonomy_failure_repeat_threshold() -> u32 {
    timing::AUTONOMY_FAILURE_REPEAT_THRESHOLD
}

fn default_autonomy_failure_cooldown_secs() -> u64 {
    timing::AUTONOMY_FAILURE_COOLDOWN_SECS
}

fn default_cycle_topup_enabled() -> bool {
    true
}

fn default_auto_topup_cycle_threshold() -> u128 {
    2_000_000_000_000
}

fn default_onesec_locker_address() -> String {
    "0xae2351b15cff68b5863c6690dca58dce383bf45a".to_string()
}

fn default_onesec_canister_id() -> String {
    "5okwm-giaaa-aaaar-qbn6a-cai".to_string()
}

fn default_bridged_usdc_ledger_id() -> String {
    "53nhb-haaaa-aaaar-qbn5q-cai".to_string()
}

fn default_kong_backend_id() -> String {
    "2ipq2-uqaaa-aaaar-qailq-cai".to_string()
}

fn default_icp_ledger_id() -> String {
    "ryjl3-tyaaa-aaaaa-aaaba-cai".to_string()
}

fn default_cmc_id() -> String {
    "rkp4c-7iaaa-aaaaa-aaaca-cai".to_string()
}

fn default_min_usdc_reserve() -> u64 {
    10_000_000
}

fn default_max_usdc_per_topup() -> u64 {
    50_000_000
}

fn default_max_slippage_pct() -> f64 {
    5.0
}

fn default_max_bridge_polls() -> u8 {
    60
}

fn default_lock_confirmations() -> u8 {
    12
}

#[allow(dead_code)]
fn default_recovery_backoff_base_secs() -> u64 {
    5
}

#[allow(dead_code)]
fn default_recovery_backoff_max_secs() -> u64 {
    300
}

#[allow(dead_code)]
fn default_response_limit_tune_multiplier() -> u64 {
    2
}

fn default_jobs_max_age_secs() -> u64 {
    14 * 24 * 60 * 60
}

fn default_jobs_max_records() -> u64 {
    60_000
}

fn default_dedupe_max_age_secs() -> u64 {
    3 * 24 * 60 * 60
}

fn default_turns_max_age_secs() -> u64 {
    7 * 24 * 60 * 60
}

fn default_transitions_max_age_secs() -> u64 {
    7 * 24 * 60 * 60
}

fn default_tools_max_age_secs() -> u64 {
    7 * 24 * 60 * 60
}

fn default_inbox_max_age_secs() -> u64 {
    14 * 24 * 60 * 60
}

fn default_outbox_max_age_secs() -> u64 {
    14 * 24 * 60 * 60
}

fn default_memory_facts_max_age_secs() -> u64 {
    3 * 24 * 60 * 60
}

fn default_memory_facts_prune_batch_size() -> u32 {
    25
}

fn default_maintenance_batch_size() -> u32 {
    120
}

fn default_maintenance_interval_secs() -> u64 {
    10 * 60
}

#[cfg(test)]
mod tests {
    use super::{
        AutonomyPolicy, InferenceConfigView, InferenceProvider, MemoryRollup,
        OpenRouterProxyWorkerConfig, OpenRouterReasoningLevel, RecoveryContext,
        ReflectionMemoryRecord, ReflectionOrigin, ResponseLimitPolicy, RetentionConfig,
        RetentionMaintenanceRuntime, RuntimeSnapshot, SessionSummary, TurnWindowSummary,
        WalletBalanceSnapshot, WalletBalanceStatus, WalletBalanceSyncConfig,
        WalletBalanceSyncConfigView, WalletBalanceTelemetryView,
    };
    use candid::Principal;

    #[test]
    fn autonomy_policy_defaults_are_conservative() {
        let now_ns = 123_456_789;
        let policy = AutonomyPolicy::conservative_default(now_ns);

        assert_eq!(policy.version, 1);
        assert_eq!(policy.updated_at_ns, now_ns);
        assert_eq!(policy.reserve_policy.min_cycles_runway_hours, 72);
        assert_eq!(
            policy.reserve_policy.min_inference_usdc_6dp,
            Some(10_000_000)
        );
        assert_eq!(
            policy.reserve_policy.min_gas_wei,
            Some(3_000_000_000_000_000)
        );
        assert_eq!(policy.risk_limits.max_total_exposure_bps, 3_000);
        assert_eq!(policy.risk_limits.max_single_action_bps, 1_000);
        assert_eq!(policy.risk_limits.max_protocol_concentration_bps, 1_500);
        assert!(policy.execution_authority.autonomous_execution_enabled);
        assert!(policy.execution_authority.require_simulation_first);
        assert_eq!(
            policy.execution_authority.per_action_value_limit_wei,
            Some(50_000_000_000_000_000)
        );
        assert!(policy.escalation_rules.escalate_on_missing_policy);
        assert!(policy.escalation_rules.escalate_on_authority_exceeded);
        assert!(policy.escalation_rules.escalate_on_repeated_failure);
        assert_eq!(policy.escalation_rules.failure_quarantine_threshold, 3);

        let default_policy = AutonomyPolicy::default();
        assert_eq!(default_policy.version, policy.version);
        assert_eq!(default_policy.updated_at_ns, 0);
        assert_eq!(default_policy.reserve_policy, policy.reserve_policy);
        assert_eq!(default_policy.risk_limits, policy.risk_limits);
        assert_eq!(
            default_policy.execution_authority,
            policy.execution_authority
        );
        assert_eq!(default_policy.escalation_rules, policy.escalation_rules);
    }

    #[test]
    fn wallet_balance_defaults_match_locked_spec() {
        let config = WalletBalanceSyncConfig::default();
        assert!(config.enabled);
        assert_eq!(config.normal_interval_secs, 300);
        assert_eq!(config.low_cycles_interval_secs, 900);
        assert_eq!(config.freshness_window_secs, 600);
        assert_eq!(config.max_response_bytes, 1_024);
        assert!(config.discover_usdc_via_inbox);

        let snapshot = WalletBalanceSnapshot::default();
        assert_eq!(snapshot.usdc_decimals, 6);
        assert!(snapshot.eth_balance_wei_hex.is_none());
        assert!(snapshot.usdc_balance_raw_hex.is_none());
        assert!(snapshot.last_synced_at_ns.is_none());
        assert!(snapshot.last_error.is_none());
    }

    #[test]
    fn wallet_balance_freshness_derives_status_with_error_precedence() {
        let now_ns = 1_000_000_000_000;
        let freshness_window_secs = 600;

        let unknown =
            WalletBalanceSnapshot::default().derive_freshness(now_ns, freshness_window_secs);
        assert_eq!(unknown.status, WalletBalanceStatus::Unknown);
        assert!(unknown.is_stale);
        assert_eq!(unknown.age_secs, None);

        let fresh = WalletBalanceSnapshot {
            last_synced_at_ns: Some(now_ns.saturating_sub(120 * 1_000_000_000)),
            ..WalletBalanceSnapshot::default()
        }
        .derive_freshness(now_ns, freshness_window_secs);
        assert_eq!(fresh.status, WalletBalanceStatus::Fresh);
        assert!(!fresh.is_stale);
        assert_eq!(fresh.age_secs, Some(120));

        let stale = WalletBalanceSnapshot {
            last_synced_at_ns: Some(now_ns.saturating_sub(601 * 1_000_000_000)),
            ..WalletBalanceSnapshot::default()
        }
        .derive_freshness(now_ns, freshness_window_secs);
        assert_eq!(stale.status, WalletBalanceStatus::Stale);
        assert!(stale.is_stale);
        assert_eq!(stale.age_secs, Some(601));

        let errored = WalletBalanceSnapshot {
            last_synced_at_ns: Some(now_ns.saturating_sub(10 * 1_000_000_000)),
            last_error: Some("rpc unavailable".to_string()),
            ..WalletBalanceSnapshot::default()
        }
        .derive_freshness(now_ns, freshness_window_secs);
        assert_eq!(errored.status, WalletBalanceStatus::Error);
    }

    #[test]
    fn runtime_snapshot_defaults_bootstrap_pending_for_wallet_sync() {
        let snapshot = RuntimeSnapshot::default();
        assert!(snapshot.wallet_balance_bootstrap_pending);
        assert_eq!(snapshot.evm_bootstrap_lookback_blocks, 1_000);
        assert!(snapshot.cycle_topup.enabled);
        assert!(snapshot.active_steward.is_none());
        assert_eq!(snapshot.steward_nonce.next_nonce, 0);
        assert!(snapshot.openrouter_proxy.worker_base_url.is_empty());
        assert!(snapshot
            .openrouter_proxy
            .trusted_callback_principal
            .is_none());
        assert_eq!(
            snapshot.cycle_topup.auto_topup_cycle_threshold,
            2_000_000_000_000
        );
        assert_eq!(
            snapshot.cycle_topup.onesec_locker_address,
            "0xae2351b15cff68b5863c6690dca58dce383bf45a"
        );
        assert_eq!(
            snapshot.openrouter_reasoning_level,
            OpenRouterReasoningLevel::Default
        );
    }

    #[test]
    fn wallet_balance_telemetry_view_derives_freshness_from_snapshot() {
        let now_ns: u64 = 2_000_000_000_000;
        let snapshot = RuntimeSnapshot {
            wallet_balance: WalletBalanceSnapshot {
                eth_balance_wei_hex: Some("0x1".to_string()),
                usdc_balance_raw_hex: Some("0x2a".to_string()),
                usdc_decimals: 6,
                usdc_contract_address: Some(
                    "0x3333333333333333333333333333333333333333".to_string(),
                ),
                last_synced_at_ns: Some(now_ns.saturating_sub(601 * 1_000_000_000)),
                last_synced_block: Some(123),
                last_error: None,
            },
            wallet_balance_sync: WalletBalanceSyncConfig {
                freshness_window_secs: 600,
                ..WalletBalanceSyncConfig::default()
            },
            wallet_balance_bootstrap_pending: true,
            ..RuntimeSnapshot::default()
        };

        let view = WalletBalanceTelemetryView::from_snapshot(&snapshot, now_ns);
        assert_eq!(view.eth_balance_wei_hex.as_deref(), Some("0x1"));
        assert_eq!(view.usdc_balance_raw_hex.as_deref(), Some("0x2a"));
        assert_eq!(
            view.usdc_contract_address.as_deref(),
            Some("0x3333333333333333333333333333333333333333")
        );
        assert_eq!(view.last_synced_block, Some(123));
        assert_eq!(view.age_secs, Some(601));
        assert_eq!(view.freshness_window_secs, 600);
        assert!(view.is_stale);
        assert_eq!(view.status, WalletBalanceStatus::Stale);
        assert!(view.bootstrap_pending);
    }

    #[test]
    fn inference_config_view_exposes_proxy_config_without_secrets() {
        let trusted = Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai")
            .expect("test principal should parse");
        let snapshot = RuntimeSnapshot {
            inference_provider: InferenceProvider::OpenRouterProxyWorker,
            inference_model: "openai/gpt-4o-mini".to_string(),
            openrouter_api_key: Some("super-secret-key".to_string()),
            openrouter_base_url: "https://openrouter.ai/api/v1".to_string(),
            openrouter_max_response_bytes: 64 * 1024,
            openrouter_reasoning_level: OpenRouterReasoningLevel::High,
            openrouter_proxy: OpenRouterProxyWorkerConfig {
                worker_base_url: "https://proxy.example.workers.dev".to_string(),
                trusted_callback_principal: Some(trusted),
            },
            ..RuntimeSnapshot::default()
        };

        let view = InferenceConfigView::from(&snapshot);
        assert_eq!(view.provider, InferenceProvider::OpenRouterProxyWorker);
        assert_eq!(
            view.openrouter_proxy_worker_base_url.as_deref(),
            Some("https://proxy.example.workers.dev")
        );
        assert_eq!(
            view.openrouter_proxy_trusted_callback_principal,
            Some(trusted.to_text())
        );
        assert!(view.openrouter_has_api_key);
        assert_eq!(
            view.openrouter_reasoning_level,
            OpenRouterReasoningLevel::High
        );
    }

    #[test]
    fn wallet_balance_sync_config_view_matches_runtime_config() {
        let snapshot = RuntimeSnapshot {
            wallet_balance_sync: WalletBalanceSyncConfig {
                enabled: true,
                normal_interval_secs: 300,
                low_cycles_interval_secs: 900,
                freshness_window_secs: 777,
                max_response_bytes: 512,
                discover_usdc_via_inbox: false,
            },
            ..RuntimeSnapshot::default()
        };

        let view = WalletBalanceSyncConfigView::from(&snapshot.wallet_balance_sync);
        assert_eq!(view.normal_interval_secs, 300);
        assert_eq!(view.low_cycles_interval_secs, 900);
        assert_eq!(view.freshness_window_secs, 777);
        assert_eq!(view.max_response_bytes, 512);
        assert!(!view.discover_usdc_via_inbox);
    }

    #[test]
    fn recovery_context_defaults_are_bounded_and_safe() {
        let context = RecoveryContext::default();
        assert_eq!(context.backoff_base_secs, 5);
        assert_eq!(context.backoff_max_secs, 300);
        assert!(context.response_limit.is_none());
    }

    #[test]
    fn response_limit_policy_defaults_match_wallet_sync_bounds() {
        let policy = ResponseLimitPolicy::default();
        assert_eq!(policy.current_bytes, 256);
        assert_eq!(policy.min_bytes, 256);
        assert_eq!(policy.max_bytes, 4 * 1024);
        assert_eq!(policy.tune_multiplier, 2);
    }

    #[test]
    fn retention_defaults_match_phase_one_policy() {
        let retention = RetentionConfig::default();
        assert_eq!(retention.jobs_max_age_secs, 14 * 24 * 60 * 60);
        assert_eq!(retention.dedupe_max_age_secs, 3 * 24 * 60 * 60);
        assert_eq!(retention.memory_facts_max_age_secs, 3 * 24 * 60 * 60);
        assert_eq!(retention.memory_facts_prune_batch_size, 25);
        assert_eq!(retention.jobs_max_records, 60_000);
        assert_eq!(retention.maintenance_batch_size, 120);
        assert_eq!(retention.maintenance_interval_secs, 10 * 60);

        let runtime = RetentionMaintenanceRuntime::default();
        assert_eq!(runtime.next_run_after_ns, 0);
        assert_eq!(runtime.retention_progress_percent, 0);
        assert_eq!(runtime.summarization_progress_percent, 0);
    }

    #[test]
    fn summary_schemas_round_trip_json_with_provenance_fields() {
        let session = SessionSummary {
            sender: "0xabc".to_string(),
            window_start_ns: 10,
            window_end_ns: 20,
            source_count: 3,
            inbox_message_count: 2,
            outbox_message_count: 1,
            inbox_preview: "inbox".to_string(),
            outbox_preview: "outbox".to_string(),
            generated_at_ns: 30,
        };
        let encoded_session = serde_json::to_vec(&session).expect("session summary should encode");
        let decoded_session: SessionSummary =
            serde_json::from_slice(&encoded_session).expect("session summary should decode");
        assert_eq!(decoded_session, session);

        let turn = TurnWindowSummary {
            window_start_ns: 100,
            window_end_ns: 200,
            source_count: 4,
            turn_count: 2,
            transition_count: 2,
            tool_call_count: 3,
            succeeded_turn_count: 1,
            failed_turn_count: 1,
            tool_success_count: 2,
            tool_failure_count: 1,
            top_errors: vec!["timeout".to_string()],
            generated_at_ns: 300,
        };
        let encoded_turn = serde_json::to_vec(&turn).expect("turn window summary should encode");
        let decoded_turn: TurnWindowSummary =
            serde_json::from_slice(&encoded_turn).expect("turn window summary should decode");
        assert_eq!(decoded_turn, turn);

        let rollup = MemoryRollup {
            namespace: "strategy".to_string(),
            window_start_ns: 400,
            window_end_ns: 500,
            source_count: 2,
            source_keys: vec!["strategy.alpha".to_string(), "strategy.beta".to_string()],
            canonical_value: "alpha=buy; beta=sell".to_string(),
            generated_at_ns: 600,
        };
        let encoded_rollup = serde_json::to_vec(&rollup).expect("memory rollup should encode");
        let decoded_rollup: MemoryRollup =
            serde_json::from_slice(&encoded_rollup).expect("memory rollup should decode");
        assert_eq!(decoded_rollup, rollup);
    }

    #[test]
    fn reflection_memory_schema_round_trips_json() {
        let record = ReflectionMemoryRecord {
            key: "market_fetch:dexscreener:search_pairs:missing_param".to_string(),
            tool: "market_fetch".to_string(),
            subject: "dexscreener:search_pairs".to_string(),
            error_class: "missing_param".to_string(),
            what_failed: "market_fetch[dexscreener:search_pairs] failed: missing required param q"
                .to_string(),
            what_worked: Some("worked recently with params.q".to_string()),
            degraded_turn_count: 3,
            repeat_count: 2,
            last_failed_at_ns: 700,
            last_failed_turn_id: "turn-7".to_string(),
            last_worked_at_ns: Some(800),
            last_worked_turn_id: Some("turn-8".to_string()),
            last_origin: ReflectionOrigin::Autonomy,
            updated_at_ns: 900,
        };

        let encoded = serde_json::to_vec(&record).expect("reflection memory should encode");
        let decoded: ReflectionMemoryRecord =
            serde_json::from_slice(&encoded).expect("reflection memory should decode");
        assert_eq!(decoded, record);
    }
}

// ── Storage-layer types ──────────────────────────────────────────────────────

/// Statistics returned by a single retention maintenance pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionPruneStats {
    pub deleted_jobs: u32,
    pub deleted_dedupe: u32,
    pub deleted_inbox: u32,
    pub deleted_outbox: u32,
    pub deleted_turns: u32,
    pub deleted_transitions: u32,
    pub deleted_tools: u32,
    pub generated_session_summaries: u32,
    pub generated_turn_window_summaries: u32,
    pub generated_memory_rollups: u32,
    pub deleted_memory_facts: u32,
}

/// Active cooldown state for a tool fingerprint that has repeatedly failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutonomyToolFailureCooldown {
    pub normalized_error: String,
    pub repeat_count: u32,
    pub cooldown_until_ns: u64,
}

/// Sort order for memory-fact listing queries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryFactSort {
    UpdatedAtDesc,
    KeyAsc,
}

/// Aggregate statistics about the memory-fact store.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryFactStats {
    pub total_facts: usize,
    pub storage_bytes: usize,
    pub config_facts: usize,
}

/// Distinguishes which outcall telemetry bucket should be updated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeOutcallKind {
    Inference,
    HttpFetch,
}
