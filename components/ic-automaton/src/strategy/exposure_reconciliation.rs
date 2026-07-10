use crate::domain::types::{
    ActiveExposure, ExposureReconciliationStatus, StrategyTemplateKey, ToolCallRecord, TurnRecord,
};
use crate::storage::stable;
use serde_json::Value;
use std::collections::BTreeMap;

const RECENT_TURN_SCAN_LIMIT: usize = 200;

#[derive(Clone, Debug)]
struct ObservedExposure {
    strategy_id: String,
    protocol: String,
    chain_id: u64,
    asset_symbol: String,
    notional_wei: Option<u128>,
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
        || existing.updated_at_ns < observed.updated_at_ns
}

fn collect_observed_exposures() -> Result<Vec<ObservedExposure>, String> {
    let turns = stable::list_turns(RECENT_TURN_SCAN_LIMIT);
    let mut observed_by_strategy_id = BTreeMap::new();

    for turn in turns {
        let tool_calls = stable::get_tools_for_turn(&turn.id);
        for tool_call in tool_calls.into_iter().rev() {
            if !tool_call.success || tool_call.tool != "execute_strategy_action" {
                continue;
            }
            let Some(observation) = observed_exposure_from_tool_call(&turn, &tool_call) else {
                continue;
            };
            observed_by_strategy_id
                .entry(observation.strategy_id.clone())
                .or_insert(observation);
        }
    }

    Ok(observed_by_strategy_id.into_values().collect())
}

fn observed_exposure_from_tool_call(
    turn: &TurnRecord,
    tool_call: &ToolCallRecord,
) -> Option<ObservedExposure> {
    let args = serde_json::from_str::<Value>(&tool_call.args_json).ok()?;
    let key = args.get("key")?;
    let strategy_key = strategy_key_from_json(key)?;
    let action_id = args.get("action_id")?.as_str()?.trim().to_string();
    if action_id.is_empty() {
        return None;
    }

    let is_open = action_id.starts_with("enter_");
    let is_close = action_id.starts_with("exit_");
    if !is_open && !is_close {
        return None;
    }

    Some(ObservedExposure {
        strategy_id: strategy_id_from_key(&strategy_key),
        protocol: strategy_key.protocol,
        chain_id: strategy_key.chain_id,
        asset_symbol: infer_asset_symbol(&args, &action_id),
        notional_wei: infer_notional_wei(&args),
        updated_at_ns: turn.created_at_ns,
        is_open,
    })
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

fn infer_asset_symbol(args: &Value, action_id: &str) -> String {
    for candidate in ["asset_symbol", "assetSymbol", "symbol"] {
        if let Some(value) = find_first_string(args, candidate) {
            if !value.trim().is_empty() {
                return value;
            }
        }
    }

    if let Some(value) = find_first_string(args, "marketParams.collateralToken") {
        return value;
    }
    if let Some(value) = find_first_string(args, "marketParams.loanToken") {
        return value;
    }

    action_id.to_string()
}

fn infer_notional_wei(args: &Value) -> Option<u128> {
    for candidate in [
        "notional_wei",
        "value_wei",
        "amount_wei",
        "assets",
        "amount",
    ] {
        if let Some(value) = find_first_scalar(args, candidate) {
            if let Some(parsed) = parse_u128_value(value) {
                return Some(parsed);
            }
        }
    }
    None
}

fn parse_u128_value(value: &Value) -> Option<u128> {
    match value {
        Value::Number(number) => number.as_u64().map(u128::from),
        Value::String(text) => text.trim().parse::<u128>().ok(),
        _ => None,
    }
}

fn find_first_string(value: &Value, dotted_key: &str) -> Option<String> {
    find_first_scalar(value, dotted_key).and_then(|value| {
        value
            .as_str()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    })
}

fn find_first_scalar<'a>(value: &'a Value, dotted_key: &str) -> Option<&'a Value> {
    let segments = dotted_key.split('.').collect::<Vec<_>>();
    find_first_scalar_recursive(value, &segments)
}

fn find_first_scalar_recursive<'a>(value: &'a Value, segments: &[&str]) -> Option<&'a Value> {
    if segments.is_empty() {
        return Some(value);
    }

    match value {
        Value::Object(map) => {
            let head = segments[0];
            if let Some(next) = map.get(head) {
                if let Some(found) = find_first_scalar_recursive(next, &segments[1..]) {
                    return Some(found);
                }
            }
            for child in map.values() {
                if let Some(found) = find_first_scalar_recursive(child, segments) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(entries) => entries
            .iter()
            .find_map(|entry| find_first_scalar_recursive(entry, segments)),
        _ => None,
    }
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
            output: r#"{"tx_hashes":["0xabc"]}"#.to_string(),
            success: true,
            outcome: Default::default(),
            error: None,
            failure_kind: None,
        }
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
        assert_eq!(exposure.asset_symbol, "USDC");
        assert_eq!(exposure.notional_wei, Some(50_000_000_000_000_000));
    }
}
