use crate::domain::types::{
    ActiveExposure, ExposureReconciliationStatus, StrategyTemplateKey, ToolCallRecord, TurnRecord,
};
use crate::storage::stable;
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;

const RECENT_TURN_SCAN_LIMIT: usize = 200;

#[derive(Clone, Debug)]
struct ObservedExposure {
    strategy_id: String,
    protocol: String,
    chain_id: u64,
    asset_symbol: String,
    notional_wei: Option<u128>,
    asset_address: Option<String>,
    decimals: Option<u8>,
    amount_raw: Option<String>,
    updated_at_ns: u64,
    is_open: bool,
}

pub fn reconcile_active_exposures_from_recent_executions(
    now_ns: u64,
) -> Result<ExposureReconciliationStatus, String> {
    let observed = collect_observed_exposures()?;
    let mut repaired = 0u32;
    let mut recreated = 0u32;
    let mut closed = 0u32;
    let mut drift_reasons = Vec::new();

    for observation in observed {
        let current = stable::active_exposure(&observation.strategy_id);
        match (observation.is_open, current) {
            (true, None) => {
                stable::set_active_exposure(ActiveExposure {
                    strategy_id: observation.strategy_id.clone(),
                    protocol: observation.protocol.clone(),
                    chain_id: observation.chain_id,
                    asset_symbol: observation.asset_symbol.clone(),
                    notional_wei: observation.notional_wei,
                    asset_address: observation.asset_address.clone(),
                    decimals: observation.decimals,
                    amount_raw: observation.amount_raw.clone(),
                    updated_at_ns: observation.updated_at_ns,
                })?;
                recreated = recreated.saturating_add(1);
                drift_reasons.push(format!(
                    "recreated_missing_exposure:{}",
                    observation.strategy_id
                ));
            }
            (true, Some(existing)) => {
                if exposure_drifted(&existing, &observation) {
                    stable::set_active_exposure(ActiveExposure {
                        strategy_id: observation.strategy_id.clone(),
                        protocol: observation.protocol.clone(),
                        chain_id: observation.chain_id,
                        asset_symbol: observation.asset_symbol.clone(),
                        notional_wei: observation.notional_wei,
                        asset_address: observation.asset_address.clone(),
                        decimals: observation.decimals,
                        amount_raw: observation.amount_raw.clone(),
                        updated_at_ns: observation.updated_at_ns,
                    })?;
                    repaired = repaired.saturating_add(1);
                    drift_reasons.push(format!(
                        "repaired_mismatched_exposure:{}",
                        observation.strategy_id
                    ));
                }
            }
            (false, Some(_)) => {
                let _ = stable::remove_active_exposure(&observation.strategy_id);
                closed = closed.saturating_add(1);
                drift_reasons.push(format!("closed_after_exit:{}", observation.strategy_id));
            }
            (false, None) => {}
        }
    }

    let status = ExposureReconciliationStatus {
        last_attempted_at_ns: Some(now_ns),
        last_succeeded_at_ns: Some(now_ns),
        repaired_exposures: repaired,
        recreated_exposures: recreated,
        closed_exposures: closed,
        drift_reason: if drift_reasons.is_empty() {
            None
        } else {
            let joined = drift_reasons.join(";");
            const MAX_DRIFT_REASON_LEN: usize = 512;
            if joined.len() > MAX_DRIFT_REASON_LEN {
                Some(format!("{}…", &joined[..MAX_DRIFT_REASON_LEN]))
            } else {
                Some(joined)
            }
        },
        last_error: None,
    };
    stable::set_exposure_reconciliation_status(status.clone())?;
    Ok(status)
}

fn exposure_drifted(existing: &ActiveExposure, observed: &ObservedExposure) -> bool {
    existing.protocol != observed.protocol
        || existing.chain_id != observed.chain_id
        || existing.asset_symbol != observed.asset_symbol
        || existing.notional_wei != observed.notional_wei
        || existing.asset_address != observed.asset_address
        || existing.decimals != observed.decimals
        || existing.amount_raw != observed.amount_raw
        || existing.updated_at_ns < observed.updated_at_ns
}

