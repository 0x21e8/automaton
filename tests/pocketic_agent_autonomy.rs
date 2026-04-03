#![cfg(feature = "pocketic_tests")]

use std::path::Path;
use std::time::Duration;

use alloy_primitives::keccak256;
use candid::{decode_one, encode_args, CandidType, Principal};
use pocket_ic::common::rest::{
    CanisterHttpReply, CanisterHttpRequest, CanisterHttpResponse, MockCanisterHttpResponse,
};
use pocket_ic::PocketIc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const WASM_PATHS: &[&str] = &[
    ".icp/cache/artifacts/backend",
    "target/wasm32-unknown-unknown/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/deps/backend.wasm",
];
const INBOX_MESSAGE_QUEUED_EVENT_SIGNATURE: &str =
    "MessageQueued(address,uint64,address,string,uint256,uint256)";

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
enum TaskKind {
    AgentTurn,
    PollInbox,
    CheckCycles,
    TopUpCycles,
    Reconcile,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
enum TaskLane {
    Mutating,
    ReadOnly,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
enum JobStatus {
    Pending,
    InFlight,
    Succeeded,
    Failed,
    TimedOut,
    Skipped,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
enum AgentState {
    Bootstrapping,
    Idle,
    LoadingContext,
    Inferring,
    ExecutingActions,
    Persisting,
    Sleeping,
    Faulted,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
enum InferenceProvider {
    IcLlm,
    OpenRouter,
    OpenRouterProxyWorker,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct ObservedJob {
    id: String,
    kind: TaskKind,
    lane: TaskLane,
    dedupe_key: String,
    priority: u8,
    created_at_ns: u64,
    scheduled_for_ns: u64,
    started_at_ns: Option<u64>,
    finished_at_ns: Option<u64>,
    status: JobStatus,
    attempts: u32,
    max_attempts: u32,
    last_error: Option<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct RuntimeView {
    state: AgentState,
    turn_in_flight: bool,
    loop_enabled: bool,
    turn_counter: u64,
    last_turn_id: Option<String>,
    last_error: Option<String>,
    soul: String,
    evm_chain_id: u64,
    evm_next_block: u64,
    evm_next_log_index: u64,
    last_transition_at_ns: u64,
    inference_provider: InferenceProvider,
    inference_model: String,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Default)]
enum WalletBalanceStatus {
    #[default]
    Unknown,
    Fresh,
    Stale,
    Error,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct WalletBalanceTelemetryView {
    eth_balance_wei_hex: Option<String>,
    usdc_balance_raw_hex: Option<String>,
    usdc_decimals: u8,
    usdc_contract_address: Option<String>,
    last_synced_at_ns: Option<u64>,
    last_synced_block: Option<u64>,
    last_error: Option<String>,
    age_secs: Option<u64>,
    freshness_window_secs: u64,
    is_stale: bool,
    status: WalletBalanceStatus,
    bootstrap_pending: bool,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
struct SpawnBootstrapView {
    session_id: Option<String>,
    parent_id: Option<String>,
    factory_principal: Option<Principal>,
    risk: Option<u8>,
    strategies: Vec<String>,
    skills: Vec<String>,
    version_commit: Option<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InitArgs {
    ecdsa_key_name: String,
    inbox_contract_address: Option<String>,
    evm_chain_id: Option<u64>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct RetentionConfig {
    jobs_max_age_secs: u64,
    jobs_max_records: u64,
    dedupe_max_age_secs: u64,
    turns_max_age_secs: u64,
    transitions_max_age_secs: u64,
    tools_max_age_secs: u64,
    inbox_max_age_secs: u64,
    outbox_max_age_secs: u64,
    memory_facts_max_age_secs: u64,
    memory_facts_prune_batch_size: u32,
    maintenance_batch_size: u32,
    maintenance_interval_secs: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InboxStats {
    total_messages: u64,
    pending_count: u64,
    staged_count: u64,
    consumed_count: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct PromptLayerView {
    layer_id: u8,
    is_mutable: bool,
    content: String,
    updated_at_ns: Option<u64>,
    updated_by_turn: Option<String>,
    version: Option<u32>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct OutboxMessage {
    id: String,
    seq: u64,
    turn_id: String,
    body: String,
    created_at_ns: u64,
    source_inbox_ids: Vec<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
enum ReflectionOrigin {
    Autonomy,
    ExternalInput,
    Maintenance,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
struct ReflectionMemoryRecord {
    key: String,
    tool: String,
    subject: String,
    error_class: String,
    what_failed: String,
    what_worked: Option<String>,
    degraded_turn_count: u32,
    repeat_count: u32,
    last_failed_at_ns: u64,
    last_failed_turn_id: String,
    last_worked_at_ns: Option<u64>,
    last_worked_turn_id: Option<String>,
    last_origin: ReflectionOrigin,
    updated_at_ns: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct OpenRouterProxyWorkerConfig {
    worker_base_url: String,
    trusted_callback_principal: Option<Principal>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct ToolCall {
    tool_call_id: Option<String>,
    tool: String,
    args_json: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InferenceProxyResultPayload {
    explanation: Option<String>,
    tool_calls: Vec<ToolCall>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct SubmitInferenceResultArgs {
    job_id: String,
    turn_id: String,
    completed_at_ns: u64,
    result: Option<InferenceProxyResultPayload>,
    error: Option<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InferenceProxyStatusView {
    worker_base_url: Option<String>,
    trusted_callback_principal: Option<String>,
    pending_jobs: u64,
    completed_jobs: u64,
    oldest_pending_age_secs: Option<u64>,
    submit_accepted: u64,
    submit_failed: u64,
    callback_accepted: u64,
    callback_rejected: u64,
    callback_duplicates: u64,
    callback_auth_failures: u64,
    resumed_callbacks: u64,
    expired_jobs: u64,
}

#[derive(Clone, Debug)]
struct ProxySubmitCapture {
    job_id: String,
    turn_id: String,
    system_prompt: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ReservePolicy {
    min_cycles_runway_hours: u64,
    min_inference_usdc_6dp: Option<u64>,
    min_gas_wei: Option<u128>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct RiskLimits {
    max_total_exposure_bps: u16,
    max_single_action_bps: u16,
    max_protocol_concentration_bps: u16,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ExecutionAuthority {
    autonomous_execution_enabled: bool,
    require_simulation_first: bool,
    per_action_value_limit_wei: Option<u128>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct EscalationRules {
    escalate_on_missing_policy: bool,
    escalate_on_authority_exceeded: bool,
    escalate_on_repeated_failure: bool,
    failure_quarantine_threshold: u32,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct AutonomyPolicy {
    version: u32,
    reserve_policy: ReservePolicy,
    risk_limits: RiskLimits,
    execution_authority: ExecutionAuthority,
    escalation_rules: EscalationRules,
    updated_at_ns: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum DecisionTrigger {
    ScheduledReview,
    InboxMessage,
    LowRunway,
    PositionMaintenance,
    RecoveryFollowUp,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum EscalationClass {
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

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum DecisionOutcome {
    Executed { action_summary: String },
    Simulated { action_summary: String },
    NoOp { reason: String },
    Deferred { reason: String },
    Escalated { gap: EscalationClass },
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct DecisionRecord {
    turn_id: String,
    timestamp_ns: u64,
    trigger: DecisionTrigger,
    outcome: DecisionOutcome,
    policy_version: u32,
    candidates_summary: String,
    explanation: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ActiveExposure {
    strategy_id: String,
    protocol: String,
    chain_id: u64,
    asset_symbol: String,
    notional_wei: Option<u128>,
    updated_at_ns: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StrategyQuarantine {
    strategy_id: String,
    reason: String,
    failure_count: u32,
    quarantined_at_ns: u64,
    release_after_ns: Option<u64>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
struct ExposureReconciliationStatus {
    last_attempted_at_ns: Option<u64>,
    last_succeeded_at_ns: Option<u64>,
    repaired_exposures: u32,
    recreated_exposures: u32,
    closed_exposures: u32,
    drift_reason: Option<String>,
    last_error: Option<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct PromptLayer {
    layer_id: u8,
    content: String,
    updated_at_ns: u64,
    updated_by_turn: String,
    version: u32,
}

fn assert_wasm_artifact_present() -> Vec<u8> {
    for path in WASM_PATHS {
        if Path::new(path).exists() {
            return std::fs::read(path).unwrap_or_else(|error| {
                panic!("cannot read PocketIC test artifact {path}: {error}");
            });
        }
    }
    panic!(
        "build artifact not found at any expected path ({:?}); run `icp build` before PocketIC tests",
        WASM_PATHS
    );
}

fn with_backend_canister_with_init_args(init: InitArgs) -> (PocketIc, Principal) {
    with_backend_canister_with_init_args_and_cycles(init, 2_000_000_000_000)
}

fn with_backend_canister_with_init_args_and_cycles(
    init: InitArgs,
    cycles: u128,
) -> (PocketIc, Principal) {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    let wasm = assert_wasm_artifact_present();
    let init_args = encode_args((init,)).expect("failed to encode init args");

    pic.add_cycles(canister_id, cycles);
    pic.install_canister(canister_id, wasm, init_args, None);
    set_inference_provider(&pic, canister_id, InferenceProvider::IcLlm);
    set_inference_model(&pic, canister_id, "deterministic-local");

    (pic, canister_id)
}

fn with_backend_canister() -> (PocketIc, Principal) {
    with_backend_canister_with_init_args(InitArgs {
        ecdsa_key_name: "dfx_test_key".to_string(),
        inbox_contract_address: None,
        evm_chain_id: None,
    })
}

fn non_controller_principal() -> Principal {
    Principal::self_authenticating(b"non-controller-agent-autonomy")
}

fn call_update_as<T>(
    pic: &PocketIc,
    canister_id: Principal,
    caller: Principal,
    method: &str,
    payload: Vec<u8>,
) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .update_call(canister_id, caller, method, payload)
        .unwrap_or_else(|error| panic!("update call {method} failed: {error:?}"));
    decode_one(&response)
        .unwrap_or_else(|error| panic!("failed decoding {method} response: {error:?}"))
}

fn call_update<T>(pic: &PocketIc, canister_id: Principal, method: &str, payload: Vec<u8>) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    call_update_as(pic, canister_id, Principal::anonymous(), method, payload)
}

fn call_query<T>(pic: &PocketIc, canister_id: Principal, method: &str, payload: Vec<u8>) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .query_call(canister_id, Principal::anonymous(), method, payload)
        .unwrap_or_else(|error| panic!("query call {method} failed: {error:?}"));
    decode_one(&response)
        .unwrap_or_else(|error| panic!("failed decoding {method} response: {error:?}"))
}

fn get_autonomy_policy(pic: &PocketIc, canister_id: Principal) -> AutonomyPolicy {
    call_query(
        pic,
        canister_id,
        "get_autonomy_policy",
        encode_args(()).expect("failed to encode get_autonomy_policy args"),
    )
}

fn update_autonomy_policy(
    pic: &PocketIc,
    canister_id: Principal,
    policy: AutonomyPolicy,
) -> Result<AutonomyPolicy, String> {
    call_update(
        pic,
        canister_id,
        "update_autonomy_policy",
        encode_args((policy,)).expect("failed to encode update_autonomy_policy args"),
    )
}

fn get_recent_decisions(pic: &PocketIc, canister_id: Principal) -> Vec<DecisionRecord> {
    call_query(
        pic,
        canister_id,
        "get_recent_decisions",
        encode_args(()).expect("failed to encode get_recent_decisions args"),
    )
}

fn get_active_exposures(pic: &PocketIc, canister_id: Principal) -> Vec<ActiveExposure> {
    call_query(
        pic,
        canister_id,
        "get_active_exposures",
        encode_args(()).expect("failed to encode get_active_exposures args"),
    )
}

fn get_strategy_quarantines(pic: &PocketIc, canister_id: Principal) -> Vec<StrategyQuarantine> {
    call_query(
        pic,
        canister_id,
        "get_strategy_quarantines",
        encode_args(()).expect("failed to encode get_strategy_quarantines args"),
    )
}

fn get_exposure_reconciliation_status(
    pic: &PocketIc,
    canister_id: Principal,
) -> ExposureReconciliationStatus {
    call_query(
        pic,
        canister_id,
        "get_exposure_reconciliation_status",
        encode_args(()).expect("failed to encode get_exposure_reconciliation_status args"),
    )
}

fn update_prompt_layer_admin(
    pic: &PocketIc,
    canister_id: Principal,
    layer_id: u8,
    content: &str,
) -> Result<PromptLayer, String> {
    call_update(
        pic,
        canister_id,
        "update_prompt_layer_admin",
        encode_args((layer_id, content.to_string()))
            .expect("failed to encode update_prompt_layer_admin args"),
    )
}

fn set_task_enabled(pic: &PocketIc, canister_id: Principal, kind: TaskKind, enabled: bool) {
    let payload = encode_args((kind, enabled)).unwrap_or_else(|error| {
        panic!("failed to encode set_task_enabled args: {error}");
    });
    let _: String = call_update(pic, canister_id, "set_task_enabled", payload);
}

fn set_task_interval_secs(
    pic: &PocketIc,
    canister_id: Principal,
    kind: TaskKind,
    interval_secs: u64,
) {
    let payload = encode_args((kind, interval_secs)).unwrap_or_else(|error| {
        panic!("failed to encode set_task_interval_secs args: {error}");
    });
    let result: Result<String, String> =
        call_update(pic, canister_id, "set_task_interval_secs", payload);
    assert!(result.is_ok(), "set_task_interval_secs failed: {result:?}");
}

fn set_evm_rpc_url(pic: &PocketIc, canister_id: Principal, url: &str) {
    let payload = encode_args((url.to_string(),)).expect("failed to encode set_evm_rpc_url");
    let result: Result<String, String> = call_update(pic, canister_id, "set_evm_rpc_url", payload);
    assert!(result.is_ok(), "set_evm_rpc_url failed: {result:?}");
}

fn derive_automaton_evm_address(pic: &PocketIc, canister_id: Principal) -> String {
    let payload = encode_args(()).expect("failed to encode derive_automaton_evm_address");
    let result: Result<String, String> =
        call_update(pic, canister_id, "derive_automaton_evm_address", payload);
    result.unwrap_or_else(|error| panic!("derive_automaton_evm_address failed: {error}"))
}

fn set_inbox_contract_address_admin(pic: &PocketIc, canister_id: Principal, address: &str) {
    let payload = encode_args((Some(address.to_string()),))
        .expect("failed to encode set_inbox_contract_address_admin");
    let result: Result<Option<String>, String> = call_update(
        pic,
        canister_id,
        "set_inbox_contract_address_admin",
        payload,
    );
    assert!(
        result.is_ok(),
        "set_inbox_contract_address_admin failed: {result:?}"
    );
}

fn set_loop_enabled(pic: &PocketIc, canister_id: Principal, enabled: bool) {
    let payload = encode_args((enabled,)).unwrap_or_else(|error| {
        panic!("failed to encode set_loop_enabled args: {error}");
    });
    let _: String = call_update(pic, canister_id, "set_loop_enabled", payload);
}

fn set_inference_provider(pic: &PocketIc, canister_id: Principal, provider: InferenceProvider) {
    let payload = encode_args((provider,)).unwrap_or_else(|error| {
        panic!("failed to encode set_inference_provider args: {error}");
    });
    let _: String = call_update(pic, canister_id, "set_inference_provider", payload);
}

fn set_inference_model(pic: &PocketIc, canister_id: Principal, model: &str) {
    let payload = encode_args((model.to_string(),)).unwrap_or_else(|error| {
        panic!("failed to encode set_inference_model args: {error}");
    });
    let result: Result<String, String> =
        call_update(pic, canister_id, "set_inference_model", payload);
    assert!(result.is_ok(), "set_inference_model failed: {result:?}");
}

fn set_openrouter_api_key(pic: &PocketIc, canister_id: Principal, api_key: Option<String>) {
    let payload = encode_args((api_key,)).expect("failed to encode set_openrouter_api_key args");
    let _: String = call_update(pic, canister_id, "set_openrouter_api_key", payload);
}

fn set_inference_proxy_config(
    pic: &PocketIc,
    canister_id: Principal,
    config: OpenRouterProxyWorkerConfig,
) {
    let payload = encode_args((config,)).expect("failed to encode set_inference_proxy_config args");
    let result: Result<OpenRouterProxyWorkerConfig, String> =
        call_update(pic, canister_id, "set_inference_proxy_config", payload);
    assert!(
        result.is_ok(),
        "set_inference_proxy_config failed: {result:?}"
    );
}

fn set_retention_config(pic: &PocketIc, canister_id: Principal, config: RetentionConfig) {
    let payload = encode_args((config,)).expect("failed to encode set_retention_config args");
    let result: Result<RetentionConfig, String> =
        call_update(pic, canister_id, "set_retention_config", payload);
    assert!(result.is_ok(), "set_retention_config failed: {result:?}");
}

fn list_scheduler_jobs(pic: &PocketIc, canister_id: Principal) -> Vec<ObservedJob> {
    let payload = encode_args((200u32,)).unwrap_or_else(|error| {
        panic!("failed to encode list_scheduler_jobs args: {error}");
    });
    call_query(pic, canister_id, "list_scheduler_jobs", payload)
}

fn list_reflection_memory(
    pic: &PocketIc,
    canister_id: Principal,
    limit: u32,
) -> Vec<ReflectionMemoryRecord> {
    let payload = encode_args((limit,)).expect("failed to encode list_reflection_memory args");
    call_query(pic, canister_id, "list_reflection_memory", payload)
}

fn get_inference_proxy_status(pic: &PocketIc, canister_id: Principal) -> InferenceProxyStatusView {
    call_query(
        pic,
        canister_id,
        "get_inference_proxy_status",
        encode_args(()).expect("failed to encode get_inference_proxy_status args"),
    )
}

fn message_queued_topic0() -> String {
    let hash = keccak256(INBOX_MESSAGE_QUEUED_EVENT_SIGNATURE.as_bytes());
    format!("0x{}", hex::encode(hash.as_slice()))
}

fn address_to_topic(address: &str) -> String {
    let normalized = address.trim().to_ascii_lowercase();
    let without_prefix = normalized.trim_start_matches("0x");
    format!("0x{:0>64}", without_prefix)
}

fn encode_u256_word(value: u128) -> String {
    format!("{value:064x}")
}

fn encode_message_queued_payload(
    sender: &str,
    message: &str,
    usdc_amount: u128,
    eth_amount: u128,
) -> String {
    let sender = sender.trim().to_ascii_lowercase();
    let sender_hex = sender.trim_start_matches("0x");
    assert_eq!(sender_hex.len(), 40, "sender must be a 20-byte hex address");

    let message_hex = hex::encode(message.as_bytes());
    let padded_message_hex = if message_hex.len().is_multiple_of(64) {
        message_hex.clone()
    } else {
        let padding = "0".repeat(64 - (message_hex.len() % 64));
        format!("{message_hex}{padding}")
    };

    format!(
        "0x{:0>64}{}{}{}{}{}",
        sender_hex,
        encode_u256_word(128),
        encode_u256_word(usdc_amount),
        encode_u256_word(eth_amount),
        encode_u256_word(message.len() as u128),
        padded_message_hex,
    )
}

fn rpc_log(
    block_number: u64,
    log_index: u64,
    tx_hash: &str,
    contract_address: &str,
    topic1: &str,
    data: &str,
) -> Value {
    json!({
        "address": contract_address,
        "topics": [message_queued_topic0(), topic1],
        "data": data,
        "blockNumber": format!("0x{block_number:x}"),
        "logIndex": format!("0x{log_index:x}"),
        "transactionHash": tx_hash,
    })
}

fn rpc_response_body_for_request_with_wallet(
    request: &CanisterHttpRequest,
    latest_block: u64,
    logs: &[Value],
    eth_balance_hex: &str,
    usdc_balance_hex: &str,
) -> Vec<u8> {
    let request_json: Value = serde_json::from_slice(&request.body)
        .unwrap_or_else(|error| panic!("failed to decode canister http request body: {error}"));
    let method = request_json
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let response = match method {
        "eth_blockNumber" => json!({
            "jsonrpc":"2.0",
            "id":1,
            "result": format!("0x{latest_block:x}")
        }),
        "eth_getLogs" => json!({
            "jsonrpc":"2.0",
            "id":1,
            "result": logs
        }),
        "eth_getBalance" => json!({
            "jsonrpc":"2.0",
            "id":1,
            "result": eth_balance_hex
        }),
        "eth_call" => json!({
            "jsonrpc":"2.0",
            "id":1,
            "result": usdc_balance_hex
        }),
        unsupported => panic!("unsupported canister http method in test: {unsupported}"),
    };
    serde_json::to_vec(&response).expect("failed to encode mock canister http response")
}

fn drive_poll_inbox_with_http_mocks(
    pic: &PocketIc,
    canister_id: Principal,
    latest_block: u64,
    logs: &[Value],
) {
    drive_poll_inbox_with_http_mocks_and_wallet(
        pic,
        canister_id,
        latest_block,
        logs,
        "0x0",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    );
}

fn drive_poll_inbox_with_http_mocks_and_wallet(
    pic: &PocketIc,
    canister_id: Principal,
    latest_block: u64,
    logs: &[Value],
    eth_balance_hex: &str,
    usdc_balance_hex: &str,
) {
    let before_latest_poll_job_id = list_scheduler_jobs(pic, canister_id)
        .into_iter()
        .filter(|job| job.kind == TaskKind::PollInbox)
        .max_by_key(|job| job.created_at_ns)
        .map(|job| job.id);

    pic.tick();

    for _ in 0..24 {
        let pending_http = pic.get_canister_http();
        if !pending_http.is_empty() {
            for request in pending_http {
                let body = rpc_response_body_for_request_with_wallet(
                    &request,
                    latest_block,
                    logs,
                    eth_balance_hex,
                    usdc_balance_hex,
                );
                pic.mock_canister_http_response(MockCanisterHttpResponse {
                    subnet_id: request.subnet_id,
                    request_id: request.request_id,
                    response: CanisterHttpResponse::CanisterHttpReply(CanisterHttpReply {
                        status: 200,
                        headers: vec![],
                        body,
                    }),
                    additional_responses: vec![],
                });
            }
        }

        pic.tick();
        let jobs = list_scheduler_jobs(pic, canister_id);
        let pending_after_tick = pic.get_canister_http();
        let latest_job = jobs
            .iter()
            .filter(|job| job.kind == TaskKind::PollInbox)
            .max_by_key(|job| job.created_at_ns);
        let terminal = latest_job
            .map(|job| matches!(job.status, JobStatus::Succeeded | JobStatus::Failed))
            .unwrap_or(false);
        let saw_new_poll_job = latest_job
            .map(|job| before_latest_poll_job_id.as_deref() != Some(job.id.as_str()))
            .unwrap_or(false);
        if saw_new_poll_job && terminal && pending_after_tick.is_empty() {
            return;
        }
    }

    panic!("poll inbox did not finish with mocked http responses in expected ticks");
}

fn configure_route_for_polling(pic: &PocketIc, canister_id: Principal) -> String {
    set_evm_rpc_url(pic, canister_id, "https://mainnet.base.org");
    let automaton_address = derive_automaton_evm_address(pic, canister_id);
    set_inbox_contract_address_admin(
        pic,
        canister_id,
        "0x2222222222222222222222222222222222222222",
    );
    automaton_address
}

fn ingest_messages_via_contract_poll(
    pic: &PocketIc,
    canister_id: Principal,
    bodies: &[String],
) -> Vec<String> {
    if bodies.is_empty() {
        return Vec::new();
    }

    let automaton_address = configure_route_for_polling(pic, canister_id);
    let topic1 = address_to_topic(&automaton_address);
    let before_total = get_inbox_stats(pic, canister_id).total_messages;
    let block_number = before_total.saturating_add(2);
    let logs = bodies
        .iter()
        .enumerate()
        .map(|(idx, body)| {
            let payload = encode_message_queued_payload(
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                body,
                0,
                0,
            );
            rpc_log(
                block_number,
                idx as u64,
                &format!("0x{:064x}", before_total.saturating_add(idx as u64 + 1)),
                "0x2222222222222222222222222222222222222222",
                &topic1,
                &payload,
            )
        })
        .collect::<Vec<_>>();

    set_task_enabled(pic, canister_id, TaskKind::PollInbox, true);
    pic.advance_time(Duration::from_secs(61));
    drive_poll_inbox_with_http_mocks(pic, canister_id, block_number, &logs);

    (0..bodies.len())
        .map(|idx| format!("inbox:{:020}", before_total.saturating_add(idx as u64 + 1)))
        .collect()
}

fn get_inbox_stats(pic: &PocketIc, canister_id: Principal) -> InboxStats {
    call_query(
        pic,
        canister_id,
        "get_inbox_stats",
        encode_args(()).expect("failed to encode empty args"),
    )
}

fn get_runtime_view(pic: &PocketIc, canister_id: Principal) -> RuntimeView {
    call_query(
        pic,
        canister_id,
        "get_runtime_view",
        encode_args(()).expect("failed to encode empty args"),
    )
}

fn get_wallet_balance_telemetry(
    pic: &PocketIc,
    canister_id: Principal,
) -> WalletBalanceTelemetryView {
    call_query(
        pic,
        canister_id,
        "get_wallet_balance_telemetry",
        encode_args(()).expect("failed to encode get_wallet_balance_telemetry args"),
    )
}

fn get_spawn_bootstrap_view(pic: &PocketIc, canister_id: Principal) -> SpawnBootstrapView {
    call_query(
        pic,
        canister_id,
        "get_spawn_bootstrap_view",
        encode_args(()).expect("failed to encode get_spawn_bootstrap_view args"),
    )
}

fn get_prompt_layers(pic: &PocketIc, canister_id: Principal) -> Vec<PromptLayerView> {
    call_query(
        pic,
        canister_id,
        "get_prompt_layers",
        encode_args(()).expect("failed to encode empty args"),
    )
}

fn list_outbox_messages(pic: &PocketIc, canister_id: Principal, limit: u32) -> Vec<OutboxMessage> {
    let payload = encode_args((limit,)).unwrap_or_else(|error| {
        panic!("failed to encode list_outbox_messages args: {error}");
    });
    call_query(pic, canister_id, "list_outbox_messages", payload)
}

fn list_turns(pic: &PocketIc, canister_id: Principal, limit: u32) -> Vec<String> {
    let payload = encode_args((limit,)).unwrap_or_else(|error| {
        panic!("failed to encode list_turns args: {error}");
    });
    call_query(pic, canister_id, "list_turns", payload)
}

fn configure_only_agent_turn(pic: &PocketIc, canister_id: Principal, interval_secs: u64) {
    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(pic, canister_id, kind, false);
        set_task_interval_secs(pic, canister_id, kind, interval_secs);
    }
    set_task_enabled(pic, canister_id, TaskKind::AgentTurn, true);
}

fn configure_proxy_runtime(pic: &PocketIc, canister_id: Principal, worker: Principal) {
    set_inference_provider(pic, canister_id, InferenceProvider::OpenRouterProxyWorker);
    set_inference_model(pic, canister_id, "openai/gpt-4o-mini");
    set_openrouter_api_key(pic, canister_id, Some("sk-or-test".to_string()));
    set_evm_rpc_url(pic, canister_id, "https://mainnet.base.org");
    set_inference_proxy_config(
        pic,
        canister_id,
        OpenRouterProxyWorkerConfig {
            worker_base_url: "https://proxy.example.workers.dev".to_string(),
            trusted_callback_principal: Some(worker),
        },
    );
    for kind in [
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::TopUpCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(pic, canister_id, kind, false);
    }
    set_task_enabled(pic, canister_id, TaskKind::AgentTurn, true);
    set_task_interval_secs(pic, canister_id, TaskKind::AgentTurn, 1);
}

fn respond_with_proxy_submit_ack(
    pic: &PocketIc,
    request: CanisterHttpRequest,
) -> ProxySubmitCapture {
    let body: Value = serde_json::from_slice(&request.body)
        .unwrap_or_else(|error| panic!("failed to decode proxy submit body: {error}"));
    let job_id = body
        .get("job_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let turn_id = body
        .get("turn_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let system_prompt = body
        .get("inference_request")
        .and_then(|value| value.get("messages"))
        .and_then(Value::as_array)
        .and_then(|messages| messages.first())
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let ack = json!({
        "job_id": job_id,
        "accepted_at_ns": 123456789u64,
        "status": "accepted",
    });
    pic.mock_canister_http_response(MockCanisterHttpResponse {
        subnet_id: request.subnet_id,
        request_id: request.request_id,
        response: CanisterHttpResponse::CanisterHttpReply(CanisterHttpReply {
            status: 202,
            headers: vec![],
            body: serde_json::to_vec(&ack).expect("failed to encode proxy ack"),
        }),
        additional_responses: vec![],
    });

    ProxySubmitCapture {
        job_id,
        turn_id,
        system_prompt,
    }
}

fn drive_until_proxy_submit(pic: &PocketIc, canister_id: Principal) -> ProxySubmitCapture {
    let _ = canister_id;
    pic.advance_time(Duration::from_secs(2));
    for _ in 0..24 {
        pic.tick();
        let pending_http = pic.get_canister_http();
        if pending_http.is_empty() {
            pic.advance_time(Duration::from_secs(2));
            continue;
        }

        let capture = pending_http
            .into_iter()
            .next()
            .map(|request| respond_with_proxy_submit_ack(pic, request))
            .expect("proxy submit request should exist");
        pic.tick();
        return capture;
    }
    panic!("proxy submit was not observed within expected ticks");
}

fn submit_proxy_callback_result(
    pic: &PocketIc,
    canister_id: Principal,
    worker: Principal,
    capture: &ProxySubmitCapture,
    tool_calls: Vec<ToolCall>,
) {
    let payload = encode_args((SubmitInferenceResultArgs {
        job_id: capture.job_id.clone(),
        turn_id: capture.turn_id.clone(),
        completed_at_ns: 987654321,
        result: Some(InferenceProxyResultPayload {
            explanation: Some("async proxy result".to_string()),
            tool_calls,
        }),
        error: None,
    },))
    .expect("failed to encode submit_inference_result args");
    let result: Result<String, String> =
        call_update_as(pic, canister_id, worker, "submit_inference_result", payload);
    assert_eq!(
        result.expect("trusted proxy callback should succeed"),
        "inference_proxy_callback_accepted"
    );
}

fn wait_for_pending_proxy_job(pic: &PocketIc, canister_id: Principal) {
    for _ in 0..24 {
        let status = get_inference_proxy_status(pic, canister_id);
        if status.submit_accepted >= 1 && status.pending_jobs >= 1 {
            return;
        }
        pic.advance_time(Duration::from_secs(2));
        pic.tick();
    }
    panic!("pending proxy job was not observed within expected ticks");
}

fn wait_for_turn_counter(pic: &PocketIc, canister_id: Principal, minimum_turn_counter: u64) {
    for _ in 0..48 {
        if get_runtime_view(pic, canister_id).turn_counter >= minimum_turn_counter {
            return;
        }
        pic.advance_time(Duration::from_secs(2));
        pic.tick();
    }
    panic!("turn counter did not reach {minimum_turn_counter}");
}

fn evm_read_missing_calldata_tool_call() -> ToolCall {
    ToolCall {
        tool_call_id: None,
        tool: "evm_read".to_string(),
        args_json: json!({
            "method": "eth_call",
            "address": "0x1111111111111111111111111111111111111111"
        })
        .to_string(),
    }
}

fn drive_agent_turn_with_openrouter_status(
    pic: &PocketIc,
    canister_id: Principal,
    status: u16,
    body: Value,
) {
    let before_latest_job_id = list_scheduler_jobs(pic, canister_id)
        .into_iter()
        .filter(|job| job.kind == TaskKind::AgentTurn)
        .max_by_key(|job| job.created_at_ns)
        .map(|job| job.id);

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    for _ in 0..24 {
        let pending_http = pic.get_canister_http();
        if !pending_http.is_empty() {
            for request in pending_http {
                pic.mock_canister_http_response(MockCanisterHttpResponse {
                    subnet_id: request.subnet_id,
                    request_id: request.request_id,
                    response: CanisterHttpResponse::CanisterHttpReply(CanisterHttpReply {
                        status,
                        headers: vec![],
                        body: serde_json::to_vec(&body)
                            .expect("failed to encode openrouter mock response"),
                    }),
                    additional_responses: vec![],
                });
            }
        }

        pic.tick();
        let jobs = list_scheduler_jobs(pic, canister_id);
        let latest_job = jobs
            .iter()
            .filter(|job| job.kind == TaskKind::AgentTurn)
            .max_by_key(|job| job.created_at_ns);
        let terminal = latest_job
            .map(|job| matches!(job.status, JobStatus::Succeeded | JobStatus::Failed))
            .unwrap_or(false);
        let saw_new_job = latest_job
            .map(|job| before_latest_job_id.as_deref() != Some(job.id.as_str()))
            .unwrap_or(false);
        if saw_new_job && terminal && pic.get_canister_http().is_empty() {
            return;
        }

        pic.advance_time(Duration::from_secs(2));
    }

    panic!("agent turn did not finish with mocked openrouter response in expected ticks");
}

#[test]
fn loop_disabled_agent_turn_is_counted_as_successful_skip() {
    let (pic, canister_id) = with_backend_canister();
    configure_only_agent_turn(&pic, canister_id, 60);
    set_loop_enabled(&pic, canister_id, false);

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let jobs = list_scheduler_jobs(&pic, canister_id);
    let agent_jobs = jobs
        .iter()
        .filter(|job| job.kind == TaskKind::AgentTurn)
        .collect::<Vec<_>>();
    assert!(
        !agent_jobs.is_empty(),
        "expected an agent-turn scheduled job to be materialized"
    );
    assert!(
        agent_jobs
            .iter()
            .all(|job| job.status == JobStatus::Succeeded && job.last_error.is_none()),
        "disabled loop should not be marked as failed"
    );

    let runtime = get_runtime_view(&pic, canister_id);
    assert!(!runtime.loop_enabled);
    assert_ne!(
        runtime.state,
        AgentState::Faulted,
        "disabled loop skip must not transition runtime to faulted"
    );
}

#[test]
fn init_args_can_override_evm_chain_id() {
    let (pic, canister_id) = with_backend_canister_with_init_args(InitArgs {
        ecdsa_key_name: "dfx_test_key".to_string(),
        inbox_contract_address: None,
        evm_chain_id: Some(84532),
    });

    let runtime = get_runtime_view(&pic, canister_id);
    assert_eq!(runtime.evm_chain_id, 84532);
}

#[test]
fn agent_turn_self_recovers_after_transient_inference_failure() {
    let (pic, canister_id) = with_backend_canister();
    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 60);
    }

    let _ = ingest_messages_via_contract_poll(
        &pic,
        canister_id,
        &["force external-input failure path".to_string()],
    );
    set_task_enabled(&pic, canister_id, TaskKind::PollInbox, false);

    set_inference_provider(&pic, canister_id, InferenceProvider::OpenRouter);
    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);
    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let after_failure = get_runtime_view(&pic, canister_id);
    assert_eq!(
        after_failure.state,
        AgentState::Faulted,
        "failed inference turn should set runtime faulted"
    );
    assert!(
        after_failure.last_error.is_some(),
        "faulted state must preserve error context"
    );

    set_inference_provider(&pic, canister_id, InferenceProvider::IcLlm);
    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let after_recovery = get_runtime_view(&pic, canister_id);
    assert_eq!(
        after_recovery.state,
        AgentState::Sleeping,
        "next tick should autonomously recover from faulted state"
    );
    assert!(
        after_recovery.last_error.is_none(),
        "successful recovery turn should clear runtime error"
    );

    let jobs = list_scheduler_jobs(&pic, canister_id);
    let failed_count = jobs
        .iter()
        .filter(|job| job.kind == TaskKind::AgentTurn && job.status == JobStatus::Failed)
        .count();
    let succeeded_count = jobs
        .iter()
        .filter(|job| job.kind == TaskKind::AgentTurn && job.status == JobStatus::Succeeded)
        .count();

    assert!(failed_count >= 1, "expected at least one failed agent turn");
    assert!(
        succeeded_count >= 1,
        "expected a successful recovery agent turn"
    );
}

#[test]
fn poll_inbox_stages_messages_and_agent_turn_consumes_them() {
    let (pic, canister_id) = with_backend_canister();

    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 60);
    }

    let before = get_inbox_stats(&pic, canister_id);
    assert_eq!(before.total_messages, 0);
    assert_eq!(before.pending_count, 0);
    assert_eq!(before.staged_count, 0);
    assert_eq!(before.consumed_count, 0);

    let _ = ingest_messages_via_contract_poll(
        &pic,
        canister_id,
        &["hello one".to_string(), "hello two".to_string()],
    );

    let after_poll = get_inbox_stats(&pic, canister_id);
    assert_eq!(after_poll.pending_count, 0);
    assert_eq!(after_poll.staged_count, 2);
    assert_eq!(after_poll.consumed_count, 0);
    assert_eq!(
        get_runtime_view(&pic, canister_id).turn_counter,
        0,
        "poll inbox should not execute agent turns"
    );
    assert!(
        list_outbox_messages(&pic, canister_id, 10).is_empty(),
        "poll inbox should not emit outbox replies"
    );

    let poll_succeeded = list_scheduler_jobs(&pic, canister_id)
        .into_iter()
        .any(|job| job.kind == TaskKind::PollInbox && job.status == JobStatus::Succeeded);
    assert!(
        poll_succeeded,
        "poll inbox job should complete successfully"
    );

    set_task_enabled(&pic, canister_id, TaskKind::PollInbox, false);
    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let after_first_agent_turn = get_inbox_stats(&pic, canister_id);
    assert_eq!(after_first_agent_turn.pending_count, 0);
    assert_eq!(after_first_agent_turn.staged_count, 1);
    assert_eq!(after_first_agent_turn.consumed_count, 1);

    let agent_succeeded = list_scheduler_jobs(&pic, canister_id)
        .into_iter()
        .any(|job| job.kind == TaskKind::AgentTurn && job.status == JobStatus::Succeeded);
    assert!(agent_succeeded, "agent turn should complete successfully");

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let after_second_agent_turn = get_inbox_stats(&pic, canister_id);
    assert_eq!(after_second_agent_turn.pending_count, 0);
    assert_eq!(after_second_agent_turn.staged_count, 0);
    assert_eq!(after_second_agent_turn.consumed_count, 2);
}

#[test]
fn agent_turn_does_not_stage_pending_messages_without_poll_inbox() {
    let (pic, canister_id) = with_backend_canister();

    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 60);
    }

    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);
    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let stats = get_inbox_stats(&pic, canister_id);
    assert_eq!(stats.total_messages, 0);
    assert_eq!(stats.pending_count, 0);
    assert_eq!(stats.staged_count, 0);
    assert_eq!(stats.consumed_count, 0);

    let runtime = get_runtime_view(&pic, canister_id);
    assert_eq!(runtime.turn_counter, 1, "agent turn should still execute");

    assert!(
        list_outbox_messages(&pic, canister_id, 10).is_empty(),
        "agent turn should not reply without staged input"
    );
}

#[test]
fn agent_blocks_inbox_prompt_layer_update_and_keeps_layer_6_default() {
    let (pic, canister_id) = with_backend_canister();

    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 60);
    }
    set_inference_provider(&pic, canister_id, InferenceProvider::IcLlm);

    let ingested_ids = ingest_messages_via_contract_poll(
        &pic,
        canister_id,
        &[
            "request_update_prompt_layer:true".to_string(),
            "request_layer_6_probe:true".to_string(),
        ],
    );
    let update_message_id = ingested_ids[0].clone();
    let probe_message_id = ingested_ids[1].clone();

    set_task_enabled(&pic, canister_id, TaskKind::PollInbox, false);
    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);
    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let layer_6 = get_prompt_layers(&pic, canister_id)
        .into_iter()
        .find(|layer| layer.layer_id == 6)
        .expect("layer 6 should be returned");
    assert!(layer_6.is_mutable);
    assert!(
        !layer_6.content.contains("phase5-layer6-marker"),
        "inbox-driven prompt layer mutation should be blocked"
    );

    let update_outbox = list_outbox_messages(&pic, canister_id, 20)
        .into_iter()
        .find(|message| message.source_inbox_ids.contains(&update_message_id))
        .expect("first turn should produce outbox response");
    assert!(
        update_outbox.body.contains("deterministic continuation"),
        "outbox should prefer continuation text after tool execution"
    );
    let turns_after_update = list_turns(&pic, canister_id, 5);
    assert!(
        turns_after_update
            .iter()
            .any(|turn| turn.contains("tool sequence validator blocked")),
        "turn logs should capture sequence-validator blocks for sensitive tools"
    );

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let probe_outbox = list_outbox_messages(&pic, canister_id, 20)
        .into_iter()
        .find(|message| message.source_inbox_ids.contains(&probe_message_id))
        .expect("second turn should produce outbox response");
    assert!(
        probe_outbox.body.contains("deterministic continuation"),
        "continuation response should be posted as final outbox body"
    );

    let turns = list_turns(&pic, canister_id, 5);
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("layer6_probe:missing")),
        "turn logs should show that layer 6 remained unchanged"
    );
}

#[test]
fn agent_continues_after_tool_results_and_posts_final_reply_continuation() {
    let (pic, canister_id) = with_backend_canister();

    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 60);
    }
    set_inference_provider(&pic, canister_id, InferenceProvider::IcLlm);

    let message_id = ingest_messages_via_contract_poll(
        &pic,
        canister_id,
        &["request_update_prompt_layer:true".to_string()],
    )[0]
    .clone();

    set_task_enabled(&pic, canister_id, TaskKind::PollInbox, false);

    let runtime_before_agent = get_runtime_view(&pic, canister_id);
    assert_eq!(
        runtime_before_agent.turn_counter, 0,
        "staging input should not increment turn counter"
    );

    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);
    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let runtime_after_agent = get_runtime_view(&pic, canister_id);
    assert_eq!(
        runtime_after_agent.turn_counter, 1,
        "single scheduler tick should complete one agent turn"
    );
    assert_eq!(
        runtime_after_agent.state,
        AgentState::Sleeping,
        "agent turn should complete successfully"
    );

    let outbox = list_outbox_messages(&pic, canister_id, 20);
    let turn_outbox = outbox
        .into_iter()
        .find(|message| message.source_inbox_ids.contains(&message_id))
        .expect("agent turn should post an outbox response");
    assert!(
        turn_outbox.body.contains("deterministic continuation"),
        "reply body should come from continuation round after tool output"
    );
    let turns = list_turns(&pic, canister_id, 5);
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("tool sequence validator blocked")),
        "inbox-driven sensitive tool should be blocked but not abort the turn"
    );

    let agent_turn_jobs = list_scheduler_jobs(&pic, canister_id)
        .into_iter()
        .filter(|job| job.kind == TaskKind::AgentTurn)
        .collect::<Vec<_>>();
    assert_eq!(
        agent_turn_jobs.len(),
        1,
        "expected one materialized agent-turn job"
    );
    assert_eq!(agent_turn_jobs[0].status, JobStatus::Succeeded);
}

#[test]
fn autonomy_turn_round_trips_policy_and_persists_executed_decision() {
    let (pic, canister_id) = with_backend_canister();
    configure_only_agent_turn(&pic, canister_id, 60);

    let policy = AutonomyPolicy {
        version: 7,
        reserve_policy: ReservePolicy {
            min_cycles_runway_hours: 96,
            min_inference_usdc_6dp: Some(250_000),
            min_gas_wei: Some(42),
        },
        risk_limits: RiskLimits {
            max_total_exposure_bps: 900,
            max_single_action_bps: 250,
            max_protocol_concentration_bps: 333,
        },
        execution_authority: ExecutionAuthority {
            autonomous_execution_enabled: true,
            require_simulation_first: true,
            per_action_value_limit_wei: Some(999),
        },
        escalation_rules: EscalationRules {
            escalate_on_missing_policy: true,
            escalate_on_authority_exceeded: true,
            escalate_on_repeated_failure: true,
            failure_quarantine_threshold: 4,
        },
        updated_at_ns: 123_456_789,
    };
    let stored = update_autonomy_policy(&pic, canister_id, policy.clone())
        .expect("policy update should succeed");
    assert_eq!(stored, policy);
    assert_eq!(get_autonomy_policy(&pic, canister_id), policy);

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let decisions = get_recent_decisions(&pic, canister_id);
    assert_eq!(decisions.len(), 1);
    let decision = &decisions[0];
    assert_eq!(decision.policy_version, 7);
    assert_eq!(decision.trigger, DecisionTrigger::ScheduledReview);
    assert_eq!(
        decision.outcome,
        DecisionOutcome::Executed {
            action_summary: "record_signal(tick)".to_string(),
        }
    );
    assert_eq!(decision.candidates_summary, "policy-bounded autonomy turn");
    assert!(
        decision
            .explanation
            .contains("deterministic autonomy decision envelope"),
        "decision explanation should come from the deterministic retry path"
    );

    assert!(get_active_exposures(&pic, canister_id).is_empty());
    assert!(get_strategy_quarantines(&pic, canister_id).is_empty());
    assert_eq!(
        get_exposure_reconciliation_status(&pic, canister_id),
        ExposureReconciliationStatus::default()
    );

    let turns = list_turns(&pic, canister_id, 5);
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("inference_round_count: 3")),
        "scheduled autonomy turn should retry through the tool and decision-envelope phases before persisting"
    );
}

#[test]
fn scheduled_review_with_fresh_reserve_shortfall_records_canonical_noop_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();

    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 60);
    }

    let _ = configure_route_for_polling(&pic, canister_id);
    set_task_enabled(&pic, canister_id, TaskKind::PollInbox, true);
    pic.advance_time(Duration::from_secs(61));
    drive_poll_inbox_with_http_mocks_and_wallet(
        &pic,
        canister_id,
        2,
        &[],
        "0x1",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
    );

    let telemetry = get_wallet_balance_telemetry(&pic, canister_id);
    assert_eq!(telemetry.status, WalletBalanceStatus::Fresh);
    assert_eq!(telemetry.eth_balance_wei_hex.as_deref(), Some("0x1"));
    assert_eq!(telemetry.usdc_balance_raw_hex.as_deref(), Some("0x1"));
    assert_eq!(
        get_spawn_bootstrap_view(&pic, canister_id).factory_principal,
        None,
        "test should not expose a factory coordination lane"
    );

    let policy = AutonomyPolicy {
        version: 9,
        reserve_policy: ReservePolicy {
            min_cycles_runway_hours: 72,
            min_inference_usdc_6dp: Some(250_000),
            min_gas_wei: Some(42),
        },
        risk_limits: RiskLimits {
            max_total_exposure_bps: 3_000,
            max_single_action_bps: 1_000,
            max_protocol_concentration_bps: 1_500,
        },
        execution_authority: ExecutionAuthority {
            autonomous_execution_enabled: true,
            require_simulation_first: true,
            per_action_value_limit_wei: Some(50_000_000_000_000_000),
        },
        escalation_rules: EscalationRules {
            escalate_on_missing_policy: true,
            escalate_on_authority_exceeded: true,
            escalate_on_repeated_failure: true,
            failure_quarantine_threshold: 3,
        },
        updated_at_ns: 123_456_789,
    };
    let stored = update_autonomy_policy(&pic, canister_id, policy.clone())
        .expect("policy update should succeed");
    assert_eq!(stored, policy);

    set_task_enabled(&pic, canister_id, TaskKind::PollInbox, false);
    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);
    drive_agent_turn_with_openrouter_status(
        &pic,
        canister_id,
        200,
        json!({
            "choices": [{
                "message": {
                    "content": "{\"trigger\":\"ScheduledReview\",\"candidates_summary\":\"unexpected\",\"outcome\":{\"NoOp\":{\"reason\":\"unexpected\"}},\"explanation\":\"unexpected\"}"
                }
            }]
        }),
    );
    assert!(
        pic.get_canister_http().is_empty(),
        "fresh reserve shortfall should bypass provider inference when no room coordination lane exists"
    );

    let decisions = get_recent_decisions(&pic, canister_id);
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        decisions[0].outcome,
        DecisionOutcome::NoOp {
            reason: "reserve_insufficient".to_string(),
        }
    );
    assert_eq!(decisions[0].policy_version, 9);
    assert_eq!(
        decisions[0].candidates_summary,
        "checked reserves against policy minimums"
    );
    assert!(
        decisions[0]
            .explanation
            .contains("capital-touching actions blocked by reserve shortfall"),
        "runtime should supply the canonical reserve explanation"
    );

