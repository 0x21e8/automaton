use crate::domain::types::{
    PendingStrategyDiscoveryJob, ProtocolWatchlistEntry, StrategyDiscoveryWorkerConfig,
};
use crate::storage::stable;
use crate::timing::current_time_ns;
use candid::CandidType;
use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs};
use serde::Deserialize;
use serde_json::json;

const STRATEGY_DISCOVERY_SUBMIT_MAX_RESPONSE_BYTES: u64 = 2_048;

#[derive(Debug, Deserialize, CandidType)]
struct StrategyDiscoverySubmitAck {
    job_id: String,
    accepted_at_ns: u64,
    status: String,
}

fn current_canister_id_text() -> String {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::id().to_text();

    #[cfg(not(target_arch = "wasm32"))]
    return "2vxsx-fae".to_string();
}

fn parse_strategy_discovery_submit_ack(
    response: ic_cdk::management_canister::HttpRequestResult,
    expected_job_id: &str,
) -> Result<StrategyDiscoverySubmitAck, String> {
    if response.status != 202_u64 && response.status != 200_u64 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "strategy discovery worker submit status {}: {}",
            response.status,
            body.chars().take(500).collect::<String>()
        ));
    }
    let ack: StrategyDiscoverySubmitAck = serde_json::from_slice(&response.body)
        .map_err(|error| format!("invalid strategy discovery submit ack json: {error}"))?;
    if ack.job_id.trim().is_empty() {
        return Err("strategy discovery submit ack job_id cannot be empty".to_string());
    }
    if !ack.status.eq_ignore_ascii_case("accepted") {
        return Err(format!(
            "strategy discovery submit ack status must be accepted, got {}",
            ack.status
        ));
    }
    if ack.job_id != expected_job_id {
        return Err(format!(
            "strategy discovery submit ack job_id mismatch expected={} received={}",
            expected_job_id, ack.job_id
        ));
    }
    Ok(ack)
}

