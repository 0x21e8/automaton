/// Strategy registry — thin persistence façade over `storage::stable`.
///
/// All mutable state for the strategy subsystem lives in IC stable memory; this module
/// provides a clean typed API so the rest of the strategy code never calls `stable::`
/// directly.  Functions are intentionally one-liners: they validate nothing themselves
/// (that is the caller's responsibility) and simply delegate to the appropriate stable
/// accessor.
///
/// # Sections
/// - **Template CRUD** — upsert/get/list [`StrategyTemplate`]s.
/// - **ABI artifacts** — upsert/get/list [`AbiArtifact`]s.
/// - **Lifecycle state** — activation, revocation, and kill-switch records.
/// - **Outcome stats** — read-only access to accumulated [`StrategyOutcomeStats`].
///
/// [`StrategyTemplate`]: crate::domain::types::StrategyTemplate
/// [`AbiArtifact`]: crate::domain::types::AbiArtifact
/// [`StrategyOutcomeStats`]: crate::domain::types::StrategyOutcomeStats
use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, StrategyKillSwitchState, StrategyOutcomeStats, StrategyTemplate,
    StrategyTemplateKey, TemplateActivationState, TemplateRevocationState,
};
use crate::storage::{sqlite, stable};

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
