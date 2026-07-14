#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeMap;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const HOST_FACTORY_CONTROLLER_PRINCIPAL: &str = "rrkah-fqaaa-aaaaa-aaaaq-cai";

use crate::base_rpc::configured_rpc_endpoints;
#[cfg(target_arch = "wasm32")]
use crate::controllers::complete_controller_handoff_live;
#[cfg(target_arch = "wasm32")]
use crate::controllers::rejection_message;
#[cfg(not(target_arch = "wasm32"))]
use crate::controllers::verify_factory_only_controller_ids;
use crate::cycles::ensure_spawn_creation_cycles;
#[cfg(target_arch = "wasm32")]
use crate::evm::derive_child_evm_address;
#[cfg(not(target_arch = "wasm32"))]
use crate::evm::derive_child_evm_address_for_key_name;
#[cfg(not(target_arch = "wasm32"))]
use crate::expiry::expire_spawn_session;
#[cfg(target_arch = "wasm32")]
use crate::init::{build_automaton_install_args, canonical_deployment_chain_id};
use crate::init::{
    build_strategy_install_recipe, initialize_automaton, validate_automaton_child_runtime_config,
};
use crate::retry::mark_session_failed_in_state;
use crate::session_transitions::{apply_session_event_in_state, SpawnSessionEvent};
#[cfg(target_arch = "wasm32")]
use crate::state::load_spawn_provider_secrets;
use crate::state::{
    clear_provider_secrets, delete_spawn_provider_secrets, read_state, write_state, FactoryState,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::types::{amount_to_string, parse_amount};
use crate::types::{
    AutomatonBootstrapEvidence, AutomatonBootstrapVerification, AutomatonRuntimeState,
    FactoryError, PaymentStatus, ReleaseBroadcastRecord, RepositoryStrategySessionSnapshot,
    SessionAuditActor, SpawnExecutionReceipt, SpawnSession, SpawnSessionState,
    SpawnedAutomatonRecord, CONTROLLER_FIELD,
};
use sha2::{Digest, Sha256};

fn constitution_hash(constitution: &str) -> String {
    format!("{:x}", Sha256::digest(constitution.as_bytes()))
}

fn redact_released_constitution(
    session: &mut SpawnSession,
    runtime: &mut AutomatonRuntimeState,
) -> (String, String) {
    let (name, constitution) = session.resolved_genesis();
    let hash = constitution_hash(&constitution);
    session.constitution = None;

    if let Some(verification) = runtime.bootstrap_verification.as_mut() {
        verification.evidence.bootstrap_constitution = None;
        verification.evidence.bootstrap_constitution_hash = Some(hash.clone());
    }

    (name, hash)
}

#[cfg(target_arch = "wasm32")]
use crate::now_ms as current_time_ms;
#[cfg(target_arch = "wasm32")]
use ic_cdk::call::Call;
#[cfg(target_arch = "wasm32")]
use ic_cdk::management_canister::{
    create_canister_with_extra_cycles, delete_canister, install_code, CanisterInstallMode,
    CanisterSettings, CreateCanisterArgs, DeleteCanisterArgs, InstallCodeArgs,
};

fn normalize_bootstrap_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

fn same_evm_address(expected: &str, observed: Option<&str>) -> bool {
    observed.is_some_and(|value| expected.eq_ignore_ascii_case(value))
}

fn build_bootstrap_verification(
    session: &SpawnSession,
    expected_factory_principal: &candid::Principal,
    expected_chain_id: u64,
    expected_version_commit: &str,
    expected_evm_address: &str,
    mut evidence: AutomatonBootstrapEvidence,
    checked_at: u64,
) -> AutomatonBootstrapVerification {
    let expected_strategies = normalize_bootstrap_list(&session.config.strategies);
    let expected_skills = normalize_bootstrap_list(&session.config.skills);
    let (expected_name, expected_constitution) = session.resolved_genesis();
    evidence.bootstrap_constitution_hash = Some(constitution_hash(expected_constitution.as_str()));
    let mut failures = Vec::new();

    if evidence.bootstrap_contract_version != Some(crate::types::SPAWN_CONTRACT_VERSION) {
        failures.push("spawn contract version mismatch".to_string());
    }
    if evidence.bootstrap_name.as_deref() != Some(expected_name.as_str()) {
        failures.push("genesis name mismatch".to_string());
    }
    if evidence.bootstrap_constitution.as_deref() != Some(expected_constitution.as_str()) {
        failures.push("genesis constitution mismatch".to_string());
    }

    if evidence.bootstrap_session_id.as_deref() != Some(session.session_id.as_str()) {
        failures.push(format!(
            "session_id mismatch: expected={}, observed={}",
            session.session_id,
            evidence
                .bootstrap_session_id
                .as_deref()
                .unwrap_or("<missing>")
        ));
    }
    if evidence.bootstrap_parent_id != session.parent_id {
        failures.push(format!(
            "parent_id mismatch: expected={}, observed={}",
            session.parent_id.as_deref().unwrap_or("<none>"),
            evidence.bootstrap_parent_id.as_deref().unwrap_or("<none>")
        ));
    }
    if evidence.bootstrap_factory_principal.as_ref() != Some(expected_factory_principal) {
        failures.push(format!(
            "factory_principal mismatch: expected={}, observed={}",
            expected_factory_principal.to_text(),
            evidence
                .bootstrap_factory_principal
                .as_ref()
                .map(|value| value.to_text())
                .unwrap_or_else(|| "<missing>".to_string())
        ));
    }
    if evidence.bootstrap_risk != Some(session.config.risk) {
        failures.push(format!(
            "risk mismatch: expected={}, observed={}",
            session.config.risk,
            evidence
                .bootstrap_risk
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<missing>".to_string())
        ));
    }
    if evidence.bootstrap_strategies != expected_strategies {
        failures.push(format!(
            "strategies mismatch: expected={expected_strategies:?}, observed={:?}",
            evidence.bootstrap_strategies
        ));
    }
    if evidence.bootstrap_skills != expected_skills {
        failures.push(format!(
            "skills mismatch: expected={expected_skills:?}, observed={:?}",
            evidence.bootstrap_skills
        ));
    }
    if evidence.bootstrap_version_commit.as_deref() != Some(expected_version_commit) {
        failures.push(format!(
            "version_commit mismatch: expected={expected_version_commit}, observed={}",
            evidence
                .bootstrap_version_commit
                .as_deref()
                .unwrap_or("<missing>")
        ));
    }
    if !same_evm_address(
        &session.steward_address,
        evidence.steward_address.as_deref(),
    ) {
        failures.push(format!(
            "steward_address mismatch: expected={}, observed={}",
            session.steward_address,
            evidence.steward_address.as_deref().unwrap_or("<missing>")
        ));
    }
    if evidence.steward_chain_id != Some(expected_chain_id) {
        failures.push(format!(
            "steward_chain_id mismatch: expected={}, observed={}",
            expected_chain_id,
            evidence
                .steward_chain_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<missing>".to_string())
        ));
    }
    if evidence.steward_enabled != Some(true) {
        failures.push(format!(
            "steward_enabled mismatch: expected=true, observed={}",
            evidence
                .steward_enabled
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<missing>".to_string())
        ));
    }
    if !same_evm_address(expected_evm_address, evidence.evm_address.as_deref()) {
        failures.push(format!(
            "evm_address mismatch: expected={expected_evm_address}, observed={}",
            evidence.evm_address.as_deref().unwrap_or("<missing>")
        ));
    }

    AutomatonBootstrapVerification {
        checked_at,
        passed: failures.is_empty(),
        evidence,
        failures,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn verify_spawned_automaton_bootstrap_sync(
    session: &SpawnSession,
    runtime: &AutomatonRuntimeState,
    expected_factory_principal: &candid::Principal,
    expected_chain_id: u64,
    expected_version_commit: &str,
    checked_at: u64,
) -> AutomatonBootstrapVerification {
    // The sync path runs in unit tests without a real child canister, so it mirrors the
    // installed child views using the runtime/session data that would back those queries.
    build_bootstrap_verification(
        session,
        expected_factory_principal,
        expected_chain_id,
        expected_version_commit,
        &runtime.evm_address,
        AutomatonBootstrapEvidence {
            bootstrap_contract_version: Some(crate::types::SPAWN_CONTRACT_VERSION),
            bootstrap_name: Some(session.resolved_genesis().0),
            bootstrap_constitution_hash: None,
            bootstrap_constitution: Some(session.resolved_genesis().1),
            bootstrap_session_id: Some(runtime.session_id.clone()),
            bootstrap_parent_id: session.parent_id.clone(),
            bootstrap_factory_principal: Some(*expected_factory_principal),
            bootstrap_risk: Some(runtime.risk),
            bootstrap_strategies: normalize_bootstrap_list(&runtime.strategies),
            bootstrap_skills: normalize_bootstrap_list(&runtime.skills),
            bootstrap_version_commit: Some(expected_version_commit.to_string()),
            steward_address: Some(runtime.steward_address.clone()),
            steward_chain_id: Some(expected_chain_id),
            steward_enabled: Some(true),
            evm_address: Some(runtime.evm_address.clone()),
        },
        checked_at,
    )
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, candid::CandidType, serde::Deserialize)]
struct ChildSpawnBootstrapView {
    contract_version: Option<u16>,
    name: Option<String>,
    constitution: Option<String>,
    session_id: Option<String>,
    parent_id: Option<String>,
    factory_principal: Option<candid::Principal>,
    risk: Option<u8>,
    strategies: Vec<String>,
    skills: Vec<String>,
    version_commit: Option<String>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, candid::CandidType, serde::Deserialize)]
struct ChildStewardState {
    chain_id: u64,
    address: String,
    enabled: bool,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, candid::CandidType, serde::Deserialize)]
