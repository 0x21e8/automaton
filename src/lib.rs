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

use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, AbiSelectorAssertion, AutonomySuppressionConfig, ConversationLog,
    ConversationSummary, EvmRouteStateView, EvmStewardProof, InboxMessage, InboxStats,
    InferenceConfigView, InferenceProvider, InferenceProxyStatusView, MemoryFact, MemoryRollup,
    ObservabilitySnapshot, OpenRouterProxyWorkerConfig, OutboxMessage, OutboxStats, PromptLayer,
    PromptLayerView, RetentionConfig, RetentionMaintenanceRuntime, RuntimeView, ScheduledJob,
    SchedulerRuntime, SessionSummary, SkillRecord, StewardCommand, StewardState, StewardStatusView,
    StrategyKillSwitchState, StrategyOutcomeStats, StrategyTemplate, StrategyTemplateKey,
    SubmitInferenceResultArgs, TaskKind, TaskLane, TaskScheduleConfig, TaskScheduleRuntime,
    TemplateActivationState, TemplateRevocationState, TemplateStatus, TemplateVersion,
    ToolCallRecord, TurnWindowSummary, WalletBalanceSyncConfigView, WalletBalanceTelemetryView,
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
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

// ── Initialization ──────────────────────────────────────────────────────────
const WALLET_SYNC_RESPONSE_BYTES_FLOOR: u64 = 1_024;

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