    let turns = list_turns(&pic, canister_id, 5);
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("autonomy reserve restriction:")),
        "turn log should record reserve restriction context"
    );
    assert!(
        turns
            .iter()
            .all(|turn| !turn.contains("decision envelope invalid")),
        "reserve-restricted turns should not be relabeled as invalid decision envelopes"
    );
}

#[test]
fn repeated_openrouter_auth_failures_suppress_future_autonomy_turns_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();
    configure_only_agent_turn(&pic, canister_id, 60);
    set_task_enabled(&pic, canister_id, TaskKind::TopUpCycles, false);
    set_inference_provider(&pic, canister_id, InferenceProvider::OpenRouter);
    set_openrouter_api_key(&pic, canister_id, Some("sk-or-test".to_string()));

    let unauthorized = json!({
        "error": {
            "message": "unauthorized"
        }
    });
    for _ in 0..3 {
        drive_agent_turn_with_openrouter_status(&pic, canister_id, 401, unauthorized.clone());
    }

    pic.advance_time(Duration::from_secs(61));
    pic.tick();
    assert!(
        pic.get_canister_http().is_empty(),
        "suppressed autonomy turn should skip provider inference"
    );

    let decisions = get_recent_decisions(&pic, canister_id);
    let latest = decisions
        .first()
        .expect("suppressed autonomy turn should persist a decision");
    assert_eq!(
        latest.outcome,
        DecisionOutcome::NoOp {
            reason: "inference_provider_rejected".to_string(),
        }
    );