struct ChildStewardStatusView {
    active_steward: Option<ChildStewardState>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, candid::CandidType, serde::Deserialize)]
enum ChildDerivedAddressResult {
    Ok(String),
    Err(String),
}

#[derive(Clone, Debug, Eq, PartialEq, candid::CandidType, serde::Deserialize)]
struct ChildStrategyTemplateKey {
    protocol: String,
    primitive: String,
    chain_id: u64,
    template_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, candid::CandidType, serde::Deserialize)]
enum ChildStrategyTemplateStatus {
    Draft,
    Active,
    Deprecated,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq, candid::CandidType, serde::Deserialize)]
struct ChildStrategyTemplate {
    key: ChildStrategyTemplateKey,
    status: ChildStrategyTemplateStatus,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, candid::CandidType, serde::Deserialize)]
enum ChildRegisterStrategyAdminResult {
    Ok(ChildStrategyTemplate),
    Err(String),
}

fn child_strategy_key(snapshot: &RepositoryStrategySessionSnapshot) -> ChildStrategyTemplateKey {
    ChildStrategyTemplateKey {
        protocol: snapshot.protocol.clone(),
        primitive: snapshot.primitive.clone(),
        chain_id: snapshot
            .resolved_chain_id
            .unwrap_or(snapshot.canonical_chain_id),
        template_id: snapshot.strategy_id.clone(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn child_strategy_template_lookup_key(key: &ChildStrategyTemplateKey) -> String {
    format!(
        "{}:{}:{}:{}",
        key.protocol, key.primitive, key.chain_id, key.template_id
    )
}

fn strategy_install_error(
    method: &str,
    snapshot: &RepositoryStrategySessionSnapshot,
    message: impl Into<String>,
) -> FactoryError {
    FactoryError::ManagementCallFailed {
        method: method.to_string(),
        message: format!("strategy {}: {}", snapshot.strategy_id, message.into()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn child_strategy_template_from_recipe(
    snapshot: &RepositoryStrategySessionSnapshot,
    recipe_json: &str,
) -> Result<ChildStrategyTemplate, FactoryError> {
    let recipe: serde_json::Value = serde_json::from_str(recipe_json).map_err(|error| {
        strategy_install_error(
            "register_strategy_admin",
            snapshot,
            format!("invalid adapted recipe JSON: {error}"),
        )
    })?;
    let recipe_object = recipe.as_object().ok_or_else(|| {
        strategy_install_error(
            "register_strategy_admin",
            snapshot,
            "adapted recipe must decode to a JSON object",
        )
    })?;
    let protocol = recipe_object
        .get("protocol")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            strategy_install_error(
                "register_strategy_admin",
                snapshot,
                "adapted recipe field protocol must be a string",
            )
        })?;
    let primitive = recipe_object
        .get("primitive")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            strategy_install_error(
                "register_strategy_admin",
                snapshot,
                "adapted recipe field primitive must be a string",
            )
        })?;
    let chain_id = recipe_object
        .get("chain_id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            strategy_install_error(
                "register_strategy_admin",
                snapshot,
                "adapted recipe field chain_id must be a u64",
            )
        })?;
    let template_id = recipe_object
        .get("template_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            strategy_install_error(
                "register_strategy_admin",
                snapshot,
                "adapted recipe field template_id must be a string",
            )
        })?;

    Ok(ChildStrategyTemplate {
        key: ChildStrategyTemplateKey {
            protocol: protocol.to_string(),
            primitive: primitive.to_string(),
            chain_id,
            template_id: template_id.to_string(),
        },
        status: ChildStrategyTemplateStatus::Active,
    })
}

