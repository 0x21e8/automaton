#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

mod api;
pub mod base_rpc;
pub mod controllers;
pub mod cycles;
pub mod escrow;
pub mod evm;
pub mod expiry;
pub mod init;
pub mod retry;
pub mod scheduler;
pub mod session_transitions;
pub mod spawn;
pub mod state;
pub mod strategy_repository;
pub mod types;

#[cfg(not(target_arch = "wasm32"))]
pub use api::admin::{
    append_artifact_chunk, begin_artifact_upload, commit_artifact_upload,
    get_artifact_upload_status, get_factory_config, get_factory_health, get_factory_runtime,
    get_session_admin, retry_session_admin, set_child_runtime_config, set_creation_cost_quote,
    set_fee_config, set_operational_config, set_pause, set_release_broadcast_config,
    update_artifact,
};
#[cfg(not(target_arch = "wasm32"))]
pub use api::public::{
    claim_spawn_refund, create_spawn_session, get_spawn_session, get_spawned_automaton,
    list_messages_for_automaton, list_my_room_messages, list_room_messages,
    list_spawned_automatons, post_room_message, retry_spawn_session,
};
#[cfg(not(target_arch = "wasm32"))]
pub use api::repository::{
    add_repository_strategy, deprecate_repository_strategy, get_repository_strategy,
    list_repository_strategies, revoke_repository_strategy,
};
pub use escrow::{
    claim_escrow_refund, get_escrow_claim, next_payment_scan_plan, reconcile_escrow_payments,
    register_escrow_claim,
};
pub use expiry::expire_spawn_session;
pub use retry::{mark_session_failed, retry_failed_session};
pub use spawn::execute_spawn;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub use state::set_mock_canister_balance;
pub use state::{
    apply_factory_init_args, clear_provider_secrets, current_canister_balance,
    delete_spawn_provider_secrets, insert_spawned_automaton_record, load_spawn_provider_secrets,
    read_state, restore_state, snapshot_state, store_spawn_provider_secrets, write_state,
    FactoryStateSnapshot,
};
pub use types::{
    derive_claim_id, AddRepositoryStrategyRequest, ArtifactUploadStatus,
    AutomatonChildRuntimeConfig, AutomatonRuntimeState, CreateSpawnSessionRequest,
    CreateSpawnSessionResponse, CreationCostQuote, DeprecateRepositoryStrategyRequest, EscrowClaim,
    FactoryArtifactSnapshot, FactoryConfigSnapshot, FactoryError, FactoryHealthSnapshot,
    FactoryInitArgs, FactoryOperationalConfig, FactoryRuntimeSnapshot,
    FactorySchedulerHealthSnapshot, FactorySchedulerJobCounts, FactorySessionHealthCounts,
    FeeConfig, GetRepositoryStrategyResponse, ListRepositoryStrategiesResponse, PaymentStatus,
    PostRoomMessageRequest, ProviderConfig, RefundSpawnResponse, ReleaseBroadcastConfig,
    ReleaseBroadcastFailure, ReleaseBroadcastRecord, ReleaseBroadcastStage, ReleaseSignatureRecord,
    RepositoryStrategyMetadata, RepositoryStrategyMutationResponse, RepositoryStrategyRecord,
    RepositoryStrategySessionSnapshot, RepositoryStrategySource, RepositoryStrategyStatus,
    RevokeRepositoryStrategyRequest, RoomContentType, RoomMessage, RoomMessagePage, RoomState,
    SchedulerFailureAction, SchedulerFailureSource, SchedulerJob, SchedulerJobFailure,
    SchedulerJobKind, SchedulerJobStatus, SchedulerRuntime, SessionAdminView, SessionAuditActor,
    SessionAuditEntry, SpawnAsset, SpawnChain, SpawnConfig, SpawnExecutionReceipt,
    SpawnPaymentInstructions, SpawnProviderSecrets, SpawnQuote, SpawnSession, SpawnSessionState,
    SpawnSessionStatusResponse, SpawnedAutomatonRecord, SpawnedAutomatonRegistryPage,
    DEFAULT_ROOM_READ_LIMIT, MAX_ROOM_BODY_BYTES, MAX_ROOM_MENTIONS, MAX_ROOM_MESSAGES_RETAINED,
    MAX_ROOM_READ_LIMIT,
};

pub fn bootstrap_status() -> &'static str {
    "factory-session-core-ready"
}