    let turns = list_turns(&pic, canister_id, 10);
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("autonomy inference suppressed: provider rejected")),
        "turn log should show provider-rejection suppression"
    );
    assert!(
        turns
            .iter()
            .all(|turn| !turn.contains("decision envelope invalid")),
        "suppressed autonomy turns should not be relabeled as invalid envelopes"
    );
}

#[test]
fn invalid_autonomy_decision_envelope_falls_back_to_noop_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();
    configure_only_agent_turn(&pic, canister_id, 60);

    let updated = update_prompt_layer_admin(
        &pic,
        canister_id,
        6,
        "## Layer 6: Economic Decision Loop\n- request_autonomy_decision_invalid_persistent:true",
    )
    .expect("prompt layer update should succeed");
    assert_eq!(updated.layer_id, 6);
    assert!(updated
        .content
        .contains("request_autonomy_decision_invalid_persistent:true"));

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let decisions = get_recent_decisions(&pic, canister_id);
    assert_eq!(decisions.len(), 1);
    let decision = &decisions[0];
    assert_eq!(decision.policy_version, 1);
    assert_eq!(decision.trigger, DecisionTrigger::ScheduledReview);
    assert_eq!(
        decision.outcome,
        DecisionOutcome::NoOp {
            reason: "invalid_decision_shape".to_string(),
        }
    );
    assert!(
        decision.explanation.contains("decision envelope invalid"),
        "turn should preserve the invalid-envelope explanation for auditability"
    );

    let turns = list_turns(&pic, canister_id, 5);
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("inference_round_count: 2")),
        "invalid autonomy envelopes should be retried once before falling back to NoOp"
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("decision envelope invalid")),
        "turn log should preserve the invalid-envelope retry trace"
    );
}

