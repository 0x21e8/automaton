#![cfg(feature = "pocketic_tests")]

use std::path::Path;

use candid::{decode_one, encode_args, CandidType, Principal};
use ic_http_certification::{HttpRequest, HttpResponse};
use pocket_ic::PocketIc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const WASM_PATHS: &[&str] = &[
    ".icp/cache/artifacts/backend",
    "target/wasm32-unknown-unknown/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/deps/backend.wasm",
];

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct SpawnProviderBootstrapArgs {
    open_router_api_key: Option<String>,
    model: Option<String>,
    brave_search_api_key: Option<String>,
    inference_transport: InferenceTransport,
    open_router_reasoning_level: OpenRouterReasoningLevel,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct SpawnBootstrapArgs {
    steward_address: String,
    session_id: String,
    parent_id: Option<String>,
    factory_principal: Principal,
    risk: u8,
    strategies: Vec<String>,
    skills: Vec<String>,
    provider: SpawnProviderBootstrapArgs,
    version_commit: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InitArgs {
    ecdsa_key_name: String,
    inbox_contract_address: Option<String>,
    evm_chain_id: Option<u64>,
    evm_rpc_url: Option<String>,
    evm_confirmation_depth: Option<u64>,
    evm_bootstrap_lookback_blocks: Option<u64>,
    http_allowed_domains: Option<Vec<String>>,
    llm_canister_id: Option<Principal>,
    search_api_key: Option<String>,
    inference_proxy_worker_base_url: Option<String>,
    inference_proxy_trusted_callback_principal: Option<Principal>,
    cycle_topup_enabled: Option<bool>,
    auto_topup_cycle_threshold: Option<u64>,
    spawn_bootstrap: Option<SpawnBootstrapArgs>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StewardState {
    chain_id: u64,
    address: String,
    enabled: bool,
    last_used_at_ns: Option<u64>,
    principal: Option<Principal>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StewardStatusView {
    active_steward: Option<StewardState>,
    next_nonce: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum InferenceProvider {
    IcLlm,
    OpenRouter,
    OpenRouterProxyWorker,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum InferenceTransport {
    OpenrouterDirect,
    OpenrouterProxyWorker,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum OpenRouterReasoningLevel {
    Default,
    Low,
    Medium,
    High,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
struct InferenceConfigView {
    provider: InferenceProvider,
    model: String,
    openrouter_base_url: String,
    openrouter_has_api_key: bool,
    openrouter_max_response_bytes: u64,
    openrouter_reasoning_level: OpenRouterReasoningLevel,
    openrouter_proxy_worker_base_url: Option<String>,
    openrouter_proxy_trusted_callback_principal: Option<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct SpawnBootstrapView {
    session_id: Option<String>,
    parent_id: Option<String>,
    factory_principal: Option<Principal>,
    risk: Option<u8>,
    strategies: Vec<String>,
    skills: Vec<String>,
    version_commit: Option<String>,
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

fn call_query<T>(pic: &PocketIc, canister_id: Principal, method: &str) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    call_query_with_payload(
        pic,
        canister_id,
        method,
        encode_args(()).expect("failed to encode query args"),
    )
}

fn call_query_with_payload<T>(
    pic: &PocketIc,
    canister_id: Principal,
    method: &str,
    payload: Vec<u8>,
) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .query_call(canister_id, Principal::anonymous(), method, payload)
        .unwrap_or_else(|error| panic!("query call {method} failed: {error:?}"));
    decode_one(&response)
        .unwrap_or_else(|error| panic!("failed decoding {method} response: {error:?}"))
}

fn sample_spawn_provider_args(
    inference_transport: InferenceTransport,
    open_router_reasoning_level: OpenRouterReasoningLevel,
) -> SpawnProviderBootstrapArgs {
    SpawnProviderBootstrapArgs {
        open_router_api_key: Some("sk-or-test".to_string()),
        model: Some("openai/gpt-4o-mini".to_string()),
        brave_search_api_key: Some("brave-test-key".to_string()),
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
            .expect("factory principal should parse"),
        risk: 4,
        strategies: vec!["carry".to_string()],
        skills: vec!["messaging".to_string()],
        provider,
        version_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
    }
}

fn sample_init_args(spawn_bootstrap: SpawnBootstrapArgs) -> InitArgs {
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
        spawn_bootstrap: Some(spawn_bootstrap),
    }
}

fn install_backend_canister(pic: &PocketIc, init: InitArgs) -> Principal {
    let canister_id = pic.create_canister();
    pic.add_cycles(canister_id, 2_000_000_000_000);
    let wasm = assert_wasm_artifact_present();
    let init_args = encode_args((init,)).expect("failed to encode init args");
    pic.install_canister(canister_id, wasm, init_args, None);
    canister_id
}

fn http_get_json(pic: &PocketIc, canister_id: Principal, path: &str) -> Value {
    let payload = encode_args((HttpRequest::get(path).build(),))
        .unwrap_or_else(|error| panic!("failed to encode http_request args: {error}"));
    let response: HttpResponse = call_query_with_payload(pic, canister_id, "http_request", payload);
    assert_eq!(
        response.status_code().as_u16(),
        200,
        "unexpected {path} status"
    );
    serde_json::from_slice(response.body())
        .unwrap_or_else(|error| panic!("{path} should return json: {error}"))
}

#[test]
fn pocketic_spawn_bootstrap_sets_direct_openrouter_runtime_and_http_config() {
    let pic = PocketIc::new();
    let canister_id = install_backend_canister(
        &pic,
        sample_init_args(sample_spawn_bootstrap_args(sample_spawn_provider_args(
            InferenceTransport::OpenrouterDirect,
            OpenRouterReasoningLevel::High,
        ))),
    );

    let steward_status: StewardStatusView = call_query(&pic, canister_id, "get_steward_status");
    let runtime_view: RuntimeView = call_query(&pic, canister_id, "get_runtime_view");
    let bootstrap_view: SpawnBootstrapView =
        call_query(&pic, canister_id, "get_spawn_bootstrap_view");
    let inference_config: InferenceConfigView =
        call_query(&pic, canister_id, "get_inference_config");
    let proxy_status: InferenceProxyStatusView =
        call_query(&pic, canister_id, "get_inference_proxy_status");
    let inference_config_json = http_get_json(&pic, canister_id, "/api/inference/config");
    let proxy_status_json = http_get_json(&pic, canister_id, "/api/inference/proxy/status");

    let steward = steward_status
        .active_steward
        .expect("spawn bootstrap should install steward");
    assert_eq!(steward.chain_id, 31337);
    assert_eq!(
        steward.address,
        "0x62daffdc4d59ea05feddb0a77a266b0a7b6f28ca"
    );
    assert!(steward.enabled);

    assert_eq!(runtime_view.evm_chain_id, 31337);
    assert_eq!(
        runtime_view.inference_provider,
        InferenceProvider::OpenRouter
    );
    assert_eq!(runtime_view.inference_model, "openai/gpt-4o-mini");
    assert_eq!(inference_config.provider, InferenceProvider::OpenRouter);
    assert_eq!(
        inference_config.openrouter_reasoning_level,
        OpenRouterReasoningLevel::High
    );
    assert_eq!(inference_config.openrouter_proxy_worker_base_url, None);
    assert_eq!(proxy_status.worker_base_url, None);
    assert_eq!(
        inference_config_json
            .get("provider")
            .and_then(Value::as_str),
        Some("OpenRouter")
    );
    assert_eq!(
        inference_config_json
            .get("openrouter_reasoning_level")
            .and_then(Value::as_str),
        Some("High")
    );
    assert_eq!(proxy_status_json.get("worker_base_url"), Some(&Value::Null));
    assert_eq!(
        bootstrap_view,
        SpawnBootstrapView {
            session_id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
            parent_id: Some("parent-automaton".to_string()),
            factory_principal: Some(
                Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                    .expect("factory principal should parse"),
            ),
            risk: Some(4),
            strategies: vec!["carry".to_string()],
            skills: vec!["messaging".to_string()],
            version_commit: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        }
    );
}

#[test]
fn pocketic_spawn_bootstrap_sets_proxy_runtime_and_http_config() {
    let pic = PocketIc::new();
    let trusted = Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai")
        .expect("trusted principal should parse");
    let mut init = sample_init_args(sample_spawn_bootstrap_args(sample_spawn_provider_args(
        InferenceTransport::OpenrouterProxyWorker,
        OpenRouterReasoningLevel::Medium,
    )));
    init.inference_proxy_worker_base_url = Some("https://proxy.example.workers.dev/".to_string());
    init.inference_proxy_trusted_callback_principal = Some(trusted);
    let canister_id = install_backend_canister(&pic, init);

    let runtime_view: RuntimeView = call_query(&pic, canister_id, "get_runtime_view");
    let inference_config: InferenceConfigView =
        call_query(&pic, canister_id, "get_inference_config");
    let proxy_status: InferenceProxyStatusView =
        call_query(&pic, canister_id, "get_inference_proxy_status");
    let inference_config_json = http_get_json(&pic, canister_id, "/api/inference/config");
    let proxy_status_json = http_get_json(&pic, canister_id, "/api/inference/proxy/status");

    assert_eq!(
        runtime_view.inference_provider,
        InferenceProvider::OpenRouterProxyWorker
    );
    assert_eq!(
        inference_config.provider,
        InferenceProvider::OpenRouterProxyWorker
    );
    assert_eq!(
        inference_config.openrouter_reasoning_level,
        OpenRouterReasoningLevel::Medium
    );
    assert_eq!(
        inference_config.openrouter_proxy_worker_base_url.as_deref(),
        Some("https://proxy.example.workers.dev")
    );
    assert_eq!(
        inference_config
            .openrouter_proxy_trusted_callback_principal
            .as_deref(),
        Some("w36hm-eqaaa-aaaal-qr76a-cai")
    );
    assert_eq!(
        proxy_status.worker_base_url.as_deref(),
        Some("https://proxy.example.workers.dev")
    );
    assert_eq!(
        proxy_status.trusted_callback_principal.as_deref(),
        Some("w36hm-eqaaa-aaaal-qr76a-cai")
    );
    assert_eq!(
        inference_config_json
            .get("provider")
            .and_then(Value::as_str),
        Some("OpenRouterProxyWorker")
    );
    assert_eq!(
        inference_config_json
            .get("openrouter_reasoning_level")
            .and_then(Value::as_str),
        Some("Medium")
    );
    assert_eq!(
        inference_config_json
            .get("openrouter_proxy_worker_base_url")
            .and_then(Value::as_str),
        Some("https://proxy.example.workers.dev")
    );
    assert_eq!(
        proxy_status_json
            .get("worker_base_url")
            .and_then(Value::as_str),
        Some("https://proxy.example.workers.dev")
    );
    assert_eq!(
        proxy_status_json
            .get("trusted_callback_principal")
            .and_then(Value::as_str),
        Some("w36hm-eqaaa-aaaal-qr76a-cai")
    );
}
