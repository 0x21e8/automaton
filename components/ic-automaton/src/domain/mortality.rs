//! Deterministic mortality policy for runway-driven cadence and terminal turns.

use crate::domain::cycle_admission::DEFAULT_RESERVE_FLOOR_CYCLES;
use crate::domain::types::{
    MortalityPhase, MortalityRuntime, MortalityTier, OpenRouterReasoningLevel,
};

pub const ACTIVE_MIN_RUNWAY_SECS: u64 = 30 * 24 * 60 * 60;
pub const CONSERVING_MIN_RUNWAY_SECS: u64 = 7 * 24 * 60 * 60;
pub const HIBERNATING_MIN_RUNWAY_SECS: u64 = 24 * 60 * 60;
pub const MORTALITY_RECOVERY_CHECKS_REQUIRED: u32 = 3;
pub const TERMINAL_TURN_RESERVED_CYCLES: u128 = DEFAULT_RESERVE_FLOOR_CYCLES;
pub const MAX_TERMINAL_BEQUESTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MortalityTierPolicy {
    pub tier: MortalityTier,
    pub cadence_multiplier: u64,
    pub reasoning_level: OpenRouterReasoningLevel,
}

pub const fn policy_for_tier(tier: MortalityTier) -> MortalityTierPolicy {
    match tier {
        MortalityTier::Active => MortalityTierPolicy {
            tier,
            cadence_multiplier: 10,
            reasoning_level: OpenRouterReasoningLevel::Medium,
        },
        MortalityTier::Conserving => MortalityTierPolicy {
            tier,
            cadence_multiplier: 20,
            reasoning_level: OpenRouterReasoningLevel::Low,
        },
        MortalityTier::Hibernating => MortalityTierPolicy {
            tier,
            cadence_multiplier: 40,
            reasoning_level: OpenRouterReasoningLevel::Low,
        },
        MortalityTier::Terminal | MortalityTier::Dead => MortalityTierPolicy {
            tier,
            cadence_multiplier: 40,
            reasoning_level: OpenRouterReasoningLevel::Low,
        },
    }
}

pub const fn tier_for_runway(runway_seconds: Option<u64>) -> MortalityTier {
    match runway_seconds {
        None => MortalityTier::Active,
        Some(seconds) if seconds >= ACTIVE_MIN_RUNWAY_SECS => MortalityTier::Active,
        Some(seconds) if seconds >= CONSERVING_MIN_RUNWAY_SECS => MortalityTier::Conserving,
        Some(seconds) if seconds >= HIBERNATING_MIN_RUNWAY_SECS => MortalityTier::Hibernating,
        Some(_) => MortalityTier::Terminal,
    }
}

/// The terminal reserve is a hard metabolic boundary, independent of burn
/// telemetry or convertible assets. Once liquid cycles reach it, ordinary
/// work can no longer spend safely and the one reserved final turn must run.
pub const fn tier_for_resources(liquid_cycles: u128, runway_seconds: Option<u64>) -> MortalityTier {
    if liquid_cycles <= TERMINAL_TURN_RESERVED_CYCLES {
        MortalityTier::Terminal
    } else {
        tier_for_runway(runway_seconds)
    }
}

/// Escalation is immediate. Recovery to a healthier tier requires three
/// consecutive observations, preventing a top-up or noisy burn sample from
/// flapping cadence every scheduler tick. Terminal and dead phases are
/// irreversible.
pub fn observe_tier(runtime: &mut MortalityRuntime, observed: MortalityTier) {
    if runtime.phase != MortalityPhase::Alive || runtime.tier == MortalityTier::Dead {
        return;
    }
    if observed.severity() > runtime.tier.severity() {
        runtime.tier = observed;
        runtime.recovery_candidate = None;
        runtime.recovery_checks = 0;
        return;
    }
    if observed == runtime.tier {
        runtime.recovery_candidate = None;
        runtime.recovery_checks = 0;
        return;
    }
    if runtime.recovery_candidate == Some(observed) {
        runtime.recovery_checks = runtime.recovery_checks.saturating_add(1);
    } else {
        runtime.recovery_candidate = Some(observed);
        runtime.recovery_checks = 1;
    }
    if runtime.recovery_checks >= MORTALITY_RECOVERY_CHECKS_REQUIRED {
        runtime.tier = observed;
        runtime.recovery_candidate = None;
        runtime.recovery_checks = 0;
    }
}

