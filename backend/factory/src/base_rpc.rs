use std::sync::OnceLock;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::cycles::ensure_http_request_cycles;
use crate::types::{hex_encode, FactoryError, RpcFailureCategory};

pub const BASE_LOG_WINDOW_LIMIT: u64 = 10_000;
pub const BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES: u64 = 64 * 1024;
pub const BASE_RPC_LOG_SCAN_MAX_RESPONSE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseDepositLog {
    pub claim_id: String,
    pub amount: String,
    pub block_number: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentScanPlan {
    pub claim_ids: Vec<String>,
    pub from_block: u64,
    pub to_block: u64,
}

#[derive(Serialize)]
struct JsonRpcRequest<'a, P> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: P,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
struct JsonRpcResponse<T> {
    jsonrpc: String,
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    error: Option<JsonRpcErrorBody>,
}

#[derive(Deserialize)]
struct JsonRpcErrorBody {
    code: i64,
    message: String,
}

#[derive(Serialize)]
struct EthGetLogsFilter<'a> {
    address: &'a str,
    #[serde(rename = "fromBlock")]
    from_block: String,
    #[serde(rename = "toBlock")]
    to_block: String,
    topics: EthLogTopics<'a>,
}

#[derive(Serialize)]
struct EthLogTopics<'a>(String, Vec<&'a str>);

#[derive(Deserialize)]
struct EthLogEntry {
    #[serde(rename = "blockNumber")]
    block_number: String,
    data: String,
    topics: Vec<String>,
}

fn parse_hex_u64(value: &str) -> Result<u64, FactoryError> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    u64::from_str_radix(trimmed, 16).map_err(|_| FactoryError::InvalidAmount {
        value: value.to_string(),
    })
}

fn parse_hex_u128(value: &str) -> Result<u128, FactoryError> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Ok(0);
    }
    u128::from_str_radix(trimmed, 16).map_err(|_| FactoryError::InvalidAmount {
        value: value.to_string(),
    })
}

pub fn endpoint_has_scheme(endpoint: &str) -> bool {
    endpoint.contains("://")
}

pub fn configured_rpc_endpoints(primary: Option<String>, fallback: Option<String>) -> Vec<String> {
    let mut endpoints = Vec::new();
    for candidate in [primary, fallback].into_iter().flatten() {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }

        let endpoint = trimmed.to_string();
        if endpoints.iter().all(|existing| existing != &endpoint) {
            endpoints.push(endpoint);
        }
    }
    endpoints
}

fn rpc_request_failed(
    endpoint: &str,
    operation: &str,
    category: RpcFailureCategory,
    code: Option<i64>,
    message: impl Into<String>,
) -> FactoryError {
    FactoryError::RpcRequestFailed {
        operation: operation.to_string(),
        endpoint: endpoint.to_string(),
        category,
        code,
        message: message.into(),
    }
}

fn classify_transport_failure(
    endpoint: &str,
    operation: &str,
    message: impl AsRef<str>,
) -> FactoryError {
    let message = message.as_ref();
    let lowercase = message.to_ascii_lowercase();
    let category = if lowercase.contains("response too large")
        || lowercase.contains("size limit")
        || lowercase.contains("max_response_bytes")
    {
        RpcFailureCategory::ResponseTooLarge
    } else if lowercase.contains("rate limit")
        || lowercase.contains("too many requests")
        || lowercase.contains("http 429")
    {
        RpcFailureCategory::RateLimited
    } else if lowercase.contains("unavailable")
        || lowercase.contains("timeout")
        || lowercase.contains("timed out")
        || lowercase.contains("connection refused")
        || lowercase.contains("connection reset")
        || lowercase.contains("dns")
        || lowercase.contains("resolve")
    {
        RpcFailureCategory::Unavailable
    } else {
        RpcFailureCategory::Transport
    };

    rpc_request_failed(endpoint, operation, category, None, message)
}

fn classify_upstream_category(code: i64, message: &str) -> RpcFailureCategory {
    let lowercase = message.to_ascii_lowercase();
    if code == 429 || lowercase.contains("rate limit") || lowercase.contains("too many requests") {
        RpcFailureCategory::RateLimited
    } else if lowercase.contains("unavailable")
        || lowercase.contains("timeout")
        || lowercase.contains("timed out")
        || lowercase.contains("connection refused")
    {
        RpcFailureCategory::Unavailable
    } else {
        RpcFailureCategory::Upstream
    }
}