fn verify_child_strategy_template(
    method: &str,
    snapshot: &RepositoryStrategySessionSnapshot,
    template: Option<&ChildStrategyTemplate>,
) -> Result<(), FactoryError> {
    let template = template.ok_or_else(|| {
        strategy_install_error(
            method,
            snapshot,
            "template is missing from the child after registration",
        )
    })?;
    let expected_key = child_strategy_key(snapshot);

    if template.key != expected_key {
        return Err(strategy_install_error(
            method,
            snapshot,
            format!(
                "template key mismatch after registration: expected {}:{}:{}:{}, got {}:{}:{}:{}",
                expected_key.protocol,
                expected_key.primitive,
                expected_key.chain_id,
                expected_key.template_id,
                template.key.protocol,
                template.key.primitive,
                template.key.chain_id,
                template.key.template_id,
            ),
        ));
    }

    if !matches!(template.status, ChildStrategyTemplateStatus::Active) {
        return Err(strategy_install_error(
            method,
            snapshot,
            "template is not active after registration",
        ));
    }

    Ok(())
}

fn prepare_retryable_runtime_after_strategy_failure(
    runtime: &AutomatonRuntimeState,
) -> AutomatonRuntimeState {
    let mut retryable_runtime = runtime.clone();
    retryable_runtime.install_succeeded_at = None;
    retryable_runtime.last_funded_at = None;
    retryable_runtime.funded_amount = "0".to_string();
    retryable_runtime.controller_handoff_completed_at = None;
    retryable_runtime
}

