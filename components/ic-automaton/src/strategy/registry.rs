/// Strategy registry — persistence façade and recipe-based registration.
///
/// All mutable state for the strategy subsystem lives in IC stable memory; this module
/// provides a clean typed API so the rest of the strategy code never calls `stable::`
/// directly.
///
/// # Sections
/// - **Template CRUD** — upsert/get/list [`StrategyTemplate`]s.
/// - **ABI artifacts** — upsert/get/list [`AbiArtifact`]s.
/// - **Lifecycle state** — activation, revocation, and kill-switch records.
/// - **Outcome stats** — read-only access to accumulated [`StrategyOutcomeStats`].
/// - **Recipe registration** — [`register_from_recipe`] converts a compact [`StrategyRecipe`]
///   into a fully validated, activated strategy template.
///
/// [`StrategyTemplate`]: crate::domain::types::StrategyTemplate
/// [`AbiArtifact`]: crate::domain::types::AbiArtifact
/// [`StrategyOutcomeStats`]: crate::domain::types::StrategyOutcomeStats
use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, AbiFunctionSpec, ActionSpec, ContractRoleBinding,
    StrategyKillSwitchState, StrategyOutcomeStats, StrategyTemplate, StrategyTemplateKey,
    TemplateActivationState, TemplateRevocationState, TemplateStatus,
};
use crate::storage::{sqlite, stable};
use crate::strategy::{abi, compiler};
use crate::timing::current_time_ns;
use crate::util::normalize_evm_address;
use alloy_primitives::U256;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;

// ── Template CRUD ────────────────────────────────────────────────────────────

/// Persist or update a [`StrategyTemplate`] in stable storage.
pub fn upsert_template(template: StrategyTemplate) -> Result<StrategyTemplate, String> {
    stable::upsert_strategy_template(template)
}

/// Retrieve a [`StrategyTemplate`] by key, or `None` if absent.
pub fn get_template(key: &StrategyTemplateKey) -> Option<StrategyTemplate> {
    sqlite::strategy_template(key)
        .ok()
        .flatten()
        .or_else(|| stable::strategy_template(key))
}

/// List up to `limit` templates for a given key.
pub fn list_templates(key: &StrategyTemplateKey, limit: usize) -> Vec<StrategyTemplate> {
    sqlite::list_strategy_templates(key, limit)
        .unwrap_or_else(|_| stable::list_strategy_templates(key, limit))
}

/// List up to `limit` templates across all keys.
pub fn list_all_templates(limit: usize) -> Vec<StrategyTemplate> {
    sqlite::list_all_strategy_templates(limit)
        .unwrap_or_else(|_| stable::list_all_strategy_templates(limit))
}

// ── ABI artifacts ────────────────────────────────────────────────────────────

/// Persist or update an [`AbiArtifact`] in stable storage.
pub fn upsert_abi_artifact(artifact: AbiArtifact) -> Result<AbiArtifact, String> {
    stable::upsert_abi_artifact(artifact)
}

/// Retrieve an [`AbiArtifact`] by its key, or `None` if absent.
pub fn get_abi_artifact(key: &AbiArtifactKey) -> Option<AbiArtifact> {
    sqlite::abi_artifact(key)
        .ok()
        .flatten()
        .or_else(|| stable::abi_artifact(key))
}

// ── Lifecycle state ──────────────────────────────────────────────────────────

/// Persist or update the activation state for a template.
///
/// Setting `enabled = false` prevents the validator from accepting execution plans
/// for this template.
pub fn set_activation(state: TemplateActivationState) -> Result<TemplateActivationState, String> {
    stable::set_strategy_template_activation(state)
}

/// Retrieve the activation state for a template, or `None` if never set.
pub fn activation(key: &StrategyTemplateKey) -> Option<TemplateActivationState> {
    stable::strategy_template_activation(key)
}

/// Persist or update the revocation state for a template.
///
/// A revoked template is blocked by the validator's policy layer.
pub fn set_revocation(state: TemplateRevocationState) -> Result<TemplateRevocationState, String> {
    stable::set_strategy_template_revocation(state)
}

/// Retrieve the revocation state for a template, or `None` if never set.
pub fn revocation(key: &StrategyTemplateKey) -> Option<TemplateRevocationState> {
    stable::strategy_template_revocation(key)
}

/// Persist or update the kill-switch state for a strategy key.
///
/// When `enabled = true` the validator rejects all execution plans for this key
/// regardless of activation or revocation state.
pub fn set_kill_switch(state: StrategyKillSwitchState) -> Result<StrategyKillSwitchState, String> {
    stable::set_strategy_kill_switch(state)
}

/// Retrieve the kill-switch state for a strategy key, or `None` if never set.
pub fn kill_switch(key: &StrategyTemplateKey) -> Option<StrategyKillSwitchState> {
    stable::strategy_kill_switch(key)
}