fn ensure_response_size(
    endpoint: &str,
    operation: &str,
    body: &[u8],
    max_response_bytes: u64,
) -> Result<(), FactoryError> {
    if body.len() as u64 > max_response_bytes {
        return Err(rpc_request_failed(
            endpoint,
            operation,
            RpcFailureCategory::ResponseTooLarge,
            None,
            format!(
                "response exceeded {max_response_bytes} bytes: actual={}",
                body.len()
            ),
        ));
    }

    Ok(())
}

fn parse_jsonrpc_result<T: DeserializeOwned>(
    endpoint: &str,
    operation: &str,
    body: &[u8],
    max_response_bytes: u64,
) -> Result<T, FactoryError> {
    ensure_response_size(endpoint, operation, body, max_response_bytes)?;

    let response: JsonRpcResponse<T> = serde_json::from_slice(body).map_err(|error| {
        rpc_request_failed(
            endpoint,
            operation,
            RpcFailureCategory::MalformedResponse,
            None,
            error.to_string(),
        )
    })?;

    if response.jsonrpc != "2.0" {
        return Err(rpc_request_failed(
            endpoint,
            operation,
            RpcFailureCategory::MalformedResponse,
            None,
            format!("unexpected jsonrpc version: {}", response.jsonrpc),
        ));
    }

    if let Some(error) = response.error {
        return Err(rpc_request_failed(
            endpoint,
            operation,
            classify_upstream_category(error.code, &error.message),
            Some(error.code),
            error.message,
        ));
    }

    response.result.ok_or_else(|| {
        rpc_request_failed(
            endpoint,
            operation,
            RpcFailureCategory::MalformedResponse,
            None,
            "missing json-rpc result",
        )
    })
}

fn parse_deposit_logs(
    endpoint: &str,
    body: &[u8],
    max_response_bytes: u64,
) -> Result<Vec<BaseDepositLog>, FactoryError> {
    let entries: Vec<EthLogEntry> =
        parse_jsonrpc_result(endpoint, "eth_getLogs", body, max_response_bytes)?;
    let mut logs = Vec::with_capacity(entries.len());

    for entry in entries {
        if entry.topics.len() < 2 {
            continue;
        }

        let data_hex = entry.data.strip_prefix("0x").unwrap_or(&entry.data);
        let amount_hex = if data_hex.len() >= 64 {
            &data_hex[data_hex.len() - 64..]
        } else {
            data_hex
        };

        logs.push(BaseDepositLog {
            claim_id: entry.topics[1].clone(),
            amount: parse_hex_u128(&format!("0x{amount_hex}"))?.to_string(),
            block_number: parse_hex_u64(&entry.block_number)?,
        });
    }

    Ok(logs)
}

pub fn deposited_event_topic() -> String {
    static TOPIC: OnceLock<String> = OnceLock::new();
    TOPIC
        .get_or_init(|| {
            let digest = Keccak256::digest(b"Deposited(bytes32,address,uint256)");
            format!("0x{}", hex_encode(&digest))
        })
        .clone()
}

fn build_request_body<P: Serialize>(
    request: &JsonRpcRequest<'_, P>,
) -> Result<Vec<u8>, FactoryError> {
    serde_json::to_vec(request).map_err(|error| FactoryError::ManagementCallFailed {
        method: request.method.to_string(),
        message: error.to_string(),
    })
}

fn build_http_request_args(
    endpoint: &str,
    request_body: Vec<u8>,
    max_response_bytes: u64,
) -> ic_cdk::management_canister::HttpRequestArgs {
    use ic_cdk::management_canister::{HttpHeader, HttpMethod, HttpRequestArgs};

    HttpRequestArgs {
        url: endpoint.to_string(),
        method: HttpMethod::POST,
        headers: vec![HttpHeader {
            name: "content-type".to_string(),
            value: "application/json".to_string(),
        }],
        body: Some(request_body),
        max_response_bytes: Some(max_response_bytes),
        transform: None,
        is_replicated: None,
    }
}