#[cfg(not(target_arch = "wasm32"))]
fn install_snapped_strategies_sync(session: &SpawnSession) -> Result<(), FactoryError> {
    let mut installed_templates = BTreeMap::new();

    for snapshot in &session.selected_strategies {
        let adapted_recipe_json = build_strategy_install_recipe(snapshot)?;
        let installed_template =
            child_strategy_template_from_recipe(snapshot, &adapted_recipe_json)?;
        installed_templates.insert(
            child_strategy_template_lookup_key(&installed_template.key),
            installed_template,
        );
    }

    for snapshot in &session.selected_strategies {
        let expected_key = child_strategy_key(snapshot);
        let installed = installed_templates.get(&child_strategy_template_lookup_key(&expected_key));
        verify_child_strategy_template("list_strategy_templates", snapshot, installed)?;
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn install_snapped_strategies_into_child(
    canister_id: &str,
    strategies: &[RepositoryStrategySessionSnapshot],
) -> Result<(), FactoryError> {
    async fn call_child_canister_tuple_with_arg<A, R>(
        principal: &candid::Principal,
        method: &'static str,
        arg: &A,
    ) -> Result<R, FactoryError>
    where
        A: candid::utils::ArgumentEncoder,
        R: for<'de> candid::utils::ArgumentDecoder<'de>,
    {
        let response = Call::bounded_wait(*principal, method)
            .with_args(arg)
            .await
            .map_err(|error| FactoryError::ManagementCallFailed {
                method: method.to_string(),
                message: rejection_message(error),
            })?;
        response
            .candid_tuple()
            .map_err(|error| FactoryError::ManagementCallFailed {
                method: method.to_string(),
                message: rejection_message(error),
            })
    }

    use candid::Principal;

    let principal =
        Principal::from_text(canister_id).map_err(|error| FactoryError::ManagementCallFailed {
            method: "parse_canister_id".to_string(),
            message: error.to_string(),
        })?;

    for snapshot in strategies {
        let adapted_recipe_json = build_strategy_install_recipe(snapshot)?;
        let registration_args = (adapted_recipe_json.clone(),);
        let (registration_result,): (ChildRegisterStrategyAdminResult,) =
            call_child_canister_tuple_with_arg(
                &principal,
                "register_strategy_admin",
                &registration_args,
            )
            .await?;
        let registered_template = match registration_result {
            ChildRegisterStrategyAdminResult::Ok(template) => template,
            ChildRegisterStrategyAdminResult::Err(message) => {
                return Err(strategy_install_error(
                    "register_strategy_admin",
                    snapshot,
                    message,
                ));
            }
        };
        verify_child_strategy_template(
            "register_strategy_admin",
            snapshot,
            Some(&registered_template),
        )?;

        let expected_key = child_strategy_key(snapshot);
        let list_args = (Some(expected_key.clone()), 1_u32);
        let (listed_templates,): (Vec<ChildStrategyTemplate>,) =
            call_child_canister_tuple_with_arg(&principal, "list_strategy_templates", &list_args)
                .await?;
        verify_child_strategy_template(
            "list_strategy_templates",
            snapshot,
            listed_templates.first(),
        )?;
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn load_spawned_automaton_bootstrap_evidence(
    canister_id: &str,
) -> Result<AutomatonBootstrapEvidence, FactoryError> {
    async fn call_child_canister_tuple<R>(
        principal: &candid::Principal,
        method: &'static str,
    ) -> Result<R, FactoryError>
    where
        R: for<'de> candid::utils::ArgumentDecoder<'de>,
    {
        let response = Call::bounded_wait(*principal, method)
            .await
            .map_err(|error| FactoryError::ManagementCallFailed {
                method: method.to_string(),
                message: rejection_message(error),
            })?;
        response
            .candid_tuple()
            .map_err(|error| FactoryError::ManagementCallFailed {
                method: method.to_string(),
                message: rejection_message(error),
            })
    }

    use candid::Principal;

    let principal =
        Principal::from_text(canister_id).map_err(|error| FactoryError::ManagementCallFailed {
            method: "parse_canister_id".to_string(),
            message: error.to_string(),
        })?;
    let (bootstrap_view,): (ChildSpawnBootstrapView,) =
        call_child_canister_tuple(&principal, "get_spawn_bootstrap_view").await?;
    let (steward_status,): (ChildStewardStatusView,) =
        call_child_canister_tuple(&principal, "get_steward_status").await?;
    let (mut evm_address,): (Option<String>,) =
        call_child_canister_tuple(&principal, "get_automaton_evm_address").await?;
    if evm_address.is_none() {
        let (derived_address_result,): (ChildDerivedAddressResult,) =
            call_child_canister_tuple(&principal, "derive_automaton_evm_address").await?;
        let derived_address = match derived_address_result {
            ChildDerivedAddressResult::Ok(address) => address,
            ChildDerivedAddressResult::Err(message) => {
                return Err(FactoryError::ManagementCallFailed {
                    method: "derive_automaton_evm_address".to_string(),
                    message,
                });
            }
        };
        evm_address = Some(derived_address);
    }

    Ok(AutomatonBootstrapEvidence {
        bootstrap_contract_version: bootstrap_view.contract_version,
        bootstrap_name: bootstrap_view.name,
        bootstrap_constitution_hash: None,
        bootstrap_constitution: bootstrap_view.constitution,
        bootstrap_session_id: bootstrap_view.session_id,
        bootstrap_parent_id: bootstrap_view.parent_id,
        bootstrap_factory_principal: bootstrap_view.factory_principal,
        bootstrap_risk: bootstrap_view.risk,
        bootstrap_strategies: normalize_bootstrap_list(&bootstrap_view.strategies),
        bootstrap_skills: normalize_bootstrap_list(&bootstrap_view.skills),
        bootstrap_version_commit: bootstrap_view.version_commit,
        steward_address: steward_status
            .active_steward
            .as_ref()
            .map(|steward| steward.address.clone()),
        steward_chain_id: steward_status
            .active_steward
            .as_ref()
            .map(|steward| steward.chain_id),
        steward_enabled: steward_status
            .active_steward
            .as_ref()
            .map(|steward| steward.enabled),
        evm_address,
    })
}

fn bootstrap_verification_error(
    canister_id: &str,
    verification: &AutomatonBootstrapVerification,
) -> FactoryError {
    FactoryError::AutomatonBootstrapVerificationFailed {
        canister_id: canister_id.to_string(),
        failures: verification.failures.clone(),
    }
}

fn persist_failed_spawn_runtime(
    state: &mut FactoryState,
    session_id: &str,
    runtime: &AutomatonRuntimeState,
) {
    state
        .runtimes
        .insert(runtime.canister_id.clone(), runtime.clone());

    if let Some(session) = state.sessions.get_mut(session_id) {
        session.automaton_canister_id = Some(runtime.canister_id.clone());
        session.automaton_evm_address = Some(runtime.evm_address.clone());
    }
}

fn persist_release_broadcast_record(
    state: &mut FactoryState,
    session_id: &str,
    record: &ReleaseBroadcastRecord,
) {
    if let Some(session) = state.sessions.get_mut(session_id) {
        session.release_broadcast = Some(record.clone());
    }
}

fn failure_audit_reason(reason: &str, error: &FactoryError) -> String {
    format!("{reason}: {error}")
}

#[cfg(not(target_arch = "wasm32"))]
fn fail_spawn_session_sync(
    session_id: &str,
    now_ms: u64,
    reason: &str,
    runtime: Option<&AutomatonRuntimeState>,
    error: FactoryError,
) -> Result<SpawnExecutionReceipt, FactoryError> {
    let _ = write_state(|state| {
        if let Some(runtime) = runtime {
            persist_failed_spawn_runtime(state, session_id, runtime);
        }

        mark_session_failed_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            now_ms,
            &failure_audit_reason(reason, &error),
        )
    });

    Err(error)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn execute_spawn(session_id: &str, now_ms: u64) -> Result<SpawnExecutionReceipt, FactoryError> {
    let session_snapshot = read_state(|state| {
        state
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| FactoryError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    })?;

    if session_snapshot.payment_status != PaymentStatus::Paid {
        return Err(FactoryError::PaymentNotSettled {
            session_id: session_id.to_string(),
            status: session_snapshot.payment_status,
        });
    }

    match session_snapshot.state {
        SpawnSessionState::PaymentDetected => {}
        SpawnSessionState::Complete => {
            return Ok(SpawnExecutionReceipt {
                session_id: session_snapshot.session_id.clone(),
                automaton_canister_id: session_snapshot
                    .automaton_canister_id
                    .clone()
                    .expect("completed session has canister id"),
                automaton_evm_address: session_snapshot
                    .automaton_evm_address
                    .clone()
                    .expect("completed session has evm address"),
                funded_amount: session_snapshot.net_forward_amount.clone(),
                controller: format!(
                    "controller:{}",
                    session_snapshot
                        .automaton_canister_id
                        .clone()
                        .expect("completed session has canister id")
                ),
                release_tx_hash: session_snapshot.release_tx_hash.clone(),
                release_broadcast_at: session_snapshot.release_broadcast_at,
                completed_at: session_snapshot.updated_at,
            });
        }
        state => {
            return Err(FactoryError::SessionNotReadyForSpawn {
                session_id: session_id.to_string(),
                state,
            });
        }
    }

    let expires_at = session_snapshot.expires_at;
    if now_ms > expires_at {
        let _ = expire_spawn_session(session_id, now_ms)?;
        return Err(FactoryError::SessionExpired {
            session_id: session_id.to_string(),
            expires_at,
        });
    }

    let (artifact_loaded, cycles_per_spawn) =
        read_state(|state| (state.wasm_bytes.is_some(), state.cycles_per_spawn));
    if !artifact_loaded {
        return fail_spawn_session_sync(
            session_id,
            now_ms,
            "spawn artifact unavailable",
            None,
            FactoryError::ManagementCallFailed {
                method: "install_code".to_string(),
                message: "artifact not loaded".to_string(),
            },
        );
    }
    let child_runtime =
        read_state(|state| validate_automaton_child_runtime_config(&state.child_runtime));
    let child_runtime = if let Err(error) = child_runtime {
        return fail_spawn_session_sync(
            session_id,
            now_ms,
            "child runtime config missing or invalid",
            None,
            error,
        );
    } else {
        child_runtime.expect("validated above")
    };
    if let Err(error @ FactoryError::InsufficientCyclesPool { .. }) =
        ensure_spawn_creation_cycles(u128::from(cycles_per_spawn))
    {
        return fail_spawn_session_sync(
            session_id,
            now_ms,
            "cycles pool below required minimum",
            None,
            error,
        );
    }

    let result = write_state(|state| {
        apply_session_event_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            now_ms,
            SpawnSessionEvent::SpawnStarted,
            "spawn execution started",
        )?;

        state.next_automaton_nonce += 1;

        let version_commit = state.version_commit.clone();
        let canister_id = session_snapshot
            .automaton_canister_id
            .clone()
            .unwrap_or_else(|| format!("automaton-{:04}", state.next_automaton_nonce));
        let evm_address = session_snapshot
            .automaton_evm_address
            .clone()
            .unwrap_or_else(|| {
                derive_child_evm_address_for_key_name(&child_runtime.ecdsa_key_name)
            });
        let mut runtime = initialize_automaton(&session_snapshot, canister_id, evm_address, now_ms);
        runtime.evm_address_derived_at = Some(now_ms);

        {
            let session = state.sessions.get_mut(session_id).expect("session exists");
            session.automaton_canister_id = Some(runtime.canister_id.clone());
            session.automaton_evm_address = Some(runtime.evm_address.clone());
        }
        apply_session_event_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            now_ms,
            SpawnSessionEvent::InstallSucceeded,
            "automaton initialized",
        )?;

        let net_forward_amount = {
            let session = state.sessions.get(session_id).expect("session exists");
            match parse_amount(&session.net_forward_amount) {
                Ok(value) => value,
                Err(error) => {
                    let _ = mark_session_failed_in_state(
                        state,
                        session_id,
                        SessionAuditActor::System,
                        now_ms,
                        "spawn funding preparation failed",
                    )?;
                    return Err(error);
                }
            }
        };
        runtime.funded_amount = amount_to_string(net_forward_amount);
        runtime.last_funded_at = Some(now_ms);
        runtime.install_succeeded_at = Some(now_ms);
        if let Err(error) = install_snapped_strategies_sync(&session_snapshot) {
            let retryable_runtime = prepare_retryable_runtime_after_strategy_failure(&runtime);
            state.runtimes.insert(
                retryable_runtime.canister_id.clone(),
                retryable_runtime.clone(),
            );
            let _ = mark_session_failed_in_state(
                state,
                session_id,
                SessionAuditActor::System,
                now_ms,
                &failure_audit_reason("strategy registration failed", &error),
            )?;
            return Err(error);
        }
        runtime.bootstrap_verification = Some(verify_spawned_automaton_bootstrap_sync(
            &session_snapshot,
            &runtime,
            &candid::Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                .expect("test factory principal should parse"),
            child_runtime.evm_chain_id,
            &version_commit,
            now_ms,
        ));
        if let Some(verification) = runtime.bootstrap_verification.as_ref() {
            if !verification.passed {
                state
                    .runtimes
                    .insert(runtime.canister_id.clone(), runtime.clone());
                let _ = mark_session_failed_in_state(
                    state,
                    session_id,
                    SessionAuditActor::System,
                    now_ms,
                    "spawned canister bootstrap verification failed",
                )?;
                return Err(bootstrap_verification_error(
                    &runtime.canister_id,
                    verification,
                ));
            }
        }

        let controller = format!("{CONTROLLER_FIELD}:{HOST_FACTORY_CONTROLLER_PRINCIPAL}");
        runtime.controller_handoff_completed_at = Some(now_ms);

        let base_rpc_endpoints = configured_rpc_endpoints(
            state.base_rpc_endpoint.clone(),
            state.base_rpc_fallback_endpoint.clone(),
        );
        if base_rpc_endpoints.is_empty() {
            persist_failed_spawn_runtime(state, session_id, &runtime);
            let error = FactoryError::ManagementCallFailed {
                method: "http_request".to_string(),
                message: "base RPC endpoint is not configured".to_string(),
            };
            let _ = mark_session_failed_in_state(
                state,
                session_id,
                SessionAuditActor::System,
                now_ms,
                "release broadcast prerequisites missing",
            )?;
            return Err(error);
        }
        let escrow_contract_address = state.escrow_contract_address.clone();
        let claim_id = state
            .sessions
            .get(session_id)
            .expect("session exists")
            .claim_id
            .clone();
        let release = match crate::evm::broadcast_release_transaction(
            &claim_id,
            &runtime.evm_address,
            &base_rpc_endpoints,
            &escrow_contract_address,
            state.next_automaton_nonce,
            now_ms,
            &state.release_broadcast_config,
        ) {
            Ok(release) => release,
            Err(error) => {
                persist_failed_spawn_runtime(state, session_id, &runtime);
                persist_release_broadcast_record(state, session_id, &error.record);
                let _ = mark_session_failed_in_state(
                    state,
                    session_id,
                    SessionAuditActor::System,
                    now_ms,
                    "release broadcast failed",
                )?;
                return Err(*error.source);
            }
        };
        let crate::evm::ReleaseBroadcastReceipt {
            release_tx_hash,
            release_broadcast_at,
            record: release_record,
        } = release;

        let registry_record = {
            let session = state.sessions.get_mut(session_id).expect("session exists");
            clear_provider_secrets(session, Some(&mut runtime));
            session.release_tx_hash = Some(release_tx_hash.clone());
            session.release_broadcast_at = Some(release_broadcast_at);
            session.release_broadcast = Some(release_record.clone());
            let (name, constitution_hash) = redact_released_constitution(session, &mut runtime);
            let verified_factory_controllers = verify_factory_only_controller_ids(
                &runtime.canister_id,
                HOST_FACTORY_CONTROLLER_PRINCIPAL,
                vec![HOST_FACTORY_CONTROLLER_PRINCIPAL.to_string()],
            )?
            .into_vec();

            SpawnedAutomatonRecord {
                name: Some(name),
                constitution_hash: Some(constitution_hash),
                canister_id: runtime.canister_id.clone(),
                steward_address: session.steward_address.clone(),
                evm_address: runtime.evm_address.clone(),
                chain: session.chain.clone(),
                session_id: session.session_id.clone(),
                parent_id: session.parent_id.clone(),
                child_ids: session.child_ids.clone(),
                created_at: now_ms,
                version_commit,
                controllers: Some(verified_factory_controllers),
                control_status: Some("upgradeable_by_factory".to_string()),
                control_verified_at: Some(now_ms),
            }
        };

        if let Some(parent_id) = registry_record.parent_id.as_ref() {
            if let Some(parent) = state.registry.get_mut(parent_id) {
                if parent
                    .child_ids
                    .iter()
                    .all(|child_id| child_id != &registry_record.canister_id)
                {
                    parent.child_ids.push(registry_record.canister_id.clone());
                }
            }
        }

        state
            .runtimes
            .insert(runtime.canister_id.clone(), runtime.clone());
        state
            .registry
            .insert(registry_record.canister_id.clone(), registry_record);
        apply_session_event_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            now_ms,
            SpawnSessionEvent::ReleaseBroadcast,
            "spawn completed after child bootstrap verification, release broadcast, and controller handoff finalized",
        )?;

        Ok(SpawnExecutionReceipt {
            session_id: session_id.to_string(),
            automaton_canister_id: runtime.canister_id,
            automaton_evm_address: runtime.evm_address,
            funded_amount: amount_to_string(net_forward_amount),
            controller,
            release_tx_hash: Some(release_tx_hash),
            release_broadcast_at: Some(release_broadcast_at),
            completed_at: now_ms,
        })
    });
    delete_spawn_provider_secrets(session_id);
    result
}

#[cfg(target_arch = "wasm32")]
async fn cleanup_orphaned_canister(canister_id: &str) {
    use candid::Principal;

    let principal = match Principal::from_text(canister_id) {
        Ok(principal) => principal,
        Err(_) => return,
    };

    let _ = delete_canister(&DeleteCanisterArgs {
        canister_id: principal,
    })
    .await;
}

#[cfg(target_arch = "wasm32")]
async fn fail_spawn_session(
    session_id: &str,
    now_ms: u64,
    reason: &str,
    cleanup_canister_id: Option<&str>,
    runtime: Option<&AutomatonRuntimeState>,
    error: FactoryError,
) -> Result<SpawnExecutionReceipt, FactoryError> {
    if let Some(canister_id) = cleanup_canister_id {
        cleanup_orphaned_canister(canister_id).await;
    }

    let _ = write_state(|state| {
        if let Some(runtime) = runtime {
            persist_failed_spawn_runtime(state, session_id, runtime);
        }

        mark_session_failed_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            now_ms,
            &failure_audit_reason(reason, &error),
        )
    });

    Err(error)
}