fn collect_observed_exposures() -> Result<Vec<ObservedExposure>, String> {
    let turns = stable::list_turns(RECENT_TURN_SCAN_LIMIT);
    let mut observed_by_strategy_id: BTreeMap<String, ObservedExposure> = BTreeMap::new();

    // SQLite returns newest-first. Replay oldest-first so each successful
    // execution is folded exactly once into a deterministic reconstructed state.
    for turn in turns.into_iter().rev() {
        let tool_calls = stable::get_tools_for_turn(&turn.id);
        for tool_call in tool_calls {
            if !tool_call.success || tool_call.tool != "execute_strategy_action" {
                continue;
            }
            let Some(observation) = observed_exposure_from_tool_call(&turn, &tool_call)? else {
                continue;
            };
            fold_observed_exposure(&mut observed_by_strategy_id, observation)?;
        }
    }

    Ok(observed_by_strategy_id.into_values().collect())
}

fn fold_observed_exposure(
    observed: &mut BTreeMap<String, ObservedExposure>,
    delta: ObservedExposure,
) -> Result<(), String> {
    let Some(current) = observed.get_mut(&delta.strategy_id) else {
        if delta.is_open {
            observed.insert(delta.strategy_id.clone(), delta);
        }
        return Ok(());
    };
    if current.chain_id != delta.chain_id
        || current
            .asset_address
            .as_ref()
            .map(|value| value.to_ascii_lowercase())
            != delta
                .asset_address
                .as_ref()
                .map(|value| value.to_ascii_lowercase())
        || current.decimals != delta.decimals
    {
        return Err(format!(
            "historical strategy effects changed asset identity:{}",
            delta.strategy_id
        ));
    }
    let current_amount = current
        .amount_raw
        .as_deref()
        .ok_or_else(|| "historical exposure amount is ambiguous".to_string())
        .and_then(|value| {
            alloy_primitives::U256::from_str(value)
                .map_err(|error| format!("invalid historical exposure amount: {error}"))
        })?;
    let delta_amount = delta
        .amount_raw
        .as_deref()
        .ok_or_else(|| "historical exposure delta is ambiguous".to_string())
        .and_then(|value| {
            alloy_primitives::U256::from_str(value)
                .map_err(|error| format!("invalid historical exposure delta: {error}"))
        })?;
    let next = if delta.is_open {
        current_amount
            .checked_add(delta_amount)
            .ok_or_else(|| "historical exposure amount overflow".to_string())?
    } else {
        current_amount
            .checked_sub(delta_amount)
            .ok_or_else(|| "historical exposure exit exceeds open amount".to_string())?
    };
    current.amount_raw = Some(next.to_string());
    current.notional_wei = if current.asset_address.is_none() && current.decimals == Some(18) {
        next.try_into().ok()
    } else {
        None
    };
    current.updated_at_ns = delta.updated_at_ns;
    current.is_open = next != alloy_primitives::U256::ZERO;
    Ok(())
}

