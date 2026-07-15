use std::path::PathBuf;

use candid::{decode_one, encode_args, CandidType, Principal};
use factory::init::{build_automaton_install_args, validate_automaton_child_runtime_config};
use factory::types::{
    ArtifactUploadStatus, AutomatonChildRuntimeConfig, FactoryArtifactSnapshot, FactoryError,
    FactoryInitArgs, ProviderConfig, ReleaseBroadcastConfig, RepositoryStrategySessionSnapshot,
    SpawnAsset, SpawnChain, SpawnConfig, SpawnProviderSecrets, SpawnSession, SpawnSessionState,
};
use pocket_ic::PocketIc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use spawn_protocol::{
    InferenceTransport, InitArgs, OpenRouterReasoningLevel, SkillRecord, SpawnBootstrapView,
    StewardStatusView, StrategyTemplate, StrategyTemplateKey,
};

const VERSION_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
fn artifact_path(env_name: &str, default: &str) -> PathBuf {
    let path = std::env::var(env_name).unwrap_or_else(|_| default.to_string());
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }
}

fn read_artifact(env_name: &str, default: &str) -> (PathBuf, Vec<u8>) {
    let path = artifact_path(env_name, default);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read {env_name} artifact at {}: {error}",
            path.display()
        )
    });
    (path, bytes)
}

fn call_query<T>(pic: &PocketIc, canister_id: Principal, method: &str, payload: Vec<u8>) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .query_call(canister_id, Principal::anonymous(), method, payload)
        .unwrap_or_else(|error| panic!("query call {method} failed: {error:?}"));
    decode_one(&response).unwrap_or_else(|error| panic!("decode {method} failed: {error:?}"))
}

fn call_update<T>(pic: &PocketIc, canister_id: Principal, method: &str, payload: Vec<u8>) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .update_call(canister_id, Principal::anonymous(), method, payload)
        .unwrap_or_else(|error| panic!("update call {method} failed: {error:?}"));
    decode_one(&response).unwrap_or_else(|error| panic!("decode {method} failed: {error:?}"))
}

fn child_runtime(chain_id: u64) -> AutomatonChildRuntimeConfig {
    AutomatonChildRuntimeConfig {
        ecdsa_key_name: Some("dfx_test_key".to_string()),
        inbox_contract_address: None,
        evm_chain_id: Some(chain_id),
        evm_rpc_url: Some("http://127.0.0.1:18545".to_string()),
        evm_confirmation_depth: Some(1),
        evm_bootstrap_lookback_blocks: Some(1),
        http_allowed_domains: None,
        llm_canister_id: None,
        search_api_key: None,
        inference_proxy_worker_base_url: None,
        inference_proxy_trusted_callback_principal: None,
        cycle_topup_enabled: None,
        auto_topup_cycle_threshold: None,
    }
}

fn factory_init_args(chain_id: u64) -> FactoryInitArgs {
    FactoryInitArgs {
        admin_principals: vec![Principal::anonymous()],
        fee_config: None,
        creation_cost_quote: None,
        release_broadcast_config: Some(ReleaseBroadcastConfig {
            chain_id,
            ..ReleaseBroadcastConfig::default()
        }),
        child_runtime: Some(child_runtime(chain_id)),
        pause: false,
        payment_address: None,
        escrow_contract_address: None,
        base_rpc_endpoint: None,
        base_rpc_fallback_endpoint: None,
        cycles_per_spawn: Some(0),
        min_pool_balance: Some(0),
        estimated_outcall_cycles_per_interval: Some(0),
        session_ttl_ms: None,
        version_commit: Some(VERSION_COMMIT.to_string()),
        wasm_sha256: None,
    }
}

fn install_factory(pic: &PocketIc, factory_wasm: &[u8], chain_id: u64) -> Principal {
    let canister_id = pic.create_canister();
    pic.add_cycles(canister_id, 5_000_000_000_000);
    let init_args =
        encode_args((Some(factory_init_args(chain_id)),)).expect("factory init args should encode");
    pic.install_canister(canister_id, factory_wasm.to_vec(), init_args, None);
    canister_id
}

