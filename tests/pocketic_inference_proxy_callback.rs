#![cfg(feature = "pocketic_tests")]

use std::path::Path;
use std::time::Duration;

use candid::{decode_one, encode_args, CandidType, Principal};
use pocket_ic::common::rest::{
    CanisterHttpHeader, CanisterHttpReply, CanisterHttpRequest, CanisterHttpResponse,
    MockCanisterHttpResponse,
};
use pocket_ic::PocketIc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const WASM_PATHS: &[&str] = &[
    ".icp/cache/artifacts/backend",
    "target/wasm32-wasip1/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/deps/backend.wasm",
];

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

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InitArgs {
    ecdsa_key_name: String,
    inbox_contract_address: Option<String>,
    evm_chain_id: Option<u64>,
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
        evm_chain_id: None,
    },))
    .expect("failed to encode init args");

    pic.add_cycles(canister_id, 2_000_000_000_000);
    pic.install_canister(canister_id, wasm, init_args, None);
    (pic, canister_id)
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

fn set_task_enabled(pic: &PocketIc, canister_id: Principal, kind: TaskKind, enabled: bool) {
    let payload = encode_args((kind, enabled)).expect("failed to encode set_task_enabled args");
    let _: String = call_update(pic, canister_id, "set_task_enabled", payload);
}

fn set_task_interval_secs(
    pic: &PocketIc,
    canister_id: Principal,
    kind: TaskKind,
    interval_secs: u64,
) {
    let payload =
        encode_args((kind, interval_secs)).expect("failed to encode set_task_interval_secs args");
    let result: Result<String, String> =
        call_update(pic, canister_id, "set_task_interval_secs", payload);
    assert!(result.is_ok(), "set_task_interval_secs failed: {result:?}");
}

fn set_inference_provider(pic: &PocketIc, canister_id: Principal, provider: InferenceProvider) {
    let payload = encode_args((provider,)).expect("failed to encode set_inference_provider args");
    let _: String = call_update(pic, canister_id, "set_inference_provider", payload);
}

fn set_inference_model(pic: &PocketIc, canister_id: Principal, model: &str) {
    let payload =
        encode_args((model.to_string(),)).expect("failed to encode set_inference_model args");
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

fn get_inference_proxy_status(pic: &PocketIc, canister_id: Principal) -> InferenceProxyStatusView {
    call_query(
        pic,
        canister_id,
        "get_inference_proxy_status",
        encode_args(()).expect("failed to encode get_inference_proxy_status args"),
    )
}

fn configure_proxy_runtime(pic: &PocketIc, canister_id: Principal, worker: Principal) {
    set_inference_provider(pic, canister_id, InferenceProvider::OpenRouterProxyWorker);
    set_inference_model(pic, canister_id, "openai/gpt-4o-mini");
    set_openrouter_api_key(pic, canister_id, Some("sk-or-test".to_string()));
    set_inference_proxy_config(
        pic,
        canister_id,
        OpenRouterProxyWorkerConfig {
            worker_base_url: "https://proxy.example.workers.dev".to_string(),
            trusted_callback_principal: Some(worker),
        },
    );
    set_task_enabled(pic, canister_id, TaskKind::PollInbox, false);
    set_task_enabled(pic, canister_id, TaskKind::CheckCycles, false);
    set_task_enabled(pic, canister_id, TaskKind::TopUpCycles, false);
    set_task_enabled(pic, canister_id, TaskKind::Reconcile, false);
    set_task_enabled(pic, canister_id, TaskKind::AgentTurn, true);
    set_task_interval_secs(pic, canister_id, TaskKind::AgentTurn, 1);
}

fn header_value(headers: &[CanisterHttpHeader], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.clone())
}

#[derive(Clone, Debug)]
struct ProxySubmitCapture {
    job_id: String,
    turn_id: String,
}

fn respond_with_submit_ack(pic: &PocketIc, request: CanisterHttpRequest) -> ProxySubmitCapture {
    assert!(
        request.url.ends_with("/v1/inference/jobs"),
        "unexpected proxy url: {}",
        request.url
    );
    let auth = header_value(&request.headers, "authorization")
        .expect("proxy submit should include authorization header");
    assert!(
        auth.starts_with("Bearer "),
        "proxy submit should use bearer auth"
    );
    let api_key = header_value(&request.headers, "x-openrouter-api-key")
        .expect("proxy submit should include x-openrouter-api-key");
    assert!(
        !api_key.trim().is_empty(),
        "proxy submit API key header should be non-empty"
    );

    let body: Value = serde_json::from_slice(&request.body)
        .unwrap_or_else(|error| panic!("failed to decode proxy submit request body: {error}"));
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
    assert!(!job_id.is_empty(), "job_id must be present in submit body");
    assert!(
        !turn_id.is_empty(),
        "turn_id must be present in submit body"
    );

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

    ProxySubmitCapture { job_id, turn_id }
}

fn drive_until_submit_accepted(pic: &PocketIc, canister_id: Principal) -> ProxySubmitCapture {
    let _ = canister_id;
    pic.advance_time(Duration::from_secs(90));
    for _ in 0..24 {
        pic.tick();
        let pending_http = pic.get_canister_http();
        if pending_http.is_empty() {
            continue;
        }
        let mut capture: Option<ProxySubmitCapture> = None;
        for request in pending_http {
            capture = Some(respond_with_submit_ack(pic, request));
        }
        pic.tick();
        return capture.expect("submit capture should be recorded");
    }
    panic!("proxy submit was not accepted within expected ticks");
}

