#![cfg(feature = "pocketic_tests")]

use std::path::Path;

use candid::{decode_one, encode_args, CandidType, Principal};
use pocket_ic::PocketIc;
use serde::{Deserialize, Serialize};

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
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct SpawnBootstrapArgs {
    steward_address: String,
    session_id: String,
    parent_id: Option<String>,
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
    let response = pic
        .query_call(
            canister_id,
            Principal::anonymous(),
            method,
            encode_args(()).expect("failed to encode query args"),
        )
        .unwrap_or_else(|error| panic!("query call {method} failed: {error:?}"));
    decode_one(&response)
        .unwrap_or_else(|error| panic!("failed decoding {method} response: {error:?}"))
}

#[test]
fn init_spawn_bootstrap_sets_steward_and_persists_factory_metadata() {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    pic.add_cycles(canister_id, 2_000_000_000_000);

    let wasm = assert_wasm_artifact_present();
    let init_args = encode_args((InitArgs {
        ecdsa_key_name: "dfx_test_key".to_string(),
        inbox_contract_address: None,
        evm_chain_id: Some(31337),
        evm_rpc_url: None,
        evm_confirmation_depth: None,
        evm_bootstrap_lookback_blocks: None,
        http_allowed_domains: None,
        llm_canister_id: None,
        search_api_key: None,
        cycle_topup_enabled: None,
        auto_topup_cycle_threshold: None,
        spawn_bootstrap: Some(SpawnBootstrapArgs {
            steward_address: "0x62dAFfDC4D59eA05fedDb0a77A266B0a7b6F28ca".to_string(),
            session_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            parent_id: Some("parent-automaton".to_string()),
            risk: 4,
            strategies: vec!["carry".to_string()],
            skills: vec!["messaging".to_string()],
            provider: SpawnProviderBootstrapArgs {
                open_router_api_key: Some("sk-or-test".to_string()),
                model: Some("openai/gpt-4o-mini".to_string()),
                brave_search_api_key: Some("brave-test-key".to_string()),
            },
            version_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
        }),
    },))
    .expect("failed to encode init args");

    pic.install_canister(canister_id, wasm, init_args, None);

    let steward_status: StewardStatusView = call_query(&pic, canister_id, "get_steward_status");
    let runtime_view: RuntimeView = call_query(&pic, canister_id, "get_runtime_view");

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
}