fn production_install_args(factory_id: Principal, transport: InferenceTransport) -> Vec<u8> {
    let session = SpawnSession {
        name: None,
        constitution: None,
        session_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        claim_id: "claim-1".to_string(),
        steward_address: "0x62dAFfDC4D59eA05fedDb0a77A266B0a7b6F28ca".to_string(),
        chain: SpawnChain::Base,
        asset: SpawnAsset::Usdc,
        gross_amount: "60000000".to_string(),
        platform_fee: "5000000".to_string(),
        creation_cost: "45000000".to_string(),
        net_forward_amount: "10000000".to_string(),
        quote_terms_hash: "quote".to_string(),
        expires_at: 1_700_100,
        state: SpawnSessionState::AwaitingPayment,
        retryable: false,
        refundable: false,
        payment_status: factory::types::PaymentStatus::Unpaid,
        last_scanned_block: None,
        automaton_canister_id: None,
        automaton_evm_address: None,
        release_tx_hash: None,
        release_broadcast_at: None,
        release_broadcast: None,
        parent_id: Some("parent-automaton".to_string()),
        origin: Some(factory::types::SpawnSessionOrigin::ReproductionOf(
            "parent-automaton".to_string(),
        )),
        generation: Some(1),
        parent_constitution_hash: None,
        memory_dowry: Some(Vec::new()),
        inherited_strategy_stats: Some(Vec::new()),
        royalty_allocations: Some(Vec::new()),
        child_ids: Vec::new(),
        selected_strategies: Vec::<RepositoryStrategySessionSnapshot>::new(),
        config: SpawnConfig {
            chain: SpawnChain::Base,
            risk: 4,
            strategies: vec!["base-aave-usdc-reserve-01".to_string()],
            skills: vec!["messaging".to_string()],
            provider: ProviderConfig {
                model: Some("openai/gpt-4o-mini".to_string()),
                inference_transport: transport,
                open_router_reasoning_level: OpenRouterReasoningLevel::High,
            },
        },
        created_at: 1_700_000,
        updated_at: 1_700_000,
    };
    let mut runtime = child_runtime(31337);
    if transport == InferenceTransport::OpenrouterProxyWorker {
        runtime.inference_proxy_worker_base_url =
            Some("https://proxy.example.workers.dev".to_string());
        runtime.inference_proxy_trusted_callback_principal =
            Some("w36hm-eqaaa-aaaal-qr76a-cai".to_string());
    }
    let validated =
        validate_automaton_child_runtime_config(&runtime).expect("child runtime should validate");
    build_automaton_install_args(
        &session,
        Some(&SpawnProviderSecrets {
            open_router_api_key: Some("sk-or-test".to_string()),
            brave_search_api_key: Some("brave-test-key".to_string()),
        }),
        factory_id,
        VERSION_COMMIT,
        &validated,
    )
}

fn query_child_contract(pic: &PocketIc, child_id: Principal) {
    let bootstrap: SpawnBootstrapView = call_query(
        pic,
        child_id,
        "get_spawn_bootstrap_view",
        encode_args(()).unwrap(),
    );
    let steward: StewardStatusView = call_query(
        pic,
        child_id,
        "get_steward_status",
        encode_args(()).unwrap(),
    );
    let evm_address: Option<String> = call_query(
        pic,
        child_id,
        "get_automaton_evm_address",
        encode_args(()).unwrap(),
    );
    let skills: Vec<SkillRecord> =
        call_query(pic, child_id, "list_skills", encode_args(()).unwrap());
    let strategies: Vec<StrategyTemplate> = call_query(
        pic,
        child_id,
        "list_strategy_templates",
        encode_args((None::<StrategyTemplateKey>, 100u32)).unwrap(),
    );

    assert_eq!(
        bootstrap.session_id.as_deref(),
        Some("550e8400-e29b-41d4-a716-446655440000")
    );
    assert_eq!(bootstrap.risk, Some(4));
    assert_eq!(bootstrap.version_commit.as_deref(), Some(VERSION_COMMIT));
    assert_eq!(
        steward.active_steward.as_ref().map(|value| value.chain_id),
        Some(31337)
    );
    assert!(evm_address.is_none());
    assert!(skills.iter().all(|skill| !skill.name.is_empty()));
    assert!(strategies.is_empty());
}