fn observed_exposure_from_tool_call(
    turn: &TurnRecord,
    tool_call: &ToolCallRecord,
) -> Result<Option<ObservedExposure>, String> {
    let args = serde_json::from_str::<Value>(&tool_call.args_json)
        .map_err(|error| format!("invalid historical strategy args: {error}"))?;
    let Some(key) = args.get("key") else {
        return Ok(None);
    };
    let Some(strategy_key) = strategy_key_from_json(key) else {
        return Ok(None);
    };
    let Some(action_id) = args
        .get("action_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    if action_id.is_empty() {
        return Ok(None);
    }

    let is_open = action_id.starts_with("enter_");
    let is_close = action_id.starts_with("exit_");
    if !is_open && !is_close {
        return Ok(None);
    }

    let output: Value = serde_json::from_str(&tool_call.output)
        .map_err(|error| format!("invalid historical strategy output: {error}"))?;
    let effects: Vec<crate::domain::types::StrategyAssetEffect> = output
        .get("asset_effects")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let direction = if is_open {
        crate::domain::types::StrategyAssetDirection::Debit
    } else {
        crate::domain::types::StrategyAssetDirection::Credit
    };
    let effect = effects.iter().find(|effect| effect.direction == direction);
    if effect.is_none() {
        let Some(value) = args
            .pointer("/typed_params/calls/0/value_wei")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u128>().ok())
        else {
            return Ok(None);
        };
        let Some(symbol) = args
            .pointer("/typed_params/calls/0/args/marketParams/collateralToken")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return Ok(None);
        };
        return Ok(Some(ObservedExposure {
            strategy_id: strategy_id_from_key(&strategy_key),
            protocol: strategy_key.protocol,
            chain_id: strategy_key.chain_id,
            asset_symbol: symbol,
            notional_wei: Some(value),
            asset_address: None,
            decimals: None,
            amount_raw: None,
            updated_at_ns: turn.created_at_ns,
            is_open,
        }));
    }
    let relevant = effects
        .iter()
        .filter(|effect| effect.direction == direction)
        .collect::<Vec<_>>();
    let Some(first) = relevant.first() else {
        return Ok(None);
    };
    let identity = (
        first.chain_id,
        first
            .asset_address
            .as_ref()
            .map(|value| value.to_ascii_lowercase()),
        first.decimals,
    );
    if relevant.iter().any(|effect| {
        (
            effect.chain_id,
            effect
                .asset_address
                .as_ref()
                .map(|value| value.to_ascii_lowercase()),
            effect.decimals,
        ) != identity
    }) {
        return Err("historical strategy effects contain multiple asset groups".to_string());
    }
    let mut amount = alloy_primitives::U256::ZERO;
    for effect in &relevant {
        amount = amount
            .checked_add(
                alloy_primitives::U256::from_str(&effect.amount_raw)
                    .map_err(|error| format!("invalid historical effect amount: {error}"))?,
            )
            .ok_or_else(|| "historical effect amount overflow".to_string())?;
    }
    Ok(Some(ObservedExposure {
        strategy_id: strategy_id_from_key(&strategy_key),
        protocol: strategy_key.protocol,
        chain_id: strategy_key.chain_id,
        asset_symbol: first.asset_symbol.clone(),
        notional_wei: if first.asset_address.is_none() && first.decimals == 18 {
            amount.try_into().ok()
        } else {
            None
        },
        asset_address: first.asset_address.clone(),
        decimals: Some(first.decimals),
        amount_raw: Some(amount.to_string()),
        updated_at_ns: turn.created_at_ns,
        is_open,
    }))
}

fn strategy_key_from_json(value: &Value) -> Option<StrategyTemplateKey> {
    Some(StrategyTemplateKey {
        protocol: value.get("protocol")?.as_str()?.trim().to_string(),
        primitive: value.get("primitive")?.as_str()?.trim().to_string(),
        chain_id: value.get("chain_id")?.as_u64()?,
        template_id: value.get("template_id")?.as_str()?.trim().to_string(),
    })
}