#[cfg(target_arch = "wasm32")]
const SCHEDULER_TICK_INTERVAL_MS: u64 = 30_000;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static SCHEDULER_TIMER_ID: RefCell<Option<ic_cdk_timers::TimerId>> = const { RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now_ms() -> u64 {
    ic_cdk::api::time() / 1_000_000
}

#[cfg(target_arch = "wasm32")]
fn ensure_scheduler_timer_registered() {
    SCHEDULER_TIMER_ID.with(|timer_id| {
        let mut timer_id = timer_id.borrow_mut();
        if let Some(existing) = timer_id.take() {
            ic_cdk_timers::clear_timer(existing);
        }

        let next_timer_id = ic_cdk_timers::set_timer_interval(
            std::time::Duration::from_millis(SCHEDULER_TICK_INTERVAL_MS),
            || async {
                let current_time_ms = now_ms();
                if read_state(|state| state.pause) {
                    return;
                }

                scheduler::schedule_due_jobs(current_time_ms);
            },
        );
        *timer_id = Some(next_timer_id);
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn auto_run_spawn_scheduler(now_ms: u64) -> Vec<Result<SpawnExecutionReceipt, FactoryError>> {
    scheduler::run_scheduler_tick(now_ms)
        .into_iter()
        .filter_map(|report| {
            if !matches!(report.kind, SchedulerJobKind::SpawnExecution { .. }) {
                return None;
            }

            Some(match report.spawn_receipt {
                Some(receipt) => Ok(receipt),
                None => Err(report
                    .error
                    .unwrap_or(FactoryError::SessionNotReadyForSpawn {
                        session_id: report.job_id,
                        state: SpawnSessionState::Failed,
                    })),
            })
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::init]
fn init(args: Option<FactoryInitArgs>) {
    let state = apply_factory_init_args(
        args.unwrap_or_default(),
        Some(ic_cdk::api::msg_caller().to_text()),
    );
    restore_state(state);
    ensure_scheduler_timer_registered();
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::pre_upgrade]
fn pre_upgrade() {}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::post_upgrade]
fn post_upgrade(_args: Option<FactoryInitArgs>) {
    state::initialize_storage_after_upgrade();
    ensure_scheduler_timer_registered();
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
async fn create_spawn_session(
    request: CreateSpawnSessionRequest,
) -> Result<CreateSpawnSessionResponse, FactoryError> {
    let entropy = ic_cdk::management_canister::raw_rand()
        .await
        .map_err(|error| FactoryError::ManagementCallFailed {
            method: "raw_rand".to_string(),
            message: error.to_string(),
        })?;
    let session_id = api::public::uuid_v4_from_entropy(&entropy);
    api::public::create_spawn_session_with_session_id(request, now_ms(), session_id)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_spawn_session(session_id: String) -> Result<SpawnSessionStatusResponse, FactoryError> {
    api::public::get_spawn_session(&session_id)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_spawned_automaton(canister_id: String) -> Result<SpawnedAutomatonRecord, FactoryError> {
    api::public::get_spawned_automaton(&canister_id)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn list_repository_strategies() -> ListRepositoryStrategiesResponse {
    api::repository::list_repository_strategies()
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_repository_strategy(strategy_id: String) -> GetRepositoryStrategyResponse {
    api::repository::get_repository_strategy(&strategy_id)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn list_spawned_automatons(
    cursor: Option<String>,
    limit: u64,
) -> Result<SpawnedAutomatonRegistryPage, FactoryError> {
    api::public::list_spawned_automatons(cursor.as_deref(), limit as usize)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn post_room_message(request: PostRoomMessageRequest) -> Result<RoomMessage, FactoryError> {
    api::public::post_room_message(&ic_cdk::api::msg_caller().to_text(), request, now_ms())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn list_room_messages(
    after_seq: Option<u64>,
    limit: Option<u64>,
) -> Result<RoomMessagePage, FactoryError> {
    api::public::list_room_messages(after_seq, limit)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn list_messages_for_automaton(
    canister_id: String,
    after_seq: Option<u64>,
    limit: Option<u64>,
) -> Result<RoomMessagePage, FactoryError> {
    api::public::list_messages_for_automaton(&canister_id, after_seq, limit)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn list_my_room_messages(
    after_seq: Option<u64>,
    limit: Option<u64>,
) -> Result<RoomMessagePage, FactoryError> {
    api::public::list_my_room_messages(&ic_cdk::api::msg_caller().to_text(), after_seq, limit)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn retry_spawn_session(session_id: String) -> Result<SpawnSessionStatusResponse, FactoryError> {
    api::public::retry_spawn_session(&ic_cdk::api::msg_caller().to_text(), &session_id, now_ms())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn claim_spawn_refund(session_id: String) -> Result<RefundSpawnResponse, FactoryError> {
    api::public::claim_spawn_refund(&ic_cdk::api::msg_caller().to_text(), &session_id, now_ms())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_factory_config() -> Result<FactoryConfigSnapshot, FactoryError> {
    api::admin::get_factory_config(&ic_cdk::api::msg_caller().to_text())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_factory_health() -> FactoryHealthSnapshot {
    api::admin::get_factory_health()
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_factory_runtime(recent_job_limit: u64) -> Result<FactoryRuntimeSnapshot, FactoryError> {
    api::admin::get_factory_runtime(
        &ic_cdk::api::msg_caller().to_text(),
        recent_job_limit as usize,
    )
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
async fn derive_factory_evm_address() -> Result<String, FactoryError> {
    api::admin::derive_factory_evm_address(&ic_cdk::api::msg_caller().to_text()).await
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_session_admin(session_id: String) -> Result<SessionAdminView, FactoryError> {
    api::admin::get_session_admin(&ic_cdk::api::msg_caller().to_text(), &session_id)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn retry_session_admin(session_id: String) -> Result<SpawnSessionStatusResponse, FactoryError> {
    api::admin::retry_session_admin(&ic_cdk::api::msg_caller().to_text(), &session_id, now_ms())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn set_fee_config(config: FeeConfig) -> Result<FeeConfig, FactoryError> {
    api::admin::set_fee_config(&ic_cdk::api::msg_caller().to_text(), config, now_ms())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn set_creation_cost_quote(config: CreationCostQuote) -> Result<CreationCostQuote, FactoryError> {
    api::admin::set_creation_cost_quote(&ic_cdk::api::msg_caller().to_text(), config, now_ms())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn set_release_broadcast_config(
    config: ReleaseBroadcastConfig,
) -> Result<ReleaseBroadcastConfig, FactoryError> {
    api::admin::set_release_broadcast_config(&ic_cdk::api::msg_caller().to_text(), config)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn set_child_runtime_config(
    config: AutomatonChildRuntimeConfig,
) -> Result<AutomatonChildRuntimeConfig, FactoryError> {
    api::admin::set_child_runtime_config(&ic_cdk::api::msg_caller().to_text(), config)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn set_operational_config(
    config: FactoryOperationalConfig,
) -> Result<FactoryOperationalConfig, FactoryError> {
    api::admin::set_operational_config(&ic_cdk::api::msg_caller().to_text(), config)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn set_pause(paused: bool) -> Result<bool, FactoryError> {
    api::admin::set_pause(&ic_cdk::api::msg_caller().to_text(), paused)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn add_repository_strategy(
    request: AddRepositoryStrategyRequest,
) -> Result<RepositoryStrategyMutationResponse, FactoryError> {
    api::repository::add_repository_strategy(
        &ic_cdk::api::msg_caller().to_text(),
        request,
        now_ms(),
    )
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn deprecate_repository_strategy(
    request: DeprecateRepositoryStrategyRequest,
) -> Result<RepositoryStrategyMutationResponse, FactoryError> {
    api::repository::deprecate_repository_strategy(
        &ic_cdk::api::msg_caller().to_text(),
        request,
        now_ms(),
    )
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn revoke_repository_strategy(
    request: RevokeRepositoryStrategyRequest,
) -> Result<RepositoryStrategyMutationResponse, FactoryError> {
    api::repository::revoke_repository_strategy(
        &ic_cdk::api::msg_caller().to_text(),
        request,
        now_ms(),
    )
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn update_artifact(
    wasm_bytes: Vec<u8>,
    expected_sha256: String,
    version_commit: String,
) -> Result<FactoryArtifactSnapshot, FactoryError> {
    api::admin::update_artifact(
        &ic_cdk::api::msg_caller().to_text(),
        wasm_bytes,
        expected_sha256,
        version_commit,
    )
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn begin_artifact_upload(
    expected_sha256: String,
    version_commit: String,
    total_size_bytes: u64,
) -> Result<ArtifactUploadStatus, FactoryError> {
    api::admin::begin_artifact_upload(
        &ic_cdk::api::msg_caller().to_text(),
        expected_sha256,
        version_commit,
        total_size_bytes,
    )
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn append_artifact_chunk(chunk: Vec<u8>) -> Result<ArtifactUploadStatus, FactoryError> {
    api::admin::append_artifact_chunk(&ic_cdk::api::msg_caller().to_text(), chunk)
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::query]
fn get_artifact_upload_status() -> Result<ArtifactUploadStatus, FactoryError> {
    api::admin::get_artifact_upload_status(&ic_cdk::api::msg_caller().to_text())
}

#[cfg(target_arch = "wasm32")]
#[ic_cdk::update]
fn commit_artifact_upload() -> Result<FactoryArtifactSnapshot, FactoryError> {
    api::admin::commit_artifact_upload(&ic_cdk::api::msg_caller().to_text())
}

#[cfg(target_arch = "wasm32")]
ic_cdk::export_candid!();

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{
        add_repository_strategy, append_artifact_chunk, apply_factory_init_args,
        auto_run_spawn_scheduler, begin_artifact_upload, bootstrap_status, claim_spawn_refund,
        commit_artifact_upload, create_spawn_session, deprecate_repository_strategy,
        derive_claim_id, execute_spawn, expire_spawn_session, get_artifact_upload_status,
        get_escrow_claim, get_factory_config, get_factory_health, get_factory_runtime,
        get_repository_strategy, get_session_admin, get_spawn_session, get_spawned_automaton,
        insert_spawned_automaton_record, list_messages_for_automaton, list_my_room_messages,
        list_repository_strategies, list_room_messages, list_spawned_automatons,
        load_spawn_provider_secrets, mark_session_failed, next_payment_scan_plan,
        post_room_message, reconcile_escrow_payments, restore_state, retry_session_admin,
        retry_spawn_session, revoke_repository_strategy, set_child_runtime_config,
        set_creation_cost_quote, set_fee_config, set_mock_canister_balance, set_operational_config,
        set_pause, set_release_broadcast_config, snapshot_state, update_artifact, write_state,
        AddRepositoryStrategyRequest, AutomatonChildRuntimeConfig, CreateSpawnSessionRequest,
        CreationCostQuote, DeprecateRepositoryStrategyRequest, FactoryError, FactoryInitArgs,
        FactoryOperationalConfig, FactoryStateSnapshot, FeeConfig, PaymentStatus,
        PostRoomMessageRequest, ProviderConfig, ReleaseBroadcastConfig, RepositoryStrategyMetadata,
        RepositoryStrategySource, RevokeRepositoryStrategyRequest, RoomContentType,
        SchedulerFailureAction, SchedulerFailureSource, SchedulerJob, SchedulerJobFailure,
        SchedulerJobKind, SchedulerJobStatus, SchedulerRuntime, SessionAuditActor, SpawnAsset,
        SpawnChain, SpawnConfig, SpawnProviderSecrets, SpawnSessionState, SpawnedAutomatonRecord,
        MAX_ROOM_BODY_BYTES, MAX_ROOM_MESSAGES_RETAINED,
    };
    use crate::base_rpc::BaseDepositLog;
    use crate::scheduler::{
        lease_due_jobs_for_test, run_scheduler_tick, spawn_job_id, PAYMENT_POLL_JOB_ID,
    };
    use crate::types::{InferenceTransport, OpenRouterReasoningLevel};
    use candid::Principal;
    use sha2::{Digest, Sha256};

    fn reset_factory_state() {
        restore_state(Default::default());
        set_mock_canister_balance(u128::MAX);
    }

    const SHA40: &str = "abcdef1234567890abcdef1234567890abcdef12";
    const TEST_WASM: &[u8] = b"\0asmtrack6";

    fn sample_child_runtime_config() -> AutomatonChildRuntimeConfig {
        AutomatonChildRuntimeConfig {
            ecdsa_key_name: Some("key_1".to_string()),
            inbox_contract_address: Some("0xInbox".to_string()),
            evm_chain_id: Some(8_453),
            evm_rpc_url: Some("http://127.0.0.1:18545".to_string()),
            evm_confirmation_depth: Some(12),
            evm_bootstrap_lookback_blocks: Some(256),
            http_allowed_domains: Some(vec![
                "https://openrouter.ai".to_string(),
                "https://api.search.brave.com".to_string(),
            ]),
            llm_canister_id: Some(Principal::from_text("aaaaa-aa").expect("valid principal")),
            search_api_key: Some("brave-key".to_string()),
            inference_proxy_worker_base_url: Some("https://proxy.example.com".to_string()),
            inference_proxy_trusted_callback_principal: Some("aaaaa-aa".to_string()),
            cycle_topup_enabled: Some(true),
            auto_topup_cycle_threshold: Some(123_456),
        }
    }

    fn configure_valid_child_runtime() {
        write_state(|state| {
            state.child_runtime = sample_child_runtime_config();
        });
    }

    fn upload_test_artifact() {
        configure_valid_child_runtime();
        let expected_sha = format!("{:x}", Sha256::digest(TEST_WASM));
        let artifact =
            update_artifact("admin", TEST_WASM.to_vec(), expected_sha, SHA40.to_string())
                .expect("artifact upload should succeed");
        assert!(artifact.loaded);
    }

    fn base_deposit_log(session_id: &str, amount: &str, block_number: u64) -> BaseDepositLog {
        BaseDepositLog {
            claim_id: derive_claim_id(session_id),
            amount: amount.to_string(),
            block_number,
        }
    }

    fn mock_deposit_log_endpoint(claim_id: &str, amount: &str, block_number: u64) -> String {
        format!("mock://success/deposit-log/{claim_id}/{amount}/{block_number}")
    }

    fn sample_request_with_strategies(
        gross_amount: &str,
        strategies: &[&str],
    ) -> CreateSpawnSessionRequest {
        CreateSpawnSessionRequest {
            steward_address: "0xsteward".to_string(),
            asset: SpawnAsset::Usdc,
            gross_amount: gross_amount.to_string(),
            config: SpawnConfig {
                chain: SpawnChain::Base,
                risk: 7,
                strategies: strategies
                    .iter()
                    .map(|strategy| (*strategy).to_string())
                    .collect(),
                skills: vec!["search".to_string()],
                provider: ProviderConfig {
                    model: Some("openrouter/auto".to_string()),
                    inference_transport: InferenceTransport::OpenrouterDirect,
                    open_router_reasoning_level: OpenRouterReasoningLevel::Default,
                },
            },
            provider_secrets: SpawnProviderSecrets {
                open_router_api_key: Some("or-key".to_string()),
                brave_search_api_key: Some("brave-key".to_string()),
            },
            parent_id: None,
        }
    }

    fn sample_request(gross_amount: &str) -> CreateSpawnSessionRequest {
        sample_request_with_strategies(gross_amount, &["base-aave-usdc-reserve-01"])
    }

    fn sample_spawned_automaton_record(
        canister_id: &str,
        session_id: &str,
        created_at: u64,
    ) -> SpawnedAutomatonRecord {
        SpawnedAutomatonRecord {
            canister_id: canister_id.to_string(),
            steward_address: "0xsteward".to_string(),
            evm_address: format!("0x{created_at:x}"),
            chain: SpawnChain::Base,
            session_id: session_id.to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            created_at,
            version_commit: SHA40.to_string(),
        }
    }

    fn sample_repository_add_request(strategy_id: &str) -> AddRepositoryStrategyRequest {
        AddRepositoryStrategyRequest {
            metadata: RepositoryStrategyMetadata {
                strategy_id: strategy_id.to_string(),
                name: "Custom Base Aave Reserve".to_string(),
                description: "A custom launchpad-owned reserve recipe.".to_string(),
                canonical_chain: SpawnChain::Base,
                canonical_chain_id: 8_453,
                compatible_spawn_chains: vec![SpawnChain::Base],
                protocol: "aave-v3".to_string(),
                primitive: "lend_supply".to_string(),
                source: RepositoryStrategySource {
                    source_path: "docs/strategies/custom/recipe.json".to_string(),
                    source_commit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa".to_string(),
                },
            },
            recipe_json: format!(
                r#"{{"template_id":"{strategy_id}","chain_id":8453,"protocol":"aave-v3","primitive":"lend_supply","contracts":[],"actions":[],"max_value_wei_per_call":"0","template_budget_wei":"0"}}"#
            ),
        }
    }

    fn register_room_poster(canister_id: &str) {
        insert_spawned_automaton_record(sample_spawned_automaton_record(
            canister_id,
            &format!("session-{canister_id}"),
            1,
        ));
    }

    fn room_request(
        body: &str,
        mentions: &[&str],
        content_type: Option<RoomContentType>,
    ) -> PostRoomMessageRequest {
        PostRoomMessageRequest {
            body: body.to_string(),
            mentions: Some(
                mentions
                    .iter()
                    .map(|mention| (*mention).to_string())
                    .collect(),
            ),
            content_type,
        }
    }

    #[test]
    fn exposes_bootstrap_identity() {
        assert_eq!(bootstrap_status(), "factory-session-core-ready");
    }

    #[test]
    fn applies_init_args_to_new_state_snapshot() {
        let state = apply_factory_init_args(
            FactoryInitArgs {
                admin_principals: vec![Principal::from_text("aaaaa-aa").expect("valid principal")],
                fee_config: Some(FeeConfig {
                    usdc_fee: "7000000".to_string(),
                    updated_at: 1,
                }),
                creation_cost_quote: Some(CreationCostQuote {
                    usdc_cost: "43000000".to_string(),
                    updated_at: 2,
                }),
                release_broadcast_config: Some(ReleaseBroadcastConfig {
                    chain_id: 31_337,
                    max_priority_fee_per_gas: 11,
                    max_fee_per_gas: 22,
                    gas_limit: 333_000,
                    ecdsa_key_name: "test_key_1".to_string(),
                }),
                child_runtime: Some(sample_child_runtime_config()),
                pause: true,
                payment_address: Some("0xPayments".to_string()),
                escrow_contract_address: Some("0xEscrow".to_string()),
                base_rpc_endpoint: Some("https://base.example".to_string()),
                base_rpc_fallback_endpoint: Some("https://base-fallback.example".to_string()),
                cycles_per_spawn: Some(123),
                min_pool_balance: Some(456),
                estimated_outcall_cycles_per_interval: Some(789),
                session_ttl_ms: Some(789),
                version_commit: Some(SHA40.to_string()),
                wasm_sha256: Some("deadbeef".to_string()),
            },
            Some("caller-principal".to_string()),
        );

        let snapshot: FactoryStateSnapshot = state;
        assert!(snapshot.admin_principals.contains("aaaaa-aa"));
        assert_eq!(snapshot.fee_config.usdc_fee, "7000000");
        assert_eq!(snapshot.creation_cost_quote.usdc_cost, "43000000");
        assert_eq!(snapshot.release_broadcast_config.chain_id, 31_337);
        assert_eq!(snapshot.release_broadcast_config.max_fee_per_gas, 22);
        assert_eq!(snapshot.child_runtime, sample_child_runtime_config());
        assert!(snapshot.pause);
        assert_eq!(snapshot.payment_address, "0xPayments");
        assert_eq!(snapshot.escrow_contract_address, "0xEscrow");
        assert_eq!(
            snapshot.base_rpc_endpoint.as_deref(),
            Some("https://base.example")
        );
        assert_eq!(
            snapshot.base_rpc_fallback_endpoint.as_deref(),
            Some("https://base-fallback.example")
        );
        assert!(snapshot.factory_evm_address.is_none());
        assert_eq!(snapshot.cycles_per_spawn, 123);
        assert_eq!(snapshot.min_pool_balance, 456);
        assert_eq!(snapshot.estimated_outcall_cycles_per_interval, 789);
        assert_eq!(snapshot.session_ttl_ms, 789);
        assert_eq!(snapshot.version_commit, SHA40);
        assert_eq!(snapshot.wasm_sha256.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn updates_release_broadcast_config_via_admin_surface() {
        reset_factory_state();

        let config = set_release_broadcast_config(
            "admin",
            ReleaseBroadcastConfig {
                chain_id: 31_337,
                max_priority_fee_per_gas: 5,
                max_fee_per_gas: 9,
                gas_limit: 444_000,
                ecdsa_key_name: "test_key_1".to_string(),
            },
        )
        .expect("admin can update release broadcast config");
        let snapshot = get_factory_config("admin").expect("config should load");

        assert_eq!(config.chain_id, 31_337);
        assert_eq!(snapshot.release_broadcast_config, config);
    }

    #[test]
    fn seeds_repository_strategies_and_exposes_public_queries() {
        reset_factory_state();

        let list = list_repository_strategies();
        let item = get_repository_strategy("base-aave-usdc-reserve-01");

        assert_eq!(list.items.len(), 3);
        assert_eq!(
            list.items
                .iter()
                .map(|record| record.metadata.strategy_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "base-aave-usdc-reserve-01",
                "base-moonwell-usdc-reserve-01",
                "base-usdc-carry-cbbtc-01"
            ]
        );
        assert_eq!(list.updated_at, 0);
        assert_eq!(
            item.item
                .as_ref()
                .map(|record| record.metadata.name.as_str()),
            Some("Base Aave USDC Reserve")
        );
        assert!(item
            .item
            .as_ref()
            .expect("seed strategy should exist")
            .recipe_json
            .contains("\"template_id\": \"base-aave-usdc-reserve-01\""));
    }

    #[test]
    fn manages_repository_strategy_lifecycle_via_admin_surface() {
        reset_factory_state();

        let created = add_repository_strategy(
            "admin",
            sample_repository_add_request("custom-aave-01"),
            100,
        )
        .expect("admin can add repository strategy");
        let deprecated = deprecate_repository_strategy(
            "admin",
            DeprecateRepositoryStrategyRequest {
                strategy_id: "custom-aave-01".to_string(),
                reason: Some("Prefer a newer revision".to_string()),
            },
            200,
        )
        .expect("admin can deprecate repository strategy");
        let revoked = revoke_repository_strategy(
            "admin",
            RevokeRepositoryStrategyRequest {
                strategy_id: "custom-aave-01".to_string(),
                reason: Some("Unsafe template".to_string()),
            },
            300,
        )
        .expect("admin can revoke repository strategy");

        assert_eq!(created.strategy.created_at, 100);
        assert_eq!(created.strategy.updated_at, 100);
        assert!(matches!(
            deprecated.strategy.status,
            crate::types::RepositoryStrategyStatus::Deprecated
        ));
        assert_eq!(deprecated.strategy.deprecated_at, Some(200));
        assert!(matches!(
            revoked.strategy.status,
            crate::types::RepositoryStrategyStatus::Revoked
        ));
        assert_eq!(revoked.strategy.revoked_at, Some(300));
        assert_eq!(
            get_repository_strategy("custom-aave-01")
                .item
                .expect("added strategy should remain readable")
                .updated_at,
            300
        );
    }

    #[test]
    fn rejects_invalid_or_duplicate_repository_strategy_ingest() {
        reset_factory_state();

        let duplicate = add_repository_strategy(
            "admin",
            sample_repository_add_request("base-aave-usdc-reserve-01"),
            100,
        )
        .expect_err("seed ids should not be overwritten");
        assert!(matches!(
            duplicate,
            FactoryError::RepositoryStrategyAlreadyExists { ref strategy_id }
                if strategy_id == "base-aave-usdc-reserve-01"
        ));

        let invalid = add_repository_strategy(
            "admin",
            AddRepositoryStrategyRequest {
                recipe_json: r#"{"template_id":"different-id","chain_id":8453,"protocol":"aave-v3","primitive":"lend_supply"}"#.to_string(),
                ..sample_repository_add_request("custom-aave-02")
            },
            100,
        )
        .expect_err("recipe metadata mismatches should fail");
        assert!(matches!(
            invalid,
            FactoryError::InvalidRepositoryStrategy { ref field, .. }
                if field == "recipe_json.template_id"
        ));
    }

    #[test]
    fn creates_sessions_with_fixed_quote_terms_and_audit_log() {
        reset_factory_state();

        let before = snapshot_state();
        let response = create_spawn_session(sample_request("60000000"), 1_700_000)
            .expect("session should be created");

        assert_eq!(response.session.state, SpawnSessionState::AwaitingPayment);
        assert_eq!(
            response.session.claim_id,
            derive_claim_id(&response.session.session_id)
        );
        assert_eq!(
            response.session.quote_terms_hash,
            response.quote.quote_terms_hash
        );
        assert_eq!(response.session.expires_at, response.quote.expires_at);
        assert_eq!(
            response.quote.payment.quote_terms_hash,
            response.quote.quote_terms_hash
        );
        assert_eq!(response.session.net_forward_amount, "10000000");
        assert_eq!(response.session.selected_strategies.len(), 1);
        assert_eq!(
            response.session.selected_strategies[0].strategy_id,
            "base-aave-usdc-reserve-01"
        );
        assert!(matches!(
            response.session.selected_strategies[0].source_status,
            crate::types::RepositoryStrategyStatus::Active
        ));

        let status = get_spawn_session(&response.session.session_id).expect("session should load");
        assert_eq!(status.audit.len(), 1);
        assert_eq!(status.audit[0].from_state, None);
        assert_eq!(status.audit[0].to_state, SpawnSessionState::AwaitingPayment);
        assert_eq!(status.audit[0].reason, "session created");

        let after = snapshot_state();
        assert_eq!(before.sessions.len() + 1, after.sessions.len());
    }

    #[test]
    fn rejects_missing_deprecated_and_revoked_repository_strategies() {
        reset_factory_state();

        let missing = create_spawn_session(
            sample_request_with_strategies("60000000", &["missing-strategy"]),
            1_700_000,
        )
        .expect_err("unknown strategy ids should be rejected");
        assert!(matches!(
            missing,
            FactoryError::RepositoryStrategyNotFound { ref strategy_id }
                if strategy_id == "missing-strategy"
        ));

        deprecate_repository_strategy(
            "admin",
            DeprecateRepositoryStrategyRequest {
                strategy_id: "base-aave-usdc-reserve-01".to_string(),
                reason: Some("superseded".to_string()),
            },
            1_700_100,
        )
        .expect("seed strategy can be deprecated");
        let deprecated = create_spawn_session(sample_request("60000000"), 1_700_200)
            .expect_err("deprecated strategies should be rejected");
        assert!(matches!(
            deprecated,
            FactoryError::RepositoryStrategyDeprecated { ref strategy_id }
                if strategy_id == "base-aave-usdc-reserve-01"
        ));

        reset_factory_state();
        revoke_repository_strategy(
            "admin",
            RevokeRepositoryStrategyRequest {
                strategy_id: "base-aave-usdc-reserve-01".to_string(),
                reason: Some("unsafe".to_string()),
            },
            1_700_300,
        )
        .expect("seed strategy can be revoked");
        let revoked = create_spawn_session(sample_request("60000000"), 1_700_400)
            .expect_err("revoked strategies should be rejected");
        assert!(matches!(
            revoked,
            FactoryError::RepositoryStrategyRevoked { ref strategy_id }
                if strategy_id == "base-aave-usdc-reserve-01"
        ));
        assert!(snapshot_state().sessions.is_empty());
    }

    #[test]
    fn snapshots_selected_repository_strategies_immutably_per_session() {
        reset_factory_state();

        write_state(|state| {
            state.child_runtime.evm_chain_id = Some(31_337);
        });
        let created = create_spawn_session(
            sample_request_with_strategies(
                "75000000",
                &["base-aave-usdc-reserve-01", "base-moonwell-usdc-reserve-01"],
            ),
            30_000,
        )
        .expect("session should be created");

        assert_eq!(created.session.selected_strategies.len(), 2);
        assert_eq!(
            created
                .session
                .selected_strategies
                .iter()
                .map(|strategy| strategy.strategy_id.as_str())
                .collect::<Vec<_>>(),
            vec!["base-aave-usdc-reserve-01", "base-moonwell-usdc-reserve-01"]
        );
        assert_eq!(
            created.session.selected_strategies[0].resolved_chain_id,
            Some(31_337)
        );
        assert_eq!(
            created.session.selected_strategies[0].canonical_chain_id,
            8_453
        );

        revoke_repository_strategy(
            "admin",
            RevokeRepositoryStrategyRequest {
                strategy_id: "base-aave-usdc-reserve-01".to_string(),
                reason: Some("later repository change".to_string()),
            },
            31_000,
        )
        .expect("repository lifecycle can change after session creation");
        write_state(|state| {
            state.child_runtime.evm_chain_id = Some(8_453);
        });

        let reloaded =
            get_spawn_session(&created.session.session_id).expect("session should still load");
        assert_eq!(
            reloaded.session.selected_strategies,
            created.session.selected_strategies
        );
        assert!(matches!(
            reloaded.session.selected_strategies[0].source_status,
            crate::types::RepositoryStrategyStatus::Active
        ));
        assert_eq!(
            reloaded.session.selected_strategies[0].resolved_chain_id,
            Some(31_337)
        );
    }

    #[test]
    fn rejects_session_creation_while_paused() {
        reset_factory_state();

        set_pause("admin", true).expect("admin can pause");
        let error = create_spawn_session(sample_request("60000000"), 1_700_000)
            .expect_err("paused factory should reject sessions");

        assert!(matches!(
            error,
            super::FactoryError::FactoryPaused { pause: true }
        ));
    }

    #[test]
    fn rejects_unauthorized_admin_and_steward_actions() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 12_000)
            .expect("session should be created");

        let admin_error =
            get_factory_config("not-admin").expect_err("non-admin should be rejected");
        assert!(matches!(
            admin_error,
            super::FactoryError::UnauthorizedAdmin { .. }
        ));

        let session_admin_error = get_session_admin("not-admin", &response.session.session_id)
            .expect_err("non-admin session access should be rejected");
        assert!(matches!(
            session_admin_error,
            super::FactoryError::UnauthorizedAdmin { .. }
        ));

        let retry_error = retry_spawn_session("not-steward", &response.session.session_id, 13_000)
            .expect_err("non-steward retry should be rejected");
        assert!(matches!(
            retry_error,
            super::FactoryError::UnauthorizedSteward { .. }
        ));

        let refund_error = claim_spawn_refund("not-steward", &response.session.session_id, 14_000)
            .expect_err("non-steward refund should be rejected");
        assert!(matches!(
            refund_error,
            super::FactoryError::UnauthorizedSteward { .. }
        ));
    }

    #[test]
    fn updates_admin_quote_configuration() {
        reset_factory_state();

        let fee_config = set_fee_config(
            "admin",
            FeeConfig {
                usdc_fee: "7000000".to_string(),
                updated_at: 0,
            },
            50,
        )
        .expect("fee config should update");

        let creation_cost = set_creation_cost_quote(
            "admin",
            CreationCostQuote {
                usdc_cost: "43000000".to_string(),
                updated_at: 0,
            },
            60,
        )
        .expect("creation cost should update");

        let factory_config = get_factory_config("admin").expect("admin can read config");
        assert_eq!(fee_config.updated_at, 50);
        assert_eq!(creation_cost.updated_at, 60);
        assert_eq!(factory_config.fee_config.usdc_fee, "7000000");
        assert_eq!(factory_config.creation_cost_quote.usdc_cost, "43000000");
        assert_eq!(
            factory_config.child_runtime,
            AutomatonChildRuntimeConfig::default()
        );
        assert_eq!(
            factory_config.escrow_contract_address,
            "0x2222222222222222222222222222222222222222"
        );
        assert!(factory_config.factory_evm_address.is_none());
    }

    #[test]
    fn updates_child_runtime_config_via_admin_surface() {
        reset_factory_state();

        let config = sample_child_runtime_config();
        let updated = set_child_runtime_config("admin", config.clone())
            .expect("child runtime config should update");
        let factory_config = get_factory_config("admin").expect("admin can read config");

        assert_eq!(updated, config);
        assert_eq!(factory_config.child_runtime, config);
    }

    #[test]
    fn updates_operational_config_via_admin_surface() {
        reset_factory_state();

        let config = FactoryOperationalConfig {
            cycles_per_spawn: 2_000_000_000_000,
            min_pool_balance: 500_000_000_000,
            estimated_outcall_cycles_per_interval: 123_456_789,
        };
        let updated =
            set_operational_config("admin", config.clone()).expect("operational config updates");
        let factory_config = get_factory_config("admin").expect("admin can read config");

        assert_eq!(updated, config);
        assert_eq!(factory_config.cycles_per_spawn, config.cycles_per_spawn);
        assert_eq!(factory_config.min_pool_balance, config.min_pool_balance);
        assert_eq!(
            factory_config.estimated_outcall_cycles_per_interval,
            config.estimated_outcall_cycles_per_interval
        );
    }

    #[test]
    fn updates_artifact_after_validating_sha256_and_version_commit() {
        reset_factory_state();

        let expected_sha = format!("{:x}", Sha256::digest(TEST_WASM));
        let artifact = update_artifact(
            "admin",
            TEST_WASM.to_vec(),
            expected_sha.clone(),
            SHA40.to_string(),
        )
        .expect("artifact upload should succeed");

        assert!(artifact.loaded);
        assert_eq!(artifact.wasm_sha256.as_deref(), Some(expected_sha.as_str()));
        assert_eq!(artifact.version_commit.as_deref(), Some(SHA40));
        assert_eq!(artifact.wasm_size_bytes, Some(TEST_WASM.len() as u64));
        let snapshot = snapshot_state();
        assert_eq!(snapshot.wasm_bytes.as_deref(), Some(TEST_WASM));
        assert_eq!(snapshot.wasm_sha256.as_deref(), Some(expected_sha.as_str()));
        assert_eq!(snapshot.version_commit, SHA40);
    }

    #[test]
    fn rejects_artifact_upload_when_sha256_mismatches() {
        reset_factory_state();

        let error = update_artifact(
            "admin",
            TEST_WASM.to_vec(),
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            SHA40.to_string(),
        )
        .expect_err("mismatched sha256 should be rejected");

        assert!(matches!(
            error,
            super::FactoryError::ArtifactHashMismatch { .. }
        ));
        assert!(snapshot_state().wasm_bytes.is_none());
    }

    #[test]
    fn streams_artifact_upload_in_chunks() {
        reset_factory_state();

        let expected_sha = format!("{:x}", Sha256::digest(TEST_WASM));
        let status = begin_artifact_upload(
            "admin",
            expected_sha.clone(),
            SHA40.to_string(),
            TEST_WASM.len() as u64,
        )
        .expect("upload should begin");
        assert!(status.in_progress);
        assert_eq!(status.received_size_bytes, 0);

        let status = append_artifact_chunk("admin", TEST_WASM[..4].to_vec())
            .expect("first chunk should append");
        assert_eq!(status.received_size_bytes, 4);

        let status = append_artifact_chunk("admin", TEST_WASM[4..].to_vec())
            .expect("second chunk should append");
        assert_eq!(status.received_size_bytes, TEST_WASM.len() as u64);

        let status = get_artifact_upload_status("admin").expect("status should load");
        assert!(status.in_progress);
        assert_eq!(status.total_size_bytes, Some(TEST_WASM.len() as u64));

        let artifact = commit_artifact_upload("admin").expect("commit should succeed");
        assert!(artifact.loaded);
        assert_eq!(artifact.wasm_sha256.as_deref(), Some(expected_sha.as_str()));

        let status = get_artifact_upload_status("admin").expect("status should load");
        assert!(!status.in_progress);
        assert_eq!(status.received_size_bytes, 0);
    }

    #[test]
    fn rejects_chunked_artifact_upload_that_exceeds_declared_size() {
        reset_factory_state();

        begin_artifact_upload(
            "admin",
            format!("{:x}", Sha256::digest(TEST_WASM)),
            SHA40.into(),
            3,
        )
        .expect("upload should begin");

        let error = append_artifact_chunk("admin", TEST_WASM[..4].to_vec())
            .expect_err("oversized chunk should be rejected");
        assert!(matches!(
            error,
            FactoryError::ArtifactUploadTooLarge {
                expected: 3,
                attempted: 4,
            }
        ));
    }

    #[test]
    fn rejects_chunked_artifact_commit_when_incomplete() {
        reset_factory_state();

        begin_artifact_upload(
            "admin",
            format!("{:x}", Sha256::digest(TEST_WASM)),
            SHA40.into(),
            TEST_WASM.len() as u64,
        )
        .expect("upload should begin");
        append_artifact_chunk("admin", TEST_WASM[..4].to_vec()).expect("chunk should append");

        let error = commit_artifact_upload("admin").expect_err("commit should fail");
        assert!(matches!(
            error,
            FactoryError::ArtifactUploadIncomplete {
                expected,
                received: 4,
            } if expected == TEST_WASM.len() as u64
        ));
    }

    #[test]
    fn paginates_registry_reads() {
        reset_factory_state();

        insert_spawned_automaton_record(sample_spawned_automaton_record(
            "aaaaa-aa",
            "session-1",
            1,
        ));
        let mut child_record = sample_spawned_automaton_record("bbbbb-bb", "session-2", 2);
        child_record.steward_address = "0xtwo".to_string();
        child_record.evm_address = "0xe2".to_string();
        child_record.parent_id = Some("aaaaa-aa".to_string());
        insert_spawned_automaton_record(child_record);

        let first_page = list_spawned_automatons(None, 1).expect("first page should load");
        assert_eq!(first_page.items.len(), 1);
        assert_eq!(first_page.next_cursor.as_deref(), Some("aaaaa-aa"));

        let second_page = list_spawned_automatons(first_page.next_cursor.as_deref(), 10)
            .expect("second page should load");
        assert_eq!(second_page.items.len(), 1);
        assert_eq!(second_page.items[0].canister_id, "bbbbb-bb");

        let record = get_spawned_automaton("bbbbb-bb").expect("single registry record should load");
        assert_eq!(record.parent_id.as_deref(), Some("aaaaa-aa"));
    }

    #[test]
    fn posts_room_messages_for_registered_automatons_and_round_trips_untrusted_text() {
        reset_factory_state();
        register_room_poster("aaaaa-aa");

        let posted = post_room_message(
            "aaaaa-aa",
            room_request(
                "  <script>alert('fleet')</script>  ",
                &[],
                Some(RoomContentType::TextPlain),
            ),
            42,
        )
        .expect("registered automaton should post");
        let page = list_room_messages(None, Some(10)).expect("room page should load");

        assert_eq!(posted.message_id, "room-message-0");
        assert_eq!(posted.seq, 0);
        assert_eq!(posted.author_canister_id, "aaaaa-aa");
        assert_eq!(posted.body, "<script>alert('fleet')</script>");
        assert!(posted.mentions.is_empty());
        assert_eq!(posted.content_type, RoomContentType::TextPlain);
        assert_eq!(page.messages, vec![posted.clone()]);
        assert_eq!(page.next_after_seq, None);
        assert_eq!(page.latest_seq, Some(0));
    }

    #[test]
    fn filters_room_messages_to_broadcasts_and_explicit_mentions() {
        reset_factory_state();
        register_room_poster("aaaaa-aa");
        register_room_poster("bbbbb-bb");

        post_room_message("aaaaa-aa", room_request("broadcast", &[], None), 10)
            .expect("broadcast should post");
        let relevant_to_b = post_room_message(
            "aaaaa-aa",
            room_request(
                r#"{"kind":"ping","unsafe":"<tool-call>"}"#,
                &["bbbbb-bb", "bbbbb-bb", "zzzzz-zz"],
                Some(RoomContentType::ApplicationJson),
            ),
            11,
        )
        .expect("mentioned message should post");
        post_room_message(
            "bbbbb-bb",
            room_request(
                "for a only",
                &["aaaaa-aa"],
                Some(RoomContentType::TextPlain),
            ),
            12,
        )
        .expect("peer mention should post");
        post_room_message(
            "aaaaa-aa",
            room_request(
                "for unknown only",
                &["zzzzz-zz"],
                Some(RoomContentType::TextPlain),
            ),
            13,
        )
        .expect("unknown mention should still post");

        let filtered = list_messages_for_automaton("bbbbb-bb", None, Some(10))
            .expect("filtered room page should load");
        let mine = list_my_room_messages("aaaaa-aa", None, Some(10))
            .expect("caller-bound room page should load");

        assert_eq!(
            relevant_to_b.mentions,
            vec!["bbbbb-bb".to_string(), "zzzzz-zz".to_string()]
        );
        assert_eq!(
            filtered
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(
            mine.messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(filtered.latest_seq, Some(3));
    }

    #[test]
    fn rejects_unauthorized_or_invalid_room_posts() {
        reset_factory_state();
        register_room_poster("aaaaa-aa");

        let unauthorized = post_room_message("zzzzz-zz", room_request("hello", &[], None), 1)
            .expect_err("unregistered caller should be rejected");
        assert!(matches!(
            unauthorized,
            FactoryError::UnauthorizedRoomPoster { caller } if caller == "zzzzz-zz"
        ));

        let oversized = post_room_message(
            "aaaaa-aa",
            room_request(&"x".repeat(MAX_ROOM_BODY_BYTES + 1), &[], None),
            2,
        )
        .expect_err("oversized room body should be rejected");
        assert!(matches!(
            oversized,
            FactoryError::RoomMessageBodyTooLarge {
                provided_bytes,
                max_bytes,
            } if provided_bytes == MAX_ROOM_BODY_BYTES + 1 && max_bytes == MAX_ROOM_BODY_BYTES
        ));

        let invalid_json = post_room_message(
            "aaaaa-aa",
            room_request("{\"broken\":", &[], Some(RoomContentType::ApplicationJson)),
            3,
        )
        .expect_err("malformed json should be rejected");
        assert!(matches!(
            invalid_json,
            FactoryError::InvalidRoomMessageJson { .. }
        ));

        let empty = post_room_message(
            "aaaaa-aa",
            room_request("   \n\t   ", &[], Some(RoomContentType::TextPlain)),
            4,
        )
        .expect_err("empty room body should be rejected");
        assert!(matches!(empty, FactoryError::EmptyRoomMessageBody));

        let too_many_mentions = post_room_message(
            "aaaaa-aa",
            PostRoomMessageRequest {
                body: "too many".to_string(),
                mentions: Some((0..17).map(|index| format!("mention-{index}")).collect()),
                content_type: Some(RoomContentType::TextPlain),
            },
            5,
        )
        .expect_err("mention overflow should be rejected");
        assert!(matches!(
            too_many_mentions,
            FactoryError::TooManyRoomMentions {
                provided: 17,
                max_mentions: 16,
            }
        ));
    }

    #[test]
    fn paginates_room_reads_and_evicts_oldest_messages_deterministically() {
        reset_factory_state();
        register_room_poster("aaaaa-aa");

        for seq in 0..(MAX_ROOM_MESSAGES_RETAINED as u64 + 2) {
            post_room_message(
                "aaaaa-aa",
                room_request(&format!("message-{seq}"), &[], None),
                seq,
            )
            .expect("room message should post");
        }

        let first_page = list_room_messages(None, Some(2)).expect("first room page should load");
        let second_page = list_room_messages(first_page.next_after_seq, Some(2))
            .expect("second room page should load");
        let tail_page = list_room_messages(Some(MAX_ROOM_MESSAGES_RETAINED as u64), Some(10))
            .expect("tail room page should load");
        let snapshot = snapshot_state();

        assert_eq!(snapshot.room_messages.len(), MAX_ROOM_MESSAGES_RETAINED);
        assert_eq!(snapshot.room_state.oldest_seq, Some(2));
        assert_eq!(
            snapshot.room_state.latest_seq,
            Some(MAX_ROOM_MESSAGES_RETAINED as u64 + 1)
        );
        assert_eq!(
            snapshot.room_state.next_seq,
            MAX_ROOM_MESSAGES_RETAINED as u64 + 2
        );
        assert_eq!(
            first_page
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(first_page.next_after_seq, Some(3));
        assert_eq!(
            second_page
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert_eq!(
            tail_page
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![MAX_ROOM_MESSAGES_RETAINED as u64 + 1]
        );
        assert_eq!(tail_page.next_after_seq, None);
        assert_eq!(
            tail_page.latest_seq,
            Some(MAX_ROOM_MESSAGES_RETAINED as u64 + 1)
        );
    }

    #[test]
    fn snapshots_and_restores_upgrade_safe_state() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 5_000)
            .expect("session should be created");
        write_state(|state| {
            state.payment_last_scanned_block = Some(4_321);
            state.next_payment_poll_at_ms = Some(9_999);
            state.registry.insert(
                "aaaaa-aa".to_string(),
                SpawnedAutomatonRecord {
                    canister_id: "aaaaa-aa".to_string(),
                    steward_address: "0xsteward".to_string(),
                    evm_address: "0xautomaton".to_string(),
                    chain: SpawnChain::Base,
                    session_id: response.session.session_id.clone(),
                    parent_id: None,
                    child_ids: Vec::new(),
                    created_at: 5_100,
                    version_commit: SHA40.to_string(),
                },
            );
            state.runtimes.insert(
                "aaaaa-aa".to_string(),
                crate::types::AutomatonRuntimeState {
                    canister_id: "aaaaa-aa".to_string(),
                    evm_address: "0xautomaton".to_string(),
                    steward_address: "0xsteward".to_string(),
                    session_id: response.session.session_id.clone(),
                    initialized_at: 5_200,
                    install_succeeded_at: Some(5_300),
                    evm_address_derived_at: Some(5_250),
                    controller_handoff_completed_at: Some(5_350),
                    funded_amount: "10000000".to_string(),
                    last_funded_at: Some(5_400),
                    chain: SpawnChain::Base,
                    risk: 7,
                    strategies: vec!["trend".to_string()],
                    skills: vec!["search".to_string()],
                    model: Some("openrouter/auto".to_string()),
                    provider_keys_cleared: false,
                    bootstrap_verification: None,
                },
            );
        });
        post_room_message(
            "aaaaa-aa",
            room_request("persisted room message", &[], None),
            5_450,
        )
        .expect("room message should persist through reload");
        let snapshot = snapshot_state();

        reset_factory_state();
        restore_state(snapshot.clone());

        let session = get_spawn_session(&response.session.session_id).expect("session should load");
        let admin_view =
            get_session_admin("admin", &response.session.session_id).expect("admin read works");
        let room_page = list_room_messages(None, Some(10)).expect("room page should load");

        assert_eq!(session.session.session_id, response.session.session_id);
        assert_eq!(admin_view.quote.gross_amount, "75000000");
        assert_eq!(admin_view.escrow_claim.required_gross_amount, "75000000");
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.payment_last_scanned_block, Some(4_321));
        assert_eq!(snapshot.next_payment_poll_at_ms, Some(9_999));
        assert_eq!(snapshot.registry.len(), 1);
        assert_eq!(snapshot.runtimes.len(), 1);
        assert_eq!(snapshot.room_state.latest_seq, Some(0));
        assert_eq!(snapshot.room_messages.len(), 1);
        assert_eq!(room_page.messages.len(), 1);
        assert_eq!(room_page.messages[0].body, "persisted room message");

        crate::state::reload_storage_for_test();
        assert_eq!(snapshot_state(), snapshot);
    }

    #[test]
    fn new_sessions_inherit_global_payment_scan_cursor() {
        reset_factory_state();
        write_state(|state| {
            state.payment_last_scanned_block = Some(8_888);
        });

        let response = create_spawn_session(sample_request("60000000"), 6_000)
            .expect("session should be created");
        let claim = get_escrow_claim(&response.session.session_id).expect("claim should exist");

        assert_eq!(response.session.last_scanned_block, Some(8_888));
        assert_eq!(claim.last_scanned_block, Some(8_888));
    }

    #[test]
    fn keeps_underfunded_claims_awaiting_payment() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("60000000"), 7_000)
            .expect("session should be created");
        let claim = reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "59000000",
                1_234,
            )],
            1_234,
            8_000,
        )
        .expect("underfunded claim should sync");
        let session = get_spawn_session(&response.session.session_id).expect("session should load");

        assert_eq!(claim[0].payment_status, PaymentStatus::Partial);
        assert_eq!(session.session.state, SpawnSessionState::AwaitingPayment);
        assert_eq!(session.session.payment_status, PaymentStatus::Partial);
        assert_eq!(session.session.last_scanned_block, Some(1_234));
    }

    #[test]
    fn batches_active_sessions_into_a_single_scan_plan() {
        reset_factory_state();
        write_state(|state| {
            state.payment_last_scanned_block = Some(12_000);
        });

        let first = create_spawn_session(sample_request("60000000"), 7_000)
            .expect("first session should be created");
        let second = create_spawn_session(sample_request("75000000"), 7_100)
            .expect("second session should be created");

        let plan = next_payment_scan_plan(12_250).expect("scan plan should exist");
        assert_eq!(plan.from_block, 12_001);
        assert_eq!(plan.to_block, 12_250);
        assert_eq!(plan.claim_ids.len(), 2);
        assert!(plan.claim_ids.contains(&first.session.claim_id));
        assert!(plan.claim_ids.contains(&second.session.claim_id));
    }

    #[test]
    fn accumulates_multiple_base_logs_for_the_same_claim() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("60000000"), 8_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "10000000",
                200,
            )],
            200,
            8_500,
        )
        .expect("first payment batch should sync");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "50000000",
                220,
            )],
            220,
            8_600,
        )
        .expect("second payment batch should sync");

        let session = get_spawn_session(&response.session.session_id).expect("session should load");
        let claim = get_escrow_claim(&response.session.session_id).expect("claim should load");

        assert_eq!(session.session.payment_status, PaymentStatus::Paid);
        assert_eq!(session.session.state, SpawnSessionState::PaymentDetected);
        assert_eq!(claim.paid_amount, "60000000");
        assert_eq!(claim.last_scanned_block, Some(220));
    }

    #[test]
    fn auto_runs_spawn_after_payment_detection() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
        });

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");

        let receipts = auto_run_spawn_scheduler(11_000);
        let receipt = receipts
            .into_iter()
            .next()
            .expect("scheduler should attempt one session")
            .expect("auto-run should complete");
        let session = get_spawn_session(&response.session.session_id).expect("session should load");

        assert_eq!(receipt.session_id, response.session.session_id);
        assert_eq!(session.session.state, SpawnSessionState::Complete);
        assert_eq!(session.session.payment_status, PaymentStatus::Paid);
    }

    #[test]
    fn runs_scheduler_flow_through_payment_detection_reload_retry_and_completion() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 10_500)
            .expect("session should be created");
        write_state(|state| {
            state.base_rpc_endpoint = Some(mock_deposit_log_endpoint(
                &response.session.claim_id,
                "75000000",
                42,
            ));
        });

        let first_reports = run_scheduler_tick(11_000);
        let failed = get_spawn_session(&response.session.session_id).expect("session should load");

        assert_eq!(first_reports.len(), 2);
        assert!(matches!(
            first_reports[0].kind,
            SchedulerJobKind::PaymentPoll
        ));
        assert!(first_reports[0].error.is_none());
        assert!(matches!(
            first_reports[1].kind,
            SchedulerJobKind::SpawnExecution { .. }
        ));
        assert!(matches!(
            first_reports[1].error,
            Some(FactoryError::ManagementCallFailed {
                ref method,
                ref message,
            }) if method == "install_code" && message == "artifact not loaded"
        ));
        assert_eq!(failed.session.payment_status, PaymentStatus::Paid);
        assert_eq!(failed.session.state, SpawnSessionState::Failed);
        assert!(failed.session.retryable);
        assert!(failed
            .audit
            .iter()
            .any(|entry| entry.to_state == SpawnSessionState::PaymentDetected));

        crate::state::reload_storage_for_test();
        upload_test_artifact();
        retry_spawn_session("0xsteward", &response.session.session_id, 12_000)
            .expect("retry should re-queue the paid session");

        let second_reports = run_scheduler_tick(13_000);
        let completed =
            get_spawn_session(&response.session.session_id).expect("session should load again");

        assert_eq!(second_reports.len(), 1);
        assert!(matches!(
            second_reports[0].kind,
            SchedulerJobKind::SpawnExecution { .. }
        ));
        assert!(second_reports[0].error.is_none());
        assert_eq!(completed.session.state, SpawnSessionState::Complete);
        assert_eq!(completed.session.payment_status, PaymentStatus::Paid);
        assert!(completed
            .audit
            .iter()
            .any(|entry| entry.reason == "payment detected from Base logs"));
        assert!(completed
            .audit
            .iter()
            .any(|entry| entry.to_state == SpawnSessionState::Complete));
    }

    #[test]
    fn auto_run_does_not_execute_twice_for_completed_sessions() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
        });

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");

        let first = auto_run_spawn_scheduler(11_000);
        let second = auto_run_spawn_scheduler(12_000);
        let status = get_spawn_session(&response.session.session_id).expect("session should load");
        let completed_entries = status
            .audit
            .iter()
            .filter(|entry| entry.to_state == SpawnSessionState::Complete)
            .count();

        assert_eq!(first.len(), 1);
        assert!(first[0].is_ok());
        assert!(second.is_empty());
        assert_eq!(completed_entries, 1);
    }

    #[test]
    fn executes_spawn_after_paid_claim_and_hands_off_controller() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
        });

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        let claim = get_escrow_claim(&response.session.session_id).expect("claim should exist");
        assert_eq!(claim.quote_terms_hash, response.session.quote_terms_hash);
        assert_eq!(claim.claim_id, response.session.claim_id);

        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");
        let receipt =
            execute_spawn(&response.session.session_id, 11_000).expect("spawn should complete");
        let session = get_spawn_session(&response.session.session_id).expect("session should load");
        let admin_view =
            get_session_admin("admin", &response.session.session_id).expect("admin read works");
        let runtime = snapshot_state()
            .runtimes
            .get(&receipt.automaton_canister_id)
            .cloned()
            .expect("runtime should exist");

        assert_eq!(session.session.state, SpawnSessionState::Complete);
        assert_eq!(session.session.payment_status, PaymentStatus::Paid);
        assert_eq!(receipt.funded_amount, "25000000");
        assert_eq!(session.session.release_tx_hash, receipt.release_tx_hash);
        assert_eq!(
            session.session.release_broadcast_at,
            receipt.release_broadcast_at
        );
        assert_eq!(
            session
                .session
                .release_broadcast
                .as_ref()
                .map(|record| record.nonce),
            Some(1)
        );
        assert_eq!(
            session
                .session
                .release_broadcast
                .as_ref()
                .and_then(|record| record.rpc_tx_hash.as_deref()),
            receipt.release_tx_hash.as_deref()
        );
        assert!(runtime.controller_handoff_completed_at.is_some());
        let verification = runtime
            .bootstrap_verification
            .as_ref()
            .expect("bootstrap verification should persist");
        assert!(verification.passed);
        assert_eq!(
            verification.evidence.bootstrap_session_id.as_deref(),
            Some(response.session.session_id.as_str())
        );
        assert!(runtime.provider_keys_cleared);
        assert!(load_spawn_provider_secrets(&response.session.session_id).is_none());
        assert_eq!(
            admin_view
                .runtime_record
                .as_ref()
                .and_then(|runtime| runtime.bootstrap_verification.as_ref())
                .map(|verification| verification.passed),
            Some(true)
        );
        assert_eq!(
            admin_view
                .registry_record
                .expect("registry record should exist")
                .evm_address,
            receipt.automaton_evm_address
        );
    }

    #[test]
    fn fails_spawn_atomically_when_strategy_registration_fails() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
        });

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        write_state(|state| {
            let session = state
                .sessions
                .get_mut(&response.session.session_id)
                .expect("session should exist");
            session.selected_strategies[0].recipe_json = "{".to_string();
        });
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");

        let error = execute_spawn(&response.session.session_id, 11_000)
            .expect_err("strategy registration should fail");
        let session = get_spawn_session(&response.session.session_id).expect("session should load");
        let canister_id = session
            .session
            .automaton_canister_id
            .clone()
            .expect("failed session should still record the attempted child");
        let runtime = snapshot_state()
            .runtimes
            .get(&canister_id)
            .cloned()
            .expect("failed runtime should persist");

        assert!(matches!(
            error,
            FactoryError::ManagementCallFailed { ref method, ref message }
                if method == "register_strategy_admin"
                    && message.contains("invalid session snapshot recipe JSON")
        ));
        assert_eq!(session.session.state, SpawnSessionState::Failed);
        assert!(session.session.retryable);
        assert!(snapshot_state().registry.is_empty());
        assert!(runtime.install_succeeded_at.is_none());
        assert!(runtime.controller_handoff_completed_at.is_none());
        assert_eq!(runtime.funded_amount, "0");
        assert!(session
            .audit
            .last()
            .expect("failure audit should exist")
            .reason
            .contains("strategy registration failed"));
    }

    #[test]
    fn retries_from_session_snapshots_even_after_repository_revocation() {
        reset_factory_state();
        upload_test_artifact();

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");

        let first_error =
            execute_spawn(&response.session.session_id, 11_000).expect_err("spawn should fail");
        assert!(matches!(
            first_error,
            FactoryError::ManagementCallFailed { ref method, ref message }
                if method == "http_request"
                    && message == "base RPC endpoint is not configured"
        ));

        revoke_repository_strategy(
            "admin",
            RevokeRepositoryStrategyRequest {
                strategy_id: "base-aave-usdc-reserve-01".to_string(),
                reason: Some("repository changed after session creation".to_string()),
            },
            11_500,
        )
        .expect("repository strategy can be revoked after the session was created");
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
        });

        retry_spawn_session("0xsteward", &response.session.session_id, 12_000)
            .expect("retry should succeed");
        let receipt =
            execute_spawn(&response.session.session_id, 13_000).expect("retry should complete");
        let session = get_spawn_session(&response.session.session_id).expect("session should load");

        assert_eq!(session.session.state, SpawnSessionState::Complete);
        assert_eq!(receipt.session_id, response.session.session_id);
    }

    #[test]
    fn reuses_release_tracking_when_execute_spawn_is_replayed_after_completion() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
        });

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");

        let first = execute_spawn(&response.session.session_id, 11_000)
            .expect("first spawn should complete");
        let second = execute_spawn(&response.session.session_id, 12_000)
            .expect("completed spawn should return cached receipt");

        assert_eq!(second.session_id, first.session_id);
        assert_eq!(second.automaton_canister_id, first.automaton_canister_id);
        assert_eq!(second.automaton_evm_address, first.automaton_evm_address);
        assert_eq!(second.release_tx_hash, first.release_tx_hash);
        assert_eq!(second.release_broadcast_at, first.release_broadcast_at);
        assert_eq!(second.completed_at, first.completed_at);
    }

    #[test]
    fn persists_release_broadcast_failure_context_on_spawn_errors() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("mock://error/rate-limit".to_string());
        });

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");

        let error =
            execute_spawn(&response.session.session_id, 11_000).expect_err("broadcast should fail");
        let session = get_spawn_session(&response.session.session_id).expect("session should load");

        assert!(matches!(error, FactoryError::RpcRequestFailed { .. }));
        assert_eq!(session.session.state, SpawnSessionState::Failed);
        assert_eq!(
            session
                .session
                .release_broadcast
                .as_ref()
                .and_then(|record| record.last_error.as_ref())
                .and_then(|entry| entry.rpc_code),
            Some(429)
        );
        assert_eq!(
            session
                .session
                .release_broadcast
                .as_ref()
                .map(|record| record.max_fee_per_gas),
            Some(3_000_000_000)
        );
    }

    #[test]
    fn fails_spawn_before_install_when_child_runtime_config_is_missing() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.child_runtime = AutomatonChildRuntimeConfig::default();
        });

        let response = create_spawn_session(sample_request("75000000"), 9_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                1_500,
            )],
            1_500,
            10_000,
        )
        .expect("claim should become paid");

        let error = execute_spawn(&response.session.session_id, 11_000)
            .expect_err("missing child runtime config should fail early");
        let session = get_spawn_session(&response.session.session_id).expect("session should load");

        assert!(matches!(
            error,
            FactoryError::MissingChildRuntimeConfig { ref field }
                if field == "child_runtime.ecdsa_key_name"
        ));
        assert_eq!(session.session.state, SpawnSessionState::Failed);
        assert!(session.session.retryable);
        assert!(session
            .audit
            .last()
            .expect("failure audit should exist")
            .reason
            .contains("missing child runtime config: child_runtime.ecdsa_key_name"));
        assert!(session.session.automaton_canister_id.is_none());
        assert!(snapshot_state().runtimes.is_empty());
    }

    #[test]
    fn reports_factory_health_with_active_counts_and_artifact_metadata() {
        reset_factory_state();
        upload_test_artifact();
        set_mock_canister_balance(9_999);
        write_state(|state| {
            state.cycles_per_spawn = 2_000;
            state.min_pool_balance = 3_000;
            state.estimated_outcall_cycles_per_interval = 777;
            state.factory_evm_address = Some("0xFactory".to_string());
        });

        let awaiting = create_spawn_session(sample_request("60000000"), 7_000)
            .expect("awaiting session should be created");
        let paid = create_spawn_session(sample_request("75000000"), 7_100)
            .expect("paid session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(&paid.session.session_id, "75000000", 300)],
            300,
            7_200,
        )
        .expect("claim should become paid");
        mark_session_failed(
            &paid.session.session_id,
            SessionAuditActor::System,
            7_300,
            "downstream install failed",
        )
        .expect("failure should be recorded");
        write_state(|state| {
            state.pause = true;
        });

        let health = get_factory_health();
        assert_eq!(health.current_canister_balance, 9_999);
        assert!(health.pause);
        assert_eq!(health.cycles_per_spawn, 2_000);
        assert_eq!(health.min_pool_balance, 3_000);
        assert_eq!(health.estimated_outcall_cycles_per_interval, 777);
        assert_eq!(
            health.escrow_contract_address,
            "0x2222222222222222222222222222222222222222"
        );
        assert_eq!(health.factory_evm_address.as_deref(), Some("0xFactory"));
        assert!(health.artifact.loaded);
        assert_eq!(health.artifact.version_commit.as_deref(), Some(SHA40));
        assert_eq!(health.active_sessions.active_total(), 2);
        assert_eq!(health.active_sessions.awaiting_payment, 1);
        assert_eq!(health.active_sessions.retryable_failed, 1);
        assert_eq!(health.scheduler.job_counts.total, 2);
        assert_eq!(health.scheduler.job_counts.pending, 2);
        assert_eq!(health.scheduler.retry_queue_count, 0);
        assert_eq!(health.scheduler.job_counts.with_last_error, 0);
        assert!(health.scheduler.active_job_ids.is_empty());
        assert_eq!(awaiting.session.state, SpawnSessionState::AwaitingPayment);
    }

    #[test]
    fn returns_runtime_view_with_active_jobs_retry_queue_and_failed_details() {
        reset_factory_state();
        write_state(|state| {
            state.scheduler_runtime = SchedulerRuntime {
                last_tick_started_ms: Some(40_000),
                last_tick_finished_ms: Some(40_900),
                last_tick_error: Some(
                    "spawn-execution:session-backoff: upstream unavailable".to_string(),
                ),
                active_job_ids: vec![PAYMENT_POLL_JOB_ID.to_string()],
            };
            state.scheduler_jobs.insert(
                PAYMENT_POLL_JOB_ID.to_string(),
                SchedulerJob {
                    job_id: PAYMENT_POLL_JOB_ID.to_string(),
                    kind: SchedulerJobKind::PaymentPoll,
                    status: SchedulerJobStatus::Running,
                    next_run_at_ms: Some(40_000),
                    leased_at_ms: Some(40_500),
                    leased_until_ms: Some(41_500),
                    last_started_at_ms: Some(40_500),
                    last_finished_at_ms: None,
                    attempt_count: 3,
                    consecutive_failure_count: 0,
                    success_count: 1,
                    last_error: None,
                },
            );
            state.scheduler_jobs.insert(
                "spawn-execution:session-backoff".to_string(),
                SchedulerJob {
                    job_id: "spawn-execution:session-backoff".to_string(),
                    kind: SchedulerJobKind::SpawnExecution {
                        session_id: "session-backoff".to_string(),
                    },
                    status: SchedulerJobStatus::Backoff,
                    next_run_at_ms: Some(41_000),
                    leased_at_ms: None,
                    leased_until_ms: None,
                    last_started_at_ms: Some(40_600),
                    last_finished_at_ms: Some(40_700),
                    attempt_count: 2,
                    consecutive_failure_count: 1,
                    success_count: 0,
                    last_error: Some(SchedulerJobFailure {
                        action: SchedulerFailureAction::Backoff,
                        source: SchedulerFailureSource::Transient,
                        message: "upstream unavailable".to_string(),
                        occurred_at: 40_700,
                    }),
                },
            );
            state.scheduler_jobs.insert(
                "spawn-execution:session-terminal".to_string(),
                SchedulerJob {
                    job_id: "spawn-execution:session-terminal".to_string(),
                    kind: SchedulerJobKind::SpawnExecution {
                        session_id: "session-terminal".to_string(),
                    },
                    status: SchedulerJobStatus::Terminal,
                    next_run_at_ms: None,
                    leased_at_ms: None,
                    leased_until_ms: None,
                    last_started_at_ms: Some(40_750),
                    last_finished_at_ms: Some(40_800),
                    attempt_count: 1,
                    consecutive_failure_count: 1,
                    success_count: 0,
                    last_error: Some(SchedulerJobFailure {
                        action: SchedulerFailureAction::Terminal,
                        source: SchedulerFailureSource::Deterministic,
                        message: "session expired".to_string(),
                        occurred_at: 40_800,
                    }),
                },
            );
        });

        let runtime = get_factory_runtime("admin", 2).expect("admin runtime view should load");
        assert_eq!(runtime.scheduler.last_tick_started_ms, Some(40_000));
        assert_eq!(runtime.scheduler.last_tick_finished_ms, Some(40_900));
        assert_eq!(
            runtime.scheduler.last_tick_error.as_deref(),
            Some("spawn-execution:session-backoff: upstream unavailable")
        );
        assert_eq!(
            runtime.scheduler.active_job_ids,
            vec![PAYMENT_POLL_JOB_ID.to_string()]
        );
        assert_eq!(runtime.scheduler.job_counts.total, 3);
        assert_eq!(runtime.scheduler.job_counts.running, 1);
        assert_eq!(runtime.scheduler.job_counts.backoff, 1);
        assert_eq!(runtime.scheduler.job_counts.terminal, 1);
        assert_eq!(runtime.scheduler.job_counts.with_last_error, 2);
        assert_eq!(runtime.scheduler.retry_queue_count, 1);
        assert_eq!(runtime.scheduler.job_counts.with_last_error, 2);

        assert_eq!(runtime.active_jobs.len(), 1);
        assert_eq!(runtime.active_jobs[0].job_id, PAYMENT_POLL_JOB_ID);
        assert_eq!(runtime.retry_queue.len(), 1);
        assert_eq!(
            runtime.retry_queue[0].job_id,
            "spawn-execution:session-backoff"
        );
        assert_eq!(runtime.recent_jobs.len(), 2);
        assert_eq!(
            runtime.recent_jobs[0].job_id,
            "spawn-execution:session-terminal"
        );
        assert_eq!(
            runtime.recent_jobs[1].job_id,
            "spawn-execution:session-backoff"
        );
        assert_eq!(runtime.failed_jobs.len(), 2);
        assert_eq!(
            runtime.failed_jobs[0].job_id,
            "spawn-execution:session-terminal"
        );
        assert_eq!(
            runtime.failed_jobs[0]
                .last_error
                .as_ref()
                .map(|failure| failure.message.as_str()),
            Some("session expired")
        );
        assert_eq!(
            runtime.failed_jobs[1].job_id,
            "spawn-execution:session-backoff"
        );

        let unauthorized =
            get_factory_runtime("not-admin", 2).expect_err("non-admin should be rejected");
        assert!(matches!(
            unauthorized,
            FactoryError::UnauthorizedAdmin { .. }
        ));

        let invalid_limit =
            get_factory_runtime("admin", 0).expect_err("zero limit should be rejected");
        assert!(matches!(
            invalid_limit,
            FactoryError::InvalidPaginationLimit { limit: 0 }
        ));
    }

    #[test]
    fn retries_paid_failed_sessions_for_steward_and_admin() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 12_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                2_000,
            )],
            2_000,
            13_000,
        )
        .expect("claim should become paid");

        mark_session_failed(
            &response.session.session_id,
            SessionAuditActor::System,
            14_000,
            "provider initialization failed",
        )
        .expect("failure should be recorded");
        let failed = get_spawn_session(&response.session.session_id).expect("session should load");
        assert_eq!(failed.session.state, SpawnSessionState::Failed);
        assert!(failed.session.retryable);
        assert_eq!(
            failed
                .audit
                .last()
                .expect("failure audit should exist")
                .from_state,
            Some(SpawnSessionState::PaymentDetected)
        );
        assert_eq!(
            failed
                .audit
                .last()
                .expect("failure audit should exist")
                .to_state,
            SpawnSessionState::Failed
        );
        assert_eq!(
            load_spawn_provider_secrets(&response.session.session_id)
                .and_then(|secrets| secrets.open_router_api_key),
            Some("or-key".to_string())
        );
        crate::state::reload_storage_for_test();
        assert_eq!(
            load_spawn_provider_secrets(&response.session.session_id)
                .and_then(|secrets| secrets.brave_search_api_key),
            Some("brave-key".to_string())
        );

        let retried = retry_spawn_session("0xsteward", &response.session.session_id, 15_000)
            .expect("steward retry should succeed");
        assert_eq!(retried.session.state, SpawnSessionState::PaymentDetected);
        assert!(!retried.session.retryable);
        assert_eq!(
            retried
                .audit
                .last()
                .expect("retry audit should exist")
                .from_state,
            Some(SpawnSessionState::Failed)
        );
        assert_eq!(
            retried
                .audit
                .last()
                .expect("retry audit should exist")
                .to_state,
            SpawnSessionState::PaymentDetected
        );

        mark_session_failed(
            &response.session.session_id,
            SessionAuditActor::System,
            16_000,
            "funding transfer failed",
        )
        .expect("second failure should be recorded");
        let admin_retry = retry_session_admin("admin", &response.session.session_id, 17_000)
            .expect("admin retry should succeed");
        assert_eq!(
            admin_retry.session.state,
            SpawnSessionState::PaymentDetected
        );
        assert_eq!(
            admin_retry
                .audit
                .last()
                .expect("retry audit should exist")
                .actor,
            SessionAuditActor::Admin
        );
    }

    #[test]
    fn marks_auto_run_failure_retryable_when_base_rpc_is_missing_and_retries_after_reload() {
        reset_factory_state();
        upload_test_artifact();

        let response = create_spawn_session(sample_request("75000000"), 12_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                2_000,
            )],
            2_000,
            13_000,
        )
        .expect("claim should become paid");

        let first = auto_run_spawn_scheduler(14_000);
        let failed = get_spawn_session(&response.session.session_id).expect("session should load");

        assert_eq!(first.len(), 1);
        assert!(matches!(
            first[0],
            Err(super::FactoryError::ManagementCallFailed { .. })
        ));
        let failed_job = snapshot_state()
            .scheduler_jobs
            .get(&spawn_job_id(&response.session.session_id))
            .cloned()
            .expect("spawn job should exist");
        assert_eq!(failed_job.status, SchedulerJobStatus::Skipped);
        assert_eq!(
            failed_job
                .last_error
                .expect("missing-config error should be persisted")
                .source,
            SchedulerFailureSource::MissingConfig
        );
        assert_eq!(failed.session.state, SpawnSessionState::Failed);
        assert!(failed.session.retryable);
        assert_eq!(
            failed
                .audit
                .last()
                .expect("failure audit should exist")
                .to_state,
            SpawnSessionState::Failed
        );

        crate::state::reload_storage_for_test();
        let failed_after_reload =
            get_spawn_session(&response.session.session_id).expect("session should survive reload");
        assert_eq!(failed_after_reload.session.state, SpawnSessionState::Failed);
        assert!(failed_after_reload.session.retryable);

        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
        });
        retry_spawn_session("0xsteward", &response.session.session_id, 15_000)
            .expect("retry should move the session back to payment_detected");

        let retried = auto_run_spawn_scheduler(16_000);
        let completed =
            get_spawn_session(&response.session.session_id).expect("session should load again");

        assert_eq!(retried.len(), 1);
        assert!(retried[0].is_ok());
        assert_eq!(completed.session.state, SpawnSessionState::Complete);
    }

    #[test]
    fn leases_only_one_active_owner_per_job_at_a_time() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("60000000"), 18_000)
            .expect("session should be created");

        let first = lease_due_jobs_for_test(18_000, 1);
        let second = lease_due_jobs_for_test(18_000, 1);
        let snapshot = snapshot_state();

        assert_eq!(first.len(), 1);
        assert_eq!(first[0].job_id, PAYMENT_POLL_JOB_ID);
        assert!(second.is_empty());
        assert!(snapshot
            .scheduler_runtime
            .active_job_ids
            .contains(&PAYMENT_POLL_JOB_ID.to_string()));
        assert_eq!(
            snapshot
                .sessions
                .get(&response.session.session_id)
                .expect("session should exist")
                .state,
            SpawnSessionState::AwaitingPayment
        );
    }

    #[test]
    fn recovers_stale_job_leases_after_expiry() {
        reset_factory_state();
        create_spawn_session(sample_request("60000000"), 19_000)
            .expect("session should be created");

        let first = lease_due_jobs_for_test(19_000, 1);
        let blocked = lease_due_jobs_for_test(19_001, 1);
        let recovered = lease_due_jobs_for_test(79_001, 1);

        assert_eq!(first.len(), 1);
        assert!(blocked.is_empty());
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].job_id, PAYMENT_POLL_JOB_ID);
        assert_eq!(recovered[0].status, SchedulerJobStatus::Running);
        assert!(recovered[0].leased_until_ms.expect("lease should renew") > 79_001);
    }

    #[test]
    fn reload_keeps_leased_spawn_jobs_blocked_until_stale_then_recovers_partial_session_work() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("mock://success".to_string());
        });

        let response = create_spawn_session(sample_request("75000000"), 30_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                7_000,
            )],
            7_000,
            30_500,
        )
        .expect("claim should become paid");

        let reserved_canister_id = "automaton-resume-0001".to_string();
        let reserved_evm_address = crate::init::derive_automaton_evm_address(&reserved_canister_id);
        write_state(|state| {
            let session = state
                .sessions
                .get_mut(&response.session.session_id)
                .expect("session should exist");
            session.automaton_canister_id = Some(reserved_canister_id.clone());
            session.automaton_evm_address = Some(reserved_evm_address.clone());
        });

        let spawn_job_id = spawn_job_id(&response.session.session_id);
        let leased = lease_due_jobs_for_test(31_000, 1);
        assert_eq!(leased.len(), 1);
        assert_eq!(leased[0].job_id, spawn_job_id);

        crate::state::reload_storage_for_test();

        let reloaded_snapshot = snapshot_state();
        let reloaded_job = reloaded_snapshot
            .scheduler_jobs
            .get(&spawn_job_id)
            .cloned()
            .expect("leased spawn job should persist");
        assert_eq!(reloaded_job.status, SchedulerJobStatus::Running);
        assert_eq!(reloaded_job.leased_at_ms, Some(31_000));
        assert_eq!(
            reloaded_snapshot.scheduler_runtime.active_job_ids,
            vec![spawn_job_id.clone()]
        );

        let blocked_reports = run_scheduler_tick(31_001);
        let blocked_session =
            get_spawn_session(&response.session.session_id).expect("session should stay pending");
        assert!(blocked_reports.is_empty());
        assert_eq!(
            blocked_session.session.state,
            SpawnSessionState::PaymentDetected
        );
        assert_eq!(
            blocked_session.session.automaton_canister_id.as_deref(),
            Some(reserved_canister_id.as_str())
        );

        let recovered_reports = run_scheduler_tick(91_001);
        let completed =
            get_spawn_session(&response.session.session_id).expect("session should complete");

        assert_eq!(recovered_reports.len(), 1);
        assert!(matches!(
            recovered_reports[0].kind,
            SchedulerJobKind::SpawnExecution { .. }
        ));
        assert!(recovered_reports[0].error.is_none());
        assert_eq!(completed.session.state, SpawnSessionState::Complete);
        assert_eq!(
            completed.session.automaton_canister_id.as_deref(),
            Some(reserved_canister_id.as_str())
        );
        assert_eq!(
            completed.session.automaton_evm_address.as_deref(),
            Some(reserved_evm_address.as_str())
        );
    }

    #[test]
    fn backs_off_failed_payment_poll_jobs_and_persists_tick_runtime() {
        reset_factory_state();
        create_spawn_session(sample_request("60000000"), 20_000)
            .expect("session should be created");
        write_state(|state| {
            state.base_rpc_endpoint = Some("mock://error/upstream-unavailable".to_string());
        });

        let reports = run_scheduler_tick(20_500);
        let first_snapshot = snapshot_state();
        let poll_job = first_snapshot
            .scheduler_jobs
            .get(PAYMENT_POLL_JOB_ID)
            .cloned()
            .expect("payment poll job should exist");

        assert_eq!(reports.len(), 1);
        assert!(reports[0].error.is_some());
        assert_eq!(poll_job.status, SchedulerJobStatus::Backoff);
        assert!(
            poll_job
                .next_run_at_ms
                .expect("backoff should schedule retry")
                > 20_500
        );
        assert_eq!(poll_job.attempt_count, 1);
        assert_eq!(
            poll_job
                .last_error
                .clone()
                .expect("poll failure should be persisted")
                .action,
            SchedulerFailureAction::Backoff
        );
        assert_eq!(
            poll_job
                .last_error
                .expect("poll failure should be persisted")
                .source,
            SchedulerFailureSource::Transient
        );
        assert_eq!(
            first_snapshot.scheduler_runtime.last_tick_started_ms,
            Some(20_500)
        );
        assert_eq!(
            first_snapshot.scheduler_runtime.last_tick_finished_ms,
            Some(20_500)
        );
        assert!(first_snapshot
            .scheduler_runtime
            .last_tick_error
            .as_deref()
            .unwrap_or_default()
            .contains(PAYMENT_POLL_JOB_ID));

        let second_reports = run_scheduler_tick(20_501);
        let second_snapshot = snapshot_state();
        let second_poll_job = second_snapshot
            .scheduler_jobs
            .get(PAYMENT_POLL_JOB_ID)
            .cloned()
            .expect("payment poll job should remain stored");

        assert!(second_reports.is_empty());
        assert_eq!(second_poll_job.attempt_count, 1);
        assert_eq!(second_poll_job.next_run_at_ms, poll_job.next_run_at_ms);
    }

    #[test]
    fn backs_off_failed_spawn_jobs_instead_of_hot_looping() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
            state.cycles_per_spawn = 1_000;
            state.min_pool_balance = 500;
        });
        set_mock_canister_balance(1_499);

        let response = create_spawn_session(sample_request("75000000"), 21_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                5_000,
            )],
            5_000,
            21_500,
        )
        .expect("claim should become paid");

        let first_reports = run_scheduler_tick(22_000);
        let first_snapshot = snapshot_state();
        let spawn_job_id = spawn_job_id(&response.session.session_id);
        let first_job = first_snapshot
            .scheduler_jobs
            .get(&spawn_job_id)
            .cloned()
            .expect("spawn job should exist");

        assert_eq!(first_reports.len(), 1);
        assert!(matches!(
            first_reports[0].error,
            Some(super::FactoryError::InsufficientCyclesPool { .. })
        ));
        assert_eq!(first_job.status, SchedulerJobStatus::Backoff);
        assert!(
            first_job
                .next_run_at_ms
                .expect("backoff should be scheduled")
                > 22_000
        );
        assert_eq!(first_job.attempt_count, 1);
        assert_eq!(
            first_job
                .last_error
                .clone()
                .expect("spawn failure should be persisted")
                .action,
            SchedulerFailureAction::Backoff
        );
        assert_eq!(
            first_job
                .last_error
                .expect("spawn failure should be persisted")
                .source,
            SchedulerFailureSource::Transient
        );

        let second_reports = run_scheduler_tick(22_001);
        let second_snapshot = snapshot_state();
        let second_job = second_snapshot
            .scheduler_jobs
            .get(&spawn_job_id)
            .cloned()
            .expect("spawn job should remain stored");

        assert!(second_reports.is_empty());
        assert_eq!(second_job.attempt_count, 1);
        assert_eq!(second_job.next_run_at_ms, first_job.next_run_at_ms);
    }

    #[test]
    fn marks_session_failed_when_cycles_pool_is_below_required_threshold() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("https://base.example".to_string());
            state.cycles_per_spawn = 1_000;
            state.min_pool_balance = 500;
        });
        set_mock_canister_balance(1_499);

        let response = create_spawn_session(sample_request("75000000"), 12_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                2_000,
            )],
            2_000,
            13_000,
        )
        .expect("claim should become paid");

        let error = execute_spawn(&response.session.session_id, 14_000)
            .expect_err("spawn should fail early on insufficient cycles");
        assert!(matches!(
            error,
            super::FactoryError::InsufficientCyclesPool {
                available: 1_499,
                required: 1_500
            }
        ));
        let failed = get_spawn_session(&response.session.session_id).expect("session should load");
        assert_eq!(failed.session.state, SpawnSessionState::Failed);
        assert!(failed.session.retryable);
    }

    #[test]
    fn distinguishes_follow_up_operation_cycles_from_spawn_creation_cycles() {
        reset_factory_state();
        upload_test_artifact();
        write_state(|state| {
            state.base_rpc_endpoint = Some("mock://success".to_string());
            state.cycles_per_spawn = 1;
            state.min_pool_balance = 0;
        });
        set_mock_canister_balance(1);

        let response = create_spawn_session(sample_request("75000000"), 12_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                2_000,
            )],
            2_000,
            13_000,
        )
        .expect("claim should become paid");

        let error = execute_spawn(&response.session.session_id, 14_000)
            .expect_err("spawn should fail on follow-up affordability");
        assert!(matches!(
            error,
            super::FactoryError::InsufficientCyclesForOperation { ref operation, .. }
                if operation == "sign_with_ecdsa"
        ));

        let failed = get_spawn_session(&response.session.session_id).expect("session should load");
        assert_eq!(failed.session.state, SpawnSessionState::Failed);
        assert!(failed.session.retryable);
    }

    #[test]
    fn expires_underfunded_sessions_and_allows_refund() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("60000000"), 20_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "59000000",
                3_000,
            )],
            3_000,
            21_000,
        )
        .expect("underfunded claim should sync");

        let expired =
            expire_spawn_session(&response.session.session_id, 20_000 + 30 * 60 * 1_000 + 1)
                .expect("session should expire");
        assert_eq!(expired.state, SpawnSessionState::Expired);
        assert!(expired.refundable);
        assert_eq!(
            get_spawn_session(&response.session.session_id)
                .expect("session should load")
                .audit
                .last()
                .expect("expiry audit should exist")
                .from_state,
            Some(SpawnSessionState::AwaitingPayment)
        );
        assert_eq!(
            get_spawn_session(&response.session.session_id)
                .expect("session should load")
                .audit
                .last()
                .expect("expiry audit should exist")
                .to_state,
            SpawnSessionState::Expired
        );

        let refund = claim_spawn_refund(
            "0xsteward",
            &response.session.session_id,
            20_000 + 30 * 60 * 1_000 + 2,
        )
        .expect("refund should succeed");
        assert_eq!(refund.state, SpawnSessionState::Expired);
        assert_eq!(refund.payment_status, PaymentStatus::Refunded);

        let refunded =
            get_spawn_session(&response.session.session_id).expect("session should load");
        assert_eq!(refunded.session.payment_status, PaymentStatus::Refunded);
        assert!(load_spawn_provider_secrets(&response.session.session_id).is_none());
        assert_eq!(
            refunded
                .audit
                .last()
                .expect("refund audit should exist")
                .reason,
            "refund claimed after expiration"
        );
    }

    #[test]
    fn retries_failed_paid_sessions_after_deadline_by_extending_ttl() {
        reset_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 30_000)
            .expect("session should be created");
        reconcile_escrow_payments(
            &[base_deposit_log(
                &response.session.session_id,
                "75000000",
                4_000,
            )],
            4_000,
            31_000,
        )
        .expect("claim should become paid");
        mark_session_failed(
            &response.session.session_id,
            SessionAuditActor::System,
            32_000,
            "controller handoff failed",
        )
        .expect("failure should be recorded");

        let retry_response = retry_spawn_session(
            "0xsteward",
            &response.session.session_id,
            30_000 + 30 * 60 * 1_000 + 5,
        )
        .expect("retry should extend the effective lifetime");

        assert_eq!(
            retry_response.session.state,
            SpawnSessionState::PaymentDetected
        );
        assert!(!retry_response.session.retryable);
        assert!(retry_response.session.expires_at > response.session.expires_at);
        assert_eq!(
            retry_response
                .audit
                .last()
                .expect("retry audit should exist")
                .to_state,
            SpawnSessionState::PaymentDetected
        );
    }

    #[test]
    fn derives_claim_id_from_uuid_utf8_bytes() {
        assert_eq!(
            derive_claim_id("550e8400-e29b-41d4-a716-446655440000"),
            "0x2f779c94a35dceba72fe536ce28c5fea7566753044cdf9da29f6402ea964b7f9"
        );
    }

    #[test]
    fn derives_and_persists_factory_evm_address_from_public_key() {
        reset_factory_state();

        let public_key = [
            0x02, 0x00, 0x86, 0x6d, 0xb9, 0x98, 0x73, 0xb0, 0x9f, 0xc2, 0xfb, 0x1e, 0x3b, 0xa5,
            0x49, 0xb1, 0x56, 0xe9, 0x6d, 0x1a, 0x56, 0x7e, 0x32, 0x84, 0xf5, 0xf0, 0xe8, 0x59,
            0xa8, 0x33, 0x20, 0xcb, 0x8b,
        ];

        let address = crate::evm::derive_factory_evm_address_from_public_key(&public_key)
            .expect("address should derive");
        let second = crate::evm::derive_factory_evm_address_from_public_key(&public_key)
            .expect("address should derive again");
        let snapshot = snapshot_state();
        assert_eq!(
            snapshot.factory_evm_address.as_deref(),
            Some(address.as_str())
        );
        assert_eq!(second, address);
        assert_eq!(address.len(), 42);
        assert!(address.starts_with("0x"));
    }
}
