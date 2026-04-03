use candid::Encode;
use serde_json::Value;

use crate::types::{
    AutomatonChildInitArgs, AutomatonChildRuntimeConfig, AutomatonRuntimeState,
    AutomatonSpawnBootstrapArgs, FactoryError, RepositoryStrategySessionSnapshot, SpawnSession,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedAutomatonChildRuntimeConfig {
    pub ecdsa_key_name: String,
    pub inbox_contract_address: Option<String>,
    pub evm_chain_id: u64,
    pub evm_rpc_url: String,
    pub evm_confirmation_depth: Option<u64>,
    pub evm_bootstrap_lookback_blocks: Option<u64>,
    pub http_allowed_domains: Option<Vec<String>>,
    pub llm_canister_id: Option<candid::Principal>,
    pub search_api_key: Option<String>,
    pub cycle_topup_enabled: Option<bool>,
    pub auto_topup_cycle_threshold: Option<u64>,
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|entry| {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn require_text_field(value: Option<&str>, field: &str) -> Result<String, FactoryError> {
    normalize_optional_text(value).ok_or_else(|| FactoryError::MissingChildRuntimeConfig {
        field: field.to_string(),
    })
}

fn require_u64_field(value: Option<u64>, field: &str) -> Result<u64, FactoryError> {
    value.ok_or_else(|| FactoryError::MissingChildRuntimeConfig {
        field: field.to_string(),
    })
}

fn normalize_string_list(value: Option<&[String]>) -> Option<Vec<String>> {
    let items = value
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

pub fn validate_automaton_child_runtime_config(
    config: &AutomatonChildRuntimeConfig,
) -> Result<ValidatedAutomatonChildRuntimeConfig, FactoryError> {
    Ok(ValidatedAutomatonChildRuntimeConfig {
        ecdsa_key_name: require_text_field(
            config.ecdsa_key_name.as_deref(),
            "child_runtime.ecdsa_key_name",
        )?,
        inbox_contract_address: normalize_optional_text(config.inbox_contract_address.as_deref()),
        evm_chain_id: require_u64_field(config.evm_chain_id, "child_runtime.evm_chain_id")?,
        evm_rpc_url: require_text_field(
            config.evm_rpc_url.as_deref(),
            "child_runtime.evm_rpc_url",
        )?,
        evm_confirmation_depth: config.evm_confirmation_depth,
        evm_bootstrap_lookback_blocks: config.evm_bootstrap_lookback_blocks,
        http_allowed_domains: normalize_string_list(config.http_allowed_domains.as_deref()),
        llm_canister_id: config.llm_canister_id,
        search_api_key: normalize_optional_text(config.search_api_key.as_deref()),
        cycle_topup_enabled: config.cycle_topup_enabled,
        auto_topup_cycle_threshold: config.auto_topup_cycle_threshold,
    })
}

fn strategy_install_error(
    snapshot: &RepositoryStrategySessionSnapshot,
    message: impl Into<String>,
) -> FactoryError {
    FactoryError::ManagementCallFailed {
        method: "register_strategy_admin".to_string(),
        message: format!("strategy {}: {}", snapshot.strategy_id, message.into()),
    }
}

fn expect_strategy_recipe_object<'a>(
    snapshot: &RepositoryStrategySessionSnapshot,
    value: &'a mut Value,
) -> Result<&'a mut serde_json::Map<String, Value>, FactoryError> {
    value.as_object_mut().ok_or_else(|| {
        strategy_install_error(
            snapshot,
            "session snapshot recipe must decode to a JSON object",
        )
    })
}

fn expect_strategy_recipe_string(
    snapshot: &RepositoryStrategySessionSnapshot,
    recipe: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<String, FactoryError> {
    recipe
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            strategy_install_error(
                snapshot,
                format!("session snapshot recipe field {field} must be a string"),
            )
        })
}

fn expect_strategy_recipe_u64(
    snapshot: &RepositoryStrategySessionSnapshot,
    recipe: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<u64, FactoryError> {
    recipe.get(field).and_then(Value::as_u64).ok_or_else(|| {
        strategy_install_error(
            snapshot,
            format!("session snapshot recipe field {field} must be a u64"),
        )
    })
}

pub fn build_strategy_install_recipe(
    snapshot: &RepositoryStrategySessionSnapshot,
) -> Result<String, FactoryError> {
    let mut recipe: Value = serde_json::from_str(snapshot.recipe_json.trim()).map_err(|error| {
        strategy_install_error(
            snapshot,
            format!("invalid session snapshot recipe JSON: {error}"),
        )
    })?;
    let recipe_object = expect_strategy_recipe_object(snapshot, &mut recipe)?;

    let template_id = expect_strategy_recipe_string(snapshot, recipe_object, "template_id")?;
    if template_id != snapshot.strategy_id {
        return Err(strategy_install_error(
            snapshot,
            format!(
                "session snapshot recipe template_id mismatch: expected {}, got {}",
                snapshot.strategy_id, template_id
            ),
        ));
    }

    let protocol = expect_strategy_recipe_string(snapshot, recipe_object, "protocol")?;
    if protocol != snapshot.protocol {
        return Err(strategy_install_error(
            snapshot,
            format!(
                "session snapshot recipe protocol mismatch: expected {}, got {}",
                snapshot.protocol, protocol
            ),
        ));
    }

    let primitive = expect_strategy_recipe_string(snapshot, recipe_object, "primitive")?;
    if primitive != snapshot.primitive {
        return Err(strategy_install_error(
            snapshot,
            format!(
                "session snapshot recipe primitive mismatch: expected {}, got {}",
                snapshot.primitive, primitive
            ),
        ));
    }

    let canonical_chain_id = expect_strategy_recipe_u64(snapshot, recipe_object, "chain_id")?;
    if canonical_chain_id != snapshot.canonical_chain_id {
        return Err(strategy_install_error(
            snapshot,
            format!(
                "session snapshot recipe chain_id mismatch: expected canonical {}, got {}",
                snapshot.canonical_chain_id, canonical_chain_id
            ),
        ));
    }

    let resolved_chain_id = snapshot
        .resolved_chain_id
        .unwrap_or(snapshot.canonical_chain_id);
    if resolved_chain_id != canonical_chain_id {
        recipe_object.insert("chain_id".to_string(), Value::from(resolved_chain_id));
    }

    serde_json::to_string(&recipe).map_err(|error| {
        strategy_install_error(
            snapshot,
            format!("failed to encode adapted child strategy recipe JSON: {error}"),
        )
    })
}

pub fn build_automaton_install_args(
    session: &SpawnSession,
    factory_principal: candid::Principal,
    version_commit: &str,
    child_runtime: &ValidatedAutomatonChildRuntimeConfig,
) -> Vec<u8> {
    Encode!(&AutomatonChildInitArgs {
        ecdsa_key_name: child_runtime.ecdsa_key_name.clone(),
        inbox_contract_address: child_runtime.inbox_contract_address.clone(),
        evm_chain_id: Some(child_runtime.evm_chain_id),
        evm_rpc_url: Some(child_runtime.evm_rpc_url.clone()),
        evm_confirmation_depth: child_runtime.evm_confirmation_depth,
        evm_bootstrap_lookback_blocks: child_runtime.evm_bootstrap_lookback_blocks,
        http_allowed_domains: child_runtime.http_allowed_domains.clone(),
        llm_canister_id: child_runtime.llm_canister_id,
        search_api_key: child_runtime.search_api_key.clone(),
        cycle_topup_enabled: child_runtime.cycle_topup_enabled,
        auto_topup_cycle_threshold: child_runtime.auto_topup_cycle_threshold,
        spawn_bootstrap: Some(AutomatonSpawnBootstrapArgs {
            steward_address: session.steward_address.clone(),
            session_id: session.session_id.clone(),
            parent_id: session.parent_id.clone(),
            factory_principal,
            risk: session.config.risk,
            strategies: session.config.strategies.clone(),
            skills: session.config.skills.clone(),
            provider: session.config.provider.clone(),
            version_commit: version_commit.to_string(),
        }),
    })
    .expect("install args should encode")
}

pub fn derive_automaton_evm_address(canister_id: &str) -> String {
    let hash = crate::types::derive_claim_id(canister_id);
    format!("0x{}", &hash[26..])
}

pub fn initialize_automaton(
    session: &SpawnSession,
    canister_id: String,
    evm_address: String,
    now_ms: u64,
) -> AutomatonRuntimeState {
    AutomatonRuntimeState {
        canister_id,
        evm_address,
        steward_address: session.steward_address.clone(),
        session_id: session.session_id.clone(),
        initialized_at: now_ms,
        install_succeeded_at: None,
        evm_address_derived_at: None,
        controller_handoff_completed_at: None,
        funded_amount: "0".to_string(),
        last_funded_at: None,
        chain: session.chain.clone(),
        risk: session.config.risk,
        strategies: session.config.strategies.clone(),
        skills: session.config.skills.clone(),
        model: session.config.provider.model.clone(),
        provider_keys_cleared: false,
        bootstrap_verification: None,
    }
}

#[cfg(test)]
mod tests {
    use candid::{decode_args, Principal};

    use super::{
        build_automaton_install_args, build_strategy_install_recipe,
        validate_automaton_child_runtime_config,
    };
    use crate::types::{
        AutomatonChildInitArgs, AutomatonChildRuntimeConfig, CreateSpawnSessionRequest,
        ProviderConfig, RepositoryStrategySessionSnapshot, RepositoryStrategySource,
        RepositoryStrategyStatus, SpawnAsset, SpawnChain, SpawnConfig,
    };

    fn sample_request() -> CreateSpawnSessionRequest {
        CreateSpawnSessionRequest {
            steward_address: "0xsteward".to_string(),
            asset: SpawnAsset::Usdc,
            gross_amount: "75000000".to_string(),
            config: SpawnConfig {
                chain: SpawnChain::Base,
                risk: 7,
                strategies: vec!["base-aave-usdc-reserve-01".to_string()],
                skills: vec![" search ".to_string()],
                provider: ProviderConfig {
                    open_router_api_key: Some(" sk-or-test ".to_string()),
                    model: Some(" openai/gpt-4o-mini ".to_string()),
                    brave_search_api_key: Some(" brave-test-key ".to_string()),
                },
            },
            parent_id: Some("parent-automaton".to_string()),
        }
    }

    fn sample_strategy_snapshot() -> RepositoryStrategySessionSnapshot {
        RepositoryStrategySessionSnapshot {
            strategy_id: "base-aave-usdc-reserve-01".to_string(),
            source_status: RepositoryStrategyStatus::Active,
            name: "Base Aave USDC Reserve".to_string(),
            description: "Example strategy".to_string(),
            canonical_chain: SpawnChain::Base,
            canonical_chain_id: 8_453,
            requested_spawn_chain: SpawnChain::Base,
            resolved_chain_id: Some(31_337),
            protocol: "aave-v3".to_string(),
            primitive: "lend_supply".to_string(),
            recipe_json: r#"{
                "protocol":"aave-v3",
                "primitive":"lend_supply",
                "chain_id":8453,
                "template_id":"base-aave-usdc-reserve-01",
                "contracts":[],
                "actions":[],
                "max_value_wei_per_call":"0",
                "template_budget_wei":"0"
            }"#
            .to_string(),
            source: RepositoryStrategySource {
                source_path: "docs/strategies/base-aave-usdc-reserve-01/recipe.json".to_string(),
                source_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
            },
            selected_at: 1,
        }
    }

    #[test]
    fn validates_required_child_runtime_fields() {
        let error =
            validate_automaton_child_runtime_config(&AutomatonChildRuntimeConfig::default())
                .expect_err("missing required child runtime fields should fail");

        assert!(matches!(
            error,
            crate::FactoryError::MissingChildRuntimeConfig { ref field }
                if field == "child_runtime.ecdsa_key_name"
        ));
    }

    #[test]
    fn encodes_the_real_child_init_args_envelope() {
        crate::restore_state(Default::default());
        let request = sample_request();
        let config = validate_automaton_child_runtime_config(&AutomatonChildRuntimeConfig {
            ecdsa_key_name: Some(" key_1 ".to_string()),
            inbox_contract_address: Some(" 0xInbox ".to_string()),
            evm_chain_id: Some(8_453),
            evm_rpc_url: Some(" http://127.0.0.1:18545 ".to_string()),
            evm_confirmation_depth: Some(12),
            evm_bootstrap_lookback_blocks: Some(256),
            http_allowed_domains: Some(vec![
                " https://openrouter.ai ".to_string(),
                "".to_string(),
                " https://api.search.brave.com ".to_string(),
            ]),
            llm_canister_id: Some(Principal::from_text("aaaaa-aa").expect("valid principal")),
            search_api_key: Some(" brave-key ".to_string()),
            cycle_topup_enabled: Some(true),
            auto_topup_cycle_threshold: Some(123_456),
        })
        .expect("child runtime config should validate");
        let session = crate::api::public::create_spawn_session_with_session_id(
            request,
            1_700_000,
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
        )
        .expect("session should be created")
        .session;

        let encoded = build_automaton_install_args(
            &session,
            Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").expect("valid factory principal"),
            "0123456789abcdef0123456789abcdef01234567",
            &config,
        );
        let (decoded,): (AutomatonChildInitArgs,) =
            decode_args(&encoded).expect("install args should decode");

        assert_eq!(decoded.ecdsa_key_name, "key_1");
        assert_eq!(decoded.inbox_contract_address.as_deref(), Some("0xInbox"));
        assert_eq!(decoded.evm_chain_id, Some(8_453));
        assert_eq!(
            decoded.evm_rpc_url.as_deref(),
            Some("http://127.0.0.1:18545")
        );
        assert_eq!(
            decoded.http_allowed_domains,
            Some(vec![
                "https://openrouter.ai".to_string(),
                "https://api.search.brave.com".to_string()
            ])
        );
        assert_eq!(
            decoded
                .spawn_bootstrap
                .as_ref()
                .map(|bootstrap| bootstrap.factory_principal),
            Some(
                Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai")
                    .expect("valid factory principal")
            )
        );
        assert_eq!(
            decoded
                .spawn_bootstrap
                .as_ref()
                .map(|bootstrap| bootstrap.version_commit.as_str()),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(
            decoded
                .spawn_bootstrap
                .as_ref()
                .and_then(|bootstrap| bootstrap.provider.open_router_api_key.as_deref()),
            Some(" sk-or-test ")
        );
        assert_eq!(
            decoded
                .spawn_bootstrap
                .as_ref()
                .map(|bootstrap| bootstrap.strategies.clone()),
            Some(vec!["base-aave-usdc-reserve-01".to_string()])
        );
    }

    #[test]
    fn rewrites_strategy_install_recipe_chain_id_for_child_runtime() {
        let adapted = build_strategy_install_recipe(&sample_strategy_snapshot())
            .expect("strategy recipe should adapt");
        let decoded: serde_json::Value =
            serde_json::from_str(&adapted).expect("adapted recipe should decode");

        assert_eq!(
            decoded
                .get("template_id")
                .and_then(serde_json::Value::as_str),
            Some("base-aave-usdc-reserve-01")
        );
        assert_eq!(
            decoded.get("chain_id").and_then(serde_json::Value::as_u64),
            Some(31_337)
        );
    }

    #[test]
    fn rejects_strategy_install_recipe_when_snapshot_and_recipe_diverge() {
        let mut snapshot = sample_strategy_snapshot();
        snapshot.recipe_json = snapshot
            .recipe_json
            .replace("base-aave-usdc-reserve-01", "different-template");

        let error = build_strategy_install_recipe(&snapshot)
            .expect_err("mismatched session snapshot recipe should fail");

        assert!(matches!(
            error,
            crate::FactoryError::ManagementCallFailed { ref method, ref message }
                if method == "register_strategy_admin"
                    && message.contains("template_id mismatch")
        ));
    }
}
