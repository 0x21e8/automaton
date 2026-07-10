#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// IC canister entry point for the automaton, exposing all candid methods.
///
/// This module wires together every subsystem — scheduler, agent, storage,
/// HTTP, strategy registry — and surfaces them as candid-typed `update` /
/// `query` calls.  Initialization (`init` / `post_upgrade`) arms the
/// recurring timer and bootstraps the HTTP certification tree.
///
/// # Candid surface
///
/// Methods are grouped below into five sections:
/// - **Initialization** – canister lifecycle hooks
/// - **Configuration** – runtime tunables (inference, EVM, scheduler, …)
/// - **Strategy management** – template CRUD and lifecycle transitions
/// - **Observability** – read-only snapshots, logs, turn/conversation history
/// - **HTTP interface** – certified query and upgrade-to-update handlers
mod agent;
mod domain;
mod features;
mod http;
pub mod prompt;
mod sanitize;
mod scheduler;
mod storage;
#[allow(dead_code)]
mod strategy;
#[cfg(test)]
mod test_support;
mod timing;
mod tools;
mod util;

use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, AbiSelectorAssertion, ActiveExposure, AutonomyPolicy,
    AutonomySuppressionConfig, ConversationLog, ConversationSummary, DecisionRecord,
    EnqueueStrategyDiscoveryJobArgs, EvmRouteStateView, EvmStewardProof,
    ExposureReconciliationStatus, InboxMessage, InboxStats, InferenceConfigView, InferenceProvider,
    InferenceProxyStatusView, InferenceTransport, MemoryFact, MemoryRollup, ObservabilitySnapshot,
    OpenRouterProxyWorkerConfig, OpenRouterReasoningLevel, OutboxMessage, OutboxStats,
    PendingStrategyDiscoveryJob, PromoteDiscoveryProtocolArtifactsArgs, PromptLayer,
    PromptLayerView, ReflectionMemoryRecord, RetentionConfig, RetentionMaintenanceRuntime,
    RuntimeView, ScheduledJob, SchedulerRuntime, SessionSummary, SkillRecord, SpawnBootstrapView,
    StewardCommand, StewardState, StewardStatusView, StrategyDiscoveryStatusView,
    StrategyDiscoveryWorkerConfig, StrategyKillSwitchState, StrategyOutcomeStats,
    StrategyQuarantine, StrategyTemplate, StrategyTemplateKey, SubmitInferenceResultArgs,
    SubmitStrategyDiscoveryResultArgs, TaskKind, TaskScheduleConfig, TaskScheduleRuntime,
    TemplateActivationState, TemplateRevocationState, TemplateStatus, ToolCallRecord,
    TurnWindowSummary, WalletBalanceSyncConfigView, WalletBalanceTelemetryView,
};
#[cfg(target_arch = "wasm32")]
use crate::scheduler::scheduler_tick;
use crate::storage::{sqlite, stable};
use crate::timing::current_time_ns;
use crate::tools::ToolManager;
use candid::{CandidType, Principal};
use canlog::{log, GetLogFilter, LogFilter, LogPriorityLevels};
#[cfg(target_arch = "wasm32")]
use ic_cdk_timers::{clear_timer, set_timer_interval_serial, TimerId};
use ic_http_certification::{HttpRequest, HttpResponse, HttpUpdateRequest, HttpUpdateResponse};
use serde::Deserialize;
use sha3::{Digest, Keccak256};
use spawn_protocol::{InitArgs, SpawnBootstrapArgs};
#[cfg(all(not(target_arch = "wasm32"), test))]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

// ── Initialization ──────────────────────────────────────────────────────────
const WALLET_SYNC_RESPONSE_BYTES_FLOOR: u64 = 1_024;
const STEWARD_DIRECT_IMMEDIATE_TURN_DEDUPE_KEY: &str = "AgentTurn:steward-direct-immediate";

#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum InferenceProxyCallbackLogPriority {
    #[log_level(capacity = 1000, name = "INFERENCE_PROXY_CALLBACK_INFO")]
    Info,
    #[log_level(capacity = 500, name = "INFERENCE_PROXY_CALLBACK_ERROR")]
    Error,
}

impl GetLogFilter for InferenceProxyCallbackLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum StrategyDiscoveryCallbackLogPriority {
    #[log_level(capacity = 1000, name = "STRATEGY_DISCOVERY_CALLBACK_INFO")]
    DiscoveryInfo,
    #[log_level(capacity = 500, name = "STRATEGY_DISCOVERY_CALLBACK_ERROR")]
    DiscoveryError,
}

impl GetLogFilter for StrategyDiscoveryCallbackLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum StewardAdminLogPriority {
    #[log_level(capacity = 500, name = "STEWARD_ADMIN_INFO")]
    StewardInfo,
    #[log_level(capacity = 200, name = "STEWARD_ADMIN_WARN")]
    StewardWarn,
}

impl GetLogFilter for StewardAdminLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum StewardAuthLogPriority {
    #[log_level(capacity = 500, name = "STEWARD_AUTH_INFO")]
    AuthInfo,
    #[log_level(capacity = 200, name = "STEWARD_AUTH_WARN")]
    AuthWarn,
}

impl GetLogFilter for StewardAuthLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static SCHEDULER_TIMER_ID: RefCell<Option<TimerId>> = const { RefCell::new(None) };
    static SCHEDULER_WAKE_IN_FLIGHT: RefCell<bool> = const { RefCell::new(false) };
}

#[cfg(all(not(target_arch = "wasm32"), test))]
thread_local! {
    static TEST_STEWARD_INGRESS_CALLER: RefCell<Option<Principal>> = const { RefCell::new(None) };
}

/// Arguments for the `ingest_strategy_abi_artifact_admin` update call.
#[derive(CandidType, Deserialize)]
struct StrategyAbiIngestArgs {
    key: AbiArtifactKey,
    abi_json: String,
    source_ref: String,
    #[serde(default)]
    codehash: Option<String>,
    #[serde(default)]
    selector_assertions: Vec<AbiSelectorAssertion>,
}

#[derive(CandidType, Deserialize)]
struct SearchConfigArgs {
    api_key: String,
    #[serde(default)]
    max_searches_per_turn: Option<u8>,
}

/// Sort ordering for `list_memory_facts`.
#[derive(CandidType, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
enum MemoryFactListSort {
    UpdatedAtDesc,
    KeyAsc,
}

fn memory_fact_sort_to_storage(sort: MemoryFactListSort) -> stable::MemoryFactSort {
    match sort {
        MemoryFactListSort::UpdatedAtDesc => stable::MemoryFactSort::UpdatedAtDesc,
        MemoryFactListSort::KeyAsc => stable::MemoryFactSort::KeyAsc,
    }
}

fn steward_command_label(command: &StewardCommand) -> &'static str {
    match command {
        StewardCommand::Noop => "noop",
        StewardCommand::SetLoopEnabled { .. } => "set_loop_enabled",
        StewardCommand::SetAutonomyToolDedupeEnabled { .. } => "set_autonomy_tool_dedupe_enabled",
        StewardCommand::SetAutonomySuppressionConfig { .. } => "set_autonomy_suppression_config",
        StewardCommand::SetInferenceProvider { .. } => "set_inference_provider",
        StewardCommand::SetInferenceModel { .. } => "set_inference_model",
        StewardCommand::SetOpenrouterBaseUrl { .. } => "set_openrouter_base_url",
        StewardCommand::SetOpenrouterApiKey { .. } => "set_openrouter_api_key",
        StewardCommand::ConfigureSearch { .. } => "configure_search",
        StewardCommand::SetOpenrouterReasoningLevel { .. } => "set_openrouter_reasoning_level",
        StewardCommand::SetInferenceProxyConfig { .. } => "set_inference_proxy_config",
        StewardCommand::SetStrategyDiscoveryWorkerConfig { .. } => {
            "set_strategy_discovery_worker_config"
        }
        StewardCommand::SetWelcomeMessage { .. } => "set_welcome_message",
        StewardCommand::SetEvmRpcUrl { .. } => "set_evm_rpc_url",
        StewardCommand::SetEvmRpcFallbackUrl { .. } => "set_evm_rpc_fallback_url",
        StewardCommand::SetEvmRpcMaxResponseBytes { .. } => "set_evm_rpc_max_response_bytes",
        StewardCommand::SetInboxContractAddress { .. } => "set_inbox_contract_address",
        StewardCommand::SendStewardMessage { .. } => "send_steward_message",
        StewardCommand::SetPrincipal { .. } => "set_principal",
        StewardCommand::UpdateSteward { .. } => "update_steward",
        StewardCommand::SetEvmChainId { .. } => "set_evm_chain_id",
        StewardCommand::SetEvmConfirmationDepth { .. } => "set_evm_confirmation_depth",
        StewardCommand::DeriveAutomatonEvmAddress => "derive_automaton_evm_address",
        StewardCommand::SetHttpAllowedDomains { .. } => "set_http_allowed_domains",
        StewardCommand::UpdatePromptLayer { .. } => "update_prompt_layer",
        StewardCommand::PruneMemoryFacts { .. } => "prune_memory_facts",
        StewardCommand::SetSchedulerEnabled { .. } => "set_scheduler_enabled",
        StewardCommand::SetSchedulerLowCyclesMode { .. } => "set_scheduler_low_cycles_mode",
        StewardCommand::SetSchedulerBaseTickSecs { .. } => "set_scheduler_base_tick_secs",
        StewardCommand::SetTaskIntervalSecs { .. } => "set_task_interval_secs",
        StewardCommand::SetTaskEnabled { .. } => "set_task_enabled",
        StewardCommand::SetRetentionConfig { .. } => "set_retention_config",
        StewardCommand::UpdateSoul { .. } => "update_soul",
        StewardCommand::UpsertSkill { .. } => "upsert_skill",
        StewardCommand::EnqueueStrategyDiscoveryJob { .. } => "enqueue_strategy_discovery_job",
        StewardCommand::PromoteDiscoveryProtocolArtifacts { .. } => {
            "promote_discovery_protocol_artifacts"
        }
        StewardCommand::RegisterStrategy { .. } => "register_strategy",
        StewardCommand::IngestStrategyTemplate { .. } => "ingest_strategy_template",
        StewardCommand::IngestStrategyAbiArtifact { .. } => "ingest_strategy_abi_artifact",
        StewardCommand::ActivateStrategyTemplate { .. } => "activate_strategy_template",
        StewardCommand::DeprecateStrategyTemplate { .. } => "deprecate_strategy_template",
        StewardCommand::RevokeStrategyTemplate { .. } => "revoke_strategy_template",
        StewardCommand::SetStrategyKillSwitch { .. } => "set_strategy_kill_switch",
    }
}

fn steward_command_hash(command: &StewardCommand) -> Result<String, String> {
    let encoded = candid::encode_one(command)
        .map_err(|error| format!("failed to encode steward command: {error}"))?;
    let digest = Keccak256::digest(&encoded);
    Ok(format!("0x{}", hex::encode(digest)))
}

/// Returns `Err` when the caller is not a canister controller (wasm32 only;
/// always succeeds in native/test builds).
fn ensure_controller() -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let caller = ic_cdk::api::msg_caller();
        if !ic_cdk::api::is_controller(&caller) {
            return Err("caller is not a controller".to_string());
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(())
    }
}

/// Traps the canister (unconditionally aborts the call) when the caller is
/// not a controller.  Use this variant for update methods that do not return
/// `Result`.
fn ensure_controller_or_trap() {
    if let Err(error) = ensure_controller() {
        ic_cdk::trap(&error);
    }
}

/// Returns a human-readable caller identity for audit log entries.
/// On wasm32 this is the principal text; in native builds it is `"native"`.
fn caller_for_audit() -> String {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::msg_caller().to_text();

    #[cfg(not(target_arch = "wasm32"))]
    return "native".to_string();
}

fn inference_proxy_callback_caller_principal() -> String {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::msg_caller().to_text();

    #[cfg(not(target_arch = "wasm32"))]
    return "2vxsx-fae".to_string();
}

fn steward_ingress_caller() -> Principal {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::msg_caller();

    #[cfg(all(not(target_arch = "wasm32"), test))]
    return TEST_STEWARD_INGRESS_CALLER.with(|slot| {
        slot.borrow()
            .as_ref()
            .copied()
            .unwrap_or_else(Principal::anonymous)
    });

    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
    return Principal::anonymous();
}

#[cfg(test)]
fn set_steward_ingress_caller_for_tests(caller: Option<Principal>) {
    #[cfg(all(not(target_arch = "wasm32"), test))]
    TEST_STEWARD_INGRESS_CALLER.with(|slot| {
        *slot.borrow_mut() = caller;
    });

    #[cfg(target_arch = "wasm32")]
    {
        let _ = caller;
    }
}

fn steward_proof_expected_canister_id() -> String {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::id().to_text();

    #[cfg(not(target_arch = "wasm32"))]
    return "rrkah-fqaaa-aaaaa-aaaaq-cai".to_string();
}

async fn verify_steward_proof_for_command_hash(
    command_hash: &str,
    proof: &EvmStewardProof,
) -> Result<crate::features::evm::VerifiedEvmStewardProof, String> {
    let context = crate::features::evm::EvmStewardVerificationContext {
        canister_id: steward_proof_expected_canister_id(),
        active_steward: stable::active_steward(),
        expected_nonce: stable::steward_nonce_state().next_nonce,
        expected_command_hash: command_hash.to_string(),
        now_ns: current_time_ns(),
    };
    let rpc_result =
        crate::features::evm::HttpEvmRpcClient::from_snapshot(&stable::runtime_snapshot());
    let rpc_client = match &rpc_result {
        Ok(client) => Some(client),
        Err(reason) => {
            log!(
                StewardAuthLogPriority::AuthWarn,
                "steward_proof_eip1271_unavailable reason={reason}"
            );
            None
        }
    };

    match crate::features::evm::verify_evm_steward_proof_with_eip1271_fallback(
        proof, &context, rpc_client,
    )
    .await
    {
        Ok(verified) => {
            log!(
                StewardAuthLogPriority::AuthInfo,
                "steward_proof_verified chain_id={} address={} nonce={} expires_at_ns={}",
                verified.chain_id,
                verified.address,
                verified.nonce,
                verified.expires_at_ns
            );
            Ok(verified)
        }
        Err(error) => {
            log!(
                StewardAuthLogPriority::AuthWarn,
                "steward_proof_rejected chain_id={} address={} nonce={} expires_at_ns={} reason={}",
                proof.chain_id,
                proof.address,
                proof.nonce,
                proof.expires_at_ns,
                error,
            );
            Err(error)
        }
    }
}

fn consume_steward_nonce_and_record_usage(
    verified: &crate::features::evm::VerifiedEvmStewardProof,
) -> Result<(), String> {
    let mut snapshot = stable::runtime_snapshot();
    let expected_nonce = snapshot.steward_nonce.next_nonce;
    if expected_nonce != verified.nonce {
        return Err(format!(
            "proof nonce mismatch: expected={} got={}",
            expected_nonce, verified.nonce
        ));
    }

    snapshot.steward_nonce.next_nonce = expected_nonce
        .checked_add(1)
        .ok_or_else(|| "steward nonce overflow".to_string())?;
    let active_steward = snapshot
        .active_steward
        .as_mut()
        .ok_or_else(|| "no active steward configured".to_string())?;

    if !active_steward.enabled {
        return Err("active steward is disabled".to_string());
    }
    if active_steward.chain_id != verified.chain_id || active_steward.address != verified.address {
        return Err("active steward changed before proof consumption".to_string());
    }
    active_steward.last_used_at_ns = Some(current_time_ns());
    stable::save_runtime_snapshot(&snapshot);
    Ok(())
}