#[cfg(target_arch = "wasm32")]
pub async fn execute_spawn(
    session_id: &str,
    started_at_ms: u64,
) -> Result<SpawnExecutionReceipt, FactoryError> {
    use candid::Principal;

    let session_snapshot = read_state(|state| {
        state
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| FactoryError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    })?;

    if session_snapshot.payment_status != PaymentStatus::Paid {
        return Err(FactoryError::PaymentNotSettled {
            session_id: session_id.to_string(),
            status: session_snapshot.payment_status,
        });
    }

    match session_snapshot.state {
        SpawnSessionState::PaymentDetected => {}
        SpawnSessionState::Complete => {
            return Ok(SpawnExecutionReceipt {
                session_id: session_snapshot.session_id.clone(),
                automaton_canister_id: session_snapshot
                    .automaton_canister_id
                    .clone()
                    .expect("completed session has canister id"),
                automaton_evm_address: session_snapshot
                    .automaton_evm_address
                    .clone()
                    .expect("completed session has evm address"),
                funded_amount: session_snapshot.net_forward_amount.clone(),
                controller: format!(
                    "controller:{}",
                    session_snapshot
                        .automaton_canister_id
                        .clone()
                        .expect("completed session has canister id")
                ),
                release_tx_hash: session_snapshot.release_tx_hash.clone(),
                release_broadcast_at: session_snapshot.release_broadcast_at,
                completed_at: session_snapshot.updated_at,
            });
        }
        state => {
            return Err(FactoryError::SessionNotReadyForSpawn {
                session_id: session_id.to_string(),
                state,
            });
        }
    }

    let expires_at = session_snapshot.expires_at;
    let (wasm_module_opt, version_commit, create_cycles) = read_state(|state| {
        (
            state.wasm_bytes.clone(),
            state.version_commit.clone(),
            state
                .cycles_per_spawn
                .max(ic_cdk::api::cost_create_canister() as u64) as u128,
        )
    });
    let wasm_module = match wasm_module_opt {
        Some(wasm_module) => wasm_module,
        None => {
            return fail_spawn_session(
                session_id,
                current_time_ms(),
                "spawn artifact unavailable",
                None,
                None,
                FactoryError::ManagementCallFailed {
                    method: "install_code".to_string(),
                    message: "artifact not loaded".to_string(),
                },
            )
            .await;
        }
    };
    if let Err(error @ FactoryError::InsufficientCyclesPool { .. }) =
        ensure_spawn_creation_cycles(create_cycles)
    {
        return fail_spawn_session(
            session_id,
            current_time_ms(),
            "cycles pool below required minimum",
            None,
            None,
            error,
        )
        .await;
    }
    let child_runtime =
        match read_state(|state| validate_automaton_child_runtime_config(&state.child_runtime)) {
            Ok(config) => config,
            Err(error) => {
                return fail_spawn_session(
                    session_id,
                    current_time_ms(),
                    "child runtime config missing or invalid",
                    None,
                    None,
                    error,
                )
                .await;
            }
        };

    write_state(|state| {
        apply_session_event_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            started_at_ms,
            SpawnSessionEvent::SpawnStarted,
            "spawn execution started",
        )
    })?;

    let mut canister_id = session_snapshot.automaton_canister_id.clone();
    let mut runtime = read_state(|state| {
        canister_id
            .as_ref()
            .and_then(|id| state.runtimes.get(id).cloned())
    });

    let needs_install = runtime
        .as_ref()
        .map(|existing| existing.install_succeeded_at.is_none())
        .unwrap_or(true);

    if needs_install {
        let create_extra_cycles = create_cycles.saturating_sub(ic_cdk::api::cost_create_canister());
        let record = match create_canister_with_extra_cycles(
            &CreateCanisterArgs {
                settings: Some(CanisterSettings {
                    controllers: None,
                    ..Default::default()
                }),
            },
            create_extra_cycles,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                return fail_spawn_session(
                    session_id,
                    current_time_ms(),
                    "create_canister failed",
                    None,
                    None,
                    FactoryError::ManagementCallFailed {
                        method: "create_canister".to_string(),
                        message: rejection_message(error),
                    },
                )
                .await;
            }
        };

        let created_canister_id = record.canister_id.to_text();
        canister_id = Some(created_canister_id.clone());
        let expected_evm_address =
            match derive_child_evm_address(&created_canister_id, &child_runtime.ecdsa_key_name)
                .await
            {
                Ok(address) => address,
                Err(error) => {
                    cleanup_orphaned_canister(&created_canister_id).await;
                    return fail_spawn_session(
                        session_id,
                        current_time_ms(),
                        "derive_automaton_evm_address failed",
                        Some(&created_canister_id),
                        None,
                        error,
                    )
                    .await;
                }
            };
        runtime = Some(initialize_automaton(
            &session_snapshot,
            created_canister_id.clone(),
            expected_evm_address,
            started_at_ms,
        ));
        runtime
            .as_mut()
            .expect("runtime should exist after create")
            .evm_address_derived_at = Some(started_at_ms);

        if let Some(runtime) = runtime.as_ref() {
            let runtime_clone = runtime.clone();
            write_state(|state| {
                let session = state.sessions.get_mut(session_id).expect("session exists");
                session.automaton_canister_id = Some(created_canister_id.clone());
                session.automaton_evm_address = Some(runtime_clone.evm_address.clone());
                state
                    .runtimes
                    .insert(created_canister_id.clone(), runtime_clone);
            });
        }

        let current_time = current_time_ms();
        if current_time > expires_at {
            cleanup_orphaned_canister(&created_canister_id).await;
            return fail_spawn_session(
                session_id,
                current_time,
                "session expired during spawn",
                Some(&created_canister_id),
                None,
                FactoryError::SessionExpired {
                    session_id: session_id.to_string(),
                    expires_at,
                },
            )
            .await;
        }

        let install_args = build_automaton_install_args(
            &session_snapshot,
            load_spawn_provider_secrets(session_id).as_ref(),
            ic_cdk::api::canister_self(),
            &version_commit,
            &child_runtime,
        );
        let canister_principal = Principal::from_text(&created_canister_id).map_err(|error| {
            FactoryError::ManagementCallFailed {
                method: "parse_canister_id".to_string(),
                message: error.to_string(),
            }
        })?;

        if let Err(error) = install_code(&InstallCodeArgs {
            mode: CanisterInstallMode::Install,
            canister_id: canister_principal,
            wasm_module,
            arg: install_args,
        })
        .await
        {
            cleanup_orphaned_canister(&created_canister_id).await;
            return fail_spawn_session(
                session_id,
                current_time_ms(),
                "install_code failed",
                Some(&created_canister_id),
                None,
                FactoryError::ManagementCallFailed {
                    method: "install_code".to_string(),
                    message: rejection_message(error),
                },
            )
            .await;
        }

        delete_spawn_provider_secrets(session_id);

        let current_time = current_time_ms();
        if current_time > expires_at {
            cleanup_orphaned_canister(&created_canister_id).await;
            return fail_spawn_session(
                session_id,
                current_time,
                "session expired during spawn",
                Some(&created_canister_id),
                None,
                FactoryError::SessionExpired {
                    session_id: session_id.to_string(),
                    expires_at,
                },
            )
            .await;
        }

        runtime
            .as_mut()
            .expect("runtime should exist after create")
            .install_succeeded_at = Some(current_time);
        runtime
            .as_mut()
            .expect("runtime should exist after create")
            .funded_amount = session_snapshot.net_forward_amount.clone();
        runtime
            .as_mut()
            .expect("runtime should exist after create")
            .last_funded_at = Some(current_time);

        write_state(|state| {
            state.runtimes.insert(
                created_canister_id.clone(),
                runtime
                    .as_ref()
                    .expect("runtime should exist after install")
                    .clone(),
            );
            {
                let session = state.sessions.get_mut(session_id).expect("session exists");
                session.automaton_canister_id = Some(created_canister_id.clone());
                session.automaton_evm_address =
                    runtime.as_ref().map(|entry| entry.evm_address.clone());
            }
            apply_session_event_in_state(
                state,
                session_id,
                SessionAuditActor::System,
                current_time,
                SpawnSessionEvent::InstallSucceeded,
                "automaton installed",
            )
        })?;
    } else {
        let current_time = current_time_ms();
        write_state(|state| {
            {
                let session = state.sessions.get_mut(session_id).expect("session exists");
                session.automaton_canister_id = canister_id.clone();
                session.automaton_evm_address =
                    runtime.as_ref().map(|entry| entry.evm_address.clone());
            }
            apply_session_event_in_state(
                state,
                session_id,
                SessionAuditActor::System,
                current_time,
                SpawnSessionEvent::InstallSucceeded,
                "automaton already installed; resuming handoff",
            )
        })?;
    }

    let canister_id = canister_id.expect("spawn path should have a canister id");
    let expected_evm_address =
        match derive_child_evm_address(&canister_id, &child_runtime.ecdsa_key_name).await {
            Ok(address) => address,
            Err(error) => {
                return fail_spawn_session(
                    session_id,
                    current_time_ms(),
                    "derive_automaton_evm_address failed",
                    Some(&canister_id),
                    runtime.as_ref(),
                    error,
                )
                .await;
            }
        };
    let mut runtime = runtime.unwrap_or_else(|| {
        initialize_automaton(
            &session_snapshot,
            canister_id.clone(),
            expected_evm_address.clone(),
            started_at_ms,
        )
    });
    runtime.evm_address = expected_evm_address.clone();
    runtime.evm_address_derived_at.get_or_insert(started_at_ms);
    let expected_child_chain_id = read_state(|state| {
        canonical_deployment_chain_id(&state.child_runtime, &state.release_broadcast_config)
    })?;
    let verification_evidence = match load_spawned_automaton_bootstrap_evidence(&canister_id).await
    {
        Ok(evidence) => evidence,
        Err(error) => {
            return fail_spawn_session(
                session_id,
                current_time_ms(),
                "spawned canister bootstrap verification failed",
                None,
                Some(&runtime),
                error,
            )
            .await;
        }
    };
    let verification = build_bootstrap_verification(
        &session_snapshot,
        &ic_cdk::api::id(),
        expected_child_chain_id,
        &version_commit,
        &expected_evm_address,
        verification_evidence,
        current_time_ms(),
    );
    runtime.bootstrap_verification = Some(verification.clone());
    write_state(|state| persist_failed_spawn_runtime(state, session_id, &runtime));
    if !verification.passed {
        return fail_spawn_session(
            session_id,
            verification.checked_at,
            "spawned canister bootstrap verification failed",
            None,
            Some(&runtime),
            bootstrap_verification_error(&canister_id, &verification),
        )
        .await;
    }

    if let Err(error) =
        install_snapped_strategies_into_child(&canister_id, &session_snapshot.selected_strategies)
            .await
    {
        let retryable_runtime = prepare_retryable_runtime_after_strategy_failure(&runtime);
        return fail_spawn_session(
            session_id,
            current_time_ms(),
            "strategy registration failed",
            Some(&canister_id),
            Some(&retryable_runtime),
            error,
        )
        .await;
    }

    let verified_factory_control = match complete_controller_handoff_live(&canister_id).await {
        Ok(controllers) => controllers,
        Err(error) => {
            return fail_spawn_session(
                session_id,
                current_time_ms(),
                "controller handoff failed",
                Some(&canister_id),
                None,
                error,
            )
            .await
        }
    };
    let current_time = current_time_ms();
    let controller = format!("{CONTROLLER_FIELD}:{}", verified_factory_control.first());
    let verified_factory_controllers = verified_factory_control.into_vec();
    runtime.controller_handoff_completed_at = Some(current_time);
    runtime.install_succeeded_at.get_or_insert(current_time);
    runtime.funded_amount = session_snapshot.net_forward_amount.clone();
    runtime.last_funded_at = Some(current_time);
    runtime.evm_address = expected_evm_address.clone();
    write_state(|state| persist_failed_spawn_runtime(state, session_id, &runtime));

    let (base_rpc_endpoints, release_broadcast_config) = read_state(|state| {
        (
            configured_rpc_endpoints(
                state.base_rpc_endpoint.clone(),
                state.base_rpc_fallback_endpoint.clone(),
            ),
            state.release_broadcast_config.clone(),
        )
    });
    if base_rpc_endpoints.is_empty() {
        return fail_spawn_session(
            session_id,
            current_time,
            "release broadcast prerequisites missing",
            None,
            Some(&runtime),
            FactoryError::ManagementCallFailed {
                method: "http_request".to_string(),
                message: "base RPC endpoint is not configured".to_string(),
            },
        )
        .await;
    }
    let escrow_contract_address = read_state(|state| state.escrow_contract_address.clone());
    let release = match crate::evm::broadcast_release_transaction(
        &session_snapshot.claim_id,
        &runtime.evm_address,
        &base_rpc_endpoints,
        &escrow_contract_address,
        started_at_ms,
        current_time,
        &release_broadcast_config,
    )
    .await
    {
        Ok(release) => release,
        Err(error) => {
            write_state(|state| {
                persist_release_broadcast_record(state, session_id, &error.record);
            });
            return fail_spawn_session(
                session_id,
                current_time_ms(),
                "release broadcast failed",
                None,
                Some(&runtime),
                *error.source,
            )
            .await;
        }
    };
    let crate::evm::ReleaseBroadcastReceipt {
        release_tx_hash,
        release_broadcast_at,
        record: release_record,
    } = release;

    write_state(|state| {
        let session = state.sessions.get_mut(session_id).expect("session exists");
        clear_provider_secrets(session, Some(&mut runtime));
        session.release_tx_hash = Some(release_tx_hash.clone());
        session.release_broadcast_at = Some(release_broadcast_at);
        session.release_broadcast = Some(release_record.clone());
        let (name, constitution_hash) = redact_released_constitution(session, &mut runtime);

        let record = SpawnedAutomatonRecord {
            name: Some(name),
            constitution_hash: Some(constitution_hash),
            canister_id: canister_id.clone(),
            steward_address: session.steward_address.clone(),
            evm_address: runtime.evm_address.clone(),
            chain: session.chain.clone(),
            session_id: session.session_id.clone(),
            parent_id: session.parent_id.clone(),
            child_ids: session.child_ids.clone(),
            created_at: release_broadcast_at,
            version_commit: version_commit.clone(),
            controllers: Some(verified_factory_controllers.clone()),
            control_status: Some("upgradeable_by_factory".to_string()),
            control_verified_at: Some(current_time),
        };

        state.runtimes.insert(canister_id.clone(), runtime.clone());
        state.registry.insert(canister_id.clone(), record);
        apply_session_event_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            release_broadcast_at,
            SpawnSessionEvent::ReleaseBroadcast,
            "spawn completed after child bootstrap verification, release broadcast, and controller handoff finalized",
        )
    })?;

    delete_spawn_provider_secrets(session_id);

    Ok(SpawnExecutionReceipt {
        session_id: session_id.to_string(),
        automaton_canister_id: canister_id,
        automaton_evm_address: runtime.evm_address,
        funded_amount: session_snapshot.net_forward_amount,
        controller,
        release_tx_hash: Some(release_tx_hash),
        release_broadcast_at: Some(release_broadcast_at),
        completed_at: current_time,
    })
}