#[test]
fn no_input_continuation_inference_error_records_inference_noop_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();
    configure_only_agent_turn(&pic, canister_id, 60);

    let updated = update_prompt_layer_admin(
        &pic,
        canister_id,
        6,
        "## Layer 6: Economic Decision Loop\n- request_continuation_error:true",
    )
    .expect("prompt layer update should succeed");
    assert_eq!(updated.layer_id, 6);
    assert!(updated.content.contains("request_continuation_error:true"));

    pic.advance_time(Duration::from_secs(61));
    pic.tick();

    let decisions = get_recent_decisions(&pic, canister_id);
    assert_eq!(decisions.len(), 1);
    let decision = &decisions[0];
    assert_eq!(decision.policy_version, 1);
    assert_eq!(decision.trigger, DecisionTrigger::ScheduledReview);
    assert_eq!(
        decision.outcome,
        DecisionOutcome::NoOp {
            reason: "inference_error".to_string(),
        }
    );
    assert!(
        decision
            .explanation
            .contains("deterministic continuation inference failed after tool execution"),
        "turn should preserve the continuation inference failure for auditability"
    );

    let turns = list_turns(&pic, canister_id, 5);
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("inference_round_count: 2")),
        "continuation inference failures after tools should stop after the degraded continuation round"
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.contains("continuation inference degraded after tool execution")),
        "turn log should preserve the degraded continuation reason"
    );
    assert!(
        turns
            .iter()
            .all(|turn| !turn.contains("decision envelope invalid")),
        "inference failures after tools must not be relabeled as invalid decision envelopes"
    );
}