/// Arguments supplied once at canister creation via `dfx deploy --argument`.
#[derive(CandidType, Deserialize)]
struct InitArgs {
    ecdsa_key_name: String,
    #[serde(default)]
    inbox_contract_address: Option<String>,
    #[serde(default)]
    evm_chain_id: Option<u64>,
    #[serde(default)]
    evm_rpc_url: Option<String>,
    #[serde(default)]
    evm_confirmation_depth: Option<u64>,
    #[serde(default)]
    evm_bootstrap_lookback_blocks: Option<u64>,
    #[serde(default)]
    http_allowed_domains: Option<Vec<String>>,
    #[serde(default)]
    llm_canister_id: Option<Principal>,
    #[serde(default)]
    cycle_topup_enabled: Option<bool>,
    #[serde(default)]
    auto_topup_cycle_threshold: Option<u64>,
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
        StewardCommand::UpdateSteward { .. } => "update_steward",
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

fn steward_proof_expected_canister_id() -> String {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::id().to_text();

    #[cfg(not(target_arch = "wasm32"))]
    return "rrkah-fqaaa-aaaaa-aaaaq-cai".to_string();
}

fn verify_steward_proof_for_command_hash(
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

    match crate::features::evm::verify_evm_steward_proof(proof, &context) {
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
    };
    let stored = stable::set_active_steward_with_nonce_reset_on_rotation(requested)?;
    let nonce_reset = previous.as_ref().is_none_or(|current| {
        current.chain_id != stored.chain_id || current.address != stored.address
    });
    Ok((previous, stored, nonce_reset))
}

/// Looks up a strategy template by key and version, returning a descriptive
/// error when not found.
fn require_strategy_template(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> Result<StrategyTemplate, String> {
    crate::strategy::registry::get_template(key, version).ok_or_else(|| {
        format!(
            "strategy template not found for {}:{}:{}:{}@{}.{}.{}",
            key.protocol,
            key.primitive,
            key.chain_id,
            key.template_id,
            version.major,
            version.minor,
            version.patch
        )
    })
}

/// Atomically sets a template's `status` field and bumps `updated_at_ns`.
fn upsert_template_status(
    key: StrategyTemplateKey,
    version: TemplateVersion,
    status: TemplateStatus,
) -> Result<StrategyTemplate, String> {
    let mut template = require_strategy_template(&key, &version)?;
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

/// Applies all `InitArgs` fields to stable storage, trapping on the first
/// validation error.  Separated from `init` so tests can call it directly.
fn apply_init_args(args: InitArgs) {
    stable::init_storage();
    let _ = sqlite::init_storage();
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
#[ic_cdk::update]
fn steward_execute(command: StewardCommand, proof: EvmStewardProof) -> Result<String, String> {
    let command_label = steward_command_label(&command);
    let command_hash = steward_command_hash(&command)?;
    let verified = verify_steward_proof_for_command_hash(&command_hash, &proof)?;
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

    let result = match command {
        StewardCommand::Noop => Ok("steward_noop_executed".to_string()),
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
    }
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
            let agent_turn_priority = stable::get_task_config(&TaskKind::AgentTurn)
                .map(|config| config.priority)
                .unwrap_or(TaskKind::AgentTurn.default_priority());
            let enqueued = stable::enqueue_job_if_absent(
                TaskKind::AgentTurn,
                TaskLane::Mutating,
                format!("AgentTurn:inference-proxy-resume:{callback_job_id}"),
                current_time_ns(),
                agent_turn_priority,
            );
            if enqueued.is_some() {
                schedule_immediate_scheduler_tick("inference_proxy_callback_resume");
            }
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

#[ic_cdk::query]
fn get_inference_proxy_status() -> InferenceProxyStatusView {
    stable::inference_proxy_status_view()
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

/// Returns a single strategy template by key and version, or `None` if absent.
#[ic_cdk::query]
fn get_strategy_template(
    key: StrategyTemplateKey,
    version: TemplateVersion,
) -> Option<StrategyTemplate> {
    crate::strategy::registry::get_template(&key, &version)
}

/// Returns accumulated outcome statistics (success/failure counts, last outcome)
/// for the given template, or `None` if no executions have been recorded yet.
#[ic_cdk::query]
fn get_strategy_outcome_stats(
    key: StrategyTemplateKey,
    version: TemplateVersion,
) -> Option<StrategyOutcomeStats> {
    crate::strategy::learner::outcome_stats(&key, &version)
}

/// Inserts or updates a strategy template (controller only).
/// Stamps `created_at_ns` on first insert and always updates `updated_at_ns`.
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

/// Transitions a template to `Active`, runs a canary probe to validate it,
/// and records an activation state entry (controller only).
#[ic_cdk::update]
fn activate_strategy_template_admin(
    key: StrategyTemplateKey,
    version: TemplateVersion,
    reason: Option<String>,
) -> Result<TemplateActivationState, String> {
    ensure_controller()?;
    let _template = upsert_template_status(key.clone(), version.clone(), TemplateStatus::Active)?;
    crate::strategy::registry::canary_probe_template(&key, &version)?;
    crate::strategy::registry::set_activation(TemplateActivationState {
        key,
        version,
        enabled: true,
        updated_at_ns: current_time_ns(),
        reason: reason.or_else(|| Some("controller activation after canary probe".to_string())),
    })
}

/// Marks a template as `Deprecated` and deactivates it.  Use this for orderly
/// rotation; the template remains readable (controller only).
#[ic_cdk::update]
fn deprecate_strategy_template_admin(
    key: StrategyTemplateKey,
    version: TemplateVersion,
    reason: Option<String>,
) -> Result<StrategyTemplate, String> {
    ensure_controller()?;
    let template =
        upsert_template_status(key.clone(), version.clone(), TemplateStatus::Deprecated)?;
    let _ = crate::strategy::registry::set_activation(TemplateActivationState {
        key,
        version,
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
    version: TemplateVersion,
    reason: Option<String>,
) -> Result<TemplateRevocationState, String> {
    ensure_controller()?;
    let _ = upsert_template_status(key.clone(), version.clone(), TemplateStatus::Revoked)?;
    let now_ns = current_time_ns();
    let revocation = crate::strategy::registry::set_revocation(TemplateRevocationState {
        key: key.clone(),
        version: version.clone(),
        revoked: true,
        updated_at_ns: now_ns,
        reason: reason.clone(),
    })?;
    let _ = crate::strategy::registry::set_activation(TemplateActivationState {
        key,
        version,
        enabled: false,
        updated_at_ns: now_ns,
        reason: reason.or_else(|| Some("revoked".to_string())),
    });
    Ok(revocation)
}

/// Arms or disarms the kill switch for all versions of a strategy template.
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
fn http_request_update(request: HttpUpdateRequest) -> HttpUpdateResponse {
    crate::http::handle_http_request_update(request)
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
        AbiFunctionSpec, AbiTypeSpec, ActionSpec, ContractRoleBinding, InferenceProxyResultPayload,
        MemoryFact, OpenRouterProxyWorkerConfig, PendingInferenceProxyJob, SkillRecord,
        StewardNonceState, StrategyTemplate, StrategyTemplateKey, SubmitInferenceResultArgs,
        TemplateStatus, TemplateVersion,
    };
    use sha3::{Digest, Keccak256};

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
        let command_hash = steward_command_hash(command).expect("command hash should encode");
        let payload = canonical_steward_signing_payload(
            &canister_id,
            chain_id,
            &normalized_address,
            &command_hash,
            nonce,
            expires_at_ns,
        );
        EvmStewardProof {
            canister_id,
            chain_id,
            address: normalized_address.to_ascii_uppercase(),
            command_hash,
            nonce,
            expires_at_ns,
            signature: sign_steward_payload(&payload, signing_key),
        }
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
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
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
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
        });

        assert_eq!(stable::get_llm_canister_id(), "w36hm-eqaaa-aaaal-qr76a-cai");
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
            cycle_topup_enabled: Some(false),
            auto_topup_cycle_threshold: Some(150_000_000_000),
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
            cycle_topup_enabled: None,
            auto_topup_cycle_threshold: None,
        });

        let snapshot = stable::runtime_snapshot();
        assert_eq!(snapshot.evm_bootstrap_lookback_blocks, 0);
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

        let result = steward_execute(command, proof).expect("proof should execute");
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
        steward_execute(command.clone(), proof.clone()).expect("first execution should pass");

        let replay_error =
            steward_execute(command, proof).expect_err("replayed proof nonce should fail");
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

        let result = steward_execute(command, proof).expect("rotation command should execute");
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
        let new_result =
            steward_execute(StewardCommand::Noop, new_proof).expect("new steward should execute");
        assert_eq!(new_result, "steward_noop_executed");
        assert_eq!(get_steward_status().next_nonce, 1);
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

    fn sample_version() -> TemplateVersion {
        TemplateVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }

    fn sample_template(status: TemplateStatus) -> StrategyTemplate {
        StrategyTemplate {
            key: sample_strategy_key(),
            version: sample_version(),
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
                            kind: "address".to_string(),
                            components: Vec::new(),
                        },
                        AbiTypeSpec {
                            kind: "uint256".to_string(),
                            components: Vec::new(),
                        },
                    ],
                    outputs: vec![AbiTypeSpec {
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
                version: sample_version(),
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
            sample_version(),
            Some("manual activation".to_string()),
        )
        .expect("activation should succeed");
        assert!(activated.enabled);

        let deprecated = deprecate_strategy_template_admin(
            sample_strategy_key(),
            sample_version(),
            Some("rotating template".to_string()),
        )
        .expect("deprecation should succeed");
        assert!(matches!(deprecated.status, TemplateStatus::Deprecated));

        let revoked = revoke_strategy_template_admin(
            sample_strategy_key(),
            sample_version(),
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
        assert_eq!(listed[0].version, sample_version());

        let fetched = get_strategy_template(sample_strategy_key(), sample_version())
            .expect("template exists");
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