#[cfg(target_arch = "wasm32")]
async fn rpc_request_once(
    endpoint: &str,
    request_body: Vec<u8>,
    max_response_bytes: u64,
    operation: &str,
) -> Result<Vec<u8>, FactoryError> {
    use ic_cdk::management_canister::http_request;

    let request = build_http_request_args(endpoint, request_body, max_response_bytes);
    ensure_http_request_cycles(operation, &request)?;

    let response = http_request(&request)
        .await
        .map_err(|error| classify_transport_failure(endpoint, operation, error.to_string()))?;

    ensure_response_size(endpoint, operation, &response.body, max_response_bytes)?;
    Ok(response.body)
}

#[cfg(not(target_arch = "wasm32"))]
fn mock_success_body(operation: &str, default_string_result: Option<&str>) -> Vec<u8> {
    match operation {
        "eth_blockNumber" => br#"{"jsonrpc":"2.0","id":1,"result":"0x0"}"#.to_vec(),
        "eth_getLogs" => br#"{"jsonrpc":"2.0","id":1,"result":[]}"#.to_vec(),
        "eth_sendRawTransaction" => format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":"{}"}}"#,
            default_string_result.unwrap_or("0x0")
        )
        .into_bytes(),
        _ => br#"{"jsonrpc":"2.0","id":1,"result":null}"#.to_vec(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_mock_deposit_log_endpoint(endpoint: &str) -> Option<(String, u128, u64)> {
    let payload = endpoint.strip_prefix("mock://success/deposit-log/")?;
    let mut parts = payload.splitn(3, '/');
    let claim_id = parts.next()?.to_string();
    let amount = parts.next()?.parse::<u128>().ok()?;
    let block_number = parts.next()?.parse::<u64>().ok()?;
    Some((claim_id, amount, block_number))
}

#[cfg(not(target_arch = "wasm32"))]
fn mock_deposit_log_body(claim_id: &str, amount: u128, block_number: u64) -> Vec<u8> {
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":[{{"blockNumber":"0x{block_number:x}","data":"0x{amount:064x}","topics":["0xevent","{claim_id}"]}}]}}"#
    )
    .into_bytes()
}