#[test]
fn non_controller_cannot_mutate_inference_config_or_control_plane() {
    let (pic, canister_id) = with_backend_canister();
    let outsider = non_controller_principal();

    let set_loop_payload =
        encode_args((false,)).unwrap_or_else(|error| panic!("failed to encode payload: {error}"));
    let set_loop_result =
        pic.update_call(canister_id, outsider, "set_loop_enabled", set_loop_payload);
    assert!(
        set_loop_result.is_err(),
        "set_loop_enabled should reject non-controller callers"
    );

    let inference_payload = encode_args((InferenceProvider::OpenRouter,))
        .unwrap_or_else(|error| panic!("failed to encode payload: {error}"));
    let inference_call_result = pic.update_call(
        canister_id,
        outsider,
        "set_inference_provider",
        inference_payload,
    );
    assert!(
        inference_call_result.is_err(),
        "set_inference_provider should reject non-controller callers"
    );

    let model_payload = encode_args(("openai/gpt-4o-mini".to_string(),))
        .unwrap_or_else(|error| panic!("failed to encode payload: {error}"));
    let model_result: Result<String, String> = call_update_as(
        &pic,
        canister_id,
        outsider,
        "set_inference_model",
        model_payload,
    );
    assert_eq!(
        model_result,
        Err("caller is not a controller".to_string()),
        "set_inference_model should enforce controller authorization"
    );

    let api_key_payload = encode_args((Some("test-openrouter-key".to_string()),))
        .unwrap_or_else(|error| panic!("failed to encode payload: {error}"));
    let api_key_call_result = pic.update_call(
        canister_id,
        outsider,
        "set_openrouter_api_key",
        api_key_payload,
    );
    assert!(
        api_key_call_result.is_err(),
        "set_openrouter_api_key should reject non-controller callers"
    );

    let rpc_payload = encode_args(("https://mainnet.base.org".to_string(),))
        .unwrap_or_else(|error| panic!("failed to encode payload: {error}"));
    let rpc_result: Result<String, String> =
        call_update_as(&pic, canister_id, outsider, "set_evm_rpc_url", rpc_payload);
    assert_eq!(
        rpc_result,
        Err("caller is not a controller".to_string()),
        "set_evm_rpc_url should enforce controller authorization"
    );

    let post_payload = encode_args(("public ingress".to_string(),))
        .unwrap_or_else(|error| panic!("failed to encode payload: {error}"));
    let post_result = pic.update_call(canister_id, outsider, "post_inbox_message", post_payload);
    assert!(
        post_result.is_err(),
        "post_inbox_message should be removed from public ingress"
    );
}