#[test]
fn spawn_contract() {
    let Some(child_path) = std::env::var_os("AUTOMATON_WASM_PATH") else {
        eprintln!("AUTOMATON_WASM_PATH is not set; skipping built-Wasm spawn contract gate");
        return;
    };
    let (_, child_wasm) = read_artifact(
        "AUTOMATON_WASM_PATH",
        child_path
            .to_str()
            .expect("child wasm path should be valid UTF-8"),
    );
    let (_, factory_wasm) = read_artifact("FACTORY_WASM_PATH", "dist/factory.wasm");
    assert!(!child_wasm.is_empty());
    assert!(!factory_wasm.is_empty());

    let pic = PocketIc::new();
    let factory_id = install_factory(&pic, &factory_wasm, 31337);
    let child_hash = format!("{:x}", Sha256::digest(&child_wasm));
    let started: Result<ArtifactUploadStatus, FactoryError> = call_update(
        &pic,
        factory_id,
        "begin_artifact_upload",
        encode_args((
            child_hash.clone(),
            VERSION_COMMIT.to_string(),
            child_wasm.len() as u64,
        ))
        .unwrap(),
    );
    assert_eq!(
        started.unwrap().total_size_bytes,
        Some(child_wasm.len() as u64)
    );
    for chunk in child_wasm.chunks(1_000_000) {
        let status: Result<ArtifactUploadStatus, FactoryError> = call_update(
            &pic,
            factory_id,
            "append_artifact_chunk",
            encode_args((chunk.to_vec(),)).unwrap(),
        );
        assert!(status.unwrap().received_size_bytes > 0);
    }
    let artifact: Result<FactoryArtifactSnapshot, FactoryError> = call_update(
        &pic,
        factory_id,
        "commit_artifact_upload",
        encode_args(()).unwrap(),
    );
    let artifact = artifact.expect("factory should accept exact child artifact");
    assert_eq!(artifact.wasm_sha256.as_deref(), Some(child_hash.as_str()));
    assert_eq!(artifact.version_commit.as_deref(), Some(VERSION_COMMIT));

    let production_args = production_install_args(factory_id, InferenceTransport::OpenrouterDirect);
    let decoded: (InitArgs,) =
        candid::decode_args(&production_args).expect("production args decode");
    assert_eq!(decoded.0.evm_chain_id, Some(31337));
    assert_eq!(
        decoded
            .0
            .spawn_bootstrap
            .as_ref()
            .unwrap()
            .factory_principal,
        factory_id
    );

    for transport in [
        InferenceTransport::OpenrouterDirect,
        InferenceTransport::OpenrouterProxyWorker,
    ] {
        let child_id = pic.create_canister();
        pic.add_cycles(child_id, 3_000_000_000_000);
        let init = production_install_args(factory_id, transport);
        pic.install_canister(child_id, child_wasm.clone(), init, None);
        query_child_contract(&pic, child_id);
    }

    let mut malformed: InitArgs = candid::decode_args::<(InitArgs,)>(&production_install_args(
        factory_id,
        InferenceTransport::OpenrouterDirect,
    ))
    .expect("production args should decode")
    .0;
    malformed.spawn_bootstrap.as_mut().unwrap().version_commit = "not-a-sha".to_string();
    let child_id = pic.create_canister();
    pic.add_cycles(child_id, 3_000_000_000_000);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pic.install_canister(
            child_id,
            child_wasm,
            encode_args((malformed,)).unwrap(),
            None,
        );
    }));
    assert!(
        result.is_err(),
        "malformed bootstrap args must fail at install"
    );
}