// ── Outcome stats ────────────────────────────────────────────────────────────

/// Retrieve accumulated outcome statistics for a template, or `None` if no outcomes
/// have been recorded yet.  Stats are written by the learner; this function is read-only.
pub fn outcome_stats(key: &StrategyTemplateKey) -> Option<StrategyOutcomeStats> {
    stable::strategy_outcome_stats(key)
}

// ── Recipe registration ──────────────────────────────────────────────────────

/// Default call-count cap for recipe-registered templates.
pub const RECIPE_DEFAULT_MAX_CALLS: usize = 5;
/// Default per-call value cap (0.1 ETH in wei).
pub const RECIPE_DEFAULT_MAX_VALUE_WEI_PER_CALL: &str = "100000000000000000";
/// Default lifetime template budget cap (1 ETH in wei).
pub const RECIPE_DEFAULT_TEMPLATE_BUDGET_WEI: &str = "1000000000000000000";

/// Compact strategy definition.  The agent or controller provides this; the system
/// expands it into a full [`StrategyTemplate`] with validated ABI artifacts.
#[derive(Debug, Deserialize)]
pub struct StrategyRecipe {
    pub protocol: String,
    pub primitive: String,
    pub chain_id: u64,
    pub template_id: String,
    pub contracts: Vec<StrategyRecipeContract>,
    pub actions: Vec<StrategyRecipeAction>,
    #[serde(default)]
    pub max_value_wei_per_call: Option<String>,
    #[serde(default)]
    pub template_budget_wei: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StrategyRecipeContract {
    pub role: String,
    pub address: String,
    pub abi_json: String,
    pub source_ref: String,
    #[serde(default)]
    pub codehash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StrategyRecipeAction {
    pub action_id: String,
    pub calls: Vec<StrategyRecipeActionCall>,
    #[serde(default)]
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
    #[serde(default)]
    pub risk_checks: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct StrategyRecipeActionCall {
    pub role: String,
    #[serde(rename = "function")]
    pub function_name: String,
}

/// Result of a successful [`register_from_recipe`] call.
pub struct RegisteredStrategy {
    pub template: StrategyTemplate,
    pub activation: TemplateActivationState,
}

struct ResolvedRecipeContract {
    binding: ContractRoleBinding,
    functions_by_name: HashMap<String, Vec<AbiFunctionSpec>>,
}

fn normalize_required_field(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must be non-empty"));
    }
    Ok(trimmed.to_string())
}

fn normalize_optional_check_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_required_check_list(values: Vec<String>, field: &str) -> Result<Vec<String>, String> {
    let normalized = normalize_optional_check_list(values);
    if normalized.is_empty() {
        return Err(format!("{field} must contain at least one non-empty entry"));
    }
    Ok(normalized)
}

fn normalize_recipe_decimal(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} value cannot be empty"));
    }
    if !trimmed.as_bytes().iter().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{field} must be a decimal string"));
    }
    U256::from_str(trimmed)
        .map_err(|error| format!("{field} failed to parse decimal quantity: {error}"))?;
    Ok(trimmed.to_string())
}