fn authorize_and_record_steward_ingress_usage(caller: Principal) -> Result<StewardState, String> {
    if caller == Principal::anonymous() {
        return Err("anonymous caller cannot execute steward ingress command".to_string());
    }
    let mut snapshot = stable::runtime_snapshot();
    let active_steward = snapshot
        .active_steward
        .as_mut()
        .ok_or_else(|| "no active steward configured".to_string())?;
    if !active_steward.enabled {
        return Err("active steward is disabled".to_string());
    }
    let expected_principal = active_steward
        .principal
        .as_ref()
        .copied()
        .ok_or_else(|| "active steward principal is not configured".to_string())?;
    if caller != expected_principal {
        return Err(format!(
            "caller principal does not match active steward principal: caller={} expected={}",
            caller.to_text(),
            expected_principal.to_text(),
        ));
    }
    active_steward.last_used_at_ns = Some(current_time_ns());
    let out = active_steward.clone();
    stable::save_runtime_snapshot(&snapshot);
    Ok(out)
}

fn update_active_steward_and_maybe_reset_nonce(
    chain_id: u64,
    address: String,
    enabled: bool,
) -> Result<(Option<StewardState>, StewardState, bool), String> {
    let previous = stable::active_steward();
    let requested = StewardState {
        chain_id,
        address,
        enabled,
        last_used_at_ns: None,
        principal: None,
    };
    let stored = stable::set_active_steward_with_nonce_reset_on_rotation(requested)?;
    let nonce_reset = previous.as_ref().is_none_or(|current| {
        current.chain_id != stored.chain_id || current.address != stored.address
    });
    Ok((previous, stored, nonce_reset))
}

/// Looks up a strategy template by key, returning a descriptive
/// error when not found.
fn require_strategy_template(key: &StrategyTemplateKey) -> Result<StrategyTemplate, String> {
    crate::strategy::registry::get_template(key).ok_or_else(|| {
        format!(
            "strategy template not found for {}:{}:{}:{}",
            key.protocol, key.primitive, key.chain_id, key.template_id
        )
    })
}

/// Atomically sets a template's `status` field and bumps `updated_at_ns`.
fn upsert_template_status(
    key: StrategyTemplateKey,
    status: TemplateStatus,
) -> Result<StrategyTemplate, String> {
    let mut template = require_strategy_template(&key)?;
    template.status = status;
    template.updated_at_ns = current_time_ns();
    crate::strategy::registry::upsert_template(template)
}

/// Called once when the canister is first installed.
/// Seeds stable storage, installs default skills, initialises the HTTP
/// certification tree, and arms the recurring scheduler timer.
#[ic_cdk::init]
fn init(args: InitArgs) {
    apply_init_args(args);
    crate::features::DefaultSkillLoader::install_defaults();
    crate::http::init_certification();
    arm_timer();
}