#[cfg(test)]
mod tests {
    use super::build_bootstrap_verification;
    use crate::types::{
        AutomatonBootstrapEvidence, InferenceTransport, OpenRouterReasoningLevel, PaymentStatus,
        ProviderConfig, SpawnAsset, SpawnChain, SpawnConfig, SpawnSession, SpawnSessionState,
    };

    fn sample_session() -> SpawnSession {
        SpawnSession {
            name: Some("Meridian".to_string()),
            constitution: Some("I am Meridian. ".repeat(30)),
            session_id: "session-1".to_string(),
            claim_id: "claim-1".to_string(),
            steward_address: "0xsteward".to_string(),
            chain: SpawnChain::Base,
            asset: SpawnAsset::Usdc,
            gross_amount: "75000000".to_string(),
            platform_fee: "5000000".to_string(),
            creation_cost: "45000000".to_string(),
            net_forward_amount: "25000000".to_string(),
            quote_terms_hash: "quote-hash".to_string(),
            expires_at: 99_999,
            state: SpawnSessionState::BroadcastingRelease,
            retryable: false,
            refundable: false,
            payment_status: PaymentStatus::Paid,
            last_scanned_block: Some(10),
            automaton_canister_id: Some("automaton-0001".to_string()),
            automaton_evm_address: Some("0xautomaton".to_string()),
            release_tx_hash: None,
            release_broadcast_at: None,
            release_broadcast: None,
            parent_id: Some("parent-1".to_string()),
            child_ids: Vec::new(),
            selected_strategies: Vec::new(),
            config: SpawnConfig {
                chain: SpawnChain::Base,
                risk: 7,
                strategies: vec!["base-aave-usdc-reserve-01".to_string()],
                skills: vec![" search ".to_string()],
                provider: ProviderConfig {
                    model: Some("openrouter/auto".to_string()),
                    inference_transport: InferenceTransport::OpenrouterDirect,
                    open_router_reasoning_level: OpenRouterReasoningLevel::Default,
                },
            },
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn bootstrap_verification_passes_with_matching_child_evidence() {
        let mut session = sample_session();
        session.steward_address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".to_string();
        let verification = build_bootstrap_verification(
            &session,
            &candid::Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                .expect("test factory principal should parse"),
            8_453,
            "0123456789abcdef0123456789abcdef01234567",
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
            AutomatonBootstrapEvidence {
                bootstrap_contract_version: Some(crate::types::SPAWN_CONTRACT_VERSION),
                bootstrap_name: Some(session.resolved_genesis().0),
                bootstrap_constitution_hash: None,
                bootstrap_constitution: Some(session.resolved_genesis().1),
                bootstrap_session_id: Some(session.session_id.clone()),
                bootstrap_parent_id: session.parent_id.clone(),
                bootstrap_factory_principal: Some(
                    candid::Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                        .expect("test factory principal should parse"),
                ),
                bootstrap_risk: Some(session.config.risk),
                bootstrap_strategies: vec!["base-aave-usdc-reserve-01".to_string()],
                bootstrap_skills: vec!["search".to_string()],
                bootstrap_version_commit: Some(
                    "0123456789abcdef0123456789abcdef01234567".to_string(),
                ),
                steward_address: Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8".to_string()),
                steward_chain_id: Some(8_453),
                steward_enabled: Some(true),
                evm_address: Some("0x70997970c51812dc3a010c7d01b50e0d17dc79c8".to_string()),
            },
            12_000,
        );

        assert!(verification.passed);
        assert!(verification.failures.is_empty());
    }

    #[test]
    fn bootstrap_verification_captures_child_mismatches() {
        let session = sample_session();
        let verification = build_bootstrap_verification(
            &session,
            &candid::Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                .expect("test factory principal should parse"),
            8_453,
            "0123456789abcdef0123456789abcdef01234567",
            "0xautomaton",
            AutomatonBootstrapEvidence {
                bootstrap_contract_version: None,
                bootstrap_name: None,
                bootstrap_constitution_hash: None,
                bootstrap_constitution: None,
                bootstrap_session_id: Some("wrong-session".to_string()),
                bootstrap_parent_id: None,
                bootstrap_factory_principal: None,
                bootstrap_risk: Some(3),
                bootstrap_strategies: vec!["mean-reversion".to_string()],
                bootstrap_skills: vec!["messaging".to_string()],
                bootstrap_version_commit: Some(
                    "fedcba9876543210fedcba9876543210fedcba98".to_string(),
                ),
                steward_address: Some("0xother".to_string()),
                steward_chain_id: Some(1),
                steward_enabled: Some(false),
                evm_address: Some("0xother".to_string()),
            },
            12_000,
        );

        assert!(!verification.passed);
        assert!(verification.failures.len() >= 5);
    }
}
