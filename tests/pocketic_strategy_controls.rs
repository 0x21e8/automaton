#![cfg(feature = "pocketic_tests")]

use std::path::Path;
use std::time::Duration;

use candid::{decode_one, encode_args, CandidType, Principal};
use pocket_ic::common::rest::{
    CanisterHttpReply, CanisterHttpRequest, CanisterHttpResponse, MockCanisterHttpResponse,
};
use pocket_ic::PocketIc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const WASM_PATHS: &[&str] = &[
    "target/wasm32-wasip1/release/backend_nowasi.wasm",
    "target/wasm32-wasip1/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/deps/backend.wasm",
];

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InitArgs {
    ecdsa_key_name: String,
    inbox_contract_address: Option<String>,
    evm_chain_id: Option<u64>,
    evm_rpc_url: Option<String>,
    evm_confirmation_depth: Option<u64>,
    http_allowed_domains: Option<Vec<String>>,
    llm_canister_id: Option<Principal>,
    cycle_topup_enabled: Option<bool>,
    auto_topup_cycle_threshold: Option<u64>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StrategyTemplateKey {
    protocol: String,
    primitive: String,
    chain_id: u64,
    template_id: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum TemplateStatus {
    Draft,
    Active,
    Deprecated,
    Revoked,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ContractRoleBinding {
    role: String,
    address: String,
    source_ref: String,
    codehash: Option<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct AbiTypeSpec {
    name: String,
    kind: String,
    components: Vec<AbiTypeSpec>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct AbiFunctionSpec {
    role: String,
    name: String,
    selector_hex: String,
    inputs: Vec<AbiTypeSpec>,
    outputs: Vec<AbiTypeSpec>,
    state_mutability: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ActionSpec {
    action_id: String,
    call_sequence: Vec<AbiFunctionSpec>,
    preconditions: Vec<String>,
    postconditions: Vec<String>,
    risk_checks: Vec<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StrategyTemplate {
    key: StrategyTemplateKey,
    status: TemplateStatus,
    contract_roles: Vec<ContractRoleBinding>,
    actions: Vec<ActionSpec>,
    constraints_json: String,
    created_at_ns: u64,
    updated_at_ns: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[allow(dead_code)]
struct TemplateActivationState {
    key: StrategyTemplateKey,
    enabled: bool,
    updated_at_ns: u64,
    reason: Option<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StrategyKillSwitchState {
    key: StrategyTemplateKey,
    enabled: bool,
    updated_at_ns: u64,
    reason: Option<String>,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
enum TaskKind {
    AgentTurn,
    PollInbox,
    CheckCycles,
    TopUpCycles,
    Reconcile,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
enum InferenceProvider {
    IcLlm,
    OpenRouter,
    OpenRouterProxyWorker,
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

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum ToolCallOutcome {
    Executed,
    SuppressedDedupe,
    SuppressedFailureCooldown,
    BlockedSequence,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum ToolFailureKind {
    MalformedInput,
    OutcallFailure,
    InternalFailure,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ToolCallRecord {
    turn_id: String,
    tool: String,
    args_json: String,
    output: String,
    success: bool,
    outcome: ToolCallOutcome,
    error: Option<String>,
    failure_kind: Option<ToolFailureKind>,
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

fn with_backend_canister() -> (PocketIc, Principal) {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    let wasm = assert_wasm_artifact_present();
    let init_args = encode_args((InitArgs {
        ecdsa_key_name: "dfx_test_key".to_string(),
        inbox_contract_address: None,
        evm_chain_id: Some(8453),
        evm_rpc_url: Some("https://mainnet.base.org".to_string()),
        evm_confirmation_depth: None,
        http_allowed_domains: None,
        llm_canister_id: None,
        cycle_topup_enabled: None,
        auto_topup_cycle_threshold: None,
    },))
    .expect("failed to encode init args");

    pic.add_cycles(canister_id, 2_000_000_000_000);
    pic.install_canister(canister_id, wasm, init_args, None);

    (pic, canister_id)
}

fn call_update<T>(pic: &PocketIc, canister_id: Principal, method: &str, payload: Vec<u8>) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .update_call(canister_id, Principal::anonymous(), method, payload)
        .unwrap_or_else(|error| panic!("update call {method} failed: {error:?}"));
    decode_one(&response)
        .unwrap_or_else(|error| panic!("failed decoding {method} response: {error:?}"))
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

fn get_runtime_view(pic: &PocketIc, canister_id: Principal) -> RuntimeView {
    call_query(
        pic,
        canister_id,
        "get_runtime_view",
        encode_args(()).expect("failed to encode get_runtime_view args"),
    )
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
    let payload = encode_args((api_key,)).unwrap_or_else(|error| {
        panic!("failed to encode set_openrouter_api_key args: {error}");
    });
    let _: String = call_update(pic, canister_id, "set_openrouter_api_key", payload);
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
    let _: Result<String, String> =
        call_update(pic, canister_id, "set_task_interval_secs", payload);
}

fn configure_only_agent_turn(pic: &PocketIc, canister_id: Principal, interval_secs: u64) {
    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::TopUpCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(pic, canister_id, kind, false);
        set_task_interval_secs(pic, canister_id, kind, interval_secs);
    }
    set_task_enabled(pic, canister_id, TaskKind::AgentTurn, true);
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

fn get_tool_calls_for_turn(
    pic: &PocketIc,
    canister_id: Principal,
    turn_id: &str,
) -> Vec<ToolCallRecord> {
    call_query(
        pic,
        canister_id,
        "get_tool_calls_for_turn",
        encode_args((turn_id.to_string(),)).expect("failed to encode get_tool_calls_for_turn args"),
    )
}

fn response_body_for_openrouter_request(request: &CanisterHttpRequest) -> Vec<u8> {
    let request_json: Value = serde_json::from_slice(&request.body)
        .unwrap_or_else(|error| panic!("failed to decode openrouter request body: {error}"));
    let messages = request_json
        .get("messages")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("openrouter request missing messages array"));
    let has_tool_message = messages.iter().any(|message| {
        message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| role == "tool")
    });

    let response = if has_tool_message {
        json!({
            "choices": [{
                "message": {
                    "content": json!({
                        "trigger": "ScheduledReview",
                        "candidates_summary": "strategy gate audit complete",
                        "outcome": {
                            "NoOp": {
                                "reason": "finalized"
                            }
                        },
                        "explanation": "deterministic strategy turn complete"
                    })
                    .to_string()
                }
            }]
        })
    } else {
        json!({
            "choices": [{
                "message": {
                    "content": "planning strategy action",
                    "tool_calls": [
                        {
                            "id": "call-simulate",
                            "type": "function",
                            "function": {
                                "name": "simulate_strategy_action",
                                "arguments": json!({
                                    "key": sample_key(),
                                    "action_id": "transfer",
                                    "typed_params": {
                                        "calls": [{
                                            "value_wei": "1",
                                            "args": [
                                                "0x3333333333333333333333333333333333333333",
                                                "1"
                                            ]
                                        }]
                                    }
                                })
                                .to_string()
                            }
                        },
                        {
                            "id": "call-execute",
                            "type": "function",
                            "function": {
                                "name": "execute_strategy_action",
                                "arguments": json!({
                                    "key": sample_key(),
                                    "action_id": "transfer",
                                    "typed_params": {
                                        "calls": [{
                                            "value_wei": "1",
                                            "args": [
                                                "0x3333333333333333333333333333333333333333",
                                                "1"
                                            ]
                                        }]
                                    }
                                })
                                .to_string()
                            }
                        }
                    ]
                }
            }]
        })
    };

    serde_json::to_vec(&response).expect("failed to encode mock openrouter response")
}

fn response_body_for_cross_round_openrouter_request(request: &CanisterHttpRequest) -> Vec<u8> {
    let request_json: Value = serde_json::from_slice(&request.body)
        .unwrap_or_else(|error| panic!("failed to decode openrouter request body: {error}"));
    let messages = request_json
        .get("messages")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("openrouter request missing messages array"));
    let tool_message_count = messages
        .iter()
        .filter(|message| {
            message
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role == "tool")
        })
        .count();

    let response = match tool_message_count {
        0 => json!({
            "choices": [{
                "message": {
                    "content": "simulate first",
                    "tool_calls": [{
                        "id": "call-simulate",
                        "type": "function",
                        "function": {
                            "name": "simulate_strategy_action",
                            "arguments": json!({
                                "key": sample_key(),
                                "action_id": "transfer",
                                "typed_params": {
                                    "calls": [{
                                        "value_wei": "1",
                                        "args": [
                                            "0x3333333333333333333333333333333333333333",
                                            "1"
                                        ]
                                    }]
                                }
                            })
                            .to_string()
                        }
                    }]
                }
            }]
        }),
        1 => json!({
            "choices": [{
                "message": {
                    "content": "execute after same-turn simulation",
                    "tool_calls": [{
                        "id": "call-execute",
                        "type": "function",
                        "function": {
                            "name": "execute_strategy_action",
                            "arguments": json!({
                                "key": sample_key(),
                                "action_id": "transfer",
                                "typed_params_json": "{\"calls\":[{\"value_wei\":\"1\",\"args\":{\"amount\":\"1\",\"to\":\"0x3333333333333333333333333333333333333333\"}}]}"
                            })
                            .to_string()
                        }
                    }]
                }
            }]
        }),
        _ => json!({
            "choices": [{
                "message": {
                    "content": json!({
                        "trigger": "ScheduledReview",
                        "candidates_summary": "same-turn verified intent complete",
                        "outcome": {
                            "NoOp": {
                                "reason": "finalized"
                            }
                        },
                        "explanation": "cross-round strategy turn complete"
                    })
                    .to_string()
                }
            }]
        }),
    };

    serde_json::to_vec(&response).expect("failed to encode mock openrouter response")
}

fn drive_openrouter_strategy_turn_with_responder(
    pic: &PocketIc,
    canister_id: Principal,
    responder: fn(&CanisterHttpRequest) -> Vec<u8>,
) -> String {
    let starting_turn_counter = get_runtime_view(pic, canister_id).turn_counter;
    for _ in 0..64 {
        let pending_http = pic.get_canister_http();
        if !pending_http.is_empty() {
            for request in pending_http {
                let body = responder(&request);
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

        pic.advance_time(Duration::from_secs(1));
        pic.tick();

        let runtime = get_runtime_view(pic, canister_id);
        if runtime.turn_counter > starting_turn_counter
            && pic.get_canister_http().is_empty()
            && runtime.last_turn_id.as_ref().is_some_and(|turn_id| {
                !get_tool_calls_for_turn(pic, canister_id, turn_id).is_empty()
            })
        {
            return runtime
                .last_turn_id
                .expect("turn id should be present when tool calls exist");
        }
    }

    panic!("autonomous strategy turn did not complete in expected ticks");
}

fn drive_openrouter_strategy_turn(pic: &PocketIc, canister_id: Principal) -> String {
    drive_openrouter_strategy_turn_with_responder(
        pic,
        canister_id,
        response_body_for_openrouter_request,
    )
}

fn wait_for_turn_tool_calls_with_responder(
    pic: &PocketIc,
    canister_id: Principal,
    target_turn_id: &str,
    responder: fn(&CanisterHttpRequest) -> Vec<u8>,
) -> String {
    for _ in 0..64 {
        let pending_http = pic.get_canister_http();
        if !pending_http.is_empty() {
            for request in pending_http {
                let body = responder(&request);
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

        pic.advance_time(Duration::from_secs(1));
        pic.tick();

        if !get_tool_calls_for_turn(pic, canister_id, target_turn_id).is_empty() {
            return target_turn_id.to_string();
        }
    }

    panic!("turn {target_turn_id} did not produce tool calls in expected ticks");
}

fn register_strategy_admin(
    pic: &PocketIc,
    canister_id: Principal,
    recipe_json: String,
) -> Result<StrategyTemplate, String> {
    call_update(
        pic,
        canister_id,
        "register_strategy_admin",
        encode_args((recipe_json,)).expect("failed to encode register_strategy_admin args"),
    )
}

fn sample_key() -> StrategyTemplateKey {
    StrategyTemplateKey {
        protocol: "erc20".to_string(),
        primitive: "transfer".to_string(),
        chain_id: 8453,
        template_id: "pocketic-strategy".to_string(),
    }
}

fn sample_recipe_json() -> String {
    r#"{
        "protocol": "erc20",
        "primitive": "transfer",
        "chain_id": 8453,
        "template_id": "pocketic-strategy",
        "contracts": [
            {
                "role": "token",
                "address": "0x2222222222222222222222222222222222222222",
                "abi_json": "[{\"type\":\"function\",\"name\":\"transfer\",\"stateMutability\":\"nonpayable\",\"inputs\":[{\"name\":\"to\",\"type\":\"address\"},{\"name\":\"amount\",\"type\":\"uint256\"}],\"outputs\":[{\"name\":\"success\",\"type\":\"bool\"}]}]",
                "source_ref": "https://example.com/token-abi"
            }
        ],
        "actions": [
            {
                "action_id": "transfer",
                "calls": [{ "role": "token", "function": "transfer" }],
                "preconditions": ["allowance_ok"],
                "postconditions": ["balance_delta_positive"],
                "risk_checks": ["max_notional"]
            }
        ],
        "max_value_wei_per_call": "100",
        "template_budget_wei": "100"
    }"#
    .to_string()
}

fn recursive_name_key() -> StrategyTemplateKey {
    StrategyTemplateKey {
        protocol: "morpho-v1".to_string(),
        primitive: "lend_supply".to_string(),
        chain_id: 8453,
        template_id: "pocketic-recursive-names".to_string(),
    }
}

fn recursive_name_recipe_json() -> String {
    r#"{
        "protocol": "morpho-v1",
        "primitive": "lend_supply",
        "chain_id": 8453,
        "template_id": "pocketic-recursive-names",
        "contracts": [
            {
                "role": "morpho",
                "address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "abi_json": "[{\"type\":\"function\",\"name\":\"supply\",\"stateMutability\":\"nonpayable\",\"inputs\":[{\"name\":\"marketParams\",\"type\":\"tuple\",\"components\":[{\"name\":\"loanToken\",\"type\":\"address\"},{\"type\":\"address\"},{\"name\":\"loanToken\",\"type\":\"address\"},{\"type\":\"uint256\"}]},{\"name\":\"assets\",\"type\":\"uint256\"}],\"outputs\":[{\"name\":\"shares\",\"type\":\"uint256\"}]}]",
                "source_ref": "https://example.com/morpho-abi"
            }
        ],
        "actions": [
            {
                "action_id": "enter_supply",
                "calls": [{ "role": "morpho", "function": "supply" }],
                "preconditions": ["market_open"],
                "postconditions": ["position_increased"],
                "risk_checks": ["max_notional"]
            }
        ],
        "max_value_wei_per_call": "0",
        "template_budget_wei": "0"
    }"#
    .to_string()
}

#[test]
fn strategy_register_and_lifecycle_work_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();

    // Register strategy from recipe (single call replaces ingest+ingest+activate)
    let recipe_json = sample_recipe_json();
    let payload = encode_args((recipe_json,))
        .unwrap_or_else(|error| panic!("failed to encode register_strategy_admin args: {error}"));
    let registered: Result<StrategyTemplate, String> =
        call_update(&pic, canister_id, "register_strategy_admin", payload);
    let template = registered.unwrap_or_else(|error| panic!("register failed: {error}"));
    assert!(
        matches!(template.status, TemplateStatus::Active),
        "register_strategy_admin should auto-activate"
    );
    assert_eq!(template.key, sample_key());

    // Verify template is listable and active
    let listed: Vec<StrategyTemplate> = call_query(
        &pic,
        canister_id,
        "list_strategy_templates",
        encode_args((Some(sample_key()), 10u32))
            .expect("failed to encode list_strategy_templates args"),
    );
    assert_eq!(listed.len(), 1, "strategy template should be listable");
    assert!(
        matches!(listed[0].status, TemplateStatus::Active),
        "listed template should be Active"
    );

    // Verify template is fetchable
    let fetched: Option<StrategyTemplate> = call_query(
        &pic,
        canister_id,
        "get_strategy_template",
        encode_args((sample_key(),)).expect("failed to encode get_strategy_template args"),
    );
    let fetched = fetched.expect("strategy template should be fetchable");
    assert!(matches!(fetched.status, TemplateStatus::Active));
    assert_eq!(fetched.actions[0].call_sequence[0].inputs[0].name, "to");
    assert_eq!(fetched.actions[0].call_sequence[0].inputs[1].name, "amount");
    assert_eq!(
        fetched.actions[0].call_sequence[0].outputs[0].name,
        "success"
    );

    // Kill switch works
    let kill_payload =
        encode_args((sample_key(), true, Some("halt".to_string()))).unwrap_or_else(|error| {
            panic!("failed to encode set_strategy_kill_switch_admin args: {error}")
        });
    let kill_state: Result<StrategyKillSwitchState, String> = call_update(
        &pic,
        canister_id,
        "set_strategy_kill_switch_admin",
        kill_payload,
    );
    let kill_state = kill_state.unwrap_or_else(|error| panic!("kill switch failed: {error}"));
    assert!(kill_state.enabled);

    // Non-controller is rejected
    let outsider = Principal::self_authenticating(b"outsider-pocketic-strategy-controls");
    let outsider_payload = encode_args((sample_key(), false, Some("outsider".to_string())))
        .expect("failed to encode outsider kill-switch args");
    let outsider_result: Result<StrategyKillSwitchState, String> = call_update_as(
        &pic,
        canister_id,
        outsider,
        "set_strategy_kill_switch_admin",
        outsider_payload,
    );
    assert_eq!(
        outsider_result,
        Err("caller is not a controller".to_string()),
        "non-controller should be rejected"
    );

    // Non-controller is rejected for register_strategy_admin too
    let outsider_recipe =
        encode_args((sample_recipe_json(),)).expect("failed to encode outsider recipe args");
    let outsider_register: Result<StrategyTemplate, String> = call_update_as(
        &pic,
        canister_id,
        outsider,
        "register_strategy_admin",
        outsider_recipe,
    );
    assert_eq!(
        outsider_register,
        Err("caller is not a controller".to_string()),
        "non-controller should be rejected for register_strategy_admin"
    );
}

#[test]
fn strategy_queries_expose_recursive_abi_names_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();

    let payload = encode_args((recursive_name_recipe_json(),))
        .expect("failed to encode recursive-name recipe args");
    let registered: Result<StrategyTemplate, String> =
        call_update(&pic, canister_id, "register_strategy_admin", payload);
    let template = registered.unwrap_or_else(|error| panic!("register failed: {error}"));
    assert_eq!(template.key, recursive_name_key());

    let fetched: Option<StrategyTemplate> = call_query(
        &pic,
        canister_id,
        "get_strategy_template",
        encode_args((recursive_name_key(),)).expect("failed to encode get_strategy_template args"),
    );
    let fetched = fetched.expect("strategy template should be fetchable");
    let inputs = &fetched.actions[0].call_sequence[0].inputs;
    assert_eq!(inputs[0].name, "marketParams");
    assert_eq!(inputs[1].name, "assets");
    assert_eq!(
        inputs[0]
            .components
            .iter()
            .map(|component| component.name.as_str())
            .collect::<Vec<_>>(),
        vec!["loanToken", "arg0", "arg1", "arg2"]
    );
}

#[test]
fn autonomous_strategy_turn_blocks_execution_when_authority_is_disabled() {
    let (pic, canister_id) = with_backend_canister();

    let registered = register_strategy_admin(&pic, canister_id, sample_recipe_json())
        .expect("strategy registration should succeed");
    assert_eq!(registered.key, sample_key());

    set_inference_provider(&pic, canister_id, InferenceProvider::OpenRouter);
    set_inference_model(&pic, canister_id, "openai/gpt-4o-mini");
    set_openrouter_api_key(&pic, canister_id, Some("sk-or-test".to_string()));
    configure_only_agent_turn(&pic, canister_id, 60);

    let policy = AutonomyPolicy {
        version: 11,
        reserve_policy: ReservePolicy {
            min_cycles_runway_hours: 72,
            min_inference_usdc_6dp: Some(10_000_000),
            min_gas_wei: Some(3_000_000_000_000_000),
        },
        risk_limits: RiskLimits {
            max_total_exposure_bps: 3_000,
            max_single_action_bps: 1_000,
            max_protocol_concentration_bps: 1_500,
        },
        execution_authority: ExecutionAuthority {
            autonomous_execution_enabled: false,
            require_simulation_first: true,
            per_action_value_limit_wei: Some(50_000_000_000_000_000),
        },
        escalation_rules: EscalationRules {
            escalate_on_missing_policy: true,
            escalate_on_authority_exceeded: true,
            escalate_on_repeated_failure: true,
            failure_quarantine_threshold: 1,
        },
        updated_at_ns: 99_999,
    };
    let stored = update_autonomy_policy(&pic, canister_id, policy.clone())
        .expect("policy update should succeed");
    assert_eq!(stored, policy);
    assert_eq!(get_autonomy_policy(&pic, canister_id), policy);

    pic.advance_time(Duration::from_secs(61));
    pic.tick();
    let turn_id = drive_openrouter_strategy_turn(&pic, canister_id);

    let runtime = get_runtime_view(&pic, canister_id);
    assert!(runtime.turn_counter >= 1);

    let tool_calls = get_tool_calls_for_turn(&pic, canister_id, &turn_id);
    assert!(
        tool_calls
            .iter()
            .any(|call| call.tool == "execute_strategy_action"
                && !call.success
                && call
                    .error
                    .as_deref()
                    .unwrap_or_default()
                    .contains("autonomous_execution_disabled")),
        "execution should be blocked by the active autonomy authority gate"
    );

    assert!(get_strategy_quarantines(&pic, canister_id).is_empty());
    assert!(get_active_exposures(&pic, canister_id).is_empty());
    assert_eq!(
        get_exposure_reconciliation_status(&pic, canister_id),
        ExposureReconciliationStatus::default()
    );
}

#[test]
fn autonomous_strategy_turn_accepts_same_turn_verified_intent_across_rounds() {
    let (pic, canister_id) = with_backend_canister();

    let registered = register_strategy_admin(&pic, canister_id, sample_recipe_json())
        .expect("strategy registration should succeed");
    assert_eq!(registered.key, sample_key());

    set_inference_provider(&pic, canister_id, InferenceProvider::OpenRouter);
    set_inference_model(&pic, canister_id, "openai/gpt-4o-mini");
    set_openrouter_api_key(&pic, canister_id, Some("sk-or-test".to_string()));
    configure_only_agent_turn(&pic, canister_id, 60);

    let policy = AutonomyPolicy {
        version: 11,
        reserve_policy: ReservePolicy {
            min_cycles_runway_hours: 72,
            min_inference_usdc_6dp: Some(10_000_000),
            min_gas_wei: Some(3_000_000_000_000_000),
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
            failure_quarantine_threshold: 1,
        },
        updated_at_ns: 100_123,
    };
    update_autonomy_policy(&pic, canister_id, policy).expect("policy update should succeed");

    pic.advance_time(Duration::from_secs(61));
    pic.tick();
    let turn_id = wait_for_turn_tool_calls_with_responder(
        &pic,
        canister_id,
        "turn-1",
        response_body_for_cross_round_openrouter_request,
    );

    let tool_calls = get_tool_calls_for_turn(&pic, canister_id, &turn_id);
    assert!(
        tool_calls
            .iter()
            .any(|call| call.tool == "simulate_strategy_action" && call.success),
        "cross-round turn should include a successful simulation"
    );
    assert!(
        tool_calls
            .iter()
            .any(|call| call.tool == "execute_strategy_action"
                && !call
                    .error
                    .as_deref()
                    .unwrap_or_default()
                    .contains("simulation_first_required")),
        "same-turn execute should not be blocked by the simulation-first gate once prior same-turn simulate succeeded"
    );
}

#[test]
fn autonomous_strategy_turn_quarantines_and_blocks_repeat_failures() {
    let (pic, canister_id) = with_backend_canister();

    let registered = register_strategy_admin(&pic, canister_id, sample_recipe_json())
        .expect("strategy registration should succeed");
    assert_eq!(registered.key, sample_key());

    set_inference_provider(&pic, canister_id, InferenceProvider::OpenRouter);
    set_inference_model(&pic, canister_id, "openai/gpt-4o-mini");
    set_openrouter_api_key(&pic, canister_id, Some("sk-or-test".to_string()));
    configure_only_agent_turn(&pic, canister_id, 60);

    let policy = AutonomyPolicy {
        version: 12,
        reserve_policy: ReservePolicy {
            min_cycles_runway_hours: 0,
            min_inference_usdc_6dp: None,
            min_gas_wei: None,
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
            failure_quarantine_threshold: 1,
        },
        updated_at_ns: 100_999,
    };
    update_autonomy_policy(&pic, canister_id, policy.clone())
        .expect("policy update should succeed");

    pic.advance_time(Duration::from_secs(61));
    pic.tick();
    let first_turn_id = drive_openrouter_strategy_turn(&pic, canister_id);

    let first_runtime = get_runtime_view(&pic, canister_id);
    assert!(first_runtime.turn_counter >= 1);
    let first_tool_calls = get_tool_calls_for_turn(&pic, canister_id, &first_turn_id);
    assert!(
        first_tool_calls
            .iter()
            .any(|call| call.tool == "execute_strategy_action"
                && !call.success
                && !call
                    .error
                    .as_deref()
                    .unwrap_or_default()
                    .contains("simulation_first_required")),
        "first turn should attempt execution and surface the underlying strategy failure"
    );

    pic.advance_time(Duration::from_secs(61));
    pic.tick();
    let mut latest_turn_id = drive_openrouter_strategy_turn(&pic, canister_id);

    for _ in 0..3 {
        let latest_tool_calls = get_tool_calls_for_turn(&pic, canister_id, &latest_turn_id);
        if latest_tool_calls.iter().any(|call| {
            call.tool == "execute_strategy_action"
                && !call.success
                && call
                    .error
                    .as_deref()
                    .unwrap_or_default()
                    .contains("strategy_quarantined")
        }) {
            break;
        }

        pic.advance_time(Duration::from_secs(61));
        pic.tick();
        latest_turn_id = drive_openrouter_strategy_turn(&pic, canister_id);
    }

    let latest_tool_calls = get_tool_calls_for_turn(&pic, canister_id, &latest_turn_id);
    assert!(
        latest_tool_calls
            .iter()
            .any(|call| call.tool == "execute_strategy_action"
                && !call.success
                && (call.outcome == ToolCallOutcome::SuppressedFailureCooldown
                    || call
                        .error
                        .as_deref()
                        .unwrap_or_default()
                        .contains("strategy_quarantined"))),
        "repeat execution should be blocked by the active failure suppression path"
    );

    assert!(get_active_exposures(&pic, canister_id).is_empty());
    assert_eq!(
        get_exposure_reconciliation_status(&pic, canister_id),
        ExposureReconciliationStatus::default()
    );
}