#[cfg(not(target_arch = "wasm32"))]
fn rpc_request_once(
    endpoint: &str,
    request_body: Vec<u8>,
    max_response_bytes: u64,
    operation: &str,
    default_string_result: Option<&str>,
) -> Result<Vec<u8>, FactoryError> {
    let request = build_http_request_args(endpoint, request_body, max_response_bytes);
    ensure_http_request_cycles(operation, &request)?;

    let body = match endpoint {
        "mock://success" => mock_success_body(operation, default_string_result),
        "mock://success/deposit-log" if operation == "eth_getLogs" => br#"{"jsonrpc":"2.0","id":1,"result":[{"blockNumber":"0x2a","data":"0x0000000000000000000000000000000000000000000000000000000003938700","topics":["0xevent","0x1111111111111111111111111111111111111111111111111111111111111111"]}]}"#.to_vec(),
        "mock://error/rate-limit" => br#"{"jsonrpc":"2.0","id":1,"error":{"code":429,"message":"rate limit exceeded"}}"#.to_vec(),
        "mock://error/upstream-unavailable" => br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"upstream unavailable"}}"#.to_vec(),
        "mock://error/malformed-json" => b"{not valid json".to_vec(),
        "mock://error/missing-result" => br#"{"jsonrpc":"2.0","id":1}"#.to_vec(),
        "mock://error/oversized" => vec![b'x'; max_response_bytes as usize + 1],
        other if other.starts_with("mock://transport-error/") => {
            let message = other.trim_start_matches("mock://transport-error/");
            return Err(classify_transport_failure(endpoint, operation, message));
        }
        _ if operation == "eth_getLogs" => {
            if let Some((claim_id, amount, block_number)) = parse_mock_deposit_log_endpoint(endpoint)
            {
                mock_deposit_log_body(&claim_id, amount, block_number)
            } else {
                mock_success_body(operation, default_string_result)
            }
        }
        _ if operation == "eth_blockNumber" => {
            if let Some((_, _, block_number)) = parse_mock_deposit_log_endpoint(endpoint) {
                format!(r#"{{"jsonrpc":"2.0","id":1,"result":"0x{block_number:x}"}}"#).into_bytes()
            } else {
                mock_success_body(operation, default_string_result)
            }
        }
        _ => mock_success_body(operation, default_string_result),
    };

    ensure_response_size(endpoint, operation, &body, max_response_bytes)?;
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
pub async fn eth_block_number(endpoints: &[String]) -> Result<u64, FactoryError> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "eth_blockNumber",
        params: Vec::<String>::new(),
    };
    let request_body = build_request_body(&request)?;
    let mut last_error = None;

    for endpoint in endpoints {
        match rpc_request_once(
            endpoint,
            request_body.clone(),
            BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            "eth_blockNumber",
        )
        .await
        {
            Ok(body) => match parse_jsonrpc_result(
                endpoint,
                "eth_blockNumber",
                &body,
                BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            )
            .and_then(|value: String| parse_hex_u64(&value))
            {
                Ok(block_number) => return Ok(block_number),
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        rpc_request_failed(
            "<none>",
            "eth_blockNumber",
            RpcFailureCategory::Transport,
            None,
            "no RPC endpoints configured",
        )
    }))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn eth_block_number(endpoints: &[String]) -> Result<u64, FactoryError> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "eth_blockNumber",
        params: Vec::<String>::new(),
    };
    let request_body = build_request_body(&request)?;
    let mut last_error = None;

    for endpoint in endpoints {
        match rpc_request_once(
            endpoint,
            request_body.clone(),
            BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            "eth_blockNumber",
            None,
        ) {
            Ok(body) => match parse_jsonrpc_result(
                endpoint,
                "eth_blockNumber",
                &body,
                BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            )
            .and_then(|value: String| parse_hex_u64(&value))
            {
                Ok(block_number) => return Ok(block_number),
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        rpc_request_failed(
            "<none>",
            "eth_blockNumber",
            RpcFailureCategory::Transport,
            None,
            "no RPC endpoints configured",
        )
    }))
}

#[cfg(target_arch = "wasm32")]
pub async fn eth_get_deposited_logs(
    endpoints: &[String],
    contract_address: &str,
    plan: &PaymentScanPlan,
) -> Result<Vec<BaseDepositLog>, FactoryError> {
    let filter = EthGetLogsFilter {
        address: contract_address,
        from_block: format!("0x{:x}", plan.from_block),
        to_block: format!("0x{:x}", plan.to_block),
        topics: EthLogTopics(
            deposited_event_topic(),
            plan.claim_ids.iter().map(String::as_str).collect(),
        ),
    };
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "eth_getLogs",
        params: vec![filter],
    };
    let request_body = build_request_body(&request)?;
    let mut last_error = None;

    for endpoint in endpoints {
        match rpc_request_once(
            endpoint,
            request_body.clone(),
            BASE_RPC_LOG_SCAN_MAX_RESPONSE_BYTES,
            "eth_getLogs",
        )
        .await
        {
            Ok(body) => {
                match parse_deposit_logs(endpoint, &body, BASE_RPC_LOG_SCAN_MAX_RESPONSE_BYTES) {
                    Ok(logs) => return Ok(logs),
                    Err(error) => last_error = Some(error),
                }
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        rpc_request_failed(
            "<none>",
            "eth_getLogs",
            RpcFailureCategory::Transport,
            None,
            "no RPC endpoints configured",
        )
    }))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn eth_get_deposited_logs(
    endpoints: &[String],
    contract_address: &str,
    plan: &PaymentScanPlan,
) -> Result<Vec<BaseDepositLog>, FactoryError> {
    let filter = EthGetLogsFilter {
        address: contract_address,
        from_block: format!("0x{:x}", plan.from_block),
        to_block: format!("0x{:x}", plan.to_block),
        topics: EthLogTopics(
            deposited_event_topic(),
            plan.claim_ids.iter().map(String::as_str).collect(),
        ),
    };
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "eth_getLogs",
        params: vec![filter],
    };
    let request_body = build_request_body(&request)?;
    let mut last_error = None;

    for endpoint in endpoints {
        match rpc_request_once(
            endpoint,
            request_body.clone(),
            BASE_RPC_LOG_SCAN_MAX_RESPONSE_BYTES,
            "eth_getLogs",
            None,
        ) {
            Ok(body) => {
                match parse_deposit_logs(endpoint, &body, BASE_RPC_LOG_SCAN_MAX_RESPONSE_BYTES) {
                    Ok(logs) => return Ok(logs),
                    Err(error) => last_error = Some(error),
                }
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        rpc_request_failed(
            "<none>",
            "eth_getLogs",
            RpcFailureCategory::Transport,
            None,
            "no RPC endpoints configured",
        )
    }))
}