fn strategy_id_from_key(key: &StrategyTemplateKey) -> String {
    format!(
        "{}:{}:{}:{}",
        key.protocol, key.primitive, key.chain_id, key.template_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{AgentState, ToolCallRecord, TurnRecord};
    use crate::storage::sqlite;

    fn sample_tool_call() -> ToolCallRecord {
        ToolCallRecord {
            turn_id: "turn-enter".to_string(),
            tool: "execute_strategy_action".to_string(),
            args_json: serde_json::json!({
                "key": {
                    "protocol": "morpho-v1",
                    "primitive": "supply",
                    "chain_id": 8453,
                    "template_id": "morpho-enter-supply"
                },
                "action_id": "enter_supply",
                "typed_params": {
                    "calls": [
                        {
                            "value_wei": "50000000000000000",
                            "args": {
                                "marketParams": {
                                    "collateralToken": "USDC"
                                }
                            }
                        }
                    ]
                }
            })
            .to_string(),
            output: r#"{"tx_hashes":["0xabc"],"asset_effects":[{"chain_id":8453,"asset_address":null,"asset_symbol":"ETH","decimals":18,"amount_raw":"50000000000000000","direction":"Debit"}]}"#.to_string(),
            success: true,
            outcome: Default::default(),
            error: None,
            failure_kind: None,
        }
    }

    fn append_execution(turn_id: &str, created_at_ns: u64, action_id: &str, amount: &str) {
        let turn = TurnRecord {
            id: turn_id.to_string(),
            created_at_ns,
            finished_at_ns: Some(created_at_ns + 1),
            duration_ms: Some(1),
            state_from: AgentState::Inferring,
            state_to: AgentState::ExecutingActions,
            source_events: 1,
            tool_call_count: 1,
            input_summary: action_id.to_string(),
            inner_dialogue: None,
            inference_round_count: 1,
            continuation_stop_reason: Default::default(),
            error: None,
        };
        let mut tool_call = sample_tool_call();
        tool_call.turn_id = turn_id.to_string();
        let mut args: Value = serde_json::from_str(&tool_call.args_json).unwrap();
        args["action_id"] = Value::String(action_id.to_string());
        tool_call.args_json = args.to_string();
        let direction = if action_id.starts_with("enter_") {
            "Debit"
        } else {
            "Credit"
        };
        tool_call.output = serde_json::json!({
            "tx_hashes": [format!("0x{created_at_ns:x}")],
            "asset_effects": [{
                "chain_id": 8453,
                "asset_address": Value::Null,
                "asset_symbol": "ETH",
                "decimals": 18,
                "amount_raw": amount,
                "direction": direction,
            }],
        })
        .to_string();
        stable::append_turn_record(&turn, &[tool_call]);
    }

    #[test]
    fn exposure_reconciliation_repairs_missing_state_after_execution() {
        let _ = sqlite::close_storage();
        stable::init_storage();

        let turn = TurnRecord {
            id: "turn-enter".to_string(),
            created_at_ns: 1_000,
            finished_at_ns: Some(1_001),
            duration_ms: Some(1),
            state_from: AgentState::Inferring,
            state_to: AgentState::ExecutingActions,
            source_events: 1,
            tool_call_count: 1,
            input_summary: "enter supply".to_string(),
            inner_dialogue: None,
            inference_round_count: 1,
            continuation_stop_reason: Default::default(),
            error: None,
        };
        let tool_call = sample_tool_call();
        stable::append_turn_record(&turn, std::slice::from_ref(&tool_call));

        assert_eq!(
            stable::active_exposure("morpho-v1:supply:8453:morpho-enter-supply"),
            None
        );

        let status = reconcile_active_exposures_from_recent_executions(2_000)
            .expect("reconciliation should succeed");
        assert_eq!(status.recreated_exposures, 1);
        assert_eq!(status.repaired_exposures, 0);
        assert_eq!(status.closed_exposures, 0);
        assert!(
            status
                .drift_reason
                .as_deref()
                .unwrap_or_default()
                .contains("recreated_missing_exposure"),
            "drift reason should describe the repair"
        );

        let exposure = stable::active_exposure("morpho-v1:supply:8453:morpho-enter-supply")
            .expect("reconciler should rebuild the missing exposure");
        assert_eq!(exposure.protocol, "morpho-v1");
        assert_eq!(exposure.chain_id, 8453);
        assert_eq!(exposure.asset_symbol, "ETH");
        assert_eq!(exposure.notional_wei, Some(50_000_000_000_000_000));
        assert_eq!(exposure.decimals, Some(18));
        assert_eq!(exposure.amount_raw.as_deref(), Some("50000000000000000"));
    }

    #[test]
    fn exposure_reconciliation_folds_repeated_enters_partial_exit_and_replay() {
        let _ = sqlite::close_storage();
        stable::init_storage();

        append_execution("turn-enter-1", 10_000, "enter_supply", "50");
        append_execution("turn-enter-2", 11_000, "enter_supply", "30");
        append_execution("turn-exit-1", 12_000, "exit_supply", "20");

        let first = reconcile_active_exposures_from_recent_executions(13_000).unwrap();
        assert_eq!(first.recreated_exposures, 1);
        let exposure = stable::active_exposure("morpho-v1:supply:8453:morpho-enter-supply")
            .expect("partial exit must leave the remaining exposure open");
        assert_eq!(exposure.amount_raw.as_deref(), Some("60"));
        assert_eq!(exposure.notional_wei, Some(60));
        assert_eq!(exposure.updated_at_ns, 12_000);

        let replay = reconcile_active_exposures_from_recent_executions(14_000).unwrap();
        assert_eq!(replay.recreated_exposures, 0);
        assert_eq!(replay.repaired_exposures, 0);
        assert_eq!(replay.closed_exposures, 0);
        assert_eq!(
            stable::active_exposure("morpho-v1:supply:8453:morpho-enter-supply")
                .unwrap()
                .amount_raw
                .as_deref(),
            Some("60")
        );

        append_execution("turn-exit-2", 15_000, "exit_supply", "60");
        let closed = reconcile_active_exposures_from_recent_executions(16_000).unwrap();
        assert_eq!(closed.closed_exposures, 1);
        assert!(stable::active_exposure("morpho-v1:supply:8453:morpho-enter-supply").is_none());
        let closed_replay = reconcile_active_exposures_from_recent_executions(17_000).unwrap();
        assert_eq!(closed_replay.closed_exposures, 0);
    }
}