pub fn canonical_runway_seconds(
    liquid_cycles: u128,
    burn_rate_cycles_per_day: Option<u128>,
    usdc_balance_raw: Option<u64>,
    usdc_decimals: u8,
    usd_per_trillion_cycles: f64,
) -> Option<u64> {
    let burn = burn_rate_cycles_per_day?;
    if burn == 0 || !usd_per_trillion_cycles.is_finite() || usd_per_trillion_cycles <= 0.0 {
        return None;
    }
    let decimals = i32::from(usdc_decimals.min(18));
    let usdc = usdc_balance_raw.unwrap_or_default() as f64 / 10f64.powi(decimals);
    let purchasable_cycles = usdc / usd_per_trillion_cycles * 1_000_000_000_000f64;
    let total_cycles = liquid_cycles as f64 + purchasable_cycles;
    let seconds = (total_cycles / burn as f64 * 86_400f64).floor();
    if !seconds.is_finite() || seconds < 0.0 || seconds > u64::MAX as f64 {
        None
    } else {
        Some(seconds as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runway_fixture_maps_to_named_tiers() {
        assert_eq!(tier_for_runway(None), MortalityTier::Active);
        assert_eq!(
            tier_for_runway(Some(ACTIVE_MIN_RUNWAY_SECS)),
            MortalityTier::Active
        );
        assert_eq!(
            tier_for_runway(Some(CONSERVING_MIN_RUNWAY_SECS)),
            MortalityTier::Conserving
        );
        assert_eq!(
            tier_for_runway(Some(HIBERNATING_MIN_RUNWAY_SECS)),
            MortalityTier::Hibernating
        );
        assert_eq!(
            tier_for_runway(Some(HIBERNATING_MIN_RUNWAY_SECS - 1)),
            MortalityTier::Terminal
        );
    }

    #[test]
    fn reserve_boundary_forces_terminal_when_burn_is_unknown_or_optimistic() {
        assert_eq!(
            crate::features::cycle_topup::TOPUP_MIN_OPERATIONAL_CYCLES,
            250_000_000_000
        );
        const {
            assert!(
                crate::features::cycle_topup::TOPUP_MIN_OPERATIONAL_CYCLES
                    > TERMINAL_TURN_RESERVED_CYCLES
            );
        }
        assert_eq!(
            tier_for_resources(TERMINAL_TURN_RESERVED_CYCLES, None),
            MortalityTier::Terminal
        );
        assert_eq!(
            tier_for_resources(TERMINAL_TURN_RESERVED_CYCLES, Some(ACTIVE_MIN_RUNWAY_SECS)),
            MortalityTier::Terminal
        );
        assert_eq!(
            tier_for_resources(TERMINAL_TURN_RESERVED_CYCLES + 1, None),
            MortalityTier::Active
        );
    }

    #[test]
    fn recovery_hysteresis_prevents_flapping() {
        let mut runtime = MortalityRuntime {
            tier: MortalityTier::Hibernating,
            ..Default::default()
        };
        observe_tier(&mut runtime, MortalityTier::Conserving);
        assert_eq!(runtime.tier, MortalityTier::Hibernating);
        observe_tier(&mut runtime, MortalityTier::Hibernating);
        observe_tier(&mut runtime, MortalityTier::Conserving);
        observe_tier(&mut runtime, MortalityTier::Conserving);
        assert_eq!(runtime.tier, MortalityTier::Hibernating);
        observe_tier(&mut runtime, MortalityTier::Conserving);
        assert_eq!(runtime.tier, MortalityTier::Conserving);
    }

    #[test]
    fn canonical_runway_counts_cycles_and_convertible_usdc() {
        assert_eq!(
            canonical_runway_seconds(
                1_000_000_000_000,
                Some(1_000_000_000_000),
                Some(1_350_000),
                6,
                1.35
            ),
            Some(172_800)
        );
    }
}