fn wait_for_pending_proxy_job(pic: &PocketIc, canister_id: Principal) -> InferenceProxyStatusView {
    for _ in 0..24 {
        let status = get_inference_proxy_status(pic, canister_id);
        if status.submit_accepted >= 1 && status.pending_jobs >= 1 {
            return status;
        }
        pic.tick();
    }
    panic!("pending proxy job was not observed within expected ticks");
}

fn callback_args_from_capture(capture: &ProxySubmitCapture) -> SubmitInferenceResultArgs {
    SubmitInferenceResultArgs {
        job_id: capture.job_id.clone(),
        turn_id: capture.turn_id.clone(),
        completed_at_ns: 987654321,
        result: Some(InferenceProxyResultPayload {
            explanation: Some("async proxy result".to_string()),
            tool_calls: vec![ToolCall {
                tool_call_id: None,
                tool: "record_signal".to_string(),
                args_json: r#"{"signal":"proxy-complete"}"#.to_string(),
            }],
        }),
        error: None,
    }
}

#[cfg(feature = "pocketic_tests")]
#[test]
fn proxy_callback_resume_roundtrip_succeeds() {
    let (pic, canister_id) = with_backend_canister();
    let worker = Principal::self_authenticating(b"proxy-worker-principal");
    configure_proxy_runtime(&pic, canister_id, worker);

    let capture = drive_until_submit_accepted(&pic, canister_id);
    let after_submit = wait_for_pending_proxy_job(&pic, canister_id);
    assert!(after_submit.pending_jobs >= 1);
    assert!(after_submit.submit_accepted >= 1);

    let callback_payload = encode_args((callback_args_from_capture(&capture),))
        .expect("failed to encode submit_inference_result args");
    let first: Result<String, String> = call_update_as(
        &pic,
        canister_id,
        worker,
        "submit_inference_result",
        callback_payload,
    );
    assert_eq!(
        first.expect("trusted worker callback should succeed"),
        "inference_proxy_callback_accepted"
    );

    for _ in 0..12 {
        pic.advance_time(Duration::from_secs(31));
        pic.tick();
        let status = get_inference_proxy_status(&pic, canister_id);
        if status.resumed_callbacks >= 1 {
            break;
        }
    }

    let final_status = get_inference_proxy_status(&pic, canister_id);
    assert_eq!(final_status.pending_jobs, 0);
    assert_eq!(final_status.callback_accepted, 1);
    assert_eq!(final_status.resumed_callbacks, 1);
}

#[cfg(feature = "pocketic_tests")]
#[test]
fn unauthorized_proxy_callback_is_rejected() {
    let (pic, canister_id) = with_backend_canister();
    let trusted_worker = Principal::self_authenticating(b"proxy-worker-principal");
    configure_proxy_runtime(&pic, canister_id, trusted_worker);

    let capture = drive_until_submit_accepted(&pic, canister_id);
    let _ = wait_for_pending_proxy_job(&pic, canister_id);
    let attacker = Principal::self_authenticating(b"proxy-attacker-principal");
    let callback_payload = encode_args((callback_args_from_capture(&capture),))
        .expect("failed to encode submit_inference_result args");
    let rejected: Result<String, String> = call_update_as(
        &pic,
        canister_id,
        attacker,
        "submit_inference_result",
        callback_payload,
    );

    let error = rejected.expect_err("unauthorized callback should fail");
    assert!(error.contains("unauthorized inference proxy callback caller"));
    let status = get_inference_proxy_status(&pic, canister_id);
    assert_eq!(status.pending_jobs, 1);
    assert_eq!(status.callback_rejected, 1);
    assert_eq!(status.callback_auth_failures, 1);
}

#[cfg(feature = "pocketic_tests")]
#[test]
fn duplicate_proxy_callback_is_idempotent() {
    let (pic, canister_id) = with_backend_canister();
    let worker = Principal::self_authenticating(b"proxy-worker-principal");
    configure_proxy_runtime(&pic, canister_id, worker);

    let capture = drive_until_submit_accepted(&pic, canister_id);
    let _ = wait_for_pending_proxy_job(&pic, canister_id);
    let callback_payload = encode_args((callback_args_from_capture(&capture),))
        .expect("failed to encode submit_inference_result args");
    let first: Result<String, String> = call_update_as(
        &pic,
        canister_id,
        worker,
        "submit_inference_result",
        callback_payload.clone(),
    );
    assert_eq!(
        first.expect("first callback should be accepted"),
        "inference_proxy_callback_accepted"
    );

    let duplicate: Result<String, String> = call_update_as(
        &pic,
        canister_id,
        worker,
        "submit_inference_result",
        callback_payload,
    );
    assert_eq!(
        duplicate.expect("duplicate callback should return duplicate marker"),
        "inference_proxy_callback_duplicate"
    );

    let status = get_inference_proxy_status(&pic, canister_id);
    assert_eq!(status.pending_jobs, 0);
    assert_eq!(status.callback_accepted, 1);
    assert_eq!(status.callback_duplicates, 1);
}

#[cfg(feature = "pocketic_tests")]
#[test]
fn stale_pending_proxy_job_expires_without_manual_reset() {
    let (pic, canister_id) = with_backend_canister();
    let worker = Principal::self_authenticating(b"proxy-worker-principal");
    configure_proxy_runtime(&pic, canister_id, worker);

    let _capture = drive_until_submit_accepted(&pic, canister_id);
    let submitted = wait_for_pending_proxy_job(&pic, canister_id);
    assert!(submitted.pending_jobs >= 1);

    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, false);
    pic.advance_time(Duration::from_secs((15 * 60) + 5));
    pic.tick();

    let expired = get_inference_proxy_status(&pic, canister_id);
    assert_eq!(expired.pending_jobs, 0);
    assert!(
        expired.expired_jobs >= 1,
        "expected at least one expired pending proxy job"
    );
}