#[cfg(target_arch = "wasm32")]
pub async fn eth_send_raw_transaction(
    endpoints: &[String],
    raw_tx_hex: &str,
) -> Result<String, FactoryError> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "eth_sendRawTransaction",
        params: vec![raw_tx_hex],
    };
    let request_body = build_request_body(&request)?;
    let mut last_error = None;

    for endpoint in endpoints {
        match rpc_request_once(
            endpoint,
            request_body.clone(),
            BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            "eth_sendRawTransaction",
        )
        .await
        {
            Ok(body) => match parse_jsonrpc_result(
                endpoint,
                "eth_sendRawTransaction",
                &body,
                BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            ) {
                Ok(tx_hash) => return Ok(tx_hash),
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        rpc_request_failed(
            "<none>",
            "eth_sendRawTransaction",
            RpcFailureCategory::Transport,
            None,
            "no RPC endpoints configured",
        )
    }))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn eth_send_raw_transaction(
    endpoints: &[String],
    raw_tx_hex: &str,
    local_tx_hash: &str,
) -> Result<String, FactoryError> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "eth_sendRawTransaction",
        params: vec![raw_tx_hex],
    };
    let request_body = build_request_body(&request)?;
    let mut last_error = None;

    for endpoint in endpoints {
        match rpc_request_once(
            endpoint,
            request_body.clone(),
            BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            "eth_sendRawTransaction",
            Some(local_tx_hash),
        ) {
            Ok(body) => match parse_jsonrpc_result(
                endpoint,
                "eth_sendRawTransaction",
                &body,
                BASE_RPC_CONTROL_PLANE_MAX_RESPONSE_BYTES,
            ) {
                Ok(tx_hash) => return Ok(tx_hash),
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        rpc_request_failed(
            "<none>",
            "eth_sendRawTransaction",
            RpcFailureCategory::Transport,
            None,
            "no RPC endpoints configured",
        )
    }))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{
        configured_rpc_endpoints, eth_block_number, eth_get_deposited_logs,
        eth_send_raw_transaction, BaseDepositLog, PaymentScanPlan,
    };
    use crate::state::{restore_state, set_mock_canister_balance};
    use crate::types::{FactoryError, RpcFailureCategory};
    use crate::FactoryStateSnapshot;

    fn assert_rpc_request_failed(
        error: FactoryError,
        expected_operation: &str,
        expected_category: RpcFailureCategory,
        expected_code: Option<i64>,
    ) {
        match error {
            FactoryError::RpcRequestFailed {
                operation,
                category,
                code,
                ..
            } => {
                assert_eq!(operation, expected_operation);
                assert_eq!(category, expected_category);
                assert_eq!(code, expected_code);
            }
            other => panic!("expected rpc request failure, got {other:?}"),
        }
    }

    #[test]
    fn deduplicates_configured_rpc_endpoints() {
        let endpoints = configured_rpc_endpoints(
            Some(" https://base-primary.example ".to_string()),
            Some("https://base-primary.example".to_string()),
        );

        assert_eq!(endpoints, vec!["https://base-primary.example".to_string()]);
    }

    #[test]
    fn falls_back_to_secondary_endpoint_for_block_number() {
        let endpoints = vec![
            "mock://error/rate-limit".to_string(),
            "mock://success".to_string(),
        ];

        let block_number = eth_block_number(&endpoints).expect("fallback should succeed");
        assert_eq!(block_number, 0);
    }

    #[test]
    fn classifies_oversized_block_number_response() {
        let endpoints = vec!["mock://error/oversized".to_string()];

        let error = eth_block_number(&endpoints).expect_err("oversized response should fail");
        assert!(matches!(
            error,
            FactoryError::RpcRequestFailed {
                category: RpcFailureCategory::ResponseTooLarge,
                ..
            }
        ));
    }

    #[test]
    fn classifies_malformed_json_logs_response() {
        let endpoints = vec!["mock://error/malformed-json".to_string()];
        let plan = PaymentScanPlan {
            claim_ids: vec![
                "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            ],
            from_block: 1,
            to_block: 10,
        };

        let error = eth_get_deposited_logs(&endpoints, "0xEscrow", &plan)
            .expect_err("response should fail");
        assert!(matches!(
            error,
            FactoryError::RpcRequestFailed {
                category: RpcFailureCategory::MalformedResponse,
                ..
            }
        ));
    }

    #[test]
    fn classifies_rate_limited_rpc_errors() {
        let endpoints = vec!["mock://error/rate-limit".to_string()];

        let error = eth_block_number(&endpoints).expect_err("rate limit should fail");
        assert!(matches!(
            error,
            FactoryError::RpcRequestFailed {
                category: RpcFailureCategory::RateLimited,
                ..
            }
        ));
    }

    #[test]
    fn classifies_unavailable_rpc_errors() {
        let endpoints = vec!["mock://error/upstream-unavailable".to_string()];

        let error = eth_block_number(&endpoints).expect_err("unavailable upstream should fail");
        assert!(matches!(
            error,
            FactoryError::RpcRequestFailed {
                category: RpcFailureCategory::Unavailable,
                ..
            }
        ));
    }

    #[test]
    fn classifies_raw_transaction_broadcast_failure_modes() {
        for (endpoint, expected_category, expected_code) in [
            (
                "mock://error/rate-limit",
                RpcFailureCategory::RateLimited,
                Some(429),
            ),
            (
                "mock://error/upstream-unavailable",
                RpcFailureCategory::Unavailable,
                Some(-32000),
            ),
            (
                "mock://error/malformed-json",
                RpcFailureCategory::MalformedResponse,
                None,
            ),
            (
                "mock://error/oversized",
                RpcFailureCategory::ResponseTooLarge,
                None,
            ),
        ] {
            let endpoints = vec![endpoint.to_string()];
            let error = eth_send_raw_transaction(&endpoints, "0xdeadbeef", "0xabc")
                .expect_err("broadcast should fail");
            assert_rpc_request_failed(
                error,
                "eth_sendRawTransaction",
                expected_category,
                expected_code,
            );
        }
    }

    #[test]
    fn rejects_http_outcalls_when_cycles_are_not_affordable() {
        restore_state(FactoryStateSnapshot::default());
        set_mock_canister_balance(0);

        let endpoints = vec!["mock://success".to_string()];
        let error = eth_block_number(&endpoints).expect_err("outcall should fail early");
        assert!(matches!(
            error,
            FactoryError::InsufficientCyclesForOperation { ref operation, .. }
                if operation == "http_request:eth_blockNumber"
        ));
        set_mock_canister_balance(u128::MAX);
    }

    #[test]
    fn parses_deposit_logs_from_typed_response() {
        let endpoints = vec!["mock://success/deposit-log".to_string()];
        let plan = PaymentScanPlan {
            claim_ids: vec![
                "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            ],
            from_block: 1,
            to_block: 100,
        };

        let logs =
            eth_get_deposited_logs(&endpoints, "0xEscrow", &plan).expect("typed logs should parse");
        assert_eq!(
            logs,
            vec![BaseDepositLog {
                claim_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
                amount: "60000000".to_string(),
                block_number: 42,
            }]
        );
    }

    #[test]
    fn falls_back_to_secondary_endpoint_for_raw_transaction_broadcast() {
        let endpoints = vec![
            "mock://transport-error/upstream unavailable".to_string(),
            "mock://success".to_string(),
        ];

        let tx_hash = eth_send_raw_transaction(&endpoints, "0xdeadbeef", "0xabc")
            .expect("fallback broadcast should succeed");
        assert_eq!(tx_hash, "0xabc");
    }
}
