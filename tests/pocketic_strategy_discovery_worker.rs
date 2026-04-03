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
    "target/wasm32-unknown-unknown/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/deps/backend.wasm",
];
const FRESH_DISCOVERY_TIMESTAMP_NS: u64 = 9_000_000_000_000_000_000;

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
enum TaskKind {
    AgentTurn,
    PollInbox,
    CheckCycles,
    TopUpCycles,
    Reconcile,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InitArgs {
    ecdsa_key_name: String,
    inbox_contract_address: Option<String>,
    evm_chain_id: Option<u64>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ProtocolWatchlistEntry {
    id: String,
    chain_id: u64,
    pool_address: String,
    market_data_api_url: String,
    abi_api_url: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct StrategyDiscoveryWorkerConfig {
    enabled: bool,
    worker_base_url: String,
    worker_api_key: Option<String>,
    trusted_callback_principal: Option<Principal>,
    result_ttl_secs: u64,
    objective: String,
    protocol_watchlist: Vec<ProtocolWatchlistEntry>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct StrategyDiscoveryStatusView {
    enabled: bool,
    worker_base_url: Option<String>,
    has_worker_api_key: bool,
    trusted_callback_principal: Option<String>,
    result_ttl_secs: u64,
    objective: Option<String>,
    protocol_watchlist_len: u64,
    pending_jobs: u64,
    result_records: u64,
    freshest_validated_job_id: Option<String>,
    freshest_validated_at_ns: Option<u64>,
    freshest_validated_age_secs: Option<u64>,
    freshest_validated_expired: bool,
    submit_accepted: u64,
    submit_failed: u64,
    callback_accepted: u64,
    callback_rejected: u64,
    callback_duplicates: u64,
    callback_auth_failures: u64,
    expired_jobs: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct AbiSelectorAssertion {
    signature: String,
    selector_hex: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct ProtocolArtifactBundle {
    bundle_id: String,
    protocol_id: String,
    chain_id: u64,
    role: String,
    contract_address: String,
    abi_json: String,
    source_ref: String,
    codehash: Option<String>,
    selector_assertions: Vec<AbiSelectorAssertion>,
    spec_summary: String,
    risk_notes: Vec<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct ProtocolMarketSnapshot {
    protocol_id: String,
    chain_id: u64,
    pool_address: String,
    tvl_usd: Option<String>,
    supply_apy_bps: Option<u64>,
    borrow_apy_bps: Option<u64>,
    utilization_bps: Option<u64>,
    summary: String,
    warnings: Vec<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct MarketSynthesisBundle {
    chain_id: u64,
    generated_at_ns: u64,
    protocols: Vec<ProtocolMarketSnapshot>,
    warnings: Vec<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct StrategyCandidateBundle {
    candidate_id: String,
    objective: String,
    protocol_id: String,
    primitive: String,
    chain_id: u64,
    rationale: String,
    required_artifacts: Vec<String>,
    assumptions: Vec<String>,
    missing_inputs: Vec<String>,
    confidence_label: String,
    freshness_deadline_ns: Option<u64>,
    suggested_template_shape: Option<String>,
    estimated_yield_bps: Option<u64>,
    warnings: Vec<String>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
enum StrategyDiscoverySourceType {
    OfficialDocs,
    BlockExplorer,
    ProtocolApi,
    MarketDataApi,
    Other,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
enum StrategyDiscoverySourceTrustTier {
    Official,
    Secondary,
    BestEffort,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct SourceRecord {
    source_id: String,
    source_type: StrategyDiscoverySourceType,
    url: String,
    fetched_at_ns: u64,
    content_hash: String,
    trust_tier: StrategyDiscoverySourceTrustTier,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct StrategyDiscoveryResultPayload {
    protocol_artifacts: Vec<ProtocolArtifactBundle>,
    market: MarketSynthesisBundle,
    candidates: Vec<StrategyCandidateBundle>,
    source_records: Vec<SourceRecord>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct SubmitStrategyDiscoveryResultArgs {
    job_id: String,
    completed_at_ns: u64,
    objective: String,
    watchlist: Vec<ProtocolWatchlistEntry>,
    payload: StrategyDiscoveryResultPayload,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
enum StrategyDiscoveryResultStatus {
    Validated,
    Rejected { reason: String },
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct StrategyDiscoveryCallbackRecord {
    job_id: String,
    completed_at_ns: u64,
    accepted_at_ns: u64,
    validated_at_ns: Option<u64>,
    caller_principal: String,
    objective: String,
    watchlist: Vec<ProtocolWatchlistEntry>,
    result_hash: String,
    status: StrategyDiscoveryResultStatus,
    payload: StrategyDiscoveryResultPayload,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct PromoteDiscoveryProtocolArtifactsArgs {
    job_id: String,
    bundle_id: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct AbiArtifactKey {
    protocol: String,
    chain_id: u64,
    role: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct AbiArtifact {
    key: AbiArtifactKey,
    source_ref: String,
    codehash: Option<String>,
    abi_json: String,
    functions: Vec<AbiFunctionSpec>,
    created_at_ns: u64,
    updated_at_ns: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct AbiTypeSpec {
    name: String,
    kind: String,
    components: Vec<AbiTypeSpec>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct AbiFunctionSpec {
    role: String,
    name: String,
    selector_hex: String,
    inputs: Vec<AbiTypeSpec>,
    outputs: Vec<AbiTypeSpec>,
    state_mutability: String,
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

fn configure_discovery_runtime(pic: &PocketIc, canister_id: Principal, worker: Principal) {
    let payload = encode_args((StrategyDiscoveryWorkerConfig {
        enabled: true,
        worker_base_url: "https://discovery.example.workers.dev".to_string(),
        worker_api_key: Some("secret".to_string()),
        trusted_callback_principal: Some(worker),
        result_ttl_secs: 3_600,
        objective: "find reserve opportunities".to_string(),
        protocol_watchlist: vec![ProtocolWatchlistEntry {
            id: "moonwell-usdc".to_string(),
            chain_id: 8453,
            pool_address: "0x1111111111111111111111111111111111111111".to_string(),
            market_data_api_url: "https://api.example.com/market/moonwell".to_string(),
            abi_api_url: "https://api.example.com/abi/moonwell".to_string(),
        }],
    },))
    .expect("failed to encode discovery config");
    let result: Result<StrategyDiscoveryStatusView, String> = call_update(
        pic,
        canister_id,
        "set_strategy_discovery_worker_config",
        payload,
    );
    assert!(result.is_ok(), "set_strategy_discovery_worker_config failed: {result:?}");

    set_task_enabled(pic, canister_id, TaskKind::AgentTurn, false);
    set_task_enabled(pic, canister_id, TaskKind::PollInbox, false);
    set_task_enabled(pic, canister_id, TaskKind::CheckCycles, false);
    set_task_enabled(pic, canister_id, TaskKind::TopUpCycles, false);
    set_task_enabled(pic, canister_id, TaskKind::Reconcile, true);
    set_task_interval_secs(pic, canister_id, TaskKind::Reconcile, 1);
}

fn header_value(headers: &[CanisterHttpHeader], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.clone())
}

fn respond_with_submit_ack(pic: &PocketIc, request: CanisterHttpRequest) -> String {
    assert!(
        request.url.ends_with("/v1/strategy-discovery/jobs"),
        "unexpected discovery url: {}",
        request.url
    );
    let auth = header_value(&request.headers, "authorization")
        .expect("discovery submit should include authorization header");
    assert_eq!(auth, "Bearer secret");
    let body: Value = serde_json::from_slice(&request.body)
        .unwrap_or_else(|error| panic!("failed to decode discovery submit request body: {error}"));
    let job_id = body
        .get("job_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert!(!job_id.is_empty(), "job_id must be present");
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
            body: serde_json::to_vec(&ack).expect("failed to encode discovery ack"),
        }),
        additional_responses: vec![],
    });
    job_id
}

fn drive_until_submit_accepted(pic: &PocketIc) -> String {
    pic.advance_time(Duration::from_secs(90));
    for _ in 0..24 {
        pic.tick();
        let pending_http = pic.get_canister_http();
        if pending_http.is_empty() {
            continue;
        }
        let mut job_id = None;
        for request in pending_http {
            job_id = Some(respond_with_submit_ack(pic, request));
        }
        pic.tick();
        return job_id.expect("job id should be captured");
    }
    panic!("strategy discovery submit was not accepted within expected ticks");
}

fn wait_for_pending_discovery_job(
    pic: &PocketIc,
    canister_id: Principal,
) -> StrategyDiscoveryStatusView {
    for _ in 0..24 {
        let status = get_discovery_status(pic, canister_id);
        if status.submit_accepted >= 1 && status.pending_jobs >= 1 {
            return status;
        }
        pic.tick();
    }
    panic!("pending discovery job was not observed within expected ticks");
}

fn get_discovery_status(pic: &PocketIc, canister_id: Principal) -> StrategyDiscoveryStatusView {
    call_query(
        pic,
        canister_id,
        "get_strategy_discovery_worker_status",
        encode_args(()).expect("failed to encode get_strategy_discovery_worker_status args"),
    )
}

fn list_discovery_results(
    pic: &PocketIc,
    canister_id: Principal,
) -> Vec<StrategyDiscoveryCallbackRecord> {
    call_query(
        pic,
        canister_id,
        "list_strategy_discovery_results",
        encode_args((25_u32,)).expect("failed to encode list_strategy_discovery_results args"),
    )
}

#[test]
fn trusted_strategy_discovery_callback_roundtrip_and_promotion_succeeds() {
    let (pic, canister_id) = with_backend_canister();
    let worker = Principal::self_authenticating(b"strategy-discovery-worker");
    configure_discovery_runtime(&pic, canister_id, worker);

    let job_id = drive_until_submit_accepted(&pic);
    let after_submit = wait_for_pending_discovery_job(&pic, canister_id);
    assert!(after_submit.submit_accepted >= 1);
    assert!(after_submit.pending_jobs >= 1);

    let callback_payload = encode_args((SubmitStrategyDiscoveryResultArgs {
        job_id: job_id.clone(),
        completed_at_ns: FRESH_DISCOVERY_TIMESTAMP_NS,
        objective: "find reserve opportunities".to_string(),
        watchlist: vec![ProtocolWatchlistEntry {
            id: "moonwell-usdc".to_string(),
            chain_id: 8453,
            pool_address: "0x1111111111111111111111111111111111111111".to_string(),
            market_data_api_url: "https://api.example.com/market/moonwell".to_string(),
            abi_api_url: "https://api.example.com/abi/moonwell".to_string(),
        }],
        payload: StrategyDiscoveryResultPayload {
            protocol_artifacts: vec![ProtocolArtifactBundle {
                bundle_id: "moonwell-usdc:pool".to_string(),
                protocol_id: "moonwell-usdc".to_string(),
                chain_id: 8453,
                role: "pool".to_string(),
                contract_address: "0x1111111111111111111111111111111111111111".to_string(),
                abi_json: r#"[{"type":"function","name":"supply","inputs":[],"outputs":[],"stateMutability":"nonpayable"}]"#.to_string(),
                source_ref: "https://api.example.com/abi/moonwell".to_string(),
                codehash: None,
                selector_assertions: Vec::new(),
                spec_summary: "pool contract".to_string(),
                risk_notes: vec!["guardian pause".to_string()],
            }],
            market: MarketSynthesisBundle {
                chain_id: 8453,
                generated_at_ns: FRESH_DISCOVERY_TIMESTAMP_NS.saturating_sub(1),
                protocols: Vec::new(),
                warnings: Vec::new(),
            },
            candidates: vec![StrategyCandidateBundle {
                candidate_id: "cand-1".to_string(),
                objective: "find reserve opportunities".to_string(),
                protocol_id: "moonwell-usdc".to_string(),
                primitive: "reserve_supply".to_string(),
                chain_id: 8453,
                rationale: "bounded reserve parking".to_string(),
                required_artifacts: vec!["moonwell-usdc:pool".to_string()],
                assumptions: vec!["liquidity remains available".to_string()],
                missing_inputs: Vec::new(),
                confidence_label: "medium".to_string(),
                freshness_deadline_ns: None,
                suggested_template_shape: Some("base-moonwell-usdc-reserve".to_string()),
                estimated_yield_bps: Some(420),
                warnings: Vec::new(),
            }],
            source_records: vec![SourceRecord {
                source_id: "market-1".to_string(),
                source_type: StrategyDiscoverySourceType::MarketDataApi,
                url: "https://api.example.com/market/moonwell".to_string(),
                fetched_at_ns: FRESH_DISCOVERY_TIMESTAMP_NS.saturating_sub(2),
                content_hash: "0xabc".to_string(),
                trust_tier: StrategyDiscoverySourceTrustTier::Official,
            }],
        },
    },))
    .expect("failed to encode discovery callback payload");
    let callback_result: Result<String, String> = call_update_as(
        &pic,
        canister_id,
        worker,
        "submit_strategy_discovery_result",
        callback_payload,
    );
    assert_eq!(
        callback_result.expect("trusted discovery callback should succeed"),
        "strategy_discovery_callback_validated"
    );

    let results = list_discovery_results(&pic, canister_id);
    assert_eq!(results.len(), 1);
    assert!(matches!(
        results[0].status,
        StrategyDiscoveryResultStatus::Validated
    ));

    let promote_payload = encode_args((PromoteDiscoveryProtocolArtifactsArgs {
        job_id,
        bundle_id: "moonwell-usdc:pool".to_string(),
    },))
    .expect("failed to encode promotion payload");
    let promoted: Result<AbiArtifact, String> = call_update(
        &pic,
        canister_id,
        "promote_discovery_protocol_artifacts_admin",
        promote_payload,
    );
    let promoted = promoted.expect("promotion should succeed");
    assert_eq!(promoted.key.protocol, "moonwell-usdc");
    assert_eq!(promoted.key.role, "pool");
    assert_eq!(promoted.key.chain_id, 8453);
}