pub async fn submit_strategy_discovery_job(
    config: &StrategyDiscoveryWorkerConfig,
    objective: String,
    watchlist: Vec<ProtocolWatchlistEntry>,
    exposure_summary: String,
    autonomy_summary: String,
) -> Result<PendingStrategyDiscoveryJob, String> {
    let worker_base_url = config.worker_base_url.trim().trim_end_matches('/');
    if worker_base_url.is_empty() {
        return Err("strategy discovery worker_base_url cannot be empty".to_string());
    }
    let worker_api_key = config
        .worker_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "strategy discovery worker_api_key is not configured".to_string())?;
    if objective.trim().is_empty() {
        return Err("strategy discovery objective cannot be empty".to_string());
    }
    if watchlist.is_empty() {
        return Err("strategy discovery watchlist cannot be empty".to_string());
    }

    let now_ns = current_time_ns();
    let job_id = format!("strategy-discovery-{now_ns}");
    let payload = serde_json::to_vec(&json!({
        "canister_id": current_canister_id_text(),
        "job_id": job_id,
        "objective": objective,
        "watchlist": watchlist,
        "exposure_summary": exposure_summary,
        "autonomy_summary": autonomy_summary,
        "freshness_constraints": {
            "result_ttl_secs": config.result_ttl_secs,
        }
    }))
    .map_err(|error| format!("failed to build strategy discovery submit payload: {error}"))?;

    let request = HttpRequestArgs {
        url: format!("{worker_base_url}/v1/strategy-discovery/jobs"),
        max_response_bytes: Some(STRATEGY_DISCOVERY_SUBMIT_MAX_RESPONSE_BYTES),
        method: HttpMethod::POST,
        headers: vec![
            HttpHeader {
                name: "content-type".to_string(),
                value: "application/json".to_string(),
            },
            HttpHeader {
                name: "authorization".to_string(),
                value: format!("Bearer {worker_api_key}"),
            },
        ],
        body: Some(payload),
        transform: None,
        is_replicated: Some(false),
    };

    let response = match http_request(&request).await {
        Ok(response) => response,
        Err(error) => {
            stable::record_strategy_discovery_submit_failed();
            return Err(format!("strategy discovery submit outcall failed: {error}"));
        }
    };
    let ack = match parse_strategy_discovery_submit_ack(response, &job_id) {
        Ok(ack) => ack,
        Err(error) => {
            stable::record_strategy_discovery_submit_failed();
            return Err(error);
        }
    };

    let pending = PendingStrategyDiscoveryJob {
        job_id: ack.job_id,
        submitted_at_ns: now_ns,
        objective,
        watchlist,
        exposure_summary,
        autonomy_summary,
    };
    stable::upsert_pending_strategy_discovery_job(pending.clone())?;
    stable::record_strategy_discovery_submit_accepted();
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::parse_strategy_discovery_submit_ack;
    use crate::domain::types::{
        PendingStrategyDiscoveryJob, ProtocolWatchlistEntry, StrategyDiscoveryResultPayload,
        StrategyDiscoveryWorkerConfig, SubmitStrategyDiscoveryResultArgs,
    };
    use crate::storage::stable;
    use ic_cdk::management_canister::HttpRequestResult;
    use candid::Principal;

    #[test]
    fn strategy_discovery_submit_ack_parses() {
        let ack = parse_strategy_discovery_submit_ack(
            HttpRequestResult {
                status: 202_u64.into(),
                headers: Vec::new(),
                body: br#"{"job_id":"sd-job-1","accepted_at_ns":11,"status":"accepted"}"#.to_vec(),
            },
            "sd-job-1",
        )
        .expect("ack should parse");
        assert_eq!(ack.job_id, "sd-job-1");
        assert_eq!(ack.accepted_at_ns, 11);
    }

    #[test]
    fn strategy_discovery_submit_ack_and_reconcile_refresh() {
        let ack = parse_strategy_discovery_submit_ack(
            HttpRequestResult {
                status: 202_u64.into(),
                headers: Vec::new(),
                body: br#"{"job_id":"sd-job-2","accepted_at_ns":22,"status":"accepted"}"#.to_vec(),
            },
            "sd-job-2",
        )
        .expect("ack should parse");
        assert_eq!(ack.job_id, "sd-job-2");

        stable::init_storage();
        let watchlist = vec![ProtocolWatchlistEntry {
            id: "moonwell-usdc".to_string(),
            chain_id: 8453,
            pool_address: "0x1111111111111111111111111111111111111111".to_string(),
            market_data_api_url: "https://api.example.com/market/moonwell".to_string(),
            abi_api_url: "https://api.example.com/abi/moonwell".to_string(),
        }];
        let config = StrategyDiscoveryWorkerConfig {
            enabled: true,
            worker_base_url: "https://discovery.example.workers.dev".to_string(),
            worker_api_key: Some("secret".to_string()),
            trusted_callback_principal: Some(
                Principal::from_text("w36hm-eqaaa-aaaal-qr76a-cai")
                    .expect("principal should parse"),
            ),
            result_ttl_secs: 3_600,
            objective: "find reserve opportunities".to_string(),
            protocol_watchlist: watchlist.clone(),
        };
        stable::set_strategy_discovery_worker_config(config.clone())
            .expect("config should persist");
        stable::upsert_pending_strategy_discovery_job(PendingStrategyDiscoveryJob {
            job_id: "sd-job-2".to_string(),
            submitted_at_ns: 10,
            objective: config.objective.clone(),
            watchlist: watchlist.clone(),
            exposure_summary: "active_exposures=0".to_string(),
            autonomy_summary: "preserve runway".to_string(),
        })
        .expect("pending job should persist");
        let applied = stable::apply_strategy_discovery_callback(
            SubmitStrategyDiscoveryResultArgs {
                job_id: "sd-job-2".to_string(),
                completed_at_ns: 50,
                objective: config.objective.clone(),
                watchlist,
                payload: StrategyDiscoveryResultPayload {
                    protocol_artifacts: Vec::new(),
                    market: crate::domain::types::MarketSynthesisBundle::default(),
                    candidates: vec![crate::domain::types::StrategyCandidateBundle {
                        candidate_id: "cand-1".to_string(),
                        objective: config.objective.clone(),
                        protocol_id: "moonwell".to_string(),
                        primitive: "reserve_supply".to_string(),
                        chain_id: 8453,
                        rationale: "deterministic candidate".to_string(),
                        required_artifacts: Vec::new(),
                        assumptions: Vec::new(),
                        missing_inputs: Vec::new(),
                        confidence_label: "medium".to_string(),
                        freshness_deadline_ns: None,
                        suggested_template_shape: None,
                        estimated_yield_bps: Some(400),
                        warnings: Vec::new(),
                    }],
                    source_records: vec![crate::domain::types::SourceRecord {
                        source_id: "market-1".to_string(),
                        source_type: crate::domain::types::StrategyDiscoverySourceType::MarketDataApi,
                        url: "https://api.example.com/market/moonwell".to_string(),
                        fetched_at_ns: 50,
                        content_hash: "0xabc".to_string(),
                        trust_tier: crate::domain::types::StrategyDiscoverySourceTrustTier::Official,
                    }],
                },
            },
            "w36hm-eqaaa-aaaal-qr76a-cai".to_string(),
            60,
        )
        .expect("callback should persist");
        assert!(matches!(
            applied,
            stable::StrategyDiscoveryCallbackApply::Accepted(
                crate::domain::types::StrategyDiscoveryResultStatus::Validated
            )
        ));
        assert!(
            stable::freshest_validated_strategy_discovery_result_for_config(&config, 61).is_some(),
            "fresh validated result should satisfy reconcile refresh guard"
        );
    }
}
