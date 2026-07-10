/// Outcome learning — persist raw strategy outcomes and enforce safety automation.
///
/// After each strategy execution the agent records an [`StrategyOutcomeEvent`] here.
/// This module:
///
/// 1. Appends the raw event to stable storage via `stable::record_strategy_outcome`.
/// 2. Auto-deactivates the template if `deterministic_failure_streak` reaches
///    [`AUTO_DEACTIVATE_DETERMINISTIC_STREAK`].
///
/// [`StrategyOutcomeEvent`]: crate::domain::types::StrategyOutcomeEvent
/// [`StrategyOutcomeStats`]: crate::domain::types::StrategyOutcomeStats
use crate::domain::types::{
    StrategyOutcomeEvent, StrategyOutcomeStats, StrategyTemplateKey, TemplateActivationState,
};
use crate::storage::stable;

// ── Constants ────────────────────────────────────────────────────────────────

/// Number of consecutive deterministic failures before the template is automatically
/// deactivated to prevent further on-chain losses.
///
/// Deterministic failures (e.g. "execution reverted", "insufficient balance") will not
/// resolve by retrying with the same parameters, so continued execution would only waste
/// gas.  The template can be re-enabled manually once the root cause is resolved.
const AUTO_DEACTIVATE_DETERMINISTIC_STREAK: u32 = 3;

// ── Public surface ───────────────────────────────────────────────────────────

/// Record a strategy execution outcome event and return the updated [`StrategyOutcomeStats`].
///
/// Appends the raw event and — if the deterministic failure streak has reached
/// [`AUTO_DEACTIVATE_DETERMINISTIC_STREAK`] — deactivates the template.
pub fn record_outcome(event: StrategyOutcomeEvent) -> Result<StrategyOutcomeStats, String> {
    let observed_at_ns = event.observed_at_ns;
    let stats = stable::record_strategy_outcome(event)?;
    maybe_auto_deactivate_on_deterministic_failures(&stats, observed_at_ns)?;
    Ok(stats)
}

/// Retrieve the current [`StrategyOutcomeStats`] for a template without recording
/// a new event.  Returns `None` if no outcomes have been recorded yet.
pub fn outcome_stats(key: &StrategyTemplateKey) -> Option<StrategyOutcomeStats> {
    stable::strategy_outcome_stats(key)
}

/// Format outcome stats into a compact sentence suitable for LLM context.
pub fn summary_for_llm(stats: &StrategyOutcomeStats) -> String {
    let last_error_segment = stats
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!(" (last error: {value})"))
        .unwrap_or_default();
    format!(
        "{} runs: {} succeeded, {} deterministic failures{}, {} transient. Streak: {} consecutive deterministic failures.",
        stats.total_runs,
        stats.success_runs,
        stats.deterministic_failures,
        last_error_segment,
        stats.nondeterministic_failures,
        stats.deterministic_failure_streak
    )
}

/// Deactivate the template if `deterministic_failure_streak` has reached the threshold.
///
/// No-ops if the template is already inactive or the streak is below the threshold,
/// keeping the operation idempotent across redundant calls.
fn maybe_auto_deactivate_on_deterministic_failures(
    stats: &StrategyOutcomeStats,
    observed_at_ns: u64,
) -> Result<(), String> {
    if stats.deterministic_failure_streak < AUTO_DEACTIVATE_DETERMINISTIC_STREAK {
        return Ok(());
    }
    let currently_enabled = stable::strategy_template_activation(&stats.key)
        .map(|state| state.enabled)
        .unwrap_or(false);
    if !currently_enabled {
        return Ok(());
    }

    stable::set_strategy_template_activation(TemplateActivationState {
        key: stats.key.clone(),
        enabled: false,
        updated_at_ns: observed_at_ns,
        reason: Some(format!(
            "auto_deactivated after {} deterministic failures in a row",
            stats.deterministic_failure_streak
        )),
    })
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{StrategyOutcomeKind, StrategyTemplateKey};

    fn key(template_id: &str) -> StrategyTemplateKey {
        StrategyTemplateKey {
            protocol: "uniswap-v3".to_string(),
            primitive: "swap_exact_in".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        }
    }

    #[test]
    fn record_outcome_updates_raw_counters() {
        stable::init_storage();
        let key = key("learner-confidence");

        record_outcome(StrategyOutcomeEvent {
            key: key.clone(),
            action_id: "swap_exact_in".to_string(),
            outcome: StrategyOutcomeKind::Success,
            tx_hash: Some("0xaaa".to_string()),
            error: None,
            observed_at_ns: 10,
        })
        .expect("success outcome should persist");
        let stats = record_outcome(StrategyOutcomeEvent {
            key,
            action_id: "swap_exact_in".to_string(),
            outcome: StrategyOutcomeKind::DeterministicFailure,
            tx_hash: Some("0xbbb".to_string()),
            error: Some("slippage exceeded".to_string()),
            observed_at_ns: 11,
        })
        .expect("failure outcome should persist");

        assert_eq!(stats.total_runs, 2);
        assert_eq!(stats.success_runs, 1);
        assert_eq!(stats.deterministic_failures, 1);
        assert_eq!(stats.deterministic_failure_streak, 1);
        assert_eq!(stats.nondeterministic_failures, 0);
        assert_eq!(stats.last_error.as_deref(), Some("slippage exceeded"));
        assert_eq!(stats.last_tx_hash.as_deref(), Some("0xbbb"));
        assert_eq!(stats.last_observed_at_ns, Some(11));
    }

    #[test]
    fn summary_for_llm_includes_counts_and_streak() {
        let summary = summary_for_llm(&StrategyOutcomeStats {
            key: key("learner-summary"),
            total_runs: 23,
            success_runs: 18,
            deterministic_failures: 3,
            nondeterministic_failures: 2,
            deterministic_failure_streak: 0,
            last_error: Some("insufficient balance".to_string()),
            last_tx_hash: Some("0xabc".to_string()),
            last_observed_at_ns: Some(100),
        });

        assert!(summary.contains("23 runs"));
        assert!(summary.contains("18 succeeded"));
        assert!(summary.contains("3 deterministic failures"));
        assert!(summary.contains("2 transient"));
        assert!(summary.contains("Streak: 0 consecutive deterministic failures."));
    }

    #[test]
    fn deterministic_failure_streak_auto_deactivates_template() {
        stable::init_storage();
        let key = key("learner-autodeactivate");
        stable::set_strategy_template_activation(TemplateActivationState {
            key: key.clone(),
            enabled: true,
            updated_at_ns: 1,
            reason: Some("seed".to_string()),
        })
        .expect("activation should seed");

        for idx in 0..AUTO_DEACTIVATE_DETERMINISTIC_STREAK {
            record_outcome(StrategyOutcomeEvent {
                key: key.clone(),
                action_id: "swap_exact_in".to_string(),
                outcome: StrategyOutcomeKind::DeterministicFailure,
                tx_hash: None,
                error: Some("execution reverted".to_string()),
                observed_at_ns: 100 + u64::from(idx),
            })
            .expect("deterministic failure should record");
        }

        let activation =
            stable::strategy_template_activation(&key).expect("activation should still exist");
        assert!(!activation.enabled);
        assert!(activation
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("auto_deactivated"));
    }
}