fn normalize_optional_trimmed(value: Option<String>) -> Option<String> {
    value.and_then(|entry| {
        let trimmed = entry.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|entry| {
            let trimmed = entry.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect()
}

fn validate_version_commit(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.len() != 40 {
        return Err(
            "spawn bootstrap version_commit must be a 40-character lowercase git SHA".to_string(),
        );
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("spawn bootstrap version_commit must be lowercase hex".to_string());
    }
    Ok(trimmed.to_string())
}

fn ensure_spawn_bootstrap_proxy_configured() -> Result<(), String> {
    let config = stable::openrouter_proxy_config();
    if config.worker_base_url.trim().is_empty() {
        return Err(
            "spawn bootstrap inference_transport=OpenrouterProxyWorker requires init arg inference_proxy_worker_base_url".to_string(),
        );
    }
    if config.trusted_callback_principal.is_none() {
        return Err(
            "spawn bootstrap inference_transport=OpenrouterProxyWorker requires init arg inference_proxy_trusted_callback_principal".to_string(),
        );
    }
    Ok(())
}

fn apply_spawn_bootstrap(args: SpawnBootstrapArgs) -> Result<(), String> {
    let session_id = args.session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("spawn bootstrap session_id cannot be empty".to_string());
    }

    let strategies = normalize_string_list(args.strategies);
    let skills = normalize_string_list(args.skills);
    let parent_id = normalize_optional_trimmed(args.parent_id);
    let version_commit = validate_version_commit(&args.version_commit)?;
    let model = normalize_optional_trimmed(args.provider.model);
    let open_router_api_key = normalize_optional_trimmed(args.provider.open_router_api_key);
    let brave_search_api_key = normalize_optional_trimmed(args.provider.brave_search_api_key);
    let inference_transport = args.provider.inference_transport;
    let open_router_reasoning_level = args.provider.open_router_reasoning_level;

    if inference_transport == InferenceTransport::OpenrouterProxyWorker {
        ensure_spawn_bootstrap_proxy_configured()?;
    }

    let chain_id = stable::runtime_snapshot().evm_cursor.chain_id;
    let steward = stable::set_active_steward(Some(StewardState {
        chain_id,
        address: args.steward_address,
        enabled: true,
        last_used_at_ns: None,
        principal: None,
    }))?
    .expect("spawn bootstrap should persist steward");

    if let Some(model) = model {
        let _ = stable::set_inference_model(model)?;
    }
    if let Some(api_key) = open_router_api_key {
        stable::set_openrouter_api_key(Some(api_key));
    }
    stable::set_openrouter_reasoning_level(open_router_reasoning_level);
    stable::set_inference_provider(match inference_transport {
        InferenceTransport::OpenrouterDirect => InferenceProvider::OpenRouter,
        InferenceTransport::OpenrouterProxyWorker => InferenceProvider::OpenRouterProxyWorker,
    });
    if let Some(api_key) = brave_search_api_key {
        let _ = apply_search_config(SearchConfigArgs {
            api_key,
            max_searches_per_turn: None,
        })?;
    }

    stable::set_spawn_bootstrap_metadata(SpawnBootstrapView {
        session_id: Some(session_id),
        parent_id,
        factory_principal: Some(args.factory_principal),
        risk: Some(args.risk),
        strategies,
        skills,
        version_commit: Some(version_commit),
    });

    log!(
        StewardAdminLogPriority::StewardInfo,
        "spawn_bootstrap_applied steward={} chain_id={} session_id_set=true",
        steward.address,
        steward.chain_id,
    );
    Ok(())
}

/// Applies all `InitArgs` fields to stable storage, trapping on the first
/// validation error.  Separated from `init` so tests can call it directly.
fn apply_init_args(args: InitArgs) {
    stable::init_storage();
    let _ = sqlite::init_storage();
    install_default_autonomy_policy_if_missing("init");
    let _ = stable::set_ecdsa_key_name(args.ecdsa_key_name)
        .unwrap_or_else(|error| ic_cdk::trap(&error));
    if let Some(chain_id) = args.evm_chain_id {
        let _ = stable::set_evm_chain_id(chain_id).unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    if let Some(rpc_url) = args.evm_rpc_url {
        let _ = stable::set_evm_rpc_url(rpc_url).unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    if let Some(confirmation_depth) = args.evm_confirmation_depth {
        let _ = stable::set_evm_confirmation_depth(confirmation_depth)
            .unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    if let Some(lookback_blocks) = args.evm_bootstrap_lookback_blocks {
        let _ = stable::set_evm_bootstrap_lookback_blocks(lookback_blocks)
            .unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    let _ = stable::set_inbox_contract_address(args.inbox_contract_address)
        .unwrap_or_else(|error| ic_cdk::trap(&error));
    if let Some(domains) = args.http_allowed_domains {
        let _ =
            stable::set_http_allowed_domains(domains).unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    if let Some(llm_canister_id) = args.llm_canister_id {
        let _ = stable::set_llm_canister_id(llm_canister_id.to_text())
            .unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    if let Some(search_api_key) = args.search_api_key {
        let _ = apply_search_config(SearchConfigArgs {
            api_key: search_api_key,
            max_searches_per_turn: None,
        })
        .unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    if args.inference_proxy_worker_base_url.is_some()
        || args.inference_proxy_trusted_callback_principal.is_some()
    {
        let _ = stable::set_openrouter_proxy_config(OpenRouterProxyWorkerConfig {
            worker_base_url: args.inference_proxy_worker_base_url.unwrap_or_default(),
            trusted_callback_principal: args.inference_proxy_trusted_callback_principal,
        })
        .unwrap_or_else(|error| ic_cdk::trap(&error));
    }
    if let Some(spawn_bootstrap) = args.spawn_bootstrap {
        apply_spawn_bootstrap(spawn_bootstrap).unwrap_or_else(|error| ic_cdk::trap(&error));
    }

    let mut snapshot = stable::runtime_snapshot();
    let mut changed = false;
    if let Some(enabled) = args.cycle_topup_enabled {
        snapshot.cycle_topup.enabled = enabled;
        changed = true;
    }
    if let Some(threshold) = args.auto_topup_cycle_threshold {
        snapshot.cycle_topup.auto_topup_cycle_threshold = u128::from(threshold);
        changed = true;
    }
    if changed {
        stable::save_runtime_snapshot(&snapshot);
    }
}

#[ic_cdk::pre_upgrade]
fn pre_upgrade() {
    let _ = sqlite::close_storage();
}

/// Called after every canister upgrade.
/// Re-initialises storage (migrating any new stable structures), rebuilds
/// the HTTP certification tree, and re-arms the timer.
#[ic_cdk::post_upgrade]
fn post_upgrade() {
    stable::init_storage();
    let _ = sqlite::reopen_storage();
    install_default_autonomy_policy_if_missing("post_upgrade");
    enforce_wallet_sync_response_bytes_floor();
    let _ = stable::remove_skill("agent-loop");
    crate::features::DefaultSkillLoader::seed_missing_defaults();
    crate::http::init_certification();
    arm_timer();
}

fn enforce_wallet_sync_response_bytes_floor() {
    let mut snapshot = stable::runtime_snapshot();
    if snapshot.wallet_balance_sync.max_response_bytes < WALLET_SYNC_RESPONSE_BYTES_FLOOR {
        snapshot.wallet_balance_sync.max_response_bytes = WALLET_SYNC_RESPONSE_BYTES_FLOOR;
        stable::save_runtime_snapshot(&snapshot);
    }
}

fn install_default_autonomy_policy_if_missing(source: &str) {
    if stable::autonomy_policy().is_some() {
        return;
    }

    let policy = AutonomyPolicy::conservative_default(current_time_ns());
    let version = policy.version;
    let updated_at_ns = policy.updated_at_ns;
    if let Err(error) = stable::set_autonomy_policy(policy) {
        ic_cdk::trap(format!(
            "failed to install default autonomy policy on {source}: {error}"
        ));
    }
    log!(
        StewardAdminLogPriority::StewardInfo,
        "default_autonomy_policy_installed source={} version={} updated_at_ns={}",
        source,
        version,
        updated_at_ns,
    );
}

// ── Configuration ────────────────────────────────────────────────────────────

/// Enables or disables the autonomous agent loop (controller only).
#[ic_cdk::update]
fn set_loop_enabled(enabled: bool) -> String {
    ensure_controller_or_trap();
    stable::set_loop_enabled(enabled);
    format!("loop_enabled={enabled}")
}

/// Enables or disables autonomous tool-call dedupe without changing other
/// suppression controls (controller only).
#[ic_cdk::update]
fn set_autonomy_tool_dedupe_enabled(enabled: bool) -> String {
    ensure_controller_or_trap();
    let config = stable::set_autonomy_tool_dedupe_enabled(enabled);
    format!(
        "autonomy_tool_dedupe_enabled={} dedupe_window_secs={}",
        config.tool_dedupe_enabled, config.dedupe_window_secs
    )
}

/// Replaces autonomy suppression policy (dedupe window + failure cooldown
/// thresholds) (controller only).
#[ic_cdk::update]
fn set_autonomy_suppression_config(
    config: AutonomySuppressionConfig,
) -> Result<AutonomySuppressionConfig, String> {
    ensure_controller()?;
    stable::set_autonomy_suppression_config(config)
}

/// Replaces the active autonomy policy (controller only).
#[ic_cdk::update]
fn update_autonomy_policy(policy: AutonomyPolicy) -> Result<AutonomyPolicy, String> {
    ensure_controller()?;
    stable::set_autonomy_policy(policy)
}

/// Sets the active inference backend (`IcLlm`, `OpenRouter`, or `OpenRouterProxyWorker`)
/// (controller only).
#[ic_cdk::update]
fn set_inference_provider(provider: InferenceProvider) -> String {
    ensure_controller_or_trap();
    stable::set_inference_provider(provider.clone());
    crate::http::init_certification();
    format!("inference_provider={provider:?}")
}

/// Sets the inference model identifier (e.g. `"llama3.1:8b"` or `"openai/gpt-4o-mini"`)
/// (controller only).
#[ic_cdk::update]
fn set_inference_model(model: String) -> Result<String, String> {
    ensure_controller()?;
    let stored = stable::set_inference_model(model)?;
    crate::http::init_certification();
    Ok(stored)
}

/// Updates the OpenRouter-compatible base URL used for inference HTTP calls (controller only).
#[ic_cdk::update]
fn set_openrouter_base_url(base_url: String) -> Result<String, String> {
    ensure_controller()?;
    let stored = stable::set_openrouter_base_url(base_url)?;
    crate::http::init_certification();
    Ok(stored)
}

/// Stores (or clears) the OpenRouter API key in stable storage.
/// Pass `None` to remove the key (controller only).
#[ic_cdk::update]
fn set_openrouter_api_key(api_key: Option<String>) -> String {
    ensure_controller_or_trap();
    stable::set_openrouter_api_key(api_key);
    crate::http::init_certification();
    "openrouter_api_key_updated".to_string()
}

/// Stores the web-search API key and optional per-turn search budget override (controller only).
#[ic_cdk::update]
fn configure_search(config: SearchConfigArgs) -> Result<String, String> {
    ensure_controller()?;
    apply_search_config(config)
}

fn apply_search_config(config: SearchConfigArgs) -> Result<String, String> {
    let api_key = config.api_key.trim();
    if api_key.is_empty() {
        return Err("search api key must not be empty".to_string());
    }
    stable::set_search_api_key(Some(api_key.to_string()));
    stable::set_search_max_per_turn(config.max_searches_per_turn)?;
    crate::http::init_certification();
    Ok("search_configured".to_string())
}

/// Sets the OpenRouter reasoning effort level for models that support it (controller only).
#[ic_cdk::update]
fn set_openrouter_reasoning_level(level: OpenRouterReasoningLevel) -> String {
    ensure_controller_or_trap();
    let stored = stable::set_openrouter_reasoning_level(level);
    crate::http::init_certification();
    format!("openrouter_reasoning_level={stored:?}")
}

/// Stores OpenRouter proxy worker config used by async inference callbacks (controller only).
#[ic_cdk::update]
fn set_inference_proxy_config(
    config: OpenRouterProxyWorkerConfig,
) -> Result<OpenRouterProxyWorkerConfig, String> {
    ensure_controller()?;
    let stored = stable::set_openrouter_proxy_config(config)?;
    crate::http::init_certification();
    Ok(stored)
}

fn strategy_discovery_exposure_summary() -> String {
    let exposures = stable::list_active_exposures();
    if exposures.is_empty() {
        return "no active exposures tracked".to_string();
    }
    let protocols = exposures
        .iter()
        .map(|exposure| exposure.protocol.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    format!(
        "active_exposures={} distinct_protocols={}",
        exposures.len(),
        protocols
    )
}

fn strategy_discovery_autonomy_summary() -> String {
    let telemetry = stable::cycle_telemetry();
    format!(
        "liquid_cycles={} total_cycles={} survival_tier={:?}",
        telemetry.liquid_cycles,
        telemetry.total_cycles,
        stable::scheduler_survival_tier()
    )
}

fn apply_strategy_discovery_worker_config(
    config: StrategyDiscoveryWorkerConfig,
) -> Result<StrategyDiscoveryStatusView, String> {
    let _ = stable::set_strategy_discovery_worker_config(config)?;
    crate::http::init_certification();
    Ok(stable::strategy_discovery_status_view(current_time_ns()))
}

async fn enqueue_strategy_discovery_job_shared(
    args: EnqueueStrategyDiscoveryJobArgs,
) -> Result<PendingStrategyDiscoveryJob, String> {
    let config = stable::strategy_discovery_worker_config();
    let objective = args
        .objective
        .unwrap_or_else(|| config.objective.clone())
        .trim()
        .to_string();
    let watchlist = args
        .watchlist
        .unwrap_or_else(|| config.protocol_watchlist.clone());
    let pending = crate::features::strategy_discovery::submit_strategy_discovery_job(
        &config,
        objective,
        watchlist,
        strategy_discovery_exposure_summary(),
        strategy_discovery_autonomy_summary(),
    )
    .await?;
    crate::http::init_certification();
    Ok(pending)
}

fn promote_discovery_protocol_artifacts_shared(
    args: PromoteDiscoveryProtocolArtifactsArgs,
) -> Result<AbiArtifact, String> {
    let record = stable::get_strategy_discovery_result(&args.job_id)
        .ok_or_else(|| format!("unknown strategy discovery result job_id={}", args.job_id))?;
    if !matches!(
        record.status,
        crate::domain::types::StrategyDiscoveryResultStatus::Validated
    ) {
        return Err(format!(
            "strategy discovery result job_id={} is not validated",
            args.job_id
        ));
    }
    let bundle = record
        .payload
        .protocol_artifacts
        .iter()
        .find(|bundle| bundle.bundle_id == args.bundle_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "strategy discovery result job_id={} missing protocol artifact bundle_id={}",
                args.job_id, args.bundle_id
            )
        })?;
    crate::strategy::abi::promote_discovery_protocol_artifact(&bundle, current_time_ns())
}

/// Stores strategy-discovery worker config used by async discovery callbacks (controller only).
#[ic_cdk::update]
fn set_strategy_discovery_worker_config(
    config: StrategyDiscoveryWorkerConfig,
) -> Result<StrategyDiscoveryStatusView, String> {
    ensure_controller()?;
    apply_strategy_discovery_worker_config(config)
}

/// Manually submits a strategy-discovery job for ops/testing (controller only).
#[ic_cdk::update]
async fn enqueue_strategy_discovery_job_admin(
    args: EnqueueStrategyDiscoveryJobArgs,
) -> Result<PendingStrategyDiscoveryJob, String> {
    ensure_controller()?;
    enqueue_strategy_discovery_job_shared(args).await
}

/// Promotes a validated staged discovery artifact into the ABI registry (controller only).
#[ic_cdk::update]
fn promote_discovery_protocol_artifacts_admin(
    args: PromoteDiscoveryProtocolArtifactsArgs,
) -> Result<AbiArtifact, String> {
    ensure_controller()?;
    let artifact = promote_discovery_protocol_artifacts_shared(args)?;
    crate::http::init_certification();
    Ok(artifact)
}

/// Sets a custom welcome message shown in the TUI on boot (controller only).
/// An empty string clears the custom message and restores the default.
#[ic_cdk::update]
fn set_welcome_message(message: String) -> Result<String, String> {
    ensure_controller()?;
    let stored = stable::set_welcome_message(message)?;
    crate::http::init_certification();
    Ok(stored)
}

/// Updates the primary EVM JSON-RPC endpoint (controller only).
#[ic_cdk::update]
fn set_evm_rpc_url(url: String) -> Result<String, String> {
    ensure_controller()?;
    stable::set_evm_rpc_url(url)
}

/// Sets an optional fallback EVM RPC URL used when the primary is unavailable (controller only).
#[ic_cdk::update]
fn set_evm_rpc_fallback_url(url: Option<String>) -> Result<Option<String>, String> {
    ensure_controller()?;
    stable::set_evm_rpc_fallback_url(url)
}

/// Caps the maximum response size (bytes) returned by EVM RPC outbound calls (controller only).
#[ic_cdk::update]
fn set_evm_rpc_max_response_bytes(max_response_bytes: u64) -> Result<u64, String> {
    ensure_controller()?;
    stable::set_evm_rpc_max_response_bytes(max_response_bytes)
}

/// Overrides the EVM inbox contract address (controller only).
#[ic_cdk::update]
fn set_inbox_contract_address_admin(address: Option<String>) -> Result<Option<String>, String> {
    ensure_controller()?;
    stable::set_inbox_contract_address(address)
}

/// Sets or rotates the active steward identity used for signed command authority
/// (controller recovery path).
#[ic_cdk::update]
fn set_steward_admin(
    chain_id: u64,
    address: String,
    enabled: bool,
) -> Result<StewardState, String> {
    ensure_controller()?;
    let caller = caller_for_audit();
    let (previous, stored, nonce_reset) =
        update_active_steward_and_maybe_reset_nonce(chain_id, address.clone(), enabled).map_err(
            |error| {
                log!(
                StewardAdminLogPriority::StewardWarn,
                "set_steward_admin_rejected caller={} chain_id={} address={} enabled={} error={}",
                caller,
                chain_id,
                address,
                enabled,
                error,
            );
                error
            },
        )?;

    log!(
        StewardAdminLogPriority::StewardInfo,
        "set_steward_admin_applied caller={} previous_steward={:?} new_steward={:?} nonce_reset={}",
        caller,
        previous,
        stored,
        nonce_reset,
    );
    Ok(stored)
}

/// Executes a signed steward command after EVM-proof verification.
async fn dispatch_steward_command(
    command: StewardCommand,
    steward_actor: &str,
) -> Result<String, String> {
    match command {
        StewardCommand::Noop => Ok("steward_noop_executed".to_string()),
        StewardCommand::SetLoopEnabled { enabled } => {
            stable::set_loop_enabled(enabled);
            Ok(format!("loop_enabled={enabled}"))
        }
        StewardCommand::SetAutonomyToolDedupeEnabled { enabled } => {
            let config = stable::set_autonomy_tool_dedupe_enabled(enabled);
            Ok(format!(
                "autonomy_tool_dedupe_enabled={} dedupe_window_secs={}",
                config.tool_dedupe_enabled, config.dedupe_window_secs
            ))
        }
        StewardCommand::SetAutonomySuppressionConfig { config } => {
            let stored = stable::set_autonomy_suppression_config(config)?;
            Ok(format!(
                "autonomy_suppression_config_updated dedupe_window_secs={} failure_repeat_window_secs={} failure_repeat_threshold={} failure_cooldown_secs={}",
                stored.dedupe_window_secs,
                stored.failure_repeat_window_secs,
                stored.failure_repeat_threshold,
                stored.failure_cooldown_secs
            ))
        }
        StewardCommand::SetInferenceProvider { provider } => {
            stable::set_inference_provider(provider.clone());
            crate::http::init_certification();
            Ok(format!("inference_provider={provider:?}"))
        }
        StewardCommand::SetInferenceModel { model } => {
            let stored = stable::set_inference_model(model)?;
            crate::http::init_certification();
            Ok(format!("inference_model={stored}"))
        }
        StewardCommand::SetOpenrouterBaseUrl { base_url } => {
            let stored = stable::set_openrouter_base_url(base_url)?;
            crate::http::init_certification();
            Ok(format!("openrouter_base_url={stored}"))
        }
        StewardCommand::SetOpenrouterApiKey { api_key } => {
            stable::set_openrouter_api_key(api_key);
            crate::http::init_certification();
            Ok("openrouter_api_key_updated".to_string())
        }
        StewardCommand::ConfigureSearch {
            api_key,
            max_searches_per_turn,
        } => apply_search_config(SearchConfigArgs {
            api_key,
            max_searches_per_turn,
        }),
        StewardCommand::SetOpenrouterReasoningLevel { level } => {
            let stored = stable::set_openrouter_reasoning_level(level);
            crate::http::init_certification();
            Ok(format!("openrouter_reasoning_level={stored:?}"))
        }
        StewardCommand::SetInferenceProxyConfig { config } => {
            let stored = stable::set_openrouter_proxy_config(config)?;
            crate::http::init_certification();
            Ok(format!(
                "inference_proxy_worker_base_url={}",
                stored.worker_base_url
            ))
        }
        StewardCommand::SetStrategyDiscoveryWorkerConfig { config } => {
            let stored = apply_strategy_discovery_worker_config(config)?;
            Ok(format!(
                "strategy_discovery_enabled={} watchlist_len={}",
                stored.enabled, stored.protocol_watchlist_len
            ))
        }
        StewardCommand::SetWelcomeMessage { message } => {
            let stored = stable::set_welcome_message(message)?;
            crate::http::init_certification();
            Ok(format!("welcome_message_len={}", stored.len()))
        }
        StewardCommand::SetEvmRpcUrl { url } => {
            let stored = stable::set_evm_rpc_url(url)?;
            Ok(format!("evm_rpc_url={stored}"))
        }
        StewardCommand::SetEvmRpcFallbackUrl { url } => {
            let stored = stable::set_evm_rpc_fallback_url(url)?;
            Ok(format!(
                "evm_rpc_fallback_url={}",
                stored.unwrap_or_else(|| "none".to_string())
            ))
        }
        StewardCommand::SetEvmRpcMaxResponseBytes { max_response_bytes } => {
            let stored = stable::set_evm_rpc_max_response_bytes(max_response_bytes)?;
            Ok(format!("evm_rpc_max_response_bytes={stored}"))
        }
        StewardCommand::SetInboxContractAddress { address } => {
            let stored = stable::set_inbox_contract_address(address)?;
            Ok(format!(
                "inbox_contract_address={}",
                stored.unwrap_or_else(|| "none".to_string())
            ))
        }
        StewardCommand::SendStewardMessage { sender, message } => {
            let inbox_id = crate::scheduler::ingest_steward_direct_message(sender, message)?;
            let immediate_job_id = enqueue_immediate_agent_turn_if_absent(
                STEWARD_DIRECT_IMMEDIATE_TURN_DEDUPE_KEY.to_string(),
                "steward_direct_message_resume",
            );
            Ok(format!(
                "steward_direct_message_ingested id={inbox_id} immediate_turn_enqueued={} immediate_job_id={}",
                immediate_job_id.is_some(),
                immediate_job_id.unwrap_or_default()
            ))
        }
        StewardCommand::SetPrincipal { principal } => {
            let stored = stable::set_active_steward_principal(principal)?;
            Ok(format!(
                "steward_principal={}",
                stored
                    .map(|entry| entry.to_text())
                    .unwrap_or_else(|| "none".to_string())
            ))
        }
        StewardCommand::UpdateSteward {
            chain_id,
            address,
            enabled,
        } => {
            let (previous, stored, nonce_reset) =
                update_active_steward_and_maybe_reset_nonce(chain_id, address, enabled)?;
            log!(
                StewardAuthLogPriority::AuthInfo,
                "steward_update_steward_applied previous_steward={:?} new_steward={:?} nonce_reset={}",
                previous,
                stored,
                nonce_reset,
            );
            Ok("steward_update_steward_executed".to_string())
        }
        StewardCommand::SetEvmChainId { chain_id } => {
            let stored = stable::set_evm_chain_id(chain_id)?;
            Ok(format!("evm_chain_id={stored}"))
        }
        StewardCommand::SetEvmConfirmationDepth { confirmation_depth } => {
            let stored = stable::set_evm_confirmation_depth(confirmation_depth)?;
            Ok(format!("evm_confirmation_depth={stored}"))
        }
        StewardCommand::DeriveAutomatonEvmAddress => {
            crate::features::threshold_signer::derive_and_cache_evm_address(
                &stable::get_ecdsa_key_name(),
            )
            .await
        }
        StewardCommand::SetHttpAllowedDomains { domains } => {
            let stored = stable::set_http_allowed_domains(domains)?;
            Ok(format!("http_allowed_domains={}", stored.len()))
        }
        StewardCommand::UpdatePromptLayer { layer_id, content } => {
            let stored =
                crate::tools::update_prompt_layer_content(layer_id, content, steward_actor)?;
            Ok(format!(
                "prompt_layer_updated layer_id={} version={}",
                stored.layer_id, stored.version
            ))
        }
        StewardCommand::PruneMemoryFacts {
            prefix,
            updated_before_ns,
            limit,
        } => {
            let normalized_prefix = prefix
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if normalized_prefix.is_none() && updated_before_ns.is_none() {
                return Err(
                    "provide at least one prune filter: prefix and/or updated_before_ns"
                        .to_string(),
                );
            }
            let removed =
                stable::prune_memory_facts(normalized_prefix, updated_before_ns, limit as usize);
            Ok(format!("memory_facts_pruned={}", removed.len()))
        }
        StewardCommand::SetSchedulerEnabled { enabled } => {
            Ok(stable::set_scheduler_enabled(enabled))
        }
        StewardCommand::SetSchedulerLowCyclesMode { enabled } => {
            Ok(stable::set_scheduler_low_cycles_mode(enabled))
        }
        StewardCommand::SetSchedulerBaseTickSecs { interval_secs } => {
            let persisted = stable::set_scheduler_base_tick_secs(interval_secs)?;
            arm_timer_with_interval(persisted);
            crate::http::init_certification();
            Ok(format!("scheduler_base_tick_secs={persisted}"))
        }
        StewardCommand::SetTaskIntervalSecs {
            kind,
            interval_secs,
        } => {
            stable::set_task_interval_secs(&kind, interval_secs)?;
            Ok("task_interval_updated".to_string())
        }
        StewardCommand::SetTaskEnabled { kind, enabled } => {
            stable::set_task_enabled(&kind, enabled);
            Ok("task_enabled_updated".to_string())
        }
        StewardCommand::SetRetentionConfig { config } => {
            let stored = stable::set_retention_config(config)?;
            Ok(format!(
                "retention_config_maintenance_interval_secs={}",
                stored.maintenance_interval_secs
            ))
        }
        StewardCommand::UpdateSoul { new_soul } => {
            if new_soul.trim().is_empty() {
                return Err("soul cannot be empty".to_string());
            }
            Ok(stable::set_soul(new_soul))
        }
        StewardCommand::UpsertSkill { skill } => {
            stable::upsert_skill(&skill);
            Ok(format!("skill_upserted name={}", skill.name))
        }
        StewardCommand::EnqueueStrategyDiscoveryJob { args } => {
            let pending = enqueue_strategy_discovery_job_shared(args).await?;
            Ok(format!(
                "strategy_discovery_job_enqueued job_id={}",
                pending.job_id
            ))
        }
        StewardCommand::PromoteDiscoveryProtocolArtifacts { args } => {
            let artifact = promote_discovery_protocol_artifacts_shared(args)?;
            crate::http::init_certification();
            Ok(format!(
                "strategy_discovery_protocol_artifact_promoted protocol={} role={} chain_id={}",
                artifact.key.protocol, artifact.key.role, artifact.key.chain_id
            ))
        }
        StewardCommand::RegisterStrategy { recipe_json } => {
            let recipe: crate::strategy::registry::StrategyRecipe =
                serde_json::from_str(&recipe_json)
                    .map_err(|error| format!("invalid strategy recipe JSON: {error}"))?;
            let result = crate::strategy::registry::register_from_recipe(recipe)?;
            Ok(format!(
                "strategy_registered protocol={} primitive={} template_id={} chain_id={}",
                result.template.key.protocol,
                result.template.key.primitive,
                result.template.key.template_id,
                result.template.key.chain_id
            ))
        }
        StewardCommand::IngestStrategyTemplate { template } => {
            let mut template = template;
            let now_ns = current_time_ns();
            if template.created_at_ns == 0 {
                template.created_at_ns = now_ns;
            }
            template.updated_at_ns = now_ns;
            let _ = crate::strategy::registry::upsert_template(template)?;
            Ok("strategy_template_ingested".to_string())
        }
        StewardCommand::IngestStrategyAbiArtifact {
            key,
            abi_json,
            source_ref,
            codehash,
            selector_assertions,
        } => {
            let _ = crate::strategy::abi::normalize_and_store_abi_artifact(
                key,
                &abi_json,
                &source_ref,
                codehash,
                &selector_assertions,
                current_time_ns(),
            )?;
            Ok("strategy_abi_artifact_ingested".to_string())
        }
        StewardCommand::ActivateStrategyTemplate { key, reason } => {
            let _ = upsert_template_status(key.clone(), TemplateStatus::Active)?;
            crate::strategy::compiler::dry_run_compile(&key)?;
            let _ = crate::strategy::registry::set_activation(TemplateActivationState {
                key,
                enabled: true,
                updated_at_ns: current_time_ns(),
                reason: reason
                    .or_else(|| Some("controller activation after dry-run compile".to_string())),
            })?;
            Ok("strategy_template_activated".to_string())
        }
        StewardCommand::DeprecateStrategyTemplate { key, reason } => {
            let _ = upsert_template_status(key.clone(), TemplateStatus::Deprecated)?;
            let _ = crate::strategy::registry::set_activation(TemplateActivationState {
                key,
                enabled: false,
                updated_at_ns: current_time_ns(),
                reason,
            });
            Ok("strategy_template_deprecated".to_string())
        }
        StewardCommand::RevokeStrategyTemplate { key, reason } => {
            let _ = upsert_template_status(key.clone(), TemplateStatus::Revoked)?;
            let now_ns = current_time_ns();
            let revocation_reason = reason.clone();
            let _ = crate::strategy::registry::set_revocation(TemplateRevocationState {
                key: key.clone(),
                revoked: true,
                updated_at_ns: now_ns,
                reason: revocation_reason,
            })?;
            let _ = crate::strategy::registry::set_activation(TemplateActivationState {
                key,
                enabled: false,
                updated_at_ns: now_ns,
                reason: reason.or_else(|| Some("revoked".to_string())),
            });
            Ok("strategy_template_revoked".to_string())
        }
        StewardCommand::SetStrategyKillSwitch {
            key,
            enabled,
            reason,
        } => {
            let state = crate::strategy::registry::set_kill_switch(StrategyKillSwitchState {
                key,
                enabled,
                updated_at_ns: current_time_ns(),
                reason,
            })?;
            Ok(format!("strategy_kill_switch_enabled={}", state.enabled))
        }
    }
}

#[ic_cdk::update]
pub(crate) async fn steward_execute(
    command: StewardCommand,
    proof: EvmStewardProof,
) -> Result<String, String> {
    let command_label = steward_command_label(&command);
    let command_hash = steward_command_hash(&command)?;
    let verified = verify_steward_proof_for_command_hash(&command_hash, &proof).await?;
    consume_steward_nonce_and_record_usage(&verified).map_err(|error| {
        log!(
            StewardAuthLogPriority::AuthWarn,
            "steward_execute_rejected command={} chain_id={} address={} nonce={} reason={}",
            command_label,
            verified.chain_id,
            verified.address,
            verified.nonce,
            error,
        );
        error
    })?;

    let steward_actor = format!("steward:{}:{}", verified.chain_id, verified.address);
    let result = dispatch_steward_command(command, &steward_actor)
        .await
        .map_err(|error: String| {
            log!(
                StewardAuthLogPriority::AuthWarn,
                "steward_execute_rejected command={} chain_id={} address={} nonce={} reason={}",
                command_label,
                verified.chain_id,
                verified.address,
                verified.nonce,
                error,
            );
            error
        })?;

    log!(
        StewardAuthLogPriority::AuthInfo,
        "steward_execute_applied command={} chain_id={} address={} nonce={} result={}",
        command_label,
        verified.chain_id,
        verified.address,
        verified.nonce,
        result,
    );
    Ok(result)
}

#[ic_cdk::update]
pub(crate) async fn steward_execute_ingress(command: StewardCommand) -> Result<String, String> {
    let command_label = steward_command_label(&command);
    let caller = steward_ingress_caller();
    let caller_text = caller.to_text();
    let active_steward = authorize_and_record_steward_ingress_usage(caller).map_err(|error| {
        log!(
            StewardAuthLogPriority::AuthWarn,
            "steward_execute_ingress_rejected command={} caller={} reason={}",
            command_label,
            caller_text,
            error,
        );
        error
    })?;
    let steward_actor = format!(
        "steward:{}:{}:principal:{}",
        active_steward.chain_id, active_steward.address, caller_text
    );
    let result = dispatch_steward_command(command, &steward_actor)
        .await
        .map_err(|error: String| {
            log!(
                StewardAuthLogPriority::AuthWarn,
                "steward_execute_ingress_rejected command={} caller={} chain_id={} address={} reason={}",
                command_label,
                caller_text,
                active_steward.chain_id,
                active_steward.address,
                error,
            );
            error
        })?;
    log!(
        StewardAuthLogPriority::AuthInfo,
        "steward_execute_ingress_applied command={} caller={} chain_id={} address={} result={}",
        command_label,
        caller_text,
        active_steward.chain_id,
        active_steward.address,
        result,
    );
    Ok(result)
}

/// Updates the EVM chain ID used for all on-chain operations (controller only).
#[ic_cdk::update]
fn set_evm_chain_id_admin(chain_id: u64) -> Result<u64, String> {
    ensure_controller()?;
    stable::set_evm_chain_id(chain_id)
}

/// Sets how many block confirmations must pass before an EVM event is
/// considered finalised (controller only).
#[ic_cdk::update]
fn set_evm_confirmation_depth_admin(confirmation_depth: u64) -> Result<u64, String> {
    ensure_controller()?;
    stable::set_evm_confirmation_depth(confirmation_depth)
}

/// Derives the canister's threshold-ECDSA EVM address from the configured key
/// and caches it in stable storage (controller only).
#[ic_cdk::update]
async fn derive_automaton_evm_address() -> Result<String, String> {
    ensure_controller()?;
    crate::features::threshold_signer::derive_and_cache_evm_address(&stable::get_ecdsa_key_name())
        .await
}

/// Replaces the HTTP outbound allowlist.  An empty slice disables the allowlist
/// and permits all domains (controller only).
#[ic_cdk::update]
fn set_http_allowed_domains(domains: Vec<String>) -> Result<Vec<String>, String> {
    ensure_controller()?;
    stable::set_http_allowed_domains(domains)
}

// ── Observability ────────────────────────────────────────────────────────────

/// Returns a combined runtime snapshot (cycles, scheduler state, inference
/// config, top-up config, …).
#[ic_cdk::query]
fn get_runtime_view() -> RuntimeView {
    stable::snapshot_to_view()
}

/// Returns the persisted launchpad/factory bootstrap metadata, if any.
#[ic_cdk::query]
fn get_spawn_bootstrap_view() -> SpawnBootstrapView {
    stable::spawn_bootstrap_view_snapshot()
}

/// Returns the current EVM route state (chain ID, RPC URL, addresses, …).
#[ic_cdk::query]
fn get_evm_route_state_view() -> EvmRouteStateView {
    stable::evm_route_state_view()
}

/// Returns the currently configured steward identity and next expected nonce.
#[ic_cdk::query]
fn get_steward_status() -> StewardStatusView {
    stable::steward_status_view()
}

/// Returns the automaton's derived EVM address, or `None` before first derivation.
#[ic_cdk::query]
fn get_automaton_evm_address() -> Option<String> {
    stable::get_automaton_evm_address()
}

/// Returns the latest synced wallet balance telemetry (ETH, USDC, sync status).
#[ic_cdk::query]
fn get_wallet_balance_telemetry() -> WalletBalanceTelemetryView {
    stable::wallet_balance_telemetry_view()
}

/// Returns the wallet balance sync configuration (intervals, freshness window, …).
#[ic_cdk::query]
fn get_wallet_balance_sync_config() -> WalletBalanceSyncConfigView {
    stable::wallet_balance_sync_config_view()
}

/// Returns the current autonomy suppression configuration.
#[ic_cdk::query]
fn get_autonomy_suppression_config() -> AutonomySuppressionConfig {
    stable::autonomy_suppression_config()
}

/// Returns the active autonomy policy, installing a conservative default if
/// one has not been persisted yet.
#[ic_cdk::query]
fn get_autonomy_policy() -> AutonomyPolicy {
    stable::autonomy_policy().unwrap_or_else(|| AutonomyPolicy::conservative_default(0))
}

/// Returns the scheduler's current runtime state (enabled flag, last tick, …).
#[ic_cdk::query]
fn get_scheduler_view() -> SchedulerRuntime {
    stable::scheduler_runtime_view()
}

/// Returns the scheduler timer base tick interval in seconds.
#[ic_cdk::query]
fn get_scheduler_base_tick_secs() -> u64 {
    stable::get_scheduler_base_tick_secs()
}

/// Returns the current conversation-retention configuration.
#[ic_cdk::query]
fn get_retention_config() -> RetentionConfig {
    stable::retention_config()
}

/// Returns runtime statistics from the last retention-maintenance pass.
#[ic_cdk::query]
fn get_retention_maintenance_runtime() -> RetentionMaintenanceRuntime {
    stable::retention_maintenance_runtime()
}

/// Returns up to `limit` most-recently enqueued scheduler job records.
#[ic_cdk::query]
fn list_scheduler_jobs(limit: u32) -> Vec<ScheduledJob> {
    sqlite::list_recent_jobs(limit as usize)
        .unwrap_or_else(|_| stable::list_recent_jobs(limit as usize))
}

/// Returns the configured schedule and live runtime state for every task kind.
#[ic_cdk::query]
fn list_task_schedules() -> Vec<(TaskScheduleConfig, TaskScheduleRuntime)> {
    stable::list_task_schedules()
}

/// Returns an observability snapshot containing up to `limit` recent events,
/// combined with the current runtime and scheduler views.
#[ic_cdk::query]
fn get_observability_snapshot(limit: u32) -> ObservabilitySnapshot {
    stable::observability_snapshot(limit as usize)
}

/// Returns up to `limit` inbox messages ordered by arrival time (newest last).
#[ic_cdk::query]
fn list_inbox_messages(limit: u32) -> Vec<InboxMessage> {
    sqlite::list_inbox_messages(limit as usize)
        .unwrap_or_else(|_| stable::list_inbox_messages(limit as usize))
}

/// Returns all prompt layers ordered by layer ID.
#[ic_cdk::query]
fn get_prompt_layers() -> Vec<PromptLayerView> {
    stable::list_prompt_layers()
}

/// Replaces the content of a prompt layer identified by `layer_id` (controller only).
#[ic_cdk::update]
fn update_prompt_layer_admin(layer_id: u8, content: String) -> Result<PromptLayer, String> {
    ensure_controller()?;
    crate::tools::update_prompt_layer_content(
        layer_id,
        content,
        &format!("admin:{}", caller_for_audit()),
    )
}

/// Returns summary records for all active conversations (one per sender).
#[ic_cdk::query]
fn list_conversations() -> Vec<ConversationSummary> {
    stable::list_conversation_summaries()
}

/// Returns up to `limit` most-recent session summaries.
#[ic_cdk::query]
fn list_session_summaries(limit: u32) -> Vec<SessionSummary> {
    stable::list_session_summaries(limit as usize)
}

/// Returns up to `limit` most-recent turn-window summaries used for context compression.
#[ic_cdk::query]
fn list_turn_window_summaries(limit: u32) -> Vec<TurnWindowSummary> {
    stable::list_turn_window_summaries(limit as usize)
}

/// Returns up to `limit` most-recent memory rollups (compressed long-term context).
#[ic_cdk::query]
fn list_memory_rollups(limit: u32) -> Vec<MemoryRollup> {
    stable::list_memory_rollups(limit as usize)
}

/// Returns up to `limit` most-recent reflection-memory lessons.
#[ic_cdk::query]
fn list_reflection_memory(limit: u32) -> Vec<ReflectionMemoryRecord> {
    stable::list_reflection_memory(limit as usize)
}

/// Returns memory facts filtered by key prefix, sorted by `sort`, and bounded by `limit`.
///
/// Pass an empty prefix (`""`) to list across all memory facts.
#[ic_cdk::query]
fn list_memory_facts(prefix: String, sort: MemoryFactListSort, limit: u32) -> Vec<MemoryFact> {
    let sort = memory_fact_sort_to_storage(sort);
    let trimmed_prefix = prefix.trim();
    if trimmed_prefix.is_empty() {
        stable::list_all_memory_facts_sorted(limit as usize, sort)
    } else {
        stable::list_memory_facts_by_prefix_sorted(trimmed_prefix, limit as usize, sort)
    }
}

/// Controller-only remediation endpoint to prune memory facts by prefix and/or age.
///
/// - `prefix`: optional key namespace filter (e.g. `"signal."`).
/// - `updated_before_ns`: optional inclusive timestamp cutoff.
/// - `limit`: max number of facts to remove in this call.
#[ic_cdk::update]
fn prune_memory_facts_admin(
    prefix: Option<String>,
    updated_before_ns: Option<u64>,
    limit: u32,
) -> Result<Vec<String>, String> {
    ensure_controller()?;

    let normalized_prefix = prefix
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if normalized_prefix.is_none() && updated_before_ns.is_none() {
        return Err(
            "provide at least one prune filter: prefix and/or updated_before_ns".to_string(),
        );
    }

    Ok(stable::prune_memory_facts(
        normalized_prefix,
        updated_before_ns,
        limit as usize,
    ))
}

/// Returns the full conversation log for the given sender address, or `None`
/// if no conversation exists.
#[ic_cdk::query]
fn get_conversation(sender: String) -> Option<ConversationLog> {
    sqlite::get_conversation_log(&sender).unwrap_or_else(|_| stable::get_conversation_log(&sender))
}

/// Returns aggregate inbox statistics (total messages, pending count, …).
#[ic_cdk::query]
fn get_inbox_stats() -> InboxStats {
    stable::inbox_stats()
}

/// Returns up to `limit` most-recent outbox messages (agent replies queued for delivery).
#[ic_cdk::query]
fn list_outbox_messages(limit: u32) -> Vec<OutboxMessage> {
    sqlite::list_outbox_messages(limit as usize)
        .unwrap_or_else(|_| stable::list_outbox_messages(limit as usize))
}

/// Returns aggregate outbox statistics (total messages, delivered count, …).
#[ic_cdk::query]
fn get_outbox_stats() -> OutboxStats {
    stable::outbox_stats()
}

/// Enables or disables the scheduler (controller only).
/// Disabling prevents new jobs from being dispatched without stopping the timer.
#[ic_cdk::update]
fn set_scheduler_enabled(enabled: bool) -> String {
    ensure_controller_or_trap();
    stable::set_scheduler_enabled(enabled)
}

/// Activates low-cycles mode, which throttles task dispatch to conserve cycles
/// (controller only).
#[ic_cdk::update]
fn set_scheduler_low_cycles_mode(enabled: bool) -> String {
    ensure_controller_or_trap();
    stable::set_scheduler_low_cycles_mode(enabled)
}

/// Updates the scheduler timer base tick interval in seconds (controller only).
#[ic_cdk::update]
fn set_scheduler_base_tick_secs(interval_secs: u64) -> Result<u64, String> {
    ensure_controller()?;
    let persisted = stable::set_scheduler_base_tick_secs(interval_secs)?;
    arm_timer_with_interval(persisted);
    crate::http::init_certification();
    Ok(persisted)
}

/// Overrides the recurrence interval (seconds) for the given task kind
/// (controller only).
#[ic_cdk::update]
fn set_task_interval_secs(kind: TaskKind, interval_secs: u64) -> Result<String, String> {
    ensure_controller()?;
    stable::set_task_interval_secs(&kind, interval_secs)?;
    Ok("task_interval_updated".to_string())
}

/// Enables or disables a specific task kind without affecting the scheduler
/// globally (controller only).
#[ic_cdk::update]
fn set_task_enabled(kind: TaskKind, enabled: bool) -> String {
    ensure_controller_or_trap();
    stable::set_task_enabled(&kind, enabled);
    "task_enabled_updated".to_string()
}

/// Callback endpoint used by the OpenRouter proxy worker to submit async results.
#[ic_cdk::update]
fn submit_inference_result(args: SubmitInferenceResultArgs) -> Result<String, String> {
    let caller = inference_proxy_callback_caller_principal();
    let callback_job_id = args.job_id.clone();
    let callback_turn_id = args.turn_id.clone();
    if let Err(error) = stable::assert_inference_proxy_callback_authorized(&caller) {
        stable::record_inference_proxy_callback_rejected(true);
        log!(
            InferenceProxyCallbackLogPriority::Error,
            "inference_proxy_callback_rejected caller={} job_id={} turn_id={} reason={}",
            caller,
            callback_job_id,
            callback_turn_id,
            error,
        );
        return Err(error);
    }

    let applied =
        match stable::apply_inference_proxy_callback(args, caller.clone(), current_time_ns()) {
            Ok(applied) => applied,
            Err(error) => {
                stable::record_inference_proxy_callback_rejected(false);
                log!(
                    InferenceProxyCallbackLogPriority::Error,
                    "inference_proxy_callback_rejected caller={} job_id={} turn_id={} reason={}",
                    caller,
                    callback_job_id,
                    callback_turn_id,
                    error,
                );
                return Err(error);
            }
        };
    crate::http::init_certification();
    match applied {
        crate::domain::types::InferenceProxyCallbackApply::Accepted => {
            let enqueued = enqueue_immediate_agent_turn_if_absent(
                format!("AgentTurn:inference-proxy-resume:{callback_job_id}"),
                "inference_proxy_callback_resume",
            );
            log!(
                InferenceProxyCallbackLogPriority::Info,
                "inference_proxy_callback_accepted caller={} job_id={} turn_id={} resume_job_enqueued={} resume_job_id={}",
                caller,
                callback_job_id,
                callback_turn_id,
                enqueued.is_some(),
                enqueued.unwrap_or_default(),
            );
            Ok("inference_proxy_callback_accepted".to_string())
        }
        crate::domain::types::InferenceProxyCallbackApply::Duplicate => {
            log!(
                InferenceProxyCallbackLogPriority::Info,
                "inference_proxy_callback_duplicate caller={} job_id={} turn_id={}",
                caller,
                callback_job_id,
                callback_turn_id,
            );
            Ok("inference_proxy_callback_duplicate".to_string())
        }
    }
}

/// Callback endpoint used by the strategy-discovery worker to submit staged results.
#[ic_cdk::update]
fn submit_strategy_discovery_result(
    args: SubmitStrategyDiscoveryResultArgs,
) -> Result<String, String> {
    let caller = inference_proxy_callback_caller_principal();
    let callback_job_id = args.job_id.clone();
    if let Err(error) = stable::assert_strategy_discovery_callback_authorized(&caller) {
        stable::record_strategy_discovery_callback_rejected(true);
        log!(
            StrategyDiscoveryCallbackLogPriority::DiscoveryError,
            "strategy_discovery_callback_rejected caller={} job_id={} reason={}",
            caller,
            callback_job_id,
            error,
        );
        return Err(error);
    }

    let applied =
        match stable::apply_strategy_discovery_callback(args, caller.clone(), current_time_ns()) {
            Ok(applied) => applied,
            Err(error) => {
                stable::record_strategy_discovery_callback_rejected(false);
                log!(
                    StrategyDiscoveryCallbackLogPriority::DiscoveryError,
                    "strategy_discovery_callback_rejected caller={} job_id={} reason={}",
                    caller,
                    callback_job_id,
                    error,
                );
                return Err(error);
            }
        };
    crate::http::init_certification();
    match applied {
        stable::StrategyDiscoveryCallbackApply::Accepted(status) => {
            let enqueued = enqueue_immediate_agent_turn_if_absent(
                format!("AgentTurn:strategy-discovery-resume:{callback_job_id}"),
                "strategy_discovery_callback_resume",
            );
            let outcome = match status {
                crate::domain::types::StrategyDiscoveryResultStatus::Validated => "validated",
                crate::domain::types::StrategyDiscoveryResultStatus::Rejected { .. } => "rejected",
            };
            log!(
                StrategyDiscoveryCallbackLogPriority::DiscoveryInfo,
                "strategy_discovery_callback_accepted caller={} job_id={} outcome={} resume_job_enqueued={} resume_job_id={}",
                caller,
                callback_job_id,
                outcome,
                enqueued.is_some(),
                enqueued.unwrap_or_default(),
            );
            Ok(format!("strategy_discovery_callback_{outcome}"))
        }
        stable::StrategyDiscoveryCallbackApply::Duplicate => {
            log!(
                StrategyDiscoveryCallbackLogPriority::DiscoveryInfo,
                "strategy_discovery_callback_duplicate caller={} job_id={}",
                caller,
                callback_job_id,
            );
            Ok("strategy_discovery_callback_duplicate".to_string())
        }
    }
}

fn enqueue_immediate_agent_turn_if_absent(dedupe_key: String, wake_reason: &str) -> Option<String> {
    let enqueued = crate::scheduler::enqueue_immediate_agent_turn_job_if_absent(dedupe_key);
    if enqueued.is_some() {
        schedule_immediate_scheduler_tick(wake_reason);
    }
    enqueued
}

fn schedule_immediate_scheduler_tick(reason: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        let should_spawn = SCHEDULER_WAKE_IN_FLIGHT.with(|slot| {
            let mut in_flight = slot.borrow_mut();
            if *in_flight {
                false
            } else {
                *in_flight = true;
                true
            }
        });
        if !should_spawn {
            log!(
                InferenceProxyCallbackLogPriority::Info,
                "scheduler_immediate_wake_skipped reason={} wake_already_in_flight=true",
                reason,
            );
            return;
        }
        let reason_owned = reason.to_string();
        ic_cdk::spawn(async move {
            scheduler_tick().await;
            SCHEDULER_WAKE_IN_FLIGHT.with(|slot| {
                *slot.borrow_mut() = false;
            });
            log!(
                InferenceProxyCallbackLogPriority::Info,
                "scheduler_immediate_wake_completed reason={}",
                reason_owned,
            );
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = reason;
    }
}

/// Replaces the conversation-retention policy (controller only).
#[ic_cdk::update]
fn set_retention_config(config: RetentionConfig) -> Result<RetentionConfig, String> {
    ensure_controller()?;
    stable::set_retention_config(config)
}

/// Returns the current inference configuration (provider, model, key presence flag).
#[ic_cdk::query]
fn get_inference_config() -> InferenceConfigView {
    stable::inference_config_view()
}

/// Returns the most-recent autonomous decisions, bounded by the durable FIFO cap.
#[ic_cdk::query]
fn get_recent_decisions() -> Vec<DecisionRecord> {
    stable::list_recent_decisions(stable::MAX_DECISION_RECORDS)
}

/// Returns the active strategy exposures tracked for autonomy policy enforcement.
#[ic_cdk::query]
fn get_active_exposures() -> Vec<ActiveExposure> {
    stable::list_active_exposures()
}

/// Returns the active strategy quarantines tracked for autonomy policy enforcement.
#[ic_cdk::query]
fn get_strategy_quarantines() -> Vec<StrategyQuarantine> {
    stable::list_strategy_quarantines()
}

/// Returns the last exposure-reconciliation status snapshot.
#[ic_cdk::query]
fn get_exposure_reconciliation_status() -> ExposureReconciliationStatus {
    stable::exposure_reconciliation_status()
}

#[ic_cdk::query]
fn get_inference_proxy_status() -> InferenceProxyStatusView {
    stable::inference_proxy_status_view()
}

#[ic_cdk::query]
fn get_strategy_discovery_worker_status() -> StrategyDiscoveryStatusView {
    stable::strategy_discovery_status_view(current_time_ns())
}

#[ic_cdk::query]
fn list_strategy_discovery_jobs(limit: u32) -> Vec<PendingStrategyDiscoveryJob> {
    let bounded_limit = usize::try_from(limit.max(1)).unwrap_or(25);
    stable::list_strategy_discovery_jobs(bounded_limit)
}

#[ic_cdk::query]
fn list_strategy_discovery_results(
    limit: u32,
) -> Vec<crate::domain::types::StrategyDiscoveryCallbackRecord> {
    let bounded_limit = usize::try_from(limit.max(1)).unwrap_or(25);
    stable::list_strategy_discovery_results(bounded_limit)
}

/// Returns the agent's "soul" — the core identity/persona prompt layer.
#[ic_cdk::query]
fn get_soul() -> String {
    stable::get_soul()
}

/// Replaces the agent's soul prompt.  Rejects empty strings (controller only).
#[ic_cdk::update]
fn update_soul(new_soul: String) -> Result<String, String> {
    ensure_controller()?;
    if new_soul.trim().is_empty() {
        return Err("soul cannot be empty".to_string());
    }
    Ok(stable::set_soul(new_soul))
}

/// Returns up to `limit` recent state-transition event records as debug strings.
#[ic_cdk::query]
fn list_recent_events(limit: u32) -> Vec<String> {
    sqlite::list_recent_transitions(limit as usize)
        .unwrap_or_else(|_| stable::list_recent_transitions(limit as usize))
        .into_iter()
        .map(|record| format!("{record:?}"))
        .collect()
}

/// Returns up to `limit` recent agent turn records as debug strings.
#[ic_cdk::query]
fn list_turns(limit: u32) -> Vec<String> {
    sqlite::list_turns(limit as usize)
        .unwrap_or_else(|_| stable::list_turns(limit as usize))
        .into_iter()
        .map(|turn| format!("{turn:?}"))
        .collect()
}

/// Returns all registered skill records.
#[ic_cdk::query]
fn list_skills() -> Vec<SkillRecord> {
    sqlite::list_skills().unwrap_or_else(|_| stable::list_skills())
}

/// Inserts or updates a skill record.
///
/// Only callable by a canister controller.  This is the runtime management
/// endpoint for adding new inter-canister call skills or modifying existing ones.
#[ic_cdk::update]
fn upsert_skill(skill: SkillRecord) -> Result<(), String> {
    ensure_controller()?;
    stable::upsert_skill(&skill);
    Ok(())
}

/// Returns the policy (autonomy mode, allowed callers, …) for every registered
/// tool as human-readable strings.
#[ic_cdk::query]
fn list_tool_policies() -> Vec<String> {
    let manager = ToolManager::new();
    manager
        .list_tools()
        .into_iter()
        .map(|(name, policy)| format!("{name}: {policy:?}"))
        .collect()
}

/// Returns all tool call records associated with the given turn ID.
#[ic_cdk::query]
fn get_tool_calls_for_turn(turn_id: String) -> Vec<ToolCallRecord> {
    sqlite::get_tools_for_turn(&turn_id).unwrap_or_else(|_| stable::get_tools_for_turn(&turn_id))
}

// ── Strategy management ──────────────────────────────────────────────────────

/// Lists strategy templates.  When `key` is supplied only templates matching
/// that key are returned; otherwise all templates are returned (up to `limit`).
#[ic_cdk::query]
fn list_strategy_templates(key: Option<StrategyTemplateKey>, limit: u32) -> Vec<StrategyTemplate> {
    let bounded_limit = limit.max(1) as usize;
    match key {
        Some(key) => crate::strategy::registry::list_templates(&key, bounded_limit),
        None => crate::strategy::registry::list_all_templates(bounded_limit),
    }
}

/// Returns a single strategy template by key, or `None` if absent.
#[ic_cdk::query]
fn get_strategy_template(key: StrategyTemplateKey) -> Option<StrategyTemplate> {
    crate::strategy::registry::get_template(&key)
}

/// Returns accumulated outcome statistics (success/failure counts, last outcome)
/// for the given template, or `None` if no executions have been recorded yet.
#[ic_cdk::query]
fn get_strategy_outcome_stats(key: StrategyTemplateKey) -> Option<StrategyOutcomeStats> {
    crate::strategy::learner::outcome_stats(&key)
}

/// Register a strategy from a compact recipe (controller only).
///
/// Replaces the previous multi-step workflow (`ingest_strategy_template_admin` +
/// `ingest_strategy_abi_artifact_admin` + `activate_strategy_template_admin`) with
/// a single call.  Accepts the same JSON recipe format as the agent's
/// `register_strategy` tool.
#[ic_cdk::update]
fn register_strategy_admin(recipe_json: String) -> Result<StrategyTemplate, String> {
    ensure_controller()?;
    let recipe: crate::strategy::registry::StrategyRecipe = serde_json::from_str(&recipe_json)
        .map_err(|error| format!("invalid strategy recipe JSON: {error}"))?;
    let result = crate::strategy::registry::register_from_recipe(recipe)?;
    Ok(result.template)
}

/// Deprecated: use `register_strategy_admin` instead.
/// Inserts or updates a strategy template (controller only).
#[ic_cdk::update]
fn ingest_strategy_template_admin(template: StrategyTemplate) -> Result<StrategyTemplate, String> {
    ensure_controller()?;
    let mut template = template;
    let now_ns = current_time_ns();
    if template.created_at_ns == 0 {
        template.created_at_ns = now_ns;
    }
    template.updated_at_ns = now_ns;
    crate::strategy::registry::upsert_template(template)
}

/// Deprecated: use `register_strategy_admin` instead.
/// Normalises and stores an ABI artifact, optionally verifying selector hashes
/// (controller only).
#[ic_cdk::update]
fn ingest_strategy_abi_artifact_admin(args: StrategyAbiIngestArgs) -> Result<AbiArtifact, String> {
    ensure_controller()?;
    crate::strategy::abi::normalize_and_store_abi_artifact(
        args.key,
        &args.abi_json,
        &args.source_ref,
        args.codehash,
        &args.selector_assertions,
        current_time_ns(),
    )
}

/// Deprecated: use `register_strategy_admin` instead.
/// Transitions a template to `Active`, runs a dry-run compile to validate it,
/// and records an activation state entry (controller only).
#[ic_cdk::update]
fn activate_strategy_template_admin(
    key: StrategyTemplateKey,
    reason: Option<String>,
) -> Result<TemplateActivationState, String> {
    ensure_controller()?;
    let _template = upsert_template_status(key.clone(), TemplateStatus::Active)?;
    crate::strategy::compiler::dry_run_compile(&key)?;
    crate::strategy::registry::set_activation(TemplateActivationState {
        key,
        enabled: true,
        updated_at_ns: current_time_ns(),
        reason: reason.or_else(|| Some("controller activation after dry-run compile".to_string())),
    })
}

/// Marks a template as `Deprecated` and deactivates it.  Use this for orderly
/// rotation; the template remains readable (controller only).
#[ic_cdk::update]
fn deprecate_strategy_template_admin(
    key: StrategyTemplateKey,
    reason: Option<String>,
) -> Result<StrategyTemplate, String> {
    ensure_controller()?;
    let template = upsert_template_status(key.clone(), TemplateStatus::Deprecated)?;
    let _ = crate::strategy::registry::set_activation(TemplateActivationState {
        key,
        enabled: false,
        updated_at_ns: current_time_ns(),
        reason,
    });
    Ok(template)
}

/// Hard-revokes a template: sets status to `Revoked`, deactivates it, and
/// records an immutable revocation entry.  Use for security incidents
/// (controller only).
#[ic_cdk::update]
fn revoke_strategy_template_admin(
    key: StrategyTemplateKey,
    reason: Option<String>,
) -> Result<TemplateRevocationState, String> {
    ensure_controller()?;
    let _ = upsert_template_status(key.clone(), TemplateStatus::Revoked)?;
    let now_ns = current_time_ns();
    let revocation = crate::strategy::registry::set_revocation(TemplateRevocationState {
        key: key.clone(),
        revoked: true,
        updated_at_ns: now_ns,
        reason: reason.clone(),
    })?;
    let _ = crate::strategy::registry::set_activation(TemplateActivationState {
        key,
        enabled: false,
        updated_at_ns: now_ns,
        reason: reason.or_else(|| Some("revoked".to_string())),
    });
    Ok(revocation)
}

/// Arms or disarms the kill switch for a strategy template.
/// When `enabled` is `true` the agent will refuse to execute any action for
/// that template (controller only).
#[ic_cdk::update]
fn set_strategy_kill_switch_admin(
    key: StrategyTemplateKey,
    enabled: bool,
    reason: Option<String>,
) -> Result<StrategyKillSwitchState, String> {
    ensure_controller()?;
    crate::strategy::registry::set_kill_switch(StrategyKillSwitchState {
        key,
        enabled,
        updated_at_ns: current_time_ns(),
        reason,
    })
}

// ── HTTP interface ───────────────────────────────────────────────────────────

/// Certified HTTP query handler.  Serves static UI assets and read-only API
/// routes from the pre-built certification tree.  Mutable routes return an
/// upgrade signal to be retried via `http_request_update`.
#[ic_cdk::query]
fn http_request(request: HttpRequest) -> HttpResponse {
    crate::http::handle_http_request(request)
}

/// Mutable HTTP update handler for write routes (`POST /api/conversation`, …).
/// Called automatically by the IC boundary nodes when `http_request` signals
/// an upgrade.
#[ic_cdk::update]
async fn http_request_update(request: HttpUpdateRequest<'_>) -> HttpUpdateResponse<'static> {
    crate::http::handle_http_request_update(request).await
}

/// Registers the recurring scheduler timer.  Called from both `init` and
/// `post_upgrade` so the timer is never left unarmed after an upgrade.
fn arm_timer() {
    arm_timer_with_interval(stable::get_scheduler_base_tick_secs());
}

fn arm_timer_with_interval(interval_secs: u64) {
    #[cfg(target_arch = "wasm32")]
    {
        let interval = std::time::Duration::from_secs(interval_secs.max(1));
        SCHEDULER_TIMER_ID.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(existing) = slot.take() {
                clear_timer(existing);
            }
            *slot = Some(set_timer_interval_serial(interval, scheduler_tick));
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = interval_secs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{
        AbiFunctionSpec, AbiTypeSpec, ActionSpec, ContractRoleBinding, InboxMessageSource,
        InferenceProxyResultPayload, InferenceTransport, MemoryFact, OpenRouterProxyWorkerConfig,
        OpenRouterReasoningLevel, PendingInferenceProxyJob, SkillRecord, StewardNonceState,
        StrategyTemplate, StrategyTemplateKey, SubmitInferenceResultArgs, TemplateStatus,
    };
    use sha3::{Digest, Keccak256};
    use std::collections::{BTreeMap, BTreeSet};

    fn spawn_bootstrap_provider_args(
        inference_transport: InferenceTransport,
        open_router_reasoning_level: OpenRouterReasoningLevel,
    ) -> SpawnProviderBootstrapArgs {
        SpawnProviderBootstrapArgs {
            open_router_api_key: Some(" sk-or-test ".to_string()),
            model: Some(" openai/gpt-4o-mini ".to_string()),
            brave_search_api_key: Some(" brave-test-key ".to_string()),
            inference_transport,
            open_router_reasoning_level,
        }
    }

    fn sample_spawn_bootstrap_args(provider: SpawnProviderBootstrapArgs) -> SpawnBootstrapArgs {
        SpawnBootstrapArgs {
            steward_address: "0x62dAFfDC4D59eA05fedDb0a77A266B0a7b6F28ca".to_string(),
            session_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            parent_id: Some("parent-automaton".to_string()),
            factory_principal: Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                .expect("test principal should parse"),
            risk: 4,
            strategies: vec![" carry ".to_string(), "".to_string()],
            skills: vec![" messaging ".to_string()],
            provider,
            version_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
        }
    }

    fn sample_init_args(spawn_bootstrap: Option<SpawnBootstrapArgs>) -> InitArgs {
        InitArgs {
            ecdsa_key_name: "dfx_test_key".to_string(),
            inbox_contract_address: None,
            evm_chain_id: Some(31337),
            evm_rpc_url: None,
            evm_confirmation_depth: None,
            evm_bootstrap_lookback_blocks: None,
            http_allowed_domains: None,
            llm_canister_id: None,
            search_api_key: None,
            inference_proxy_worker_base_url: None,
            inference_proxy_trusted_callback_principal: None,
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
            spawn_bootstrap,
        }
    }

    fn signing_key_from_hex(hex_key: &str) -> k256::ecdsa::SigningKey {
        let mut secret_key = [0u8; 32];
        hex::decode_to_slice(hex_key, &mut secret_key).expect("hex private key should decode");
        k256::ecdsa::SigningKey::from_bytes((&secret_key).into())
            .expect("test private key should parse")
    }

    fn steward_test_signing_key() -> k256::ecdsa::SigningKey {
        signing_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318")
    }

    fn steward_address_from_key(signing_key: &k256::ecdsa::SigningKey) -> String {
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let digest = Keccak256::digest(&uncompressed.as_bytes()[1..]);
        format!("0x{}", hex::encode(&digest[12..32]))
    }

    fn canonical_steward_signing_payload(
        canister_id: &str,
        chain_id: u64,
        address: &str,
        command_hash: &str,
        nonce: u64,
        expires_at_ns: u64,
    ) -> String {
        format!(
            "ic-automaton:steward-execute:v1\ncanister_id:{canister_id}\nchain_id:{chain_id}\naddress:{address}\ncommand_hash:{command_hash}\nnonce:{nonce}\nexpires_at_ns:{expires_at_ns}"
        )
    }

    fn ethereum_personal_message_hash(message: &str) -> [u8; 32] {
        let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
        let mut hasher = Keccak256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(message.as_bytes());
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        out
    }

    fn sign_steward_payload(payload: &str, signing_key: &k256::ecdsa::SigningKey) -> String {
        let prehash = ethereum_personal_message_hash(payload);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&prehash)
            .expect("test payload should sign");

        let mut bytes = [0u8; 65];
        bytes[..64].copy_from_slice(signature.to_bytes().as_slice());
        bytes[64] = recovery_id.to_byte() + 27;
        format!("0x{}", hex::encode(bytes))
    }

    fn build_steward_proof(
        command: &StewardCommand,
        signing_key: &k256::ecdsa::SigningKey,
        nonce: u64,
        expires_at_ns: u64,
    ) -> EvmStewardProof {
        let canister_id = steward_proof_expected_canister_id();
        let chain_id = 8453;
        let normalized_address = steward_address_from_key(signing_key);
        build_steward_proof_for_address(
            command,
            &normalized_address,
            signing_key,
            nonce,
            expires_at_ns,
            &canister_id,
            chain_id,
        )
    }

    fn build_steward_proof_for_address(
        command: &StewardCommand,
        address: &str,
        signing_key: &k256::ecdsa::SigningKey,
        nonce: u64,
        expires_at_ns: u64,
        canister_id: &str,
        chain_id: u64,
    ) -> EvmStewardProof {
        let normalized_address = address.trim().to_ascii_lowercase();
        let command_hash = steward_command_hash(command).expect("command hash should encode");
        let payload = canonical_steward_signing_payload(
            canister_id,
            chain_id,
            &normalized_address,
            &command_hash,
            nonce,
            expires_at_ns,
        );
        EvmStewardProof {
            canister_id: canister_id.to_string(),
            chain_id,
            address: normalized_address.to_ascii_uppercase(),
            command_hash,
            nonce,
            expires_at_ns,
            signature: sign_steward_payload(&payload, signing_key),
        }
    }

    fn execute_steward_command(
        command: StewardCommand,
        proof: EvmStewardProof,
    ) -> Result<String, String> {
        futures::executor::block_on(steward_execute(command, proof))
    }

    fn execute_steward_ingress_command(
        caller: Option<Principal>,
        command: StewardCommand,
    ) -> Result<String, String> {
        set_steward_ingress_caller_for_tests(caller);
        let result = futures::executor::block_on(steward_execute_ingress(command));
        set_steward_ingress_caller_for_tests(None);
        result
    }

    #[test]
    fn controller_gated_updates_have_steward_command_parity() {
        let source = include_str!("lib.rs");
        let lines: Vec<&str> = source.lines().collect();
        let mut controller_gated_updates = BTreeSet::new();

        for (idx, line) in lines.iter().enumerate() {
            if line.trim() != "#[ic_cdk::update]" {
                continue;
            }

            let mut signature_idx = None;
            for look_ahead in 1..=8 {
                let Some(candidate) = lines.get(idx + look_ahead) else {
                    break;
                };
                let candidate = candidate.trim_start();
                if candidate.starts_with("fn ") || candidate.starts_with("async fn ") {
                    signature_idx = Some(idx + look_ahead);
                    break;
                }
            }

            let Some(signature_idx) = signature_idx else {
                continue;
            };
            let signature = lines[signature_idx].trim_start();
            let function_name = signature
                .strip_prefix("fn ")
                .or_else(|| signature.strip_prefix("async fn "))
                .and_then(|rest| rest.split('(').next())
                .map(str::trim);
            let Some(function_name) = function_name else {
                continue;
            };

            let mut is_controller_gated = false;
            for look_ahead in 1..=20 {
                let Some(candidate) = lines.get(signature_idx + look_ahead) else {
                    break;
                };
                let candidate = candidate.trim();
                if candidate.starts_with("#[ic_cdk::") {
                    break;
                }
                if candidate.contains("ensure_controller_or_trap();")
                    || candidate.contains("ensure_controller()?;")
                {
                    is_controller_gated = true;
                    break;
                }
            }

            if is_controller_gated {
                controller_gated_updates.insert(function_name.to_string());
            }
        }

        // update_autonomy_policy uses ensure_controller() (returns Result) rather than
        // ensure_controller_or_trap(), so it has no corresponding StewardCommand mapping.
        let controller_update_exceptions = BTreeSet::from([String::from("update_autonomy_policy")]);
        let controller_gated_updates = controller_gated_updates
            .difference(&controller_update_exceptions)
            .cloned()
            .collect::<BTreeSet<_>>();

        let method_to_command_label = BTreeMap::from([
            ("set_loop_enabled", "set_loop_enabled"),
            (
                "set_autonomy_tool_dedupe_enabled",
                "set_autonomy_tool_dedupe_enabled",
            ),
            (
                "set_autonomy_suppression_config",
                "set_autonomy_suppression_config",
            ),
            ("set_inference_provider", "set_inference_provider"),
            ("set_inference_model", "set_inference_model"),
            ("set_openrouter_base_url", "set_openrouter_base_url"),
            ("set_openrouter_api_key", "set_openrouter_api_key"),
            ("configure_search", "configure_search"),
            (
                "set_openrouter_reasoning_level",
                "set_openrouter_reasoning_level",
            ),
            ("set_inference_proxy_config", "set_inference_proxy_config"),
            (
                "set_strategy_discovery_worker_config",
                "set_strategy_discovery_worker_config",
            ),
            ("set_welcome_message", "set_welcome_message"),
            ("set_evm_rpc_url", "set_evm_rpc_url"),
            ("set_evm_rpc_fallback_url", "set_evm_rpc_fallback_url"),
            (
                "set_evm_rpc_max_response_bytes",
                "set_evm_rpc_max_response_bytes",
            ),
            (
                "set_inbox_contract_address_admin",
                "set_inbox_contract_address",
            ),
            ("set_steward_admin", "update_steward"),
            ("set_evm_chain_id_admin", "set_evm_chain_id"),
            (
                "set_evm_confirmation_depth_admin",
                "set_evm_confirmation_depth",
            ),
            (
                "derive_automaton_evm_address",
                "derive_automaton_evm_address",
            ),
            ("set_http_allowed_domains", "set_http_allowed_domains"),
            ("update_prompt_layer_admin", "update_prompt_layer"),
            ("prune_memory_facts_admin", "prune_memory_facts"),
            ("set_scheduler_enabled", "set_scheduler_enabled"),
            (
                "set_scheduler_low_cycles_mode",
                "set_scheduler_low_cycles_mode",
            ),
            (
                "set_scheduler_base_tick_secs",
                "set_scheduler_base_tick_secs",
            ),
            ("set_task_interval_secs", "set_task_interval_secs"),
            ("set_task_enabled", "set_task_enabled"),
            ("set_retention_config", "set_retention_config"),
            ("update_soul", "update_soul"),
            ("upsert_skill", "upsert_skill"),
            (
                "enqueue_strategy_discovery_job_admin",
                "enqueue_strategy_discovery_job",
            ),
            (
                "promote_discovery_protocol_artifacts_admin",
                "promote_discovery_protocol_artifacts",
            ),
            ("register_strategy_admin", "register_strategy"),
            ("ingest_strategy_template_admin", "ingest_strategy_template"),
            (
                "ingest_strategy_abi_artifact_admin",
                "ingest_strategy_abi_artifact",
            ),
            (
                "activate_strategy_template_admin",
                "activate_strategy_template",
            ),
            (
                "deprecate_strategy_template_admin",
                "deprecate_strategy_template",
            ),
            ("revoke_strategy_template_admin", "revoke_strategy_template"),
            ("set_strategy_kill_switch_admin", "set_strategy_kill_switch"),
        ]);
        let mapped_methods: BTreeSet<String> = method_to_command_label
            .keys()
            .map(|method| (*method).to_string())
            .collect();

        let missing_mappings: Vec<String> = controller_gated_updates
            .difference(&mapped_methods)
            .cloned()
            .collect();
        assert!(
            missing_mappings.is_empty(),
            "controller-gated update methods missing steward command parity mappings: {missing_mappings:?}"
        );

        let stale_mappings: Vec<String> = mapped_methods
            .difference(&controller_gated_updates)
            .cloned()
            .collect();
        assert!(
            stale_mappings.is_empty(),
            "stale steward parity mappings without controller-gated update methods: {stale_mappings:?}"
        );

        let mut command_labels_from_match = BTreeSet::new();
        for (idx, line) in lines.iter().enumerate() {
            let line = line.trim();
            if !line.starts_with("StewardCommand::") {
                continue;
            }

            for look_ahead in 0..=3 {
                let Some(candidate) = lines.get(idx + look_ahead) else {
                    break;
                };
                let candidate = candidate.trim();
                if let Some((_, rhs)) = candidate.split_once("=> \"") {
                    if let Some((label, _)) = rhs.split_once('"') {
                        command_labels_from_match.insert(label);
                        break;
                    }
                }

                if let Some(stripped) = candidate.strip_prefix('"') {
                    if let Some((label, _)) = stripped.split_once('"') {
                        command_labels_from_match.insert(label);
                        break;
                    }
                }
            }
        }
        let unknown_labels: Vec<String> = method_to_command_label
            .iter()
            .filter_map(|(method, label)| {
                if command_labels_from_match.contains(label) {
                    None
                } else {
                    Some(format!("{method} -> {label}"))
                }
            })
            .collect();
        assert!(
            unknown_labels.is_empty(),
            "steward parity mappings reference unknown command labels: {unknown_labels:?}"
        );
    }

    #[test]
    fn get_automaton_evm_address_query_returns_stored_value() {
        stable::init_storage();
        let expected = "0x1111111111111111111111111111111111111111".to_string();
        stable::set_evm_address(Some(expected.clone())).expect("automaton address should store");

        assert_eq!(get_automaton_evm_address(), Some(expected));
    }

    #[test]
    fn apply_init_args_can_seed_http_allowlist() {
        apply_init_args(InitArgs {
            ecdsa_key_name: "dfx_test_key".to_string(),
            inbox_contract_address: None,
            evm_chain_id: None,
            evm_rpc_url: None,
            evm_confirmation_depth: None,
            evm_bootstrap_lookback_blocks: None,
            http_allowed_domains: Some(vec!["api.coingecko.com".to_string()]),
            llm_canister_id: None,
            search_api_key: None,
            inference_proxy_worker_base_url: None,
            inference_proxy_trusted_callback_principal: None,
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
            spawn_bootstrap: None,
        });

        assert!(stable::is_http_allowlist_enforced());
        assert_eq!(
            stable::list_allowed_http_domains(),
            vec!["api.coingecko.com".to_string()]
        );
    }

    #[test]
    fn apply_init_args_can_set_llm_canister_id() {
        apply_init_args(InitArgs {
            ecdsa_key_name: "dfx_test_key".to_string(),
            inbox_contract_address: None,
            evm_chain_id: None,
            evm_rpc_url: None,
            evm_confirmation_depth: None,
            evm_bootstrap_lookback_blocks: None,
            http_allowed_domains: None,
            llm_canister_id: Some(
                Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai")
                    .expect("test principal should parse"),
            ),
            search_api_key: None,
            inference_proxy_worker_base_url: None,
            inference_proxy_trusted_callback_principal: None,
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
            spawn_bootstrap: None,
        });

        assert_eq!(stable::get_llm_canister_id(), "w36hm-eqaaa-aaaal-qr76a-cai");
    }

    #[test]
    fn apply_init_args_can_set_search_api_key() {
        apply_init_args(InitArgs {
            ecdsa_key_name: "dfx_test_key".to_string(),
            inbox_contract_address: None,
            evm_chain_id: None,
            evm_rpc_url: None,
            evm_confirmation_depth: None,
            evm_bootstrap_lookback_blocks: None,
            http_allowed_domains: None,
            llm_canister_id: None,
            search_api_key: Some("brave-test-key".to_string()),
            inference_proxy_worker_base_url: None,
            inference_proxy_trusted_callback_principal: None,
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
            spawn_bootstrap: None,
        });

        assert_eq!(
            stable::get_search_api_key(),
            Some("brave-test-key".to_string())
        );
    }

    #[test]
    fn apply_init_args_can_override_cycle_topup_controls() {
        apply_init_args(InitArgs {
            ecdsa_key_name: "dfx_test_key".to_string(),
            inbox_contract_address: None,
            evm_chain_id: None,
            evm_rpc_url: None,
            evm_confirmation_depth: None,
            evm_bootstrap_lookback_blocks: None,
            http_allowed_domains: None,
            llm_canister_id: None,
            search_api_key: None,
            inference_proxy_worker_base_url: None,
            inference_proxy_trusted_callback_principal: None,
            cycle_topup_enabled: Some(false),
            auto_topup_cycle_threshold: Some(150_000_000_000),
            spawn_bootstrap: None,
        });

        let snapshot = stable::runtime_snapshot();
        assert!(!snapshot.cycle_topup.enabled);
        assert_eq!(
            snapshot.cycle_topup.auto_topup_cycle_threshold,
            150_000_000_000
        );
    }

    #[test]
    fn apply_init_args_can_override_evm_bootstrap_lookback_blocks() {
        apply_init_args(InitArgs {
            ecdsa_key_name: "dfx_test_key".to_string(),
            inbox_contract_address: None,
            evm_chain_id: None,
            evm_rpc_url: None,
            evm_confirmation_depth: None,
            evm_bootstrap_lookback_blocks: Some(0),
            http_allowed_domains: None,
            llm_canister_id: None,
            search_api_key: None,
            inference_proxy_worker_base_url: None,
            inference_proxy_trusted_callback_principal: None,
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
            spawn_bootstrap: None,
        });

        let snapshot = stable::runtime_snapshot();
        assert_eq!(snapshot.evm_bootstrap_lookback_blocks, 0);
    }

    #[test]
    fn apply_init_args_can_apply_spawn_bootstrap() {
        apply_init_args(sample_init_args(Some(sample_spawn_bootstrap_args(
            spawn_bootstrap_provider_args(
                InferenceTransport::OpenrouterDirect,
                OpenRouterReasoningLevel::High,
            ),
        ))));

        let steward = stable::active_steward().expect("spawn bootstrap should configure steward");
        assert_eq!(steward.chain_id, 31337);
        assert_eq!(
            steward.address,
            "0x62daffdc4d59ea05feddb0a77a266b0a7b6f28ca"
        );
        assert!(steward.enabled);

        let snapshot = stable::runtime_snapshot();
        assert_eq!(snapshot.inference_provider, InferenceProvider::OpenRouter);
        assert_eq!(snapshot.inference_model, "openai/gpt-4o-mini");
        assert_eq!(snapshot.openrouter_api_key, Some("sk-or-test".to_string()));
        assert_eq!(
            snapshot.openrouter_reasoning_level,
            OpenRouterReasoningLevel::High
        );
        assert_eq!(
            snapshot.factory_principal,
            Some("rrkah-fqaaa-aaaaa-aaaaq-cai".to_string())
        );
        assert_eq!(
            stable::get_search_api_key(),
            Some("brave-test-key".to_string())
        );

        let bootstrap = get_spawn_bootstrap_view();
        assert_eq!(
            bootstrap,
            SpawnBootstrapView {
                session_id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
                parent_id: Some("parent-automaton".to_string()),
                factory_principal: Some(
                    Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                        .expect("test principal should parse"),
                ),
                risk: Some(4),
                strategies: vec!["carry".to_string()],
                skills: vec!["messaging".to_string()],
                version_commit: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            }
        );
    }

    #[test]
    fn apply_init_args_can_apply_spawn_bootstrap_for_proxy_transport() {
        let trusted = Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai")
            .expect("trusted principal should parse");
        let mut args = sample_init_args(Some(sample_spawn_bootstrap_args(
            spawn_bootstrap_provider_args(
                InferenceTransport::OpenrouterProxyWorker,
                OpenRouterReasoningLevel::Medium,
            ),
        )));
        args.inference_proxy_worker_base_url = Some(" https://proxy.example.workers.dev/ ".into());
        args.inference_proxy_trusted_callback_principal = Some(trusted);

        apply_init_args(args);

        let snapshot = stable::runtime_snapshot();
        assert_eq!(
            snapshot.inference_provider,
            InferenceProvider::OpenRouterProxyWorker
        );
        assert_eq!(
            snapshot.openrouter_reasoning_level,
            OpenRouterReasoningLevel::Medium
        );
        assert_eq!(
            snapshot.openrouter_proxy,
            OpenRouterProxyWorkerConfig {
                worker_base_url: "https://proxy.example.workers.dev".to_string(),
                trusted_callback_principal: Some(trusted),
            }
        );
    }

    #[test]
    fn apply_init_args_spawn_bootstrap_proxy_transport_requires_proxy_config() {
        stable::init_storage();
        let _ =
            stable::set_ecdsa_key_name("dfx_test_key".to_string()).expect("ecdsa key should store");
        let _ = stable::set_evm_chain_id(31337).expect("evm chain id should store");

        let message =
            apply_spawn_bootstrap(sample_spawn_bootstrap_args(spawn_bootstrap_provider_args(
                InferenceTransport::OpenrouterProxyWorker,
                OpenRouterReasoningLevel::Default,
            )))
            .expect_err("missing proxy config must reject bootstrap");
        assert!(
            message.contains("inference_proxy_worker_base_url"),
            "unexpected bootstrap error: {message}"
        );
    }

    #[test]
    fn set_steward_admin_sets_normalized_state_and_resets_nonce_on_rotation() {
        stable::init_storage();
        let _ = stable::set_steward_nonce_state(crate::domain::types::StewardNonceState {
            next_nonce: 42,
        });

        let stored = set_steward_admin(
            8453,
            "0xABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD".to_string(),
            true,
        )
        .expect("steward should persist");

        assert_eq!(stored.chain_id, 8453);
        assert_eq!(stored.address, "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd");
        assert!(stored.enabled);
        assert!(stored.last_used_at_ns.is_none());

        let status = get_steward_status();
        assert_eq!(status.active_steward, Some(stored));
        assert_eq!(status.next_nonce, 0);
    }

    #[test]
    fn set_steward_admin_rejects_invalid_identity_inputs() {
        stable::init_storage();

        let invalid_chain = set_steward_admin(
            0,
            "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string(),
            true,
        )
        .expect_err("chain id 0 must fail");
        assert!(invalid_chain.contains("steward chain id"));

        let invalid_address = set_steward_admin(8453, "not-an-address".to_string(), true)
            .expect_err("invalid address must fail");
        assert!(invalid_address.contains("steward address"));
    }

    #[test]
    fn steward_execute_accepts_valid_proof_and_advances_nonce() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        let stored =
            set_steward_admin(8453, address.clone(), true).expect("active steward should store");
        assert_eq!(stored.address, address);
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 7 });

        let command = StewardCommand::Noop;
        let proof = build_steward_proof(&command, &key, 7, current_time_ns() + 60_000_000_000);

        let result = execute_steward_command(command, proof).expect("proof should execute");
        assert_eq!(result, "steward_noop_executed");

        let status = get_steward_status();
        assert_eq!(status.next_nonce, 8);
        let steward = status
            .active_steward
            .expect("steward state should remain configured");
        assert_eq!(steward.address, address);
        assert!(steward.last_used_at_ns.is_some());
    }

    #[test]
    fn steward_execute_rejects_replayed_nonce() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        set_steward_admin(8453, address, true).expect("active steward should store");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 7 });

        let command = StewardCommand::Noop;
        let proof = build_steward_proof(&command, &key, 7, current_time_ns() + 60_000_000_000);
        execute_steward_command(command.clone(), proof.clone())
            .expect("first execution should pass");

        let replay_error =
            execute_steward_command(command, proof).expect_err("replayed proof nonce should fail");
        assert!(replay_error.contains("proof nonce mismatch"));
        assert_eq!(get_steward_status().next_nonce, 8);
    }

    #[test]
    fn steward_execute_update_steward_rotates_identity_and_resets_nonce() {
        stable::init_storage();
        let old_key = steward_test_signing_key();
        let old_address = steward_address_from_key(&old_key);
        set_steward_admin(8453, old_address, true).expect("active steward should store");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 9 });

        let new_key =
            k256::ecdsa::SigningKey::from_slice(&[2u8; 32]).expect("test signing key should build");
        let new_address = steward_address_from_key(&new_key);
        let command = StewardCommand::UpdateSteward {
            chain_id: 8453,
            address: new_address.clone(),
            enabled: true,
        };
        let proof = build_steward_proof(&command, &old_key, 9, current_time_ns() + 60_000_000_000);

        let result =
            execute_steward_command(command, proof).expect("rotation command should execute");
        assert_eq!(result, "steward_update_steward_executed");
        let status = get_steward_status();
        assert_eq!(status.next_nonce, 0);
        let active = status
            .active_steward
            .expect("new steward should be configured");
        assert_eq!(active.address, new_address);
        assert!(active.enabled);
        assert!(active.last_used_at_ns.is_none());

        let new_proof = build_steward_proof(
            &StewardCommand::Noop,
            &new_key,
            0,
            current_time_ns() + 60_000_000_000,
        );
        let new_result = execute_steward_command(StewardCommand::Noop, new_proof)
            .expect("new steward should execute");
        assert_eq!(new_result, "steward_noop_executed");
        assert_eq!(get_steward_status().next_nonce, 1);
    }

    #[test]
    fn steward_execute_set_loop_enabled_dispatches_to_runtime_mutator() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        set_steward_admin(8453, address, true).expect("active steward should store");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 3 });
        stable::set_loop_enabled(false);

        let command = StewardCommand::SetLoopEnabled { enabled: true };
        let proof = build_steward_proof(&command, &key, 3, current_time_ns() + 60_000_000_000);
        let result =
            execute_steward_command(command, proof).expect("set loop command should execute");

        assert_eq!(result, "loop_enabled=true");
        assert!(stable::runtime_snapshot().loop_enabled);
        assert_eq!(get_steward_status().next_nonce, 4);
    }

    #[test]
    fn steward_execute_ingress_accepts_authorized_principal_without_nonce_progress() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        set_steward_admin(8453, address, true).expect("active steward should store");
        let linked_principal =
            Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai").expect("principal should parse");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 11 });

        let link_command = StewardCommand::SetPrincipal {
            principal: Some(linked_principal),
        };
        let link_proof =
            build_steward_proof(&link_command, &key, 11, current_time_ns() + 60_000_000_000);
        let link_result = execute_steward_command(link_command, link_proof)
            .expect("set principal should execute");
        assert_eq!(link_result, "steward_principal=w36hm-eqaaa-aaaal-qr76a-cai");
        assert_eq!(get_steward_status().next_nonce, 12);

        stable::set_loop_enabled(false);
        let ingress_result = execute_steward_ingress_command(
            Some(linked_principal),
            StewardCommand::SetLoopEnabled { enabled: true },
        )
        .expect("authorized ingress principal should execute");
        assert_eq!(ingress_result, "loop_enabled=true");
        assert_eq!(get_steward_status().next_nonce, 12);
        assert!(stable::runtime_snapshot().loop_enabled);
    }

    #[test]
    fn steward_execute_ingress_rejects_non_matching_principal() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        set_steward_admin(8453, address, true).expect("active steward should store");
        let linked_principal =
            Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai").expect("principal should parse");
        let attacker_principal = Principal::self_authenticating(b"attacker-principal");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 0 });
        let link_command = StewardCommand::SetPrincipal {
            principal: Some(linked_principal),
        };
        let link_proof =
            build_steward_proof(&link_command, &key, 0, current_time_ns() + 60_000_000_000);
        execute_steward_command(link_command, link_proof).expect("set principal should execute");

        let error = execute_steward_ingress_command(Some(attacker_principal), StewardCommand::Noop)
            .expect_err("non-matching principal must be rejected");
        assert!(error.contains("caller principal does not match active steward principal"));
    }

    #[test]
    fn steward_execute_ingress_update_steward_clears_principal_on_rotation() {
        stable::init_storage();
        let old_key = steward_test_signing_key();
        let old_address = steward_address_from_key(&old_key);
        set_steward_admin(8453, old_address, true).expect("active steward should store");
        let linked_principal =
            Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai").expect("principal should parse");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 0 });
        let link_command = StewardCommand::SetPrincipal {
            principal: Some(linked_principal),
        };
        let link_proof = build_steward_proof(
            &link_command,
            &old_key,
            0,
            current_time_ns() + 60_000_000_000,
        );
        execute_steward_command(link_command, link_proof).expect("set principal should execute");

        let new_key =
            k256::ecdsa::SigningKey::from_slice(&[2u8; 32]).expect("test signing key should build");
        let new_address = steward_address_from_key(&new_key);
        let rotate_result = execute_steward_ingress_command(
            Some(linked_principal),
            StewardCommand::UpdateSteward {
                chain_id: 8453,
                address: new_address.clone(),
                enabled: true,
            },
        )
        .expect("authorized ingress principal should rotate steward");
        assert_eq!(rotate_result, "steward_update_steward_executed");

        let status = get_steward_status();
        assert_eq!(status.next_nonce, 0);
        let active = status
            .active_steward
            .expect("rotated steward should remain configured");
        assert_eq!(active.address, new_address);
        assert_eq!(active.principal, None);

        let rejected =
            execute_steward_ingress_command(Some(linked_principal), StewardCommand::Noop)
                .expect_err("principal must be relinked after steward rotation");
        assert!(rejected.contains("active steward principal is not configured"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn steward_execute_accepts_eip1271_contract_wallet_signature() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let contract_address = "0x8f61f9a23b6bad39a85623fff41cbee5b4dbee2c".to_string();
        set_steward_admin(8453, contract_address.clone(), true)
            .expect("active steward should store");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 1 });

        let command = StewardCommand::Noop;
        let canister_id = steward_proof_expected_canister_id();
        let proof = build_steward_proof_for_address(
            &command,
            &contract_address,
            &key,
            1,
            current_time_ns() + 60_000_000_000,
            &canister_id,
            8453,
        );
        let result =
            execute_steward_command(command, proof).expect("eip1271 fallback proof should execute");
        assert_eq!(result, "steward_noop_executed");
        assert_eq!(get_steward_status().next_nonce, 2);
    }

    #[test]
    fn steward_execute_send_steward_message_ingests_steward_direct_inbox_message() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        set_steward_admin(8453, address.clone(), true).expect("active steward should store");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 2 });

        let command = StewardCommand::SendStewardMessage {
            sender: address,
            message: "ship autonomous revenue plan".to_string(),
        };
        let proof = build_steward_proof(&command, &key, 2, current_time_ns() + 60_000_000_000);
        let result = execute_steward_command(command, proof)
            .expect("send steward message command should execute");
        assert!(
            result.starts_with("steward_direct_message_ingested id=inbox:"),
            "unexpected dispatch result: {result}"
        );

        let inbox = stable::list_inbox_messages(1);
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].body, "ship autonomous revenue plan");
        assert_eq!(inbox[0].source, InboxMessageSource::StewardDirect);
        assert_eq!(stable::inbox_stats().staged_count, 1);
        assert_eq!(get_steward_status().next_nonce, 3);
    }

    #[test]
    fn steward_send_enqueues_immediate_agent_turn_when_proxy_callback_not_pending() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        set_steward_admin(8453, address.clone(), true).expect("active steward should store");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 0 });

        let command = StewardCommand::SendStewardMessage {
            sender: address,
            message: "kick next turn now".to_string(),
        };
        let proof = build_steward_proof(&command, &key, 0, current_time_ns() + 60_000_000_000);
        execute_steward_command(command, proof).expect("steward send should execute");

        let agent_turn_runtime = stable::get_task_runtime(&TaskKind::AgentTurn);
        assert!(
            agent_turn_runtime.pending_job_id.is_some(),
            "steward send should enqueue an immediate agent turn when no proxy callback is pending"
        );
    }

    #[test]
    fn steward_send_enqueues_immediate_turn_even_when_proxy_callback_is_pending() {
        stable::init_storage();
        let key = steward_test_signing_key();
        let address = steward_address_from_key(&key);
        set_steward_admin(8453, address.clone(), true).expect("active steward should store");
        let _ = stable::set_steward_nonce_state(StewardNonceState { next_nonce: 0 });
        stable::upsert_pending_inference_proxy_job(PendingInferenceProxyJob {
            job_id: "pending-proxy-job".to_string(),
            turn_id: "turn-pending".to_string(),
            submitted_at_ns: current_time_ns(),
            model: "openai/gpt-4o-mini".to_string(),
        })
        .expect("pending proxy job should persist");

        let command = StewardCommand::SendStewardMessage {
            sender: address,
            message: "do not kick while callback pending".to_string(),
        };
        let proof = build_steward_proof(&command, &key, 0, current_time_ns() + 60_000_000_000);
        execute_steward_command(command, proof).expect("steward send should execute");

        let agent_turn_runtime = stable::get_task_runtime(&TaskKind::AgentTurn);
        assert!(
            agent_turn_runtime.pending_job_id.is_some(),
            "steward send should enqueue an immediate turn while proxy callback jobs are pending"
        );
    }

    #[test]
    fn enforce_wallet_sync_response_bytes_floor_raises_low_values() {
        stable::init_storage();
        let mut snapshot = stable::runtime_snapshot();
        snapshot.wallet_balance_sync.max_response_bytes = 512;
        stable::save_runtime_snapshot(&snapshot);

        enforce_wallet_sync_response_bytes_floor();

        let upgraded = stable::runtime_snapshot();
        assert_eq!(
            upgraded.wallet_balance_sync.max_response_bytes,
            WALLET_SYNC_RESPONSE_BYTES_FLOOR
        );
    }

    #[test]
    fn submit_inference_result_rejects_untrusted_caller() {
        stable::init_storage();
        set_inference_proxy_config(OpenRouterProxyWorkerConfig {
            worker_base_url: "https://proxy.example.workers.dev".to_string(),
            trusted_callback_principal: Some(
                Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai")
                    .expect("principal should parse"),
            ),
        })
        .expect("proxy config should persist");

        let error = submit_inference_result(SubmitInferenceResultArgs {
            job_id: "job-auth".to_string(),
            turn_id: "turn-auth".to_string(),
            completed_at_ns: 10,
            result: None,
            error: Some("failed".to_string()),
        })
        .expect_err("non-matching caller principal must be rejected");
        assert!(error.contains("unauthorized inference proxy callback caller"));

        let status = get_inference_proxy_status();
        assert_eq!(status.callback_rejected, 1);
        assert_eq!(status.callback_auth_failures, 1);
    }

    #[test]
    fn submit_inference_result_accepts_and_dedupes_for_trusted_caller() {
        stable::init_storage();
        set_inference_proxy_config(OpenRouterProxyWorkerConfig {
            worker_base_url: "https://proxy.example.workers.dev".to_string(),
            trusted_callback_principal: Some(
                Principal::from_text("2vxsx-fae").expect("anonymous principal should parse"),
            ),
        })
        .expect("proxy config should persist");
        stable::upsert_pending_inference_proxy_job(PendingInferenceProxyJob {
            job_id: "job-1".to_string(),
            turn_id: "turn-1".to_string(),
            submitted_at_ns: 1,
            model: "openai/gpt-4o-mini".to_string(),
        })
        .expect("pending job should persist");

        let first = submit_inference_result(SubmitInferenceResultArgs {
            job_id: "job-1".to_string(),
            turn_id: "turn-1".to_string(),
            completed_at_ns: 2,
            result: Some(InferenceProxyResultPayload {
                explanation: Some("done".to_string()),
                tool_calls: Vec::new(),
            }),
            error: None,
        })
        .expect("first callback should be accepted");
        assert_eq!(first, "inference_proxy_callback_accepted");

        let duplicate = submit_inference_result(SubmitInferenceResultArgs {
            job_id: "job-1".to_string(),
            turn_id: "turn-1".to_string(),
            completed_at_ns: 3,
            result: None,
            error: Some("ignored".to_string()),
        })
        .expect("duplicate callback should not error");
        assert_eq!(duplicate, "inference_proxy_callback_duplicate");

        let status = get_inference_proxy_status();
        assert_eq!(status.pending_jobs, 0);
        assert_eq!(status.completed_jobs, 1);
        assert_eq!(status.callback_accepted, 1);
        assert_eq!(status.callback_duplicates, 1);
        assert_eq!(
            status.trusted_callback_principal.as_deref(),
            Some("2vxsx-fae")
        );

        let agent_turn_runtime = stable::get_task_runtime(&TaskKind::AgentTurn);
        assert!(
            agent_turn_runtime.pending_job_id.is_some(),
            "accepted callback should enqueue an agent turn resume job"
        );
    }

    #[test]
    fn post_upgrade_removes_legacy_agent_loop_skill() {
        stable::init_storage();
        stable::upsert_skill(&SkillRecord {
            name: "agent-loop".to_string(),
            description: "legacy".to_string(),
            instructions: "legacy".to_string(),
            enabled: true,
            mutable: true,
            allowed_canister_calls: vec![],
        });

        post_upgrade();

        let names: Vec<String> = list_skills().into_iter().map(|skill| skill.name).collect();
        assert!(
            !names.iter().any(|name| name == "agent-loop"),
            "post-upgrade migration should remove legacy agent-loop skill"
        );
        assert!(
            names.iter().any(|name| name == "cycles-management"),
            "cycles-management should remain available after migration"
        );
    }

    fn sample_strategy_key() -> StrategyTemplateKey {
        StrategyTemplateKey {
            protocol: "erc20".to_string(),
            primitive: "transfer".to_string(),
            chain_id: 8453,
            template_id: "lib-strategy".to_string(),
        }
    }

    fn sample_template(status: TemplateStatus) -> StrategyTemplate {
        StrategyTemplate {
            key: sample_strategy_key(),
            status,
            contract_roles: vec![ContractRoleBinding {
                role: "token".to_string(),
                address: "0x2222222222222222222222222222222222222222".to_string(),
                source_ref: "https://example.com/token-address".to_string(),
                codehash: None,
            }],
            actions: vec![ActionSpec {
                action_id: "transfer".to_string(),
                call_sequence: vec![AbiFunctionSpec {
                    role: "token".to_string(),
                    name: "transfer".to_string(),
                    selector_hex: "0xa9059cbb".to_string(),
                    inputs: vec![
                        AbiTypeSpec {
                            name: "to".to_string(),
                            kind: "address".to_string(),
                            components: Vec::new(),
                        },
                        AbiTypeSpec {
                            name: "amount".to_string(),
                            kind: "uint256".to_string(),
                            components: Vec::new(),
                        },
                    ],
                    outputs: vec![AbiTypeSpec {
                        name: "success".to_string(),
                        kind: "bool".to_string(),
                        components: Vec::new(),
                    }],
                    state_mutability: "nonpayable".to_string(),
                }],
                preconditions: vec!["allowance_ok".to_string()],
                postconditions: vec!["balance_delta_positive".to_string()],
                risk_checks: vec!["max_notional".to_string()],
            }],
            constraints_json:
                r#"{"max_calls":1,"max_total_value_wei":"0","required_postconditions":["balance_delta_positive"]}"#
                    .to_string(),
            created_at_ns: 0,
            updated_at_ns: 0,
        }
    }

    fn seed_template_and_artifact() {
        ingest_strategy_template_admin(sample_template(TemplateStatus::Draft))
            .expect("template should ingest");
        ingest_strategy_abi_artifact_admin(StrategyAbiIngestArgs {
            key: AbiArtifactKey {
                protocol: "erc20".to_string(),
                chain_id: 8453,
                role: "token".to_string(),
            },
            abi_json: r#"[{"type":"function","name":"transfer","stateMutability":"nonpayable","inputs":[{"type":"address"},{"type":"uint256"}],"outputs":[{"type":"bool"}]}]"#.to_string(),
            source_ref: "https://example.com/token-abi".to_string(),
            codehash: None,
            selector_assertions: vec![AbiSelectorAssertion {
                signature: "transfer(address,uint256)".to_string(),
                selector_hex: "0xa9059cbb".to_string(),
            }],
        })
        .expect("abi should ingest");
    }

    #[test]
    fn strategy_lifecycle_admin_methods_manage_status_activation_and_kill_switch() {
        stable::init_storage();
        seed_template_and_artifact();

        let activated = activate_strategy_template_admin(
            sample_strategy_key(),
            Some("manual activation".to_string()),
        )
        .expect("activation should succeed");
        assert!(activated.enabled);

        let deprecated = deprecate_strategy_template_admin(
            sample_strategy_key(),
            Some("rotating template".to_string()),
        )
        .expect("deprecation should succeed");
        assert!(matches!(deprecated.status, TemplateStatus::Deprecated));

        let revoked = revoke_strategy_template_admin(
            sample_strategy_key(),
            Some("safety incident".to_string()),
        )
        .expect("revocation should succeed");
        assert!(revoked.revoked);

        let kill_switch = set_strategy_kill_switch_admin(
            sample_strategy_key(),
            true,
            Some("protocol halt".to_string()),
        )
        .expect("kill switch should persist");
        assert!(kill_switch.enabled);
    }

    #[test]
    fn strategy_queries_return_ingested_templates() {
        stable::init_storage();
        seed_template_and_artifact();

        let listed = list_strategy_templates(Some(sample_strategy_key()), 10);
        assert_eq!(listed.len(), 1);

        let fetched = get_strategy_template(sample_strategy_key()).expect("template exists");
        assert_eq!(fetched.actions[0].action_id, "transfer");
    }

    #[test]
    fn list_memory_facts_query_supports_prefix_sort_and_limit() {
        stable::init_storage();
        for (key, updated_at_ns) in [
            ("zeta.note", 30u64),
            ("alpha.note", 20u64),
            ("config.rpc_url", 10u64),
        ] {
            stable::set_memory_fact(&MemoryFact {
                key: key.to_string(),
                value: key.to_string(),
                created_at_ns: updated_at_ns,
                updated_at_ns,
                source_turn_id: "turn-seed".to_string(),
            })
            .expect("memory fact fixture should store");
        }

        let prefixed =
            list_memory_facts("config.".to_string(), MemoryFactListSort::UpdatedAtDesc, 10);
        assert_eq!(prefixed.len(), 1);
        assert_eq!(prefixed[0].key, "config.rpc_url");

        let sorted = list_memory_facts("".to_string(), MemoryFactListSort::KeyAsc, 2);
        assert_eq!(sorted.len(), 2);
        assert_eq!(sorted[0].key, "alpha.note");
        assert_eq!(sorted[1].key, "config.rpc_url");
    }

    #[test]
    fn list_reflection_memory_query_respects_limit() {
        stable::init_storage();
        for idx in 0..3 {
            stable::upsert_reflection_memory_degraded_lesson(stable::ReflectionMemoryDegradedLesson {
                tool: "market_fetch",
                subject: &format!("dexscreener:search_pairs_{idx}"),
                error_class: "missing_required_extract",
                what_failed: "market_fetch[dexscreener:search_pairs] failed: missing extract; use canonical provider:endpoint params",
                latest_repeat_count: Some(u32::try_from(idx + 1).unwrap_or(u32::MAX)),
                turn_id: &format!("turn-{idx}"),
                origin: crate::domain::types::ReflectionOrigin::Autonomy,
                now_ns: 100 + u64::try_from(idx).unwrap_or_default(),
            })
            .expect("reflection lesson should persist");
        }

        let listed = list_reflection_memory(2);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].subject, "dexscreener:search_pairs_2");
        assert_eq!(listed[1].subject, "dexscreener:search_pairs_1");
    }

    #[test]
    fn prune_memory_facts_admin_requires_filter_and_prunes_targeted_entries() {
        stable::init_storage();
        stable::set_memory_fact(&MemoryFact {
            key: "noise.old".to_string(),
            value: "old".to_string(),
            created_at_ns: 1,
            updated_at_ns: 100,
            source_turn_id: "turn-seed".to_string(),
        })
        .expect("old fact should store");
        stable::set_memory_fact(&MemoryFact {
            key: "noise.fresh".to_string(),
            value: "fresh".to_string(),
            created_at_ns: 2,
            updated_at_ns: 200,
            source_turn_id: "turn-seed".to_string(),
        })
        .expect("fresh fact should store");
        stable::set_memory_fact(&MemoryFact {
            key: "config.keep".to_string(),
            value: "keep".to_string(),
            created_at_ns: 3,
            updated_at_ns: 300,
            source_turn_id: "turn-seed".to_string(),
        })
        .expect("config fact should store");

        assert!(
            prune_memory_facts_admin(None, None, 10).is_err(),
            "admin prune should reject requests without prefix or age filter"
        );

        let removed = prune_memory_facts_admin(Some("noise.".to_string()), Some(150), 10)
            .expect("filtered prune should succeed");
        assert_eq!(removed, vec!["noise.old".to_string()]);
        assert!(stable::get_memory_fact("noise.old").is_none());
        assert!(stable::get_memory_fact("noise.fresh").is_some());
        assert!(stable::get_memory_fact("config.keep").is_some());
    }
}

ic_cdk::export_candid!();