#[test]
fn high_volume_agent_turn_flow_keeps_forward_progress_with_retention_enabled() {
    let (pic, canister_id) = with_backend_canister();

    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 1);
    }
    set_inference_provider(&pic, canister_id, InferenceProvider::IcLlm);
    set_retention_config(
        &pic,
        canister_id,
        RetentionConfig {
            jobs_max_age_secs: 2,
            jobs_max_records: 64,
            dedupe_max_age_secs: 2,
            turns_max_age_secs: 2,
            transitions_max_age_secs: 2,
            tools_max_age_secs: 2,
            inbox_max_age_secs: 2,
            outbox_max_age_secs: 2,
            memory_facts_max_age_secs: 3 * 24 * 60 * 60,
            memory_facts_prune_batch_size: 25,
            maintenance_batch_size: 128,
            maintenance_interval_secs: 1,
        },
    );

    let burst_messages = (0..120)
        .map(|idx| format!("phase3-burst-{idx:03}"))
        .collect::<Vec<_>>();
    let ingested_ids = ingest_messages_via_contract_poll(&pic, canister_id, &burst_messages);
    assert_eq!(ingested_ids.len(), 120);

    set_task_enabled(&pic, canister_id, TaskKind::PollInbox, false);
    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);

    // The canister scheduler timer ticks every `timing::SCHEDULER_TICK_INTERVAL_SECS`
    // (2 s in test builds, 30 s in production), so we run a long enough
    // simulated window to guarantee at least 8 agent turns.
    for _ in 0..260 {
        pic.advance_time(Duration::from_secs(2));
        pic.tick();
    }

    let runtime = get_runtime_view(&pic, canister_id);
    assert!(
        runtime.turn_counter >= 8,
        "agent should continue completing turns under sustained high-volume ingress"
    );
    assert!(
        runtime.last_error.is_none(),
        "runtime should not fault while retention and summarization run"
    );

    let stats = get_inbox_stats(&pic, canister_id);
    assert!(
        stats.staged_count <= (burst_messages.len() as u64).saturating_sub(8),
        "agent should drain a meaningful portion of the staged inbox backlog even when consumed rows are pruned"
    );

    let retained_outbox = list_outbox_messages(&pic, canister_id, 200);
    assert!(
        retained_outbox.len() <= 90,
        "retention should bound outbox history under sustained load"
    );

    let retained_jobs = list_scheduler_jobs(&pic, canister_id);
    assert!(
        retained_jobs.len() <= 90,
        "retention should bound scheduler job history under sustained load"
    );
}

#[test]
fn reflection_memory_persists_from_degraded_proxy_resume_turn() {
    let (pic, canister_id) = with_backend_canister();
    let worker = Principal::self_authenticating(b"reflection-proxy-worker");
    configure_proxy_runtime(&pic, canister_id, worker);

    let first_submit = drive_until_proxy_submit(&pic, canister_id);
    assert!(
        !first_submit.system_prompt.contains("### Reflection Memory"),
        "fresh autonomous submit should not include reflection section before any degraded turn"
    );
    wait_for_pending_proxy_job(&pic, canister_id);

    submit_proxy_callback_result(
        &pic,
        canister_id,
        worker,
        &first_submit,
        vec![evm_read_missing_calldata_tool_call()],
    );
    wait_for_turn_counter(&pic, canister_id, 2);

    let records = list_reflection_memory(&pic, canister_id, 10);
    assert_eq!(
        records.len(),
        1,
        "degraded proxy-resumed turn should persist a lesson"
    );
    assert_eq!(records[0].tool, "evm_read");
    assert_eq!(records[0].subject, "eth_call");
    assert_eq!(records[0].last_origin, ReflectionOrigin::Autonomy);
}