/// Register a strategy from a compact recipe.
///
/// Pipeline:
/// 1. Validate and normalise all recipe fields.
/// 2. For each contract: parse ABI, extract referenced functions, store artifact.
/// 3. Build a full [`StrategyTemplate`] with `Draft` status and persist it.
/// 4. Run [`compiler::dry_run_compile`] to validate the full compilation path.
/// 5. On success: promote to `Active` and enable activation.
///
/// Returns the activated template and activation state.
pub fn register_from_recipe(recipe: StrategyRecipe) -> Result<RegisteredStrategy, String> {
    let protocol = normalize_required_field(&recipe.protocol, "protocol")?;
    let primitive = normalize_required_field(&recipe.primitive, "primitive")?;
    let template_id = normalize_required_field(&recipe.template_id, "template_id")?;
    if recipe.chain_id == 0 {
        return Err("chain_id must be greater than zero".to_string());
    }
    if recipe.contracts.is_empty() {
        return Err("contracts must contain at least one entry".to_string());
    }
    if recipe.actions.is_empty() {
        return Err("actions must contain at least one entry".to_string());
    }

    let key = StrategyTemplateKey {
        protocol,
        primitive,
        chain_id: recipe.chain_id,
        template_id,
    };
    let now_ns = current_time_ns();

    let mut resolved_contracts: HashMap<String, ResolvedRecipeContract> = HashMap::new();
    for contract in recipe.contracts {
        let role = normalize_required_field(&contract.role, "contracts.role")?;
        if resolved_contracts.contains_key(&role) {
            return Err(format!("duplicate contract role in recipe: {role}"));
        }
        let address = normalize_evm_address(&contract.address)?;
        let source_ref = normalize_required_field(&contract.source_ref, "contracts.source_ref")?;
        let artifact_key = AbiArtifactKey {
            protocol: key.protocol.clone(),
            chain_id: key.chain_id,
            role: role.clone(),
        };
        let artifact = abi::normalize_abi_artifact(
            artifact_key,
            &contract.abi_json,
            &source_ref,
            contract.codehash.clone(),
            &[],
            now_ns,
        )?;
        let artifact = upsert_abi_artifact(artifact)?;
        let mut functions_by_name: HashMap<String, Vec<AbiFunctionSpec>> = HashMap::new();
        for function in artifact.functions {
            functions_by_name
                .entry(function.name.clone())
                .or_default()
                .push(function);
        }
        resolved_contracts.insert(
            role.clone(),
            ResolvedRecipeContract {
                binding: ContractRoleBinding {
                    role,
                    address,
                    source_ref,
                    codehash: contract.codehash,
                },
                functions_by_name,
            },
        );
    }

    let mut actions = Vec::with_capacity(recipe.actions.len());
    for action in recipe.actions {
        let action_id = normalize_required_field(&action.action_id, "actions.action_id")?;
        if action.calls.is_empty() {
            return Err(format!(
                "action `{action_id}` must contain at least one call"
            ));
        }
        let mut call_sequence = Vec::with_capacity(action.calls.len());
        for call in action.calls {
            let role = normalize_required_field(&call.role, "actions.calls.role")?;
            let function_name =
                normalize_required_field(&call.function_name, "actions.calls.function")?;
            let resolved = resolved_contracts
                .get(&role)
                .ok_or_else(|| format!("action `{action_id}` references unknown role `{role}`"))?;
            let Some(candidates) = resolved.functions_by_name.get(&function_name) else {
                return Err(format!(
                    "action `{action_id}` function `{function_name}` not found in ABI for role `{role}`"
                ));
            };
            if candidates.len() > 1 {
                return Err(format!(
                    "action `{action_id}` function `{function_name}` is overloaded in role `{role}` ABI; provide a non-overloaded function"
                ));
            }
            let function = candidates
                .first()
                .cloned()
                .ok_or_else(|| "recipe function lookup failed unexpectedly".to_string())?;
            abi::verify_function_selector(&function).map_err(|error| {
                format!(
                    "invalid selector for action `{action_id}` role `{role}` function `{function_name}`: {error}"
                )
            })?;
            call_sequence.push(function);
        }
        actions.push(ActionSpec {
            action_id,
            call_sequence,
            preconditions: normalize_optional_check_list(action.preconditions),
            postconditions: normalize_required_check_list(
                action.postconditions,
                "actions.postconditions",
            )?,
            risk_checks: normalize_optional_check_list(action.risk_checks),
        });
    }

    let mut contract_roles = resolved_contracts
        .into_values()
        .map(|resolved| resolved.binding)
        .collect::<Vec<_>>();
    contract_roles.sort_by(|left, right| left.role.cmp(&right.role));

    let max_value_wei_per_call = normalize_recipe_decimal(
        recipe
            .max_value_wei_per_call
            .as_deref()
            .unwrap_or(RECIPE_DEFAULT_MAX_VALUE_WEI_PER_CALL),
        "max_value_wei_per_call",
    )?;
    let template_budget_wei = normalize_recipe_decimal(
        recipe
            .template_budget_wei
            .as_deref()
            .unwrap_or(RECIPE_DEFAULT_TEMPLATE_BUDGET_WEI),
        "template_budget_wei",
    )?;
    let constraints_json = serde_json::json!({
        "max_calls": RECIPE_DEFAULT_MAX_CALLS,
        "max_value_wei_per_call": max_value_wei_per_call,
        "max_total_value_wei": max_value_wei_per_call,
        "template_budget_wei": template_budget_wei,
    })
    .to_string();

    let stored_draft = upsert_template(StrategyTemplate {
        key: key.clone(),
        status: TemplateStatus::Draft,
        contract_roles,
        actions,
        constraints_json,
        created_at_ns: now_ns,
        updated_at_ns: now_ns,
    })?;

    compiler::dry_run_compile(&key)?;

    let activated_template = upsert_template(StrategyTemplate {
        status: TemplateStatus::Active,
        updated_at_ns: current_time_ns(),
        ..stored_draft
    })?;
    let activation = set_activation(TemplateActivationState {
        key,
        enabled: true,
        updated_at_ns: current_time_ns(),
        reason: Some("auto-activated after dry-run compile".to_string()),
    })?;

    Ok(RegisteredStrategy {
        template: activated_template,
        activation,
    })
}
