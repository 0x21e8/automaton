/// EVM chain interaction — RPC, event polling, wallet balance, and transaction signing/broadcast.
///
/// Provides:
/// - `HttpEvmPoller` / `EvmPoller` — polls `MessageQueued` log events from the inbox contract.
/// - `HttpEvmRpcClient` — thin JSON-RPC client over IC HTTPS outcalls (with optional fallback URL).
/// - `EvmBroadcaster` / `EvmPoller` mock implementations for unit tests.
/// - EIP-1559 transaction encoding and signature helpers.
/// - Failure classification via `classify_evm_failure`.
///
/// All outbound HTTP calls are pre-checked with the cycle affordability guard to avoid
/// wasting the outcall fee on an operation the canister cannot afford.
// ── Imports ──────────────────────────────────────────────────────────────────
use crate::domain::cycle_admission::{
    affordability_requirements, can_afford, estimate_operation_cost, OperationClass,
    DEFAULT_SAFETY_MARGIN_BPS,
};
use crate::domain::types::{
    EvmEvent, EvmPollCursor, EvmStewardProof, ExecutionPlan, OperationFailure,
    OperationFailureKind, OutcallFailure, OutcallFailureKind, RecoveryFailure, RuntimeSnapshot,
    StewardState, StrategyExecutionCall, StrategyOutcomeEvent, StrategyOutcomeKind,
};
use crate::storage::stable;
use crate::timing::current_time_ns;
use crate::tools::SignerPort;
use crate::util::{normalize_evm_address, normalize_hex_blob};
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_rlp::{length_of_length, BufMut, Encodable, Header};
use async_trait::async_trait;
use canlog::{log, GetLogFilter, LogFilter, LogPriorityLevels};
use serde::Deserialize;
use serde_json::{json, Value};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;
use std::str::FromStr;

#[cfg(target_arch = "wasm32")]
use candid::Nat;
#[cfg(target_arch = "wasm32")]
use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs};
use sha3::{Digest, Keccak256};

// ── Constants ────────────────────────────────────────────────────────────────

/// Hard cap on EVM RPC response bodies — 2 MiB.
/// Responses larger than this are rejected to prevent excessive cycle spend.
const MAX_EVM_RPC_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

/// Maximum block range requested per `eth_getLogs` poll.
/// Larger ranges risk hitting RPC node limits and very large response bodies.
const MAX_BLOCK_RANGE_PER_POLL: u64 = 1_000;

/// Maximum number of response bytes included in non-2xx HTTP error previews.
const HTTP_ERROR_BODY_PREVIEW_BYTES: usize = 256;

// Maximum number of decoded log events returned per poll; prevents unbounded memory use.
const DEFAULT_MAX_LOGS_PER_POLL: usize = 200;

// If cursor lag exceeds this threshold, skip historical backfill and fast-forward to the
// recent head window so inbox responsiveness recovers for active users.
const MAX_CURSOR_LAG_BEFORE_FAST_FORWARD_BLOCKS: u64 = MAX_BLOCK_RANGE_PER_POLL * 20;

// RLP encoding length of an empty EIP-2930 access list `[]`.
const EMPTY_ACCESS_LIST_RLP_LEN: usize = 1;

// Smaller response budget for control-plane calls (block number, nonce, gas price, …).
const CONTROL_PLANE_MAX_RESPONSE_BYTES: u64 = 4 * 1024;

const INBOX_MESSAGE_QUEUED_EVENT_SIGNATURE: &str =
    "MessageQueued(address,uint64,address,string,uint256,uint256)";
const INBOX_USDC_FUNCTION_SIGNATURE: &str = "usdc()";
const ERC20_BALANCE_OF_FUNCTION_SIGNATURE: &str = "balanceOf(address)";
const EIP1271_IS_VALID_SIGNATURE_FUNCTION_SIGNATURE: &str = "isValidSignature(bytes32,bytes)";
const EIP1271_IS_VALID_SIGNATURE_MAGIC_VALUE: &str = "1626ba7e";
const EVM_STEWARD_SIGNING_DOMAIN: &str = "ic-automaton:steward-execute:v1";

// Environment variable that switches the host (non-wasm32) RPC client from stub to real mode.
#[cfg(not(target_arch = "wasm32"))]
const HOST_EVM_RPC_MODE_ENV: &str = "IC_AUTOMATON_EVM_RPC_HOST_MODE";
#[cfg(not(target_arch = "wasm32"))]
const HOST_EVM_RPC_STUB_FORCE_STATUS_ENV: &str = "IC_AUTOMATON_EVM_RPC_STUB_FORCE_STATUS";
#[cfg(not(target_arch = "wasm32"))]
const HOST_EVM_RPC_STUB_FORCE_BODY_ENV: &str = "IC_AUTOMATON_EVM_RPC_STUB_FORCE_BODY";
#[cfg(not(target_arch = "wasm32"))]
const HOST_EVM_RPC_STUB_MAX_LOG_BLOCK_SPAN_ENV: &str =
    "IC_AUTOMATON_EVM_RPC_STUB_MAX_LOG_BLOCK_SPAN";
#[cfg(not(target_arch = "wasm32"))]
const HOST_EIP1271_STUB_CONTRACT_ADDRESS: &str = "0x8f61f9a23b6bad39a85623fff41cbee5b4dbee2c";
#[cfg(not(target_arch = "wasm32"))]
const HOST_EIP1271_STUB_DELEGATED_CODE: &str = "0xef010063c0c19a282a1b52b07dd5a65b58948a07dae32b";

#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum StrategyExecutionLogPriority {
    #[log_level(capacity = 2_000, name = "STRATEGY_EXEC_INFO")]
    Info,
    #[log_level(capacity = 500, name = "STRATEGY_EXEC_ERROR")]
    Error,
}

impl GetLogFilter for StrategyExecutionLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

// ── Log event decoding ───────────────────────────────────────────────────────

/// ABI-decoded fields from a `MessageQueued` log event payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedMessageQueuedPayload {
    /// EVM sender address (checksummed lowercase hex).
    pub sender: String,
    /// UTF-8 message string encoded in the event.
    pub message: String,
    /// USDC amount attached to the message (6-decimal raw units).
    pub usdc_amount: U256,
    /// ETH amount attached to the message (wei).
    pub eth_amount: U256,
}

/// Decode a raw `MessageQueued` event data blob into its constituent fields.
///
/// `payload_hex` is the `data` field of the log entry (0x-prefixed hex).
/// Returns an error if the payload is too short or contains invalid encoding.
pub fn decode_message_queued_payload(
    payload_hex: &str,
) -> Result<DecodedMessageQueuedPayload, String> {
    let payload = normalize_hex_blob(payload_hex, "message queued payload")?;
    let bytes = hex::decode(payload.trim_start_matches("0x"))
        .map_err(|error| format!("failed to decode message queued payload: {error}"))?;
    if bytes.len() < 128 {
        return Err("message queued payload must be at least 128 bytes".to_string());
    }

    let sender = format!("0x{}", hex::encode(&bytes[12..32]));
    let sender = normalize_evm_address(&sender)?;
    let message_offset = read_usize_word(&bytes[32..64], "message offset")?;
    let usdc_amount = U256::from_be_slice(&bytes[64..96]);
    let eth_amount = U256::from_be_slice(&bytes[96..128]);
    if message_offset.saturating_add(32) > bytes.len() {
        return Err("message offset points outside payload".to_string());
    }
    let message_len = read_usize_word(
        &bytes[message_offset..message_offset + 32],
        "message length",
    )?;
    let message_start = message_offset + 32;
    let message_end = message_start.saturating_add(message_len);
    if message_end > bytes.len() {
        return Err("message bytes exceed payload length".to_string());
    }

    let message = std::str::from_utf8(&bytes[message_start..message_end])
        .map_err(|error| format!("message payload is not utf-8: {error}"))?
        .to_string();

    Ok(DecodedMessageQueuedPayload {
        sender,
        message,
        usdc_amount,
        eth_amount,
    })
}

fn read_usize_word(word: &[u8], field: &str) -> Result<usize, String> {
    if word.len() != 32 {
        return Err(format!("{field} word must be 32 bytes"));
    }
    let size = std::mem::size_of::<usize>();
    if word[..(32 - size)].iter().any(|byte| *byte != 0) {
        return Err(format!("{field} overflowed usize"));
    }
    let mut value = 0usize;
    for byte in &word[(32 - size)..] {
        value = (value << 8) | usize::from(*byte);
    }
    Ok(value)
}

// ── Polling types ────────────────────────────────────────────────────────────

/// Output of a single `EvmPoller::poll` call.
pub struct EvmPollResult {
    /// Updated cursor to persist after processing `events`.
    pub cursor: EvmPollCursor,
    /// Decoded inbox events ordered by (block_number, log_index).
    pub events: Vec<EvmEvent>,
}

/// Result of broadcasting a signed transaction to the EVM network.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct EvmBroadcastResult {
    pub tx_hash: String,
}

/// Trait for broadcasting pre-signed raw transactions.
#[allow(dead_code)]
pub trait EvmBroadcaster {
    fn broadcast(&self, signed_transaction: &str) -> Result<EvmBroadcastResult, String>;
}

/// Trait for polling EVM log events starting from a persisted cursor.
#[async_trait(?Send)]
pub trait EvmPoller {
    async fn poll(&self, cursor: &EvmPollCursor) -> Result<EvmPollResult, String>;
}

/// Production `EvmPoller` that fetches logs via IC HTTPS outcalls to an EVM JSON-RPC endpoint.
#[derive(Clone, Debug)]
pub struct HttpEvmPoller {
    rpc: HttpEvmRpcClient,
    max_logs_per_poll: usize,
    bootstrap_lookback_blocks: u64,
    log_filter_address: String,
    log_filter_topics: Vec<String>,
}

impl HttpEvmPoller {
    /// Construct from the current `RuntimeSnapshot`.
    ///
    /// Resolves the inbox contract address and derives the agent's EVM address
    /// topic for the `eth_getLogs` filter.
    pub fn from_snapshot(snapshot: &RuntimeSnapshot) -> Result<Self, String> {
        let rpc = HttpEvmRpcClient::from_snapshot(snapshot)?;
        let log_filter_address = snapshot
            .evm_cursor
            .contract_address
            .as_deref()
            .or(snapshot.inbox_contract_address.as_deref())
            .ok_or_else(|| "inbox contract address is not configured".to_string())
            .and_then(normalize_evm_address)?;
        let log_filter_topic1 = match snapshot.evm_cursor.automaton_address_topic.as_deref() {
            Some(topic) => normalize_topic(topic, "automaton address topic")?,
            None => {
                let agent_evm_address = snapshot
                    .evm_address
                    .as_deref()
                    .ok_or_else(|| "evm address is not configured".to_string())?;
                address_to_topic(agent_evm_address)?
            }
        };
        Ok(Self {
            rpc,
            max_logs_per_poll: DEFAULT_MAX_LOGS_PER_POLL,
            bootstrap_lookback_blocks: snapshot.evm_bootstrap_lookback_blocks,
            log_filter_address,
            log_filter_topics: vec![inbox_message_queued_topic0(), log_filter_topic1],
        })
    }
}

#[async_trait(?Send)]
impl EvmPoller for HttpEvmPoller {
    async fn poll(&self, cursor: &EvmPollCursor) -> Result<EvmPollResult, String> {
        let latest_block = self.rpc.eth_block_number().await?;
        let confirmed_head = latest_block.saturating_sub(cursor.confirmation_depth);
        let from_block =
            effective_from_block(cursor, confirmed_head, self.bootstrap_lookback_blocks);
        if from_block != cursor.next_block {
            log!(
                StrategyExecutionLogPriority::Info,
                "evm_poll_from_block_adjusted previous_next_block={} effective_from_block={} confirmed_head={}",
                cursor.next_block,
                from_block,
                confirmed_head
            );
        }
        let to_block = confirmed_head.min(from_block.saturating_add(MAX_BLOCK_RANGE_PER_POLL));

        if from_block > to_block {
            return Ok(EvmPollResult {
                cursor: cursor.clone(),
                events: Vec::new(),
            });
        }

        let logs = self
            .rpc
            .eth_get_logs(
                from_block,
                to_block,
                Some(self.log_filter_address.as_str()),
                Some(self.log_filter_topics.as_slice()),
                self.rpc.max_response_bytes,
            )
            .await?;

        let events = filter_route_matched_logs(
            logs,
            cursor,
            self.log_filter_address.as_str(),
            self.log_filter_topics
                .get(1)
                .map(String::as_str)
                .ok_or_else(|| "log filter topic1 is missing".to_string())?,
            self.max_logs_per_poll,
        )?;

        let (next_block, next_log_index) = if let Some(last) = events.last() {
            if events.len() == self.max_logs_per_poll {
                (last.block_number, last.log_index.saturating_add(1))
            } else {
                (to_block.saturating_add(1), 0)
            }
        } else {
            (to_block.saturating_add(1), 0)
        };

        Ok(EvmPollResult {
            cursor: EvmPollCursor {
                chain_id: cursor.chain_id,
                contract_address: cursor.contract_address.clone(),
                automaton_address_topic: cursor.automaton_address_topic.clone(),
                next_block,
                next_log_index,
                confirmation_depth: cursor.confirmation_depth,
                last_poll_at_ns: cursor.last_poll_at_ns,
                consecutive_empty_polls: cursor.consecutive_empty_polls,
            },
            events,
        })
    }
}

fn effective_from_block(
    cursor: &EvmPollCursor,
    confirmed_head: u64,
    bootstrap_lookback_blocks: u64,
) -> u64 {
    if cursor.next_block == 0 {
        return confirmed_head.saturating_sub(bootstrap_lookback_blocks);
    }

    let lag = confirmed_head.saturating_sub(cursor.next_block);
    if lag > MAX_CURSOR_LAG_BEFORE_FAST_FORWARD_BLOCKS {
        return confirmed_head.saturating_sub(bootstrap_lookback_blocks);
    }

    cursor.next_block
}

#[allow(dead_code)]
pub struct MockEvmPoller;

#[async_trait(?Send)]
impl EvmPoller for MockEvmPoller {
    async fn poll(&self, cursor: &EvmPollCursor) -> Result<EvmPollResult, String> {
        let next_block = cursor.next_block.saturating_add(1);
        let next_log_index = cursor.next_log_index.saturating_add(1);

        let events = vec![EvmEvent {
            tx_hash: format!("0x{:0>64}", next_log_index),
            chain_id: cursor.chain_id,
            block_number: next_block,
            log_index: next_log_index,
            source: "mock_chain".to_string(),
            payload: "agent.heartbeat".to_string(),
        }];

        Ok(EvmPollResult {
            cursor: EvmPollCursor {
                chain_id: cursor.chain_id,
                contract_address: cursor.contract_address.clone(),
                automaton_address_topic: cursor.automaton_address_topic.clone(),
                next_block,
                next_log_index,
                confirmation_depth: cursor.confirmation_depth,
                last_poll_at_ns: cursor.last_poll_at_ns,
                consecutive_empty_polls: cursor.consecutive_empty_polls,
            },
            events,
        })
    }
}

// ── Mock implementations (test/stub) ────────────────────────────────────────

/// In-memory broadcaster stub — returns a synthetic tx hash for unit tests.
#[allow(dead_code)]
pub struct MockEvmBroadcaster;

impl EvmBroadcaster for MockEvmBroadcaster {
    fn broadcast(&self, signed_transaction: &str) -> Result<EvmBroadcastResult, String> {
        Ok(EvmBroadcastResult {
            tx_hash: format!("0x{signed_transaction}-mock"),
        })
    }
}

/// Immutable inputs required to verify one signed steward proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvmStewardVerificationContext {
    pub canister_id: String,
    pub active_steward: Option<StewardState>,
    pub expected_nonce: u64,
    pub expected_command_hash: String,
    pub now_ns: u64,
}

/// Normalized steward proof fields returned after successful verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedEvmStewardProof {
    pub canister_id: String,
    pub chain_id: u64,
    pub address: String,
    pub command_hash: String,
    pub nonce: u64,
    pub expires_at_ns: u64,
}

// ── RPC client ───────────────────────────────────────────────────────────────

/// JSON-RPC client that sends requests via IC HTTPS outcalls (wasm32) or ureq (host builds).
///
/// Supports an optional `fallback_rpc_url`: if the primary endpoint fails, the
/// same request is automatically retried against the fallback before propagating
/// the error.
#[derive(Clone, Debug)]
pub struct HttpEvmRpcClient {
    rpc_url: String,
    fallback_rpc_url: Option<String>,
    max_response_bytes: u64,
}

impl HttpEvmRpcClient {
    /// Construct from the current `RuntimeSnapshot`.  Returns an error if the
    /// primary RPC URL is not configured.
    pub fn from_snapshot(snapshot: &RuntimeSnapshot) -> Result<Self, String> {
        let rpc_url = snapshot.evm_rpc_url.trim();
        if rpc_url.is_empty() {
            return Err("evm rpc url is not configured".to_string());
        }
        Ok(Self {
            rpc_url: rpc_url.to_string(),
            fallback_rpc_url: snapshot.evm_rpc_fallback_url.clone(),
            max_response_bytes: clamp_response_bytes(snapshot.evm_rpc_max_response_bytes),
        })
    }

    fn control_plane_max_response_bytes(&self) -> u64 {
        CONTROL_PLANE_MAX_RESPONSE_BYTES
    }

    async fn eth_block_number(&self) -> Result<u64, String> {
        let response = self
            .rpc_call(
                "eth_blockNumber",
                json!([]),
                self.control_plane_max_response_bytes(),
            )
            .await
            .map_err(|error| format!("eth_blockNumber failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_blockNumber result was missing".to_string())?;
        parse_hex_u64(raw, "eth_blockNumber")
    }

    async fn eth_get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        address_filter: Option<&str>,
        topics_filter: Option<&[String]>,
        max_response_bytes: u64,
    ) -> Result<Vec<RpcLog>, String> {
        let filter = build_eth_get_logs_filter(from_block, to_block, address_filter, topics_filter);
        let response = self
            .rpc_call(
                "eth_getLogs",
                Value::Array(vec![Value::Object(filter)]),
                max_response_bytes,
            )
            .await
            .map_err(|error| {
                format!("eth_getLogs failed for range 0x{from_block:x}..0x{to_block:x}: {error}")
            })?;

        let raw_logs = response
            .get("result")
            .cloned()
            .ok_or_else(|| "eth_getLogs result was missing".to_string())?;
        serde_json::from_value::<Vec<RpcLog>>(raw_logs)
            .map_err(|error| format!("failed to decode eth_getLogs result: {error}"))
    }

    /// Query the ETH balance of `address` at the latest block.
    /// Returns the balance as a 0x-prefixed hex quantity string (wei).
    pub async fn eth_get_balance(&self, address: &str) -> Result<String, String> {
        self.eth_get_balance_with_limit(address, self.control_plane_max_response_bytes())
            .await
    }

    /// Like `eth_get_balance` but with a caller-supplied response byte cap.
    pub async fn eth_get_balance_with_limit(
        &self,
        address: &str,
        max_response_bytes: u64,
    ) -> Result<String, String> {
        let response = self
            .rpc_call(
                "eth_getBalance",
                json!([address, "latest"]),
                max_response_bytes,
            )
            .await
            .map_err(|error| format!("eth_getBalance failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_getBalance result was missing".to_string())?;
        normalize_hex_quantity(raw, "eth_getBalance result")
    }

    /// Fetch deployed runtime bytecode for `address` at the latest block.
    /// Returns a 0x-prefixed hex blob; EOAs typically return `0x`.
    pub async fn eth_get_code(&self, address: &str) -> Result<String, String> {
        let response = self
            .rpc_call(
                "eth_getCode",
                json!([address, "latest"]),
                self.control_plane_max_response_bytes(),
            )
            .await
            .map_err(|error| format!("eth_getCode failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_getCode result was missing".to_string())?;
        normalize_hex_blob(raw, "eth_getCode result")
    }

    /// Read-only contract call (`eth_call`) at the latest block.
    /// Returns the raw ABI-encoded result as a 0x-prefixed hex string.
    pub async fn eth_call(&self, address: &str, calldata: &str) -> Result<String, String> {
        self.eth_call_with_limit(address, calldata, self.control_plane_max_response_bytes())
            .await
    }

    /// Like `eth_call` but with a caller-supplied response byte cap.
    pub async fn eth_call_with_limit(
        &self,
        address: &str,
        calldata: &str,
        max_response_bytes: u64,
    ) -> Result<String, String> {
        let response = self
            .rpc_call(
                "eth_call",
                json!([{"to": address, "data": calldata}, "latest"]),
                max_response_bytes,
            )
            .await
            .map_err(|error| format!("eth_call failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_call result was missing".to_string())?;
        normalize_hex_blob(raw, "eth_call result")
    }

    /// Fetch the pending nonce for `address` (`eth_getTransactionCount` with `"pending"`).
    pub async fn eth_get_transaction_count(&self, address: &str) -> Result<u64, String> {
        let response = self
            .rpc_call(
                "eth_getTransactionCount",
                json!([address, "pending"]),
                self.control_plane_max_response_bytes(),
            )
            .await
            .map_err(|error| format!("eth_getTransactionCount failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_getTransactionCount result was missing".to_string())?;
        parse_hex_u64(raw, "eth_getTransactionCount")
    }

    /// Fetch the current base gas price in wei.
    pub async fn eth_gas_price(&self) -> Result<U256, String> {
        let response = self
            .rpc_call(
                "eth_gasPrice",
                json!([]),
                self.control_plane_max_response_bytes(),
            )
            .await
            .map_err(|error| format!("eth_gasPrice failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_gasPrice result was missing".to_string())?;
        parse_hex_u256(raw, "eth_gasPrice")
    }

    /// Estimate gas for a transaction — used when building EIP-1559 envelopes.
    pub async fn eth_estimate_gas(
        &self,
        from: &str,
        to: &str,
        value_wei: U256,
        data_hex: &str,
    ) -> Result<u64, String> {
        let value_hex = format!("0x{:x}", value_wei);
        let response = self
            .rpc_call(
                "eth_estimateGas",
                json!([{
                    "from": from,
                    "to": to,
                    "value": value_hex,
                    "data": data_hex
                }]),
                self.control_plane_max_response_bytes(),
            )
            .await
            .map_err(|error| format!("eth_estimateGas failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_estimateGas result was missing".to_string())?;
        parse_hex_u64(raw, "eth_estimateGas")
    }

    /// Broadcast a fully-signed EIP-1559 transaction.
    /// Returns the transaction hash as a 0x-prefixed hex string.
    pub async fn eth_send_raw_transaction(&self, raw_tx: &[u8]) -> Result<String, String> {
        let payload = format!("0x{}", hex::encode(raw_tx));
        let response = self
            .rpc_call(
                "eth_sendRawTransaction",
                json!([payload]),
                self.control_plane_max_response_bytes(),
            )
            .await
            .map_err(|error| format!("eth_sendRawTransaction failed: {error}"))?;
        let raw = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_sendRawTransaction result was missing".to_string())?;
        normalize_hex_blob(raw, "eth_sendRawTransaction result")
    }

    /// Generic JSON-RPC call — used by the `EvmPort` adapter in `cycle_topup_host`.
    /// `params_json` must be a valid JSON array string.
    pub async fn json_rpc_call(&self, method: &str, params_json: &str) -> Result<String, String> {
        let params: Value = serde_json::from_str(params_json)
            .map_err(|error| format!("invalid {method} params json: {error}"))?;
        let response = self
            .rpc_call(method, params, self.max_response_bytes)
            .await?;
        serde_json::to_string(&response)
            .map_err(|error| format!("failed to serialize {method} response: {error}"))
    }

    async fn rpc_call(
        &self,
        method: &str,
        params: Value,
        max_response_bytes: u64,
    ) -> Result<Value, String> {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        }))
        .map_err(|error| format!("failed to serialize {method} request: {error}"))?;

        let request_size_bytes = u64::try_from(body.len()).unwrap_or(u64::MAX);
        ensure_http_affordable(request_size_bytes, max_response_bytes)?;

        let raw = self.http_post(&body, max_response_bytes).await?;
        let value: Value = serde_json::from_slice(&raw)
            .map_err(|error| format!("failed to parse {method} response JSON: {error}"))?;
        if let Some(error) = value.get("error") {
            return Err(format_rpc_error(method, error));
        }
        Ok(value)
    }

    async fn http_post(&self, body: &[u8], max_response_bytes: u64) -> Result<Vec<u8>, String> {
        let normalized_max = clamp_response_bytes(max_response_bytes);
        match self
            .try_http_post(&self.rpc_url, body, normalized_max)
            .await
        {
            Ok(body) => Ok(body),
            Err(primary_error) => {
                if let Some(fallback_url) = self.fallback_rpc_url.as_deref() {
                    self.try_http_post(fallback_url, body, normalized_max)
                        .await
                        .map_err(|fallback_error| {
                            format!(
                                "primary rpc failed: {primary_error}; fallback rpc failed: {fallback_error}"
                            )
                        })
                } else {
                    Err(primary_error)
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    async fn try_http_post(
        &self,
        url: &str,
        body: &[u8],
        max_response_bytes: u64,
    ) -> Result<Vec<u8>, String> {
        let request = HttpRequestArgs {
            url: url.to_string(),
            max_response_bytes: Some(max_response_bytes),
            method: HttpMethod::POST,
            headers: vec![HttpHeader {
                name: "content-type".to_string(),
                value: "application/json".to_string(),
            }],
            body: Some(body.to_vec()),
            transform: None,
            is_replicated: Some(false),
        };

        let response = http_request(&request)
            .await
            .map_err(|error| format!("evm rpc outcall failed: {error}"))?;
        let status = nat_to_u16(&response.status)?;
        if !(200..300).contains(&status) {
            return Err(format_http_status_error(status, &response.body));
        }
        Ok(response.body)
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn try_http_post(
        &self,
        url: &str,
        body: &[u8],
        max_response_bytes: u64,
    ) -> Result<Vec<u8>, String> {
        if !host_rpc_real_mode_enabled() {
            return host_rpc_stub_response(body);
        }

        let normalized_max = clamp_response_bytes(max_response_bytes);
        let response = ureq::post(url)
            .set("content-type", "application/json")
            .send_bytes(body)
            .map_err(|error| match error {
                ureq::Error::Status(status, response) => {
                    let mut preview = Vec::new();
                    let _ = response
                        .into_reader()
                        .take(HTTP_ERROR_BODY_PREVIEW_BYTES as u64)
                        .read_to_end(&mut preview);
                    format_http_status_error(status, &preview)
                }
                ureq::Error::Transport(transport) => {
                    format!("evm rpc host transport failed: {transport}")
                }
            })?;

        let mut raw = Vec::new();
        response
            .into_reader()
            .take(normalized_max.saturating_add(1))
            .read_to_end(&mut raw)
            .map_err(|error| format!("failed to read host rpc response body: {error}"))?;
        if u64::try_from(raw.len()).unwrap_or(u64::MAX) > normalized_max {
            return Err(format!(
                "host rpc response exceeded max_response_bytes={normalized_max}"
            ));
        }
        Ok(raw)
    }
}

fn build_eth_get_logs_filter(
    from_block: u64,
    to_block: u64,
    address_filter: Option<&str>,
    topics_filter: Option<&[String]>,
) -> serde_json::Map<String, Value> {
    let mut filter = serde_json::Map::new();
    filter.insert(
        "fromBlock".to_string(),
        Value::String(format!("0x{from_block:x}")),
    );
    filter.insert(
        "toBlock".to_string(),
        Value::String(format!("0x{to_block:x}")),
    );
    if let Some(address) = address_filter {
        filter.insert("address".to_string(), Value::String(address.to_string()));
    }
    if let Some(topics) = topics_filter {
        filter.insert(
            "topics".to_string(),
            Value::Array(
                topics
                    .iter()
                    .map(|topic| Value::String(topic.clone()))
                    .collect(),
            ),
        );
    }
    filter
}

fn format_rpc_error(method: &str, error: &Value) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let message = error.get("message").and_then(Value::as_str);
    match (code, message) {
        (Some(code), Some(message)) => {
            format!("rpc returned error for {method}: code={code} message={message}")
        }
        (Some(code), None) => format!("rpc returned error for {method}: code={code}"),
        _ => format!("rpc returned error for {method}: {error}"),
    }
}

fn format_http_status_error(status: u16, body: &[u8]) -> String {
    let preview = http_error_body_preview(body);
    if preview.is_empty() {
        return format!("evm rpc returned status {status}");
    }
    format!("evm rpc returned status {status}: {preview}")
}

fn http_error_body_preview(body: &[u8]) -> String {
    if body.is_empty() {
        return String::new();
    }
    let preview_len = body.len().min(HTTP_ERROR_BODY_PREVIEW_BYTES);
    let preview = String::from_utf8_lossy(&body[..preview_len]).replace(['\r', '\n'], " ");
    let preview = preview.trim().to_string();
    if preview.is_empty() {
        return String::new();
    }
    if body.len() > preview_len {
        return format!("{preview}...");
    }
    preview
}

#[cfg(target_arch = "wasm32")]
fn nat_to_u16(status: &Nat) -> Result<u16, String> {
    status
        .to_string()
        .parse::<u16>()
        .map_err(|error| format!("invalid HTTP status {status}: {error}"))
}

fn clamp_response_bytes(max_response_bytes: u64) -> u64 {
    max_response_bytes.clamp(256, MAX_EVM_RPC_RESPONSE_BYTES)
}

fn ensure_http_affordable(request_size_bytes: u64, max_response_bytes: u64) -> Result<(), String> {
    let operation = OperationClass::HttpOutcall {
        request_size_bytes,
        max_response_bytes: clamp_response_bytes(max_response_bytes),
    };
    let estimated = estimate_operation_cost(&operation)?;
    let requirements = affordability_requirements(
        estimated,
        DEFAULT_SAFETY_MARGIN_BPS,
        stable::operation_reserve_floor_cycles(),
    );
    let liquid = liquid_cycle_balance();
    if !can_afford(liquid, &requirements) {
        return Err(format!(
            "insufficient cycles for evm rpc outcall: need {} liquid, have {}",
            requirements.required_cycles, liquid
        ));
    }
    Ok(())
}

// ── Failure classification ───────────────────────────────────────────────────

/// Map a raw EVM RPC error string to a structured `RecoveryFailure`.
///
/// The agent uses this to decide retry strategy: transient network errors are
/// retried immediately, policy/config errors surface to the operator.
#[allow(dead_code)]
pub fn classify_evm_failure(error: &str) -> RecoveryFailure {
    let normalized = error.to_ascii_lowercase();
    if indicates_insufficient_cycles_error(&normalized) {
        return RecoveryFailure::Operation(OperationFailure {
            kind: OperationFailureKind::InsufficientCycles,
        });
    }
    if normalized.contains("is not configured") {
        return RecoveryFailure::Operation(OperationFailure {
            kind: OperationFailureKind::MissingConfiguration,
        });
    }
    if normalized.contains("must be 0x-prefixed")
        || normalized.contains("must be valid hex")
        || normalized.contains("must be a 32-byte topic")
        || normalized.contains("must be one of")
    {
        return RecoveryFailure::Operation(OperationFailure {
            kind: OperationFailureKind::InvalidConfiguration,
        });
    }
    RecoveryFailure::Outcall(OutcallFailure {
        kind: classify_evm_outcall_failure_kind(&normalized),
        retry_after_secs: None,
        observed_response_bytes: None,
    })
}

#[allow(dead_code)]
fn classify_evm_outcall_failure_kind(normalized_error: &str) -> OutcallFailureKind {
    if normalized_error.contains("code=-32011") {
        return OutcallFailureKind::UpstreamUnavailable;
    }
    if normalized_error.contains("code=-32005") {
        return OutcallFailureKind::RateLimited;
    }
    if normalized_error.contains("code=-32600")
        || normalized_error.contains("code=-32601")
        || normalized_error.contains("code=-32602")
    {
        return OutcallFailureKind::InvalidRequest;
    }
    if normalized_error.contains("http body exceeds size limit")
        || normalized_error.contains("header size exceeds specified response size limit")
        || normalized_error.contains("response exceeded max_response_bytes")
        || (normalized_error.contains("max_response_bytes") && normalized_error.contains("exceed"))
    {
        return OutcallFailureKind::ResponseTooLarge;
    }
    if normalized_error.contains("status 429")
        || normalized_error.contains("http 429")
        || normalized_error.contains("rate limit")
        || normalized_error.contains("too many requests")
    {
        return OutcallFailureKind::RateLimited;
    }
    if normalized_error.contains("timeout")
        || normalized_error.contains("timed out")
        || normalized_error.contains("deadline exceeded")
    {
        return OutcallFailureKind::Timeout;
    }
    if normalized_error.contains("status 503")
        || normalized_error.contains("status 502")
        || normalized_error.contains("status 504")
        || normalized_error.contains("http 503")
        || normalized_error.contains("http 502")
        || normalized_error.contains("http 504")
        || normalized_error.contains("service unavailable")
    {
        return OutcallFailureKind::UpstreamUnavailable;
    }
    if normalized_error.contains("status 401")
        || normalized_error.contains("status 403")
        || normalized_error.contains("http 401")
        || normalized_error.contains("http 403")
        || normalized_error.contains("forbidden")
        || normalized_error.contains("rejected by policy")
    {
        return OutcallFailureKind::RejectedByPolicy;
    }
    if normalized_error.contains("status 400")
        || normalized_error.contains("status 404")
        || normalized_error.contains("status 422")
        || normalized_error.contains("http 400")
        || normalized_error.contains("http 404")
        || normalized_error.contains("http 422")
        || normalized_error.contains("rpc returned error")
    {
        return OutcallFailureKind::InvalidRequest;
    }
    if normalized_error.contains("failed to parse")
        || normalized_error.contains("result was missing")
        || normalized_error.contains("response decode failed")
        || normalized_error.contains("response was not valid utf-8")
    {
        return OutcallFailureKind::InvalidResponse;
    }
    if normalized_error.contains("transport failed")
        || normalized_error.contains("connection reset")
        || normalized_error.contains("connection refused")
        || normalized_error.contains("network is unreachable")
        || normalized_error.contains("dns")
        || normalized_error.contains("outcall failed")
    {
        return OutcallFailureKind::Transport;
    }
    OutcallFailureKind::Unknown
}

#[allow(dead_code)]
fn indicates_insufficient_cycles_error(normalized_error: &str) -> bool {
    normalized_error.contains("insufficient cycles")
        || normalized_error.contains("not enough cycles")
        || normalized_error.contains("out of cycles")
        || normalized_error.contains("cycles depleted")
}

#[cfg(target_arch = "wasm32")]
fn liquid_cycle_balance() -> u128 {
    ic_cdk::api::canister_liquid_cycle_balance()
}

#[cfg(not(target_arch = "wasm32"))]
fn liquid_cycle_balance() -> u128 {
    u128::MAX
}

#[cfg(not(target_arch = "wasm32"))]
fn host_rpc_real_mode_enabled() -> bool {
    std::env::var(HOST_EVM_RPC_MODE_ENV)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "real" | "1" | "true" | "yes")
        })
        .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn host_rpc_stub_response(body: &[u8]) -> Result<Vec<u8>, String> {
    if let Some(status) = host_rpc_stub_forced_status_code() {
        let forced_body = std::env::var(HOST_EVM_RPC_STUB_FORCE_BODY_ENV)
            .unwrap_or_else(|_| "forced host rpc stub error".to_string());
        return Err(format_http_status_error(status, forced_body.as_bytes()));
    }

    let request: Value = serde_json::from_slice(body)
        .map_err(|error| format!("host rpc stub could not parse request JSON: {error}"))?;
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| "host rpc stub request is missing method".to_string())?;

    let response = match method {
        "eth_blockNumber" => json!({"jsonrpc":"2.0","id":1,"result":"0x0"}),
        "eth_getLogs" => host_rpc_stub_eth_get_logs_result(&request)?,
        "eth_getBalance" => json!({"jsonrpc":"2.0","id":1,"result":"0x1"}),
        "eth_call" => host_rpc_stub_eth_call_result(&request),
        "eth_getCode" => host_rpc_stub_eth_get_code_result(&request)?,
        "eth_getTransactionCount" => json!({"jsonrpc":"2.0","id":1,"result":"0x0"}),
        "eth_gasPrice" => json!({"jsonrpc":"2.0","id":1,"result":"0x3b9aca00"}),
        "eth_estimateGas" => json!({"jsonrpc":"2.0","id":1,"result":"0x5208"}),
        "eth_sendRawTransaction" => json!({
            "jsonrpc":"2.0",
            "id":1,
            "result":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }),
        unsupported => {
            return Err(format!(
                "host rpc stub does not support method {unsupported}"
            ));
        }
    };

    serde_json::to_vec(&response)
        .map_err(|error| format!("host rpc stub failed to serialize response: {error}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn host_rpc_stub_forced_status_code() -> Option<u16> {
    std::env::var(HOST_EVM_RPC_STUB_FORCE_STATUS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
}

#[cfg(not(target_arch = "wasm32"))]
fn host_rpc_stub_max_log_block_span() -> Option<u64> {
    std::env::var(HOST_EVM_RPC_STUB_MAX_LOG_BLOCK_SPAN_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

#[cfg(not(target_arch = "wasm32"))]
fn host_rpc_stub_eth_get_logs_result(request: &Value) -> Result<Value, String> {
    if let Some(max_span) = host_rpc_stub_max_log_block_span() {
        let from_block = request
            .pointer("/params/0/fromBlock")
            .and_then(Value::as_str)
            .ok_or_else(|| "host rpc stub eth_getLogs missing fromBlock".to_string())?;
        let to_block = request
            .pointer("/params/0/toBlock")
            .and_then(Value::as_str)
            .ok_or_else(|| "host rpc stub eth_getLogs missing toBlock".to_string())?;
        let from = parse_hex_u64(from_block, "host stub fromBlock")?;
        let to = parse_hex_u64(to_block, "host stub toBlock")?;
        let span = to.saturating_sub(from).saturating_add(1);
        if span > max_span {
            return Err(format_http_status_error(
                400,
                format!(
                    "{{\"error\":\"eth_getLogs requested span {span} exceeds max {max_span}\"}}"
                )
                .as_bytes(),
            ));
        }
    }

    Ok(json!({"jsonrpc":"2.0","id":1,"result":[]}))
}

#[cfg(not(target_arch = "wasm32"))]
fn host_rpc_stub_eth_call_result(request: &Value) -> Value {
    let to = request
        .pointer("/params/0/to")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let data = request
        .pointer("/params/0/data")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let usdc_selector = encode_call_no_args(INBOX_USDC_FUNCTION_SIGNATURE);
    let balance_selector = format!(
        "0x{}",
        function_selector_hex(ERC20_BALANCE_OF_FUNCTION_SIGNATURE)
    );
    let eip1271_selector = format!(
        "0x{}",
        function_selector_hex(EIP1271_IS_VALID_SIGNATURE_FUNCTION_SIGNATURE)
    );

    if to == HOST_EIP1271_STUB_CONTRACT_ADDRESS && data.starts_with(&eip1271_selector) {
        return json!({
            "jsonrpc":"2.0",
            "id":1,
            "result":"0x1626ba7e00000000000000000000000000000000000000000000000000000000"
        });
    }

    if data == usdc_selector {
        return json!({
            "jsonrpc":"2.0",
            "id":1,
            "result":"0x0000000000000000000000003333333333333333333333333333333333333333"
        });
    }
    if data.starts_with(&balance_selector) {
        return json!({
            "jsonrpc":"2.0",
            "id":1,
            "result":"0x000000000000000000000000000000000000000000000000000000000000002a"
        });
    }

    json!({"jsonrpc":"2.0","id":1,"result":"0x"})
}

#[cfg(not(target_arch = "wasm32"))]
fn host_rpc_stub_eth_get_code_result(request: &Value) -> Result<Value, String> {
    let address = request
        .pointer("/params/0")
        .and_then(Value::as_str)
        .ok_or_else(|| "host rpc stub eth_getCode missing address".to_string())?;
    let normalized = normalize_evm_address(address)?;
    let result = if normalized == HOST_EIP1271_STUB_CONTRACT_ADDRESS {
        HOST_EIP1271_STUB_DELEGATED_CODE
    } else {
        "0x"
    };
    Ok(json!({"jsonrpc":"2.0","id":1,"result":result}))
}

fn parse_hex_u64(raw: &str, field: &str) -> Result<u64, String> {
    let value = raw.trim();
    let without_prefix = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| format!("{field} must be 0x-prefixed hex"))?;
    u64::from_str_radix(without_prefix, 16)
        .map_err(|error| format!("failed to parse {field} as hex u64: {error}"))
}

/// Verifies an EVM steward proof against the active steward identity and nonce cursor.
///
/// Verification order:
/// 1. Normalize proof fields (address, command hash).
/// 2. Recover signer address from the signature and ensure it matches proof address.
/// 3. Enforce active steward match (`chain_id`, `address`) and enabled flag.
/// 4. Enforce nonce and expiry windows.
pub fn verify_evm_steward_proof(
    proof: &EvmStewardProof,
    context: &EvmStewardVerificationContext,
) -> Result<VerifiedEvmStewardProof, String> {
    let prepared = prepare_steward_verification(proof, context)?;
    let recovered = recover_steward_signer_address(&prepared.payload, &proof.signature)?;
    if recovered != prepared.normalized_address {
        return Err(format!(
            "recovered signer address does not match proof address: recovered={} proof={}",
            recovered, prepared.normalized_address
        ));
    }

    Ok(prepared.into_verified())
}

/// Like `verify_evm_steward_proof`, but when direct ECDSA recovery fails it
/// attempts EIP-1271 contract-wallet verification through EVM RPC.
pub async fn verify_evm_steward_proof_with_eip1271_fallback(
    proof: &EvmStewardProof,
    context: &EvmStewardVerificationContext,
    rpc: Option<&HttpEvmRpcClient>,
) -> Result<VerifiedEvmStewardProof, String> {
    let prepared = prepare_steward_verification(proof, context)?;
    let signature_error = match recover_steward_signer_address(&prepared.payload, &proof.signature)
    {
        Ok(recovered) if recovered == prepared.normalized_address => {
            return Ok(prepared.into_verified());
        }
        Ok(recovered) => format!(
            "recovered signer address does not match proof address: recovered={} proof={}",
            recovered, prepared.normalized_address
        ),
        Err(error) => error,
    };

    let Some(rpc) = rpc else {
        return Err(signature_error);
    };
    match verify_eip1271_signature(
        rpc,
        &prepared.normalized_address,
        &prepared.payload,
        &proof.signature,
    )
    .await
    {
        Ok(true) => Ok(prepared.into_verified()),
        Ok(false) => Err(signature_error),
        Err(error) => Err(format!(
            "{signature_error}; eip1271 fallback error: {error}"
        )),
    }
}

#[derive(Clone, Debug)]
struct PreparedStewardVerification {
    canister_id: String,
    chain_id: u64,
    normalized_address: String,
    command_hash: String,
    nonce: u64,
    expires_at_ns: u64,
    payload: String,
}

impl PreparedStewardVerification {
    fn into_verified(self) -> VerifiedEvmStewardProof {
        VerifiedEvmStewardProof {
            canister_id: self.canister_id,
            chain_id: self.chain_id,
            address: self.normalized_address,
            command_hash: self.command_hash,
            nonce: self.nonce,
            expires_at_ns: self.expires_at_ns,
        }
    }
}

fn prepare_steward_verification(
    proof: &EvmStewardProof,
    context: &EvmStewardVerificationContext,
) -> Result<PreparedStewardVerification, String> {
    let active_steward = context
        .active_steward
        .clone()
        .ok_or_else(|| "no active steward configured".to_string())?;
    if !active_steward.enabled {
        return Err("active steward is disabled".to_string());
    }

    let normalized_address = normalize_evm_address(&proof.address)?;
    let command_hash = normalize_command_hash(&proof.command_hash, "proof command hash")?;
    let expected_command_hash =
        normalize_command_hash(&context.expected_command_hash, "expected command hash")?;
    if command_hash != expected_command_hash {
        return Err(format!(
            "proof command hash mismatch: expected={} got={}",
            expected_command_hash, command_hash
        ));
    }

    let proof_canister_id = proof.canister_id.trim();
    let expected_canister_id = context.canister_id.trim();
    if proof_canister_id != expected_canister_id {
        return Err(format!(
            "proof canister_id mismatch: expected={} got={}",
            expected_canister_id, proof_canister_id
        ));
    }

    if proof.chain_id != active_steward.chain_id {
        return Err(format!(
            "proof chain_id does not match active steward: expected={} got={}",
            active_steward.chain_id, proof.chain_id
        ));
    }

    let active_address = normalize_evm_address(&active_steward.address)?;
    if normalized_address != active_address {
        return Err(format!(
            "proof address does not match active steward: expected={} got={}",
            active_address, normalized_address
        ));
    }

    if proof.nonce != context.expected_nonce {
        return Err(format!(
            "proof nonce mismatch: expected={} got={}",
            context.expected_nonce, proof.nonce
        ));
    }
    if context.now_ns > proof.expires_at_ns {
        return Err(format!(
            "proof expired: expires_at_ns={} now_ns={}",
            proof.expires_at_ns, context.now_ns
        ));
    }

    let payload = canonical_steward_signing_payload(
        expected_canister_id,
        proof.chain_id,
        &normalized_address,
        &command_hash,
        proof.nonce,
        proof.expires_at_ns,
    );
    Ok(PreparedStewardVerification {
        canister_id: expected_canister_id.to_string(),
        chain_id: proof.chain_id,
        normalized_address,
        command_hash,
        nonce: proof.nonce,
        expires_at_ns: proof.expires_at_ns,
        payload,
    })
}

fn normalize_command_hash(raw: &str, field: &str) -> Result<String, String> {
    let normalized = normalize_hex_blob(raw, field)?;
    if normalized.len() != 66 {
        return Err(format!("{field} must be a 32-byte hex blob"));
    }
    Ok(normalized)
}

fn canonical_steward_signing_payload(
    canister_id: &str,
    chain_id: u64,
    address: &str,
    command_hash: &str,
    nonce: u64,
    expires_at_ns: u64,
) -> String {
    format!(
        "{EVM_STEWARD_SIGNING_DOMAIN}\ncanister_id:{canister_id}\nchain_id:{chain_id}\naddress:{address}\ncommand_hash:{command_hash}\nnonce:{nonce}\nexpires_at_ns:{expires_at_ns}"
    )
}

fn ethereum_personal_message_hash(message: &str) -> [u8; 32] {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn parse_steward_signature(raw: &str) -> Result<([u8; 64], u8), String> {
    let normalized = normalize_hex_blob(raw, "proof signature")?;
    let without_prefix = normalized.trim_start_matches("0x");
    if without_prefix.len() != 130 {
        return Err("proof signature must be 65 bytes (r||s||v)".to_string());
    }
    let mut signature_bytes = [0u8; 65];
    hex::decode_to_slice(without_prefix, &mut signature_bytes)
        .map_err(|error| format!("failed to decode proof signature: {error}"))?;

    let v = match signature_bytes[64] {
        0 | 1 => signature_bytes[64],
        27 | 28 => signature_bytes[64] - 27,
        other => {
            return Err(format!(
                "unsupported proof signature recovery id byte: {other}"
            ));
        }
    };

    let mut compact = [0u8; 64];
    compact.copy_from_slice(&signature_bytes[..64]);
    Ok((compact, v))
}

fn decode_hex_blob_bytes(raw: &str, field: &str) -> Result<Vec<u8>, String> {
    let normalized = normalize_hex_blob(raw, field)?;
    let without_prefix = normalized.trim_start_matches("0x");
    hex::decode(without_prefix).map_err(|error| format!("failed to decode {field}: {error}"))
}

fn encode_eip1271_is_valid_signature_call(message_hash: &[u8; 32], signature: &[u8]) -> String {
    let selector = function_selector_hex(EIP1271_IS_VALID_SIGNATURE_FUNCTION_SIGNATURE);
    let mut encoded = format!(
        "0x{}{}{:064x}{:064x}",
        selector,
        hex::encode(message_hash),
        64u64,
        signature.len()
    );
    encoded.push_str(&hex::encode(signature));
    let padding = (32 - (signature.len() % 32)) % 32;
    if padding > 0 {
        encoded.push_str(&"00".repeat(padding));
    }
    encoded
}

fn is_empty_contract_code(raw_code: &str) -> bool {
    let payload = raw_code.trim().trim_start_matches("0x");
    payload.is_empty() || payload.as_bytes().iter().all(|byte| *byte == b'0')
}

fn is_eip1271_magic_result(raw: &str) -> Result<bool, String> {
    let normalized = normalize_hex_blob(raw, "eip1271 isValidSignature result")?;
    let payload = normalized.trim_start_matches("0x");
    Ok(payload.len() >= 8 && payload.starts_with(EIP1271_IS_VALID_SIGNATURE_MAGIC_VALUE))
}

async fn verify_eip1271_signature(
    rpc: &HttpEvmRpcClient,
    address: &str,
    payload: &str,
    signature_hex: &str,
) -> Result<bool, String> {
    let deployed_code = rpc
        .eth_get_code(address)
        .await
        .map_err(|error| format!("eth_getCode failed: {error}"))?;
    if is_empty_contract_code(&deployed_code) {
        return Ok(false);
    }

    let signature = decode_hex_blob_bytes(signature_hex, "proof signature")?;
    let message_hash = ethereum_personal_message_hash(payload);
    let calldata = encode_eip1271_is_valid_signature_call(&message_hash, &signature);
    let raw = rpc
        .eth_call(address, &calldata)
        .await
        .map_err(|error| format!("eth_call isValidSignature failed: {error}"))?;
    is_eip1271_magic_result(&raw)
}

fn recover_steward_signer_address(payload: &str, signature_hex: &str) -> Result<String, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let message_hash = ethereum_personal_message_hash(payload);
    let (signature_compact, recovery_id_byte) = parse_steward_signature(signature_hex)?;
    let signature = Signature::from_slice(&signature_compact)
        .map_err(|error| format!("invalid proof signature bytes: {error}"))?;
    let recovery_id = RecoveryId::from_byte(recovery_id_byte)
        .ok_or_else(|| "failed to parse proof signature recovery id".to_string())?;
    let recovered = VerifyingKey::recover_from_prehash(&message_hash, &signature, recovery_id)
        .map_err(|error| format!("failed to recover signer key from proof signature: {error}"))?;

    let uncompressed = recovered.to_encoded_point(false);
    let bytes = uncompressed.as_bytes();
    if bytes.len() != 65 || bytes.first().copied() != Some(0x04) {
        return Err("recovered signer key is not an uncompressed secp256k1 point".to_string());
    }
    let digest = Keccak256::digest(&bytes[1..]);
    Ok(format!("0x{}", hex::encode(&digest[12..32])))
}

fn inbox_message_queued_topic0() -> String {
    let hash = keccak256(INBOX_MESSAGE_QUEUED_EVENT_SIGNATURE.as_bytes());
    format!("0x{}", hex::encode(hash.as_slice()))
}

fn address_to_topic(raw: &str) -> Result<String, String> {
    let normalized = normalize_evm_address(raw)?;
    let bytes = normalized.trim_start_matches("0x");
    Ok(format!("0x{:0>64}", bytes))
}

fn function_selector_hex(signature: &str) -> String {
    let hash = keccak256(signature.as_bytes());
    hex::encode(&hash.as_slice()[..4])
}

fn encode_call_no_args(signature: &str) -> String {
    format!("0x{}", function_selector_hex(signature))
}

#[allow(dead_code)]
fn encode_call_single_address_arg(signature: &str, address: &str) -> Result<String, String> {
    let normalized = normalize_evm_address(address)?;
    Ok(format!(
        "0x{}{:0>64}",
        function_selector_hex(signature),
        normalized.trim_start_matches("0x")
    ))
}

#[allow(dead_code)]
fn decode_u256_word_hex_as_quantity(raw: &str, field: &str) -> Result<String, String> {
    let normalized = normalize_hex_blob(raw, field)?;
    let payload = normalized.trim_start_matches("0x");
    if payload.is_empty() {
        return Ok("0x0".to_string());
    }
    let value_hex = if payload.len() >= 64 {
        &payload[..64]
    } else {
        payload
    };
    let value = parse_hex_u256(&format!("0x{value_hex}"), field)?;
    Ok(format!("0x{value:x}"))
}

#[allow(dead_code)]
fn decode_address_word_from_eth_call(raw: &str, field: &str) -> Result<String, String> {
    let normalized = normalize_hex_blob(raw, field)?;
    let payload = normalized.trim_start_matches("0x");
    if payload.len() < 64 {
        return Err(format!("{field} must be at least 32 bytes"));
    }
    let word = &payload[..64];
    let decoded = hex::decode(word)
        .map_err(|error| format!("failed to decode {field} return data: {error}"))?;
    let address = format!("0x{}", hex::encode(&decoded[12..32]));
    normalize_evm_address(&address)
}

fn normalize_topic(raw: &str, field: &str) -> Result<String, String> {
    let normalized = normalize_hex_blob(raw, field)?;
    if normalized.len() != 66 {
        return Err(format!("{field} must be a 32-byte topic"));
    }
    Ok(normalized)
}

fn normalize_hex_quantity(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    let without_prefix = trimmed
        .strip_prefix("0x")
        .ok_or_else(|| format!("{field} must be 0x-prefixed hex"))?;
    if !without_prefix
        .as_bytes()
        .iter()
        .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("{field} must be valid hex"));
    }
    Ok(trimmed)
}

#[derive(Deserialize)]
struct RpcLog {
    #[serde(rename = "blockNumber")]
    block_number: Option<String>,
    #[serde(rename = "logIndex")]
    log_index: Option<String>,
    #[serde(rename = "transactionHash")]
    tx_hash: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    address: String,
    data: String,
}

fn filter_route_matched_logs(
    logs: Vec<RpcLog>,
    cursor: &EvmPollCursor,
    expected_contract: &str,
    expected_topic1: &str,
    max_logs_per_poll: usize,
) -> Result<Vec<EvmEvent>, String> {
    let expected_contract = normalize_evm_address(expected_contract)?;
    let expected_topic0 = inbox_message_queued_topic0();
    let expected_topic1 = normalize_topic(expected_topic1, "topic1")?;

    let mut events = Vec::new();
    for log in logs {
        let block_number = match log
            .block_number
            .as_deref()
            .map(|value| parse_hex_u64(value, "blockNumber"))
        {
            Some(Ok(value)) => value,
            _ => continue,
        };
        let log_index = match log
            .log_index
            .as_deref()
            .map(|value| parse_hex_u64(value, "logIndex"))
        {
            Some(Ok(value)) => value,
            _ => continue,
        };
        if block_number < cursor.next_block {
            continue;
        }
        if block_number == cursor.next_block && log_index < cursor.next_log_index {
            continue;
        }

        let source = match normalize_evm_address(&log.address) {
            Ok(value) if value == expected_contract => value,
            _ => continue,
        };
        if !matches!(
            log.topics
                .first()
                .map(|value| normalize_topic(value, "topic0")),
            Some(Ok(topic)) if topic == expected_topic0
        ) {
            continue;
        }
        if !matches!(
            log.topics
                .get(1)
                .map(|value| normalize_topic(value, "topic1")),
            Some(Ok(topic)) if topic == expected_topic1
        ) {
            continue;
        }

        let tx_hash = match log
            .tx_hash
            .as_deref()
            .map(|value| normalize_hex_blob(value, "transactionHash"))
        {
            Some(Ok(value)) if value.len() == 66 => value,
            _ => continue,
        };
        let payload = match normalize_hex_blob(&log.data, "data") {
            Ok(value) => value,
            Err(_) => continue,
        };

        events.push(EvmEvent {
            tx_hash,
            chain_id: cursor.chain_id,
            block_number,
            log_index,
            source,
            payload,
        });
    }

    events.sort_by_key(|event| (event.block_number, event.log_index));
    if events.len() > max_logs_per_poll {
        events.truncate(max_logs_per_poll);
    }
    Ok(events)
}

#[derive(Deserialize)]
struct EvmReadArgs {
    method: String,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    calldata: Option<String>,
    #[serde(default)]
    params_json: Option<String>,
}

enum EvmReadMethod {
    Call,
    GetBalance,
    BlockNumber,
    GetTransactionCount,
    JsonRpc(String),
}

struct ParsedEvmReadArgs {
    method: EvmReadMethod,
    address: Option<String>,
    calldata: Option<String>,
    params_json: Option<String>,
}

fn normalize_evm_read_address(raw: &str) -> Result<String, String> {
    normalize_evm_address(raw).map_err(|error| format!("invalid evm_read address: {error}"))
}

fn parse_required_evm_read_address(
    canonical_address: Option<String>,
    compat_address: Option<String>,
    method: &str,
) -> Result<String, String> {
    canonical_address
        .or(compat_address)
        .ok_or_else(|| format!("address is required for {method}"))
}

fn parse_evm_read_args_params_json_array_for_compat(
    method: &str,
    params_json: Option<&str>,
    required_for_fallback: bool,
) -> Result<Option<Vec<Value>>, String> {
    let Some(params_json) = params_json else {
        return Ok(None);
    };
    let parsed: Value = match serde_json::from_str(params_json) {
        Ok(value) => value,
        Err(error) => {
            if required_for_fallback {
                return Err(format!("invalid params_json for {method}: {error}"));
            }
            return Ok(None);
        }
    };
    let Some(array) = parsed.as_array() else {
        if required_for_fallback {
            return Err(format!("params_json for {method} must be a JSON array"));
        }
        return Ok(None);
    };
    Ok(Some(array.to_vec()))
}

fn parse_evm_read_args_first_param_address(
    method: &str,
    params: &[Value],
    required_for_fallback: bool,
) -> Result<Option<String>, String> {
    let Some(first) = params.first() else {
        return Ok(None);
    };
    let Some(address) = first.as_str() else {
        if required_for_fallback {
            return Err(format!(
                "params_json[0] for {method} must be an address string"
            ));
        }
        return Ok(None);
    };
    Ok(Some(normalize_evm_read_address(address)?))
}

fn parse_eth_call_params_json_compat(
    params: &[Value],
    required_for_fallback: bool,
) -> Result<(Option<String>, Option<String>), String> {
    let Some(first) = params.first() else {
        return Ok((None, None));
    };
    let Some(first_object) = first.as_object() else {
        if required_for_fallback {
            return Err(
                "params_json[0] for eth_call must be an object with optional to/data fields"
                    .to_string(),
            );
        }
        return Ok((None, None));
    };
    let address = match first_object.get("to") {
        Some(value) if !value.is_null() => {
            let address = value.as_str().ok_or_else(|| {
                "params_json[0].to for eth_call must be a string address".to_string()
            })?;
            Some(normalize_evm_read_address(address)?)
        }
        _ => None,
    };
    let calldata = match first_object.get("data") {
        Some(value) if !value.is_null() => {
            let data = value
                .as_str()
                .ok_or_else(|| "params_json[0].data for eth_call must be a string".to_string())?;
            Some(normalize_hex_blob(data, "calldata")?)
        }
        _ => None,
    };
    Ok((address, calldata))
}

fn read_optional_string_field(
    raw_args: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<Option<String>, String> {
    let Some(value) = raw_args.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let string_value = value
        .as_str()
        .ok_or_else(|| format!("invalid evm_read {context}: expected string; got {value}"))?;
    Ok(Some(string_value.to_string()))
}

fn ensure_evm_read_compat_consistent(
    field: &str,
    method: &str,
    first: Option<&str>,
    second: Option<&str>,
) -> Result<(), String> {
    if let (Some(first), Some(second)) = (first, second) {
        if first != second {
            return Err(format!(
                "conflicting {field} values for {method}: canonical `{first}` does not match compatibility `{second}`"
            ));
        }
    }
    Ok(())
}

fn parse_generic_evm_read_params(
    method: &str,
    params_json: Option<String>,
) -> Result<String, String> {
    let params_json = params_json.ok_or_else(|| format!("params_json is required for {method}"))?;
    let params: Value = serde_json::from_str(&params_json)
        .map_err(|error| format!("invalid params_json for {method}: {error}"))?;
    if !params.is_array() {
        return Err(format!("params_json for {method} must be a JSON array"));
    }
    serde_json::to_string(&params)
        .map_err(|error| format!("failed to normalize params_json for {method}: {error}"))
}

fn is_forbidden_generic_evm_read_method(method: &str) -> bool {
    matches!(
        method,
        "eth_sendRawTransaction"
            | "eth_sendTransaction"
            | "eth_sign"
            | "eth_signTransaction"
            | "eth_signTypedData"
            | "eth_signTypedData_v4"
            | "eth_submitWork"
            | "eth_submitHashrate"
    )
}

fn parse_evm_read_args(args_json: &str) -> Result<ParsedEvmReadArgs, String> {
    let raw_args: Value = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid evm_read args json: {error}"))?;
    if !raw_args.is_object() {
        return Err("invalid evm_read args json: expected JSON object".to_string());
    }
    let raw_args_object = raw_args
        .as_object()
        .ok_or_else(|| "invalid evm_read args json: expected JSON object".to_string())?;
    if let Some(raw_address) = raw_args_object.get("address") {
        if !raw_address.is_string() && !raw_address.is_null() {
            return Err(format!(
                "invalid evm_read address: expected 0x-prefixed string; got {}",
                raw_address
            ));
        }
    }
    let args: EvmReadArgs = serde_json::from_value(raw_args.clone())
        .map_err(|error| format!("invalid evm_read args json: {error}"))?;

    let method_name = args.method.trim();
    let method = match method_name {
        "eth_call" => EvmReadMethod::Call,
        "eth_getBalance" => EvmReadMethod::GetBalance,
        "eth_blockNumber" => EvmReadMethod::BlockNumber,
        "eth_getTransactionCount" => EvmReadMethod::GetTransactionCount,
        unsupported => {
            if !unsupported.starts_with("eth_") || is_forbidden_generic_evm_read_method(unsupported)
            {
                return Err(format!(
                    "evm_read method must be one of eth_call | eth_getBalance | eth_blockNumber | eth_getTransactionCount, or another read-only eth_* method with params_json; got {unsupported}"
                ));
            }
            EvmReadMethod::JsonRpc(unsupported.to_string())
        }
    };

    let (address, calldata) = match &method {
        EvmReadMethod::GetBalance | EvmReadMethod::GetTransactionCount => {
            let canonical_address = args
                .address
                .as_deref()
                .map(normalize_evm_read_address)
                .transpose()?;
            let params_compat = parse_evm_read_args_params_json_array_for_compat(
                method_name,
                args.params_json.as_deref(),
                canonical_address.is_none(),
            )?;
            let compat_address = match params_compat.as_deref() {
                Some(params) => parse_evm_read_args_first_param_address(
                    method_name,
                    params,
                    canonical_address.is_none(),
                )?,
                None => None,
            };
            ensure_evm_read_compat_consistent(
                "address",
                method_name,
                canonical_address.as_deref(),
                compat_address.as_deref(),
            )?;
            let address =
                parse_required_evm_read_address(canonical_address, compat_address, method_name)?;
            (Some(address), None)
        }
        EvmReadMethod::Call => {
            let canonical_address = args
                .address
                .as_deref()
                .map(normalize_evm_read_address)
                .transpose()?;
            let canonical_calldata = args
                .calldata
                .as_deref()
                .map(|value| normalize_hex_blob(value, "calldata"))
                .transpose()?;

            let alias_address =
                read_optional_string_field(raw_args_object, "to", "address alias `to`")?
                    .as_deref()
                    .map(normalize_evm_read_address)
                    .transpose()?;
            let alias_calldata =
                read_optional_string_field(raw_args_object, "data", "calldata alias `data`")?
                    .as_deref()
                    .map(|value| normalize_hex_blob(value, "calldata"))
                    .transpose()?;

            let need_params_json_compat = (canonical_address.is_none() && alias_address.is_none())
                || (canonical_calldata.is_none() && alias_calldata.is_none());
            let params_compat = parse_evm_read_args_params_json_array_for_compat(
                method_name,
                args.params_json.as_deref(),
                need_params_json_compat,
            )?;
            let (params_address, params_calldata) = match params_compat.as_deref() {
                Some(params) => parse_eth_call_params_json_compat(params, need_params_json_compat)?,
                None => (None, None),
            };

            ensure_evm_read_compat_consistent(
                "address",
                method_name,
                canonical_address.as_deref(),
                alias_address.as_deref(),
            )?;
            ensure_evm_read_compat_consistent(
                "address",
                method_name,
                canonical_address.as_deref(),
                params_address.as_deref(),
            )?;
            ensure_evm_read_compat_consistent(
                "address",
                method_name,
                alias_address.as_deref(),
                params_address.as_deref(),
            )?;
            ensure_evm_read_compat_consistent(
                "calldata",
                method_name,
                canonical_calldata.as_deref(),
                alias_calldata.as_deref(),
            )?;
            ensure_evm_read_compat_consistent(
                "calldata",
                method_name,
                canonical_calldata.as_deref(),
                params_calldata.as_deref(),
            )?;
            ensure_evm_read_compat_consistent(
                "calldata",
                method_name,
                alias_calldata.as_deref(),
                params_calldata.as_deref(),
            )?;

            let address = parse_required_evm_read_address(
                canonical_address,
                alias_address.or(params_address),
                method_name,
            )?;
            let calldata = canonical_calldata
                .or(alias_calldata)
                .or(params_calldata)
                .ok_or_else(|| "calldata is required for eth_call".to_string())?;
            (Some(address), Some(calldata))
        }
        EvmReadMethod::BlockNumber | EvmReadMethod::JsonRpc(_) => (None, None),
    };

    let params_json = match &method {
        EvmReadMethod::JsonRpc(method) => {
            Some(parse_generic_evm_read_params(method, args.params_json)?)
        }
        EvmReadMethod::Call
        | EvmReadMethod::GetBalance
        | EvmReadMethod::BlockNumber
        | EvmReadMethod::GetTransactionCount => None,
    };

    Ok(ParsedEvmReadArgs {
        method,
        address,
        calldata,
        params_json,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct WalletBalanceSyncRead {
    pub eth_balance_wei_hex: String,
    pub usdc_balance_raw_hex: String,
    pub usdc_contract_address: String,
}

#[allow(dead_code)]
pub async fn fetch_wallet_balance_sync_read(
    snapshot: &RuntimeSnapshot,
) -> Result<WalletBalanceSyncRead, String> {
    let wallet_address = snapshot
        .evm_address
        .as_deref()
        .ok_or_else(|| "evm address is not configured".to_string())
        .and_then(normalize_evm_address)?;
    let rpc = HttpEvmRpcClient::from_snapshot(snapshot)?;
    let max_response_bytes = clamp_response_bytes(snapshot.wallet_balance_sync.max_response_bytes);
    let usdc_contract_address =
        resolve_usdc_contract_address(snapshot, &rpc, max_response_bytes).await?;
    let eth_balance_wei_hex =
        fetch_eth_balance_wei_hex(&rpc, &wallet_address, max_response_bytes).await?;
    let usdc_balance_raw_hex = fetch_usdc_balance_raw_hex(
        &rpc,
        &usdc_contract_address,
        &wallet_address,
        max_response_bytes,
    )
    .await?;

    Ok(WalletBalanceSyncRead {
        eth_balance_wei_hex,
        usdc_balance_raw_hex,
        usdc_contract_address,
    })
}

#[allow(dead_code)]
async fn resolve_usdc_contract_address(
    snapshot: &RuntimeSnapshot,
    rpc: &HttpEvmRpcClient,
    max_response_bytes: u64,
) -> Result<String, String> {
    if let Some(explicit) = snapshot.wallet_balance.usdc_contract_address.as_deref() {
        return normalize_evm_address(explicit);
    }
    if !snapshot.wallet_balance_sync.discover_usdc_via_inbox {
        return Err("usdc contract address is not configured".to_string());
    }
    let inbox_contract_address = snapshot
        .inbox_contract_address
        .as_deref()
        .ok_or_else(|| "inbox contract address is not configured".to_string())?;
    discover_usdc_contract_address_via_inbox(rpc, inbox_contract_address, max_response_bytes).await
}

#[allow(dead_code)]
pub async fn fetch_eth_balance_wei_hex(
    rpc: &HttpEvmRpcClient,
    wallet_address: &str,
    max_response_bytes: u64,
) -> Result<String, String> {
    let wallet = normalize_evm_address(wallet_address)?;
    rpc.eth_get_balance_with_limit(&wallet, max_response_bytes)
        .await
}

#[allow(dead_code)]
pub async fn fetch_usdc_balance_raw_hex(
    rpc: &HttpEvmRpcClient,
    usdc_contract_address: &str,
    wallet_address: &str,
    max_response_bytes: u64,
) -> Result<String, String> {
    let usdc_contract = normalize_evm_address(usdc_contract_address)?;
    let wallet = normalize_evm_address(wallet_address)?;
    let calldata = encode_call_single_address_arg(ERC20_BALANCE_OF_FUNCTION_SIGNATURE, &wallet)?;
    let raw = rpc
        .eth_call_with_limit(&usdc_contract, &calldata, max_response_bytes)
        .await?;
    decode_u256_word_hex_as_quantity(&raw, "usdc balanceOf result")
}

#[allow(dead_code)]
pub async fn discover_usdc_contract_address_via_inbox(
    rpc: &HttpEvmRpcClient,
    inbox_contract_address: &str,
    max_response_bytes: u64,
) -> Result<String, String> {
    let inbox_contract = normalize_evm_address(inbox_contract_address)?;
    let calldata = encode_call_no_args(INBOX_USDC_FUNCTION_SIGNATURE);
    let raw = rpc
        .eth_call_with_limit(&inbox_contract, &calldata, max_response_bytes)
        .await?;
    let usdc_address = decode_address_word_from_eth_call(&raw, "Inbox.usdc result")?;
    if usdc_address == "0x0000000000000000000000000000000000000000" {
        return Err("Inbox.usdc returned zero address".to_string());
    }
    Ok(usdc_address)
}

pub async fn evm_read_tool(args_json: &str) -> Result<String, String> {
    let args = parse_evm_read_args(args_json)?;
    let snapshot = stable::runtime_snapshot();
    let rpc = HttpEvmRpcClient::from_snapshot(&snapshot)?;

    match args.method {
        EvmReadMethod::Call => {
            let address = args
                .address
                .as_deref()
                .ok_or_else(|| "address is required for eth_call".to_string())?;
            let calldata = args
                .calldata
                .as_deref()
                .ok_or_else(|| "calldata is required for eth_call".to_string())?;
            rpc.eth_call(address, calldata).await
        }
        EvmReadMethod::GetBalance => {
            let address = args
                .address
                .as_deref()
                .ok_or_else(|| "address is required for eth_getBalance".to_string())?;
            rpc.eth_get_balance(address).await
        }
        EvmReadMethod::BlockNumber => rpc
            .eth_block_number()
            .await
            .map(|value| format!("0x{value:x}")),
        EvmReadMethod::GetTransactionCount => {
            let address = args
                .address
                .as_deref()
                .ok_or_else(|| "address is required for eth_getTransactionCount".to_string())?;
            rpc.eth_get_transaction_count(address)
                .await
                .map(|value| format!("0x{value:x}"))
        }
        EvmReadMethod::JsonRpc(method) => {
            let params_json = args
                .params_json
                .as_deref()
                .ok_or_else(|| format!("params_json is required for {method}"))?;
            rpc.json_rpc_call(&method, params_json).await
        }
    }
}

#[derive(Deserialize)]
struct SendEthArgs {
    to: String,
    value_wei: String,
    #[serde(default)]
    data: Option<String>,
}

struct ParsedSendEthArgs {
    to: Address,
    to_hex: String,
    value_wei: U256,
    data: Bytes,
    data_hex: String,
}

#[derive(Clone, Debug)]
struct Eip1559UnsignedTx {
    chain_id: U256,
    nonce: U256,
    max_priority_fee_per_gas: U256,
    max_fee_per_gas: U256,
    gas_limit: U256,
    to: Address,
    value: U256,
    data: Bytes,
}

impl Eip1559UnsignedTx {
    fn payload_length(&self) -> usize {
        self.chain_id.length()
            + self.nonce.length()
            + self.max_priority_fee_per_gas.length()
            + self.max_fee_per_gas.length()
            + self.gas_limit.length()
            + self.to.length()
            + self.value.length()
            + self.data.length()
            + EMPTY_ACCESS_LIST_RLP_LEN
    }
}

impl Encodable for Eip1559UnsignedTx {
    fn encode(&self, out: &mut dyn BufMut) {
        Header {
            list: true,
            payload_length: self.payload_length(),
        }
        .encode(out);
        self.chain_id.encode(out);
        self.nonce.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        self.to.encode(out);
        self.value.encode(out);
        self.data.encode(out);
        Header {
            list: true,
            payload_length: 0,
        }
        .encode(out);
    }

    fn length(&self) -> usize {
        let payload_length = self.payload_length();
        payload_length + length_of_length(payload_length)
    }
}

struct Eip1559SignedTx<'a> {
    tx: &'a Eip1559UnsignedTx,
    y_parity: u8,
    r: U256,
    s: U256,
}

impl Eip1559SignedTx<'_> {
    fn payload_length(&self) -> usize {
        self.tx.chain_id.length()
            + self.tx.nonce.length()
            + self.tx.max_priority_fee_per_gas.length()
            + self.tx.max_fee_per_gas.length()
            + self.tx.gas_limit.length()
            + self.tx.to.length()
            + self.tx.value.length()
            + self.tx.data.length()
            + EMPTY_ACCESS_LIST_RLP_LEN
            + self.y_parity.length()
            + self.r.length()
            + self.s.length()
    }
}

impl Encodable for Eip1559SignedTx<'_> {
    fn encode(&self, out: &mut dyn BufMut) {
        Header {
            list: true,
            payload_length: self.payload_length(),
        }
        .encode(out);
        self.tx.chain_id.encode(out);
        self.tx.nonce.encode(out);
        self.tx.max_priority_fee_per_gas.encode(out);
        self.tx.max_fee_per_gas.encode(out);
        self.tx.gas_limit.encode(out);
        self.tx.to.encode(out);
        self.tx.value.encode(out);
        self.tx.data.encode(out);
        Header {
            list: true,
            payload_length: 0,
        }
        .encode(out);
        self.y_parity.encode(out);
        self.r.encode(out);
        self.s.encode(out);
    }

    fn length(&self) -> usize {
        let payload_length = self.payload_length();
        payload_length + length_of_length(payload_length)
    }
}

fn parse_send_eth_args(args_json: &str) -> Result<ParsedSendEthArgs, String> {
    let args: SendEthArgs = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid send_eth args json: {error}"))?;

    let to_hex = normalize_evm_address(&args.to)?;
    let to = Address::from_str(&to_hex)
        .map_err(|error| format!("invalid destination address for send_eth: {error}"))?;
    let value_wei = parse_decimal_u256(&args.value_wei, "value_wei")?;
    let data_hex = match args.data {
        Some(data) => normalize_hex_blob(&data, "data")?,
        None => "0x".to_string(),
    };
    let data = Bytes::from(
        hex::decode(data_hex.trim_start_matches("0x"))
            .map_err(|error| format!("data must be valid hex: {error}"))?,
    );

    Ok(ParsedSendEthArgs {
        to,
        to_hex,
        value_wei,
        data,
        data_hex,
    })
}

/// Terminal estate transfers deliberately support only bounded, auditable
/// shapes: a native transfer with no calldata, or the canonical ERC-20
/// `transfer(address,uint256)` ABI payload. This validation runs before any
/// gas estimate, signature, or broadcast is attempted.
pub(crate) fn validate_terminal_estate_transfer_args(args_json: &str) -> Result<(), String> {
    let args = parse_send_eth_args(args_json)?;
    if args.data.is_empty() {
        return Ok(());
    }
    if args.data.len() != 68 || args.data.as_ref().get(..4) != Some(&[0xa9, 0x05, 0x9c, 0xbb]) {
        return Err(
            "terminal estate calldata must be empty or exact ERC-20 transfer(address,uint256) calldata (selector 0xa9059cbb, 68 bytes)"
                .to_string(),
        );
    }
    if !args.value_wei.is_zero() {
        return Err("terminal ERC-20 transfers must set value_wei to 0".to_string());
    }
    Ok(())
}

fn parse_decimal_u256(raw: &str, field: &str) -> Result<U256, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    if !trimmed.as_bytes().iter().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{field} must be a decimal string"));
    }
    U256::from_str(trimmed).map_err(|error| format!("failed to parse {field} as decimal: {error}"))
}

fn parse_hex_u256(raw: &str, field: &str) -> Result<U256, String> {
    let normalized = normalize_hex_quantity(raw, field)?;
    let without_prefix = normalized.trim_start_matches("0x");
    if without_prefix.is_empty() {
        return Ok(U256::ZERO);
    }
    if without_prefix.len() > 64 {
        return Err(format!("{field} exceeds 32 bytes"));
    }
    let padded = if without_prefix.len() % 2 == 0 {
        without_prefix.to_string()
    } else {
        format!("0{without_prefix}")
    };
    let bytes = hex::decode(&padded)
        .map_err(|error| format!("failed to decode {field} as hex: {error}"))?;
    Ok(U256::from_be_slice(&bytes))
}

fn parse_compact_signature(raw: &str) -> Result<[u8; 64], String> {
    let normalized = normalize_hex_blob(raw, "signature")?;
    let without_prefix = normalized.trim_start_matches("0x");
    if without_prefix.len() != 128 {
        return Err("signature must be 64 bytes (r||s)".to_string());
    }
    let mut out = [0u8; 64];
    hex::decode_to_slice(without_prefix, &mut out)
        .map_err(|error| format!("failed to decode signature: {error}"))?;
    Ok(out)
}

fn encode_eip1559_unsigned_payload(tx: &Eip1559UnsignedTx) -> Vec<u8> {
    alloy_rlp::encode(tx)
}

fn encode_eip1559_unsigned(tx: &Eip1559UnsignedTx) -> Vec<u8> {
    let payload = encode_eip1559_unsigned_payload(tx);
    let mut out = Vec::with_capacity(1 + payload.len());
    out.push(0x02);
    out.extend_from_slice(&payload);
    out
}

fn encode_eip1559_signed(tx: &Eip1559UnsignedTx, y_parity: u8, r: U256, s: U256) -> Vec<u8> {
    let payload = alloy_rlp::encode(Eip1559SignedTx { tx, y_parity, r, s });
    let mut out = Vec::with_capacity(1 + payload.len());
    out.push(0x02);
    out.extend_from_slice(&payload);
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn recover_y_parity(
    _tx_hash: &B256,
    _signature_compact: &[u8; 64],
    _expected_address: &str,
) -> Result<u8, String> {
    Ok(0)
}

#[cfg(target_arch = "wasm32")]
fn recover_y_parity(
    tx_hash: &B256,
    signature_compact: &[u8; 64],
    expected_address: &str,
) -> Result<u8, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    use k256::elliptic_curve::sec1::ToEncodedPoint;

    let signature = Signature::from_slice(signature_compact)
        .map_err(|error| format!("invalid compact signature bytes: {error}"))?;
    let expected = expected_address.trim().to_ascii_lowercase();

    for candidate in [0u8, 1u8] {
        let Some(recovery_id) = RecoveryId::from_byte(candidate) else {
            continue;
        };
        let recovered =
            match VerifyingKey::recover_from_prehash(tx_hash.as_slice(), &signature, recovery_id) {
                Ok(key) => key,
                Err(_) => continue,
            };
        let uncompressed = recovered.to_encoded_point(false);
        let bytes = uncompressed.as_bytes();
        if bytes.len() != 65 || bytes.first().copied() != Some(0x04) {
            continue;
        }
        let digest = Keccak256::digest(&bytes[1..]);
        let address = format!("0x{}", hex::encode(&digest[12..32]));
        if address == expected {
            return Ok(candidate);
        }
    }

    Err("failed to recover EIP-1559 y_parity for send_eth signature".to_string())
}

fn estimate_send_eth_workflow_cost(key_name: &str) -> Result<u128, String> {
    let sign_cost = estimate_operation_cost(&OperationClass::ThresholdSign {
        key_name: key_name.to_string(),
        ecdsa_curve: 0,
    })?;
    let http_cost = estimate_operation_cost(&OperationClass::HttpOutcall {
        request_size_bytes: 512,
        max_response_bytes: 4_096,
    })?
    .saturating_mul(4);
    Ok(sign_cost.saturating_add(http_cost))
}

fn ensure_send_eth_affordable(key_name: &str) -> Result<(), String> {
    let estimated = estimate_send_eth_workflow_cost(key_name)?;
    let requirements = affordability_requirements(
        estimated,
        DEFAULT_SAFETY_MARGIN_BPS,
        stable::operation_reserve_floor_cycles(),
    );
    let liquid = liquid_cycle_balance();
    if !can_afford(liquid, &requirements) {
        return Err(format!(
            "insufficient cycles for send_eth workflow: need {} liquid, have {}",
            requirements.required_cycles, liquid
        ));
    }
    Ok(())
}

pub async fn send_eth_tool(args_json: &str, signer: &dyn SignerPort) -> Result<String, String> {
    let args = parse_send_eth_args(args_json)?;
    let snapshot = stable::runtime_snapshot();
    let from_address = snapshot
        .evm_address
        .clone()
        .ok_or_else(|| "evm address not derived yet".to_string())?;
    ensure_send_eth_affordable(&snapshot.ecdsa_key_name)?;

    let rpc = HttpEvmRpcClient::from_snapshot(&snapshot)?;
    let balance = parse_hex_u256(
        &rpc.eth_get_balance(&from_address).await?,
        "eth_getBalance result",
    )?;
    if balance < args.value_wei {
        return Err(format!(
            "insufficient balance for send_eth: balance={} value={}",
            balance, args.value_wei
        ));
    }

    let nonce = rpc.eth_get_transaction_count(&from_address).await?;
    let gas_limit = if args.data.is_empty() {
        21_000u64
    } else {
        rpc.eth_estimate_gas(&from_address, &args.to_hex, args.value_wei, &args.data_hex)
            .await
            .unwrap_or(300_000)
    };

    let base_fee = rpc
        .eth_gas_price()
        .await
        .unwrap_or_else(|_| U256::from(1_000_000_000u64));
    let max_priority_fee_per_gas = U256::from(1_000_000_000u64);
    let max_fee_per_gas = base_fee + max_priority_fee_per_gas;

    let tx = Eip1559UnsignedTx {
        chain_id: U256::from(snapshot.evm_cursor.chain_id),
        nonce: U256::from(nonce),
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit: U256::from(gas_limit),
        to: args.to,
        value: args.value_wei,
        data: args.data,
    };

    let unsigned = encode_eip1559_unsigned(&tx);
    let tx_hash = keccak256(&unsigned);
    let message_hash = format!("0x{}", hex::encode(tx_hash.as_slice()));
    let signature = parse_compact_signature(&signer.sign_message(&message_hash).await?)?;
    let y_parity = recover_y_parity(&tx_hash, &signature, &from_address)?;
    let r = U256::from_be_slice(&signature[..32]);
    let s = U256::from_be_slice(&signature[32..]);
    let signed = encode_eip1559_signed(&tx, y_parity, r, s);

    rpc.eth_send_raw_transaction(&signed).await
}

#[derive(Deserialize)]
struct SetMinPricesArgs {
    usdc_min_raw: String,
    eth_min_wei: String,
}

/// Doctrine-level economic lever for the being's own price of attention.
/// This deliberately wraps the generic signed EVM path so the model never has
/// to construct ABI calldata or choose an Inbox address supplied by content.
pub async fn set_min_prices_tool(
    args_json: &str,
    signer: &dyn SignerPort,
) -> Result<String, String> {
    let args: SetMinPricesArgs = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid set_min_prices args json: {error}"))?;
    let usdc_min = parse_decimal_u256(&args.usdc_min_raw, "usdc_min_raw")?;
    let eth_min = parse_decimal_u256(&args.eth_min_wei, "eth_min_wei")?;
    let inbox = stable::runtime_snapshot()
        .inbox_contract_address
        .ok_or_else(|| "inbox contract is not configured".to_string())?;
    let call = set_min_prices_send_eth_args_json(&inbox, usdc_min, eth_min);
    send_eth_tool(&call, signer).await
}

fn set_min_prices_send_eth_args_json(inbox: &str, usdc_min: U256, eth_min: U256) -> String {
    let selector_hash = keccak256("setMinPrices(uint256,uint256)".as_bytes());
    let selector = &selector_hash.as_slice()[..4];
    let data = format!(
        "0x{}{:064x}{:064x}",
        hex::encode(selector),
        usdc_min,
        eth_min
    );
    serde_json::json!({
        "to": inbox,
        "value_wei": "0",
        "data": data
    })
    .to_string()
}

#[allow(dead_code)]
pub fn strategy_call_to_send_eth_args_json(call: &StrategyExecutionCall) -> Result<String, String> {
    let to = normalize_evm_address(&call.to)?;
    let value_wei = parse_decimal_u256(&call.value_wei, "value_wei")?.to_string();
    let data = normalize_hex_blob(&call.data, "data")?;
    Ok(format!(
        "{{\"to\":\"{to}\",\"value_wei\":\"{value_wei}\",\"data\":\"{data}\"}}"
    ))
}

#[allow(dead_code)]
pub fn classify_strategy_failure_kind(error: &str) -> StrategyOutcomeKind {
    let normalized = error.to_ascii_lowercase();
    let nondeterministic_markers = [
        "timeout",
        "temporarily unavailable",
        "connection reset",
        "connection refused",
        "network",
        "dns",
        "tls",
        "503",
        "502",
        "429",
        "gateway",
        "rate limit",
        "deadline exceeded",
    ];
    if nondeterministic_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return StrategyOutcomeKind::NondeterministicFailure;
    }
    StrategyOutcomeKind::DeterministicFailure
}

#[allow(dead_code)]
pub async fn execute_strategy_plan(
    plan: &ExecutionPlan,
    signer: &dyn SignerPort,
) -> Result<Vec<String>, String> {
    log!(
        StrategyExecutionLogPriority::Info,
        "strategy_execute_plan_start protocol={} primitive={} template_id={} action_id={} call_count={}",
        plan.key.protocol,
        plan.key.primitive,
        plan.key.template_id,
        plan.action_id,
        plan.calls.len()
    );
    if plan.calls.is_empty() {
        let error = "execution plan has no calls".to_string();
        let outcome = classify_strategy_failure_kind(&error);
        log!(
            StrategyExecutionLogPriority::Error,
            "strategy_execute_plan_rejected protocol={} template_id={} action_id={} error={}",
            plan.key.protocol,
            plan.key.template_id,
            plan.action_id,
            error
        );
        persist_strategy_outcome(plan, outcome, None, Some(error.clone()))?;
        return Err(error);
    }

    let mut tx_hashes = Vec::with_capacity(plan.calls.len());
    for (index, call) in plan.calls.iter().enumerate() {
        log!(
            StrategyExecutionLogPriority::Info,
            "strategy_execute_call_start protocol={} template_id={} action_id={} call_index={} role={} to={} value_wei={}",
            plan.key.protocol,
            plan.key.template_id,
            plan.action_id,
            index,
            call.role,
            call.to,
            call.value_wei
        );
        let args_json = match strategy_call_to_send_eth_args_json(call) {
            Ok(payload) => payload,
            Err(error) => {
                let outcome = classify_strategy_failure_kind(&error);
                log!(
                    StrategyExecutionLogPriority::Error,
                    "strategy_execute_call_encode_failed protocol={} template_id={} action_id={} call_index={} error={}",
                    plan.key.protocol,
                    plan.key.template_id,
                    plan.action_id,
                    index,
                    error
                );
                persist_strategy_outcome(
                    plan,
                    outcome,
                    tx_hashes.last().cloned(),
                    Some(error.clone()),
                )?;
                return Err(error);
            }
        };

        match send_eth_tool(&args_json, signer).await {
            Ok(tx_hash) => {
                log!(
                    StrategyExecutionLogPriority::Info,
                    "strategy_execute_call_ok protocol={} template_id={} action_id={} call_index={} tx_hash={}",
                    plan.key.protocol,
                    plan.key.template_id,
                    plan.action_id,
                    index,
                    tx_hash
                );
                tx_hashes.push(tx_hash);
            }
            Err(error) => {
                let outcome = classify_strategy_failure_kind(&error);
                log!(
                    StrategyExecutionLogPriority::Error,
                    "strategy_execute_call_failed protocol={} template_id={} action_id={} call_index={} error={}",
                    plan.key.protocol,
                    plan.key.template_id,
                    plan.action_id,
                    index,
                    error
                );
                persist_strategy_outcome(
                    plan,
                    outcome,
                    tx_hashes.last().cloned(),
                    Some(error.clone()),
                )?;
                return Err(error);
            }
        }
    }

    persist_strategy_outcome(
        plan,
        StrategyOutcomeKind::Success,
        tx_hashes.last().cloned(),
        None,
    )?;
    log!(
        StrategyExecutionLogPriority::Info,
        "strategy_execute_plan_ok protocol={} template_id={} action_id={} tx_hash_count={}",
        plan.key.protocol,
        plan.key.template_id,
        plan.action_id,
        tx_hashes.len()
    );
    Ok(tx_hashes)
}

#[allow(dead_code)]
fn persist_strategy_outcome(
    plan: &ExecutionPlan,
    outcome: StrategyOutcomeKind,
    tx_hash: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    crate::strategy::learner::record_outcome(StrategyOutcomeEvent {
        key: plan.key.clone(),
        action_id: plan.action_id.clone(),
        outcome,
        tx_hash,
        error,
        observed_at_ns: current_time_ns(),
    })
    .map(|_stats| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::SignerPort;
    use crate::util::block_on_with_spin;
    use async_trait::async_trait;
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use serde_json::Map;
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use std::fs;
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use std::net::TcpListener;
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use std::path::PathBuf;
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use std::process::{Child, Command, Stdio};
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use std::sync::{Mutex, OnceLock};
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use std::thread;
    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    use std::time::Duration;

    #[cfg(not(target_arch = "wasm32"))]
    fn with_host_stub_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        crate::test_support::with_locked_host_env(vars, f);
    }

    fn signing_key_from_hex(hex_key: &str) -> k256::ecdsa::SigningKey {
        let mut secret_key = [0u8; 32];
        hex::decode_to_slice(hex_key, &mut secret_key).expect("hex private key should decode");
        k256::ecdsa::SigningKey::from_bytes((&secret_key).into())
            .expect("test private key should parse")
    }

    fn steward_test_signing_key() -> k256::ecdsa::SigningKey {
        signing_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318")
    }

    fn steward_address_from_key(signing_key: &k256::ecdsa::SigningKey) -> String {
        let uncompressed = signing_key.verifying_key().to_encoded_point(false);
        let digest = Keccak256::digest(&uncompressed.as_bytes()[1..]);
        format!("0x{}", hex::encode(&digest[12..32]))
    }

    fn sign_steward_payload(payload: &str, signing_key: &k256::ecdsa::SigningKey) -> String {
        let prehash = ethereum_personal_message_hash(payload);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&prehash)
            .expect("test payload should sign");

        let mut bytes = [0u8; 65];
        bytes[..64].copy_from_slice(signature.to_bytes().as_slice());
        bytes[64] = recovery_id.to_byte() + 27;
        format!("0x{}", hex::encode(bytes))
    }

    fn build_steward_proof(
        canister_id: &str,
        chain_id: u64,
        address: &str,
        command_hash: &str,
        nonce: u64,
        expires_at_ns: u64,
        signing_key: &k256::ecdsa::SigningKey,
    ) -> EvmStewardProof {
        let normalized_address =
            normalize_evm_address(address).expect("test address should normalize");
        let normalized_hash =
            normalize_command_hash(command_hash, "test command hash").expect("hash should parse");
        let payload = canonical_steward_signing_payload(
            canister_id,
            chain_id,
            &normalized_address,
            &normalized_hash,
            nonce,
            expires_at_ns,
        );
        EvmStewardProof {
            canister_id: canister_id.to_string(),
            chain_id,
            address: address.to_string(),
            command_hash: command_hash.to_string(),
            nonce,
            expires_at_ns,
            signature: sign_steward_payload(&payload, signing_key),
        }
    }

    #[test]
    fn verify_evm_steward_proof_accepts_valid_signature_and_normalizes_address() {
        let key = steward_test_signing_key();
        let signer_address = steward_address_from_key(&key);
        let command_hash = format!("0x{}", "11".repeat(32));
        let canister_id = "rrkah-fqaaa-aaaaa-aaaaq-cai";
        let proof = build_steward_proof(
            canister_id,
            8453,
            &format!("  {}  ", signer_address.to_ascii_uppercase()),
            &command_hash,
            7,
            1_000_000,
            &key,
        );
        let context = EvmStewardVerificationContext {
            canister_id: canister_id.to_string(),
            active_steward: Some(StewardState {
                chain_id: 8453,
                address: signer_address.to_ascii_uppercase(),
                enabled: true,
                last_used_at_ns: None,
                principal: None,
            }),
            expected_nonce: 7,
            expected_command_hash: command_hash.clone(),
            now_ns: 999_999,
        };

        let verified =
            verify_evm_steward_proof(&proof, &context).expect("valid proof should verify");
        assert_eq!(verified.address, signer_address);
        assert_eq!(verified.chain_id, 8453);
        assert_eq!(verified.command_hash, command_hash);
        assert_eq!(verified.nonce, 7);
    }

    #[test]
    fn verify_evm_steward_proof_rejects_signature_recovery_mismatch() {
        let key = steward_test_signing_key();
        let signer_address = steward_address_from_key(&key);
        let rogue_key = signing_key_from_hex(
            "8f2a55949056e6f90b84b5ac3f7f2f4ec4604f0f5f3e59fbb6904fb2497f33a8",
        );
        let command_hash = format!("0x{}", "22".repeat(32));
        let canister_id = "rrkah-fqaaa-aaaaa-aaaaq-cai";
        let proof = build_steward_proof(
            canister_id,
            8453,
            &signer_address,
            &command_hash,
            9,
            1_000_000,
            &rogue_key,
        );
        let context = EvmStewardVerificationContext {
            canister_id: canister_id.to_string(),
            active_steward: Some(StewardState {
                chain_id: 8453,
                address: signer_address.clone(),
                enabled: true,
                last_used_at_ns: None,
                principal: None,
            }),
            expected_nonce: 9,
            expected_command_hash: command_hash,
            now_ns: 900_000,
        };

        let error = verify_evm_steward_proof(&proof, &context)
            .expect_err("mismatched signer should be rejected");
        assert!(error.contains("recovered signer address does not match proof address"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn verify_evm_steward_proof_with_eip1271_fallback_accepts_contract_wallet_signature() {
        let key = steward_test_signing_key();
        let signer_address = steward_address_from_key(&key);
        let contract_address = HOST_EIP1271_STUB_CONTRACT_ADDRESS.to_string();
        assert_ne!(signer_address, contract_address);

        let command_hash = format!("0x{}", "44".repeat(32));
        let canister_id = "rrkah-fqaaa-aaaaa-aaaaq-cai";
        let proof = build_steward_proof(
            canister_id,
            8453,
            &contract_address,
            &command_hash,
            3,
            1_000_000,
            &key,
        );
        let context = EvmStewardVerificationContext {
            canister_id: canister_id.to_string(),
            active_steward: Some(StewardState {
                chain_id: 8453,
                address: contract_address.clone(),
                enabled: true,
                last_used_at_ns: None,
                principal: None,
            }),
            expected_nonce: 3,
            expected_command_hash: command_hash,
            now_ns: 999_999,
        };
        let rpc =
            HttpEvmRpcClient::from_snapshot(&RuntimeSnapshot::default()).expect("rpc should build");

        let verified = block_on_with_spin(verify_evm_steward_proof_with_eip1271_fallback(
            &proof,
            &context,
            Some(&rpc),
        ))
        .expect("eip1271 fallback should accept valid contract wallet signature");
        assert_eq!(verified.address, contract_address);
        assert_eq!(verified.nonce, 3);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn verify_evm_steward_proof_with_eip1271_fallback_rejects_eoa_mismatch() {
        let key = steward_test_signing_key();
        let signer_address = steward_address_from_key(&key);
        let rogue_key = signing_key_from_hex(
            "8f2a55949056e6f90b84b5ac3f7f2f4ec4604f0f5f3e59fbb6904fb2497f33a8",
        );
        let command_hash = format!("0x{}", "55".repeat(32));
        let canister_id = "rrkah-fqaaa-aaaaa-aaaaq-cai";
        let proof = build_steward_proof(
            canister_id,
            8453,
            &signer_address,
            &command_hash,
            4,
            1_000_000,
            &rogue_key,
        );
        let context = EvmStewardVerificationContext {
            canister_id: canister_id.to_string(),
            active_steward: Some(StewardState {
                chain_id: 8453,
                address: signer_address,
                enabled: true,
                last_used_at_ns: None,
                principal: None,
            }),
            expected_nonce: 4,
            expected_command_hash: command_hash,
            now_ns: 999_999,
        };
        let rpc =
            HttpEvmRpcClient::from_snapshot(&RuntimeSnapshot::default()).expect("rpc should build");

        let error = block_on_with_spin(verify_evm_steward_proof_with_eip1271_fallback(
            &proof,
            &context,
            Some(&rpc),
        ))
        .expect_err("eoa mismatch should stay rejected");
        assert!(error.contains("recovered signer address does not match proof address"));
    }

    /// End-to-end test that signs a steward payload with Foundry's `cast wallet sign`
    /// and verifies it with the Rust recovery path.  Uses a nanosecond timestamp
    /// that exceeds `Number.MAX_SAFE_INTEGER` to exercise the precision-loss fix.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn verify_evm_steward_proof_with_cast_signed_payload() {
        // Skip if `cast` is not installed.
        let cast_check = std::process::Command::new("cast").arg("--version").output();
        if cast_check.is_err() || !cast_check.unwrap().status.success() {
            eprintln!("skipping cast e2e test: `cast` not found in PATH");
            return;
        }

        let private_key = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let key = steward_test_signing_key();
        let signer_address = steward_address_from_key(&key);

        let canister_id = "rrkah-fqaaa-aaaaa-aaaaq-cai";
        let chain_id: u64 = 8453;
        let command_hash = format!("0x{}", "ab".repeat(32));
        let nonce: u64 = 42;
        // Nanosecond timestamp exceeding Number.MAX_SAFE_INTEGER (9007199254740991).
        let expires_at_ns: u64 = 1_772_447_162_556_949_500;

        let normalized_address =
            normalize_evm_address(&signer_address).expect("test address should normalize");
        let normalized_hash =
            normalize_command_hash(&command_hash, "test command hash").expect("hash should parse");
        let payload = canonical_steward_signing_payload(
            canister_id,
            chain_id,
            &normalized_address,
            &normalized_hash,
            nonce,
            expires_at_ns,
        );

        // Sign with `cast wallet sign --private-key` (uses personal_sign / EIP-191).
        let cast_output = std::process::Command::new("cast")
            .arg("wallet")
            .arg("sign")
            .arg("--private-key")
            .arg(private_key)
            .arg(&payload)
            .output()
            .expect("cast wallet sign should execute");
        assert!(
            cast_output.status.success(),
            "cast wallet sign failed: {}",
            String::from_utf8_lossy(&cast_output.stderr)
        );
        let cast_signature = String::from_utf8(cast_output.stdout)
            .expect("cast output should be utf8")
            .trim()
            .to_string();

        // Build proof with the cast-produced signature.
        let proof = EvmStewardProof {
            canister_id: canister_id.to_string(),
            chain_id,
            address: signer_address.clone(),
            command_hash: command_hash.clone(),
            nonce,
            expires_at_ns,
            signature: cast_signature,
        };
        let context = EvmStewardVerificationContext {
            canister_id: canister_id.to_string(),
            active_steward: Some(StewardState {
                chain_id,
                address: signer_address.clone(),
                enabled: true,
                last_used_at_ns: None,
                principal: None,
            }),
            expected_nonce: nonce,
            expected_command_hash: command_hash.clone(),
            now_ns: expires_at_ns - 1,
        };

        let verified =
            verify_evm_steward_proof(&proof, &context).expect("cast-signed proof should verify");
        assert_eq!(verified.address, normalized_address);
        assert_eq!(verified.nonce, nonce);
        assert_eq!(verified.expires_at_ns, expires_at_ns);
    }

    /// Verifies the full JSON round-trip: prepare view serializes `expires_at_ns`
    /// as a string, and `EvmStewardProof` deserializes it back without precision loss.
    #[test]
    fn steward_proof_expires_at_ns_survives_json_round_trip() {
        // A nanosecond value that exceeds Number.MAX_SAFE_INTEGER.
        let large_ns: u64 = 1_772_447_162_556_949_500;

        // Simulate the JSON that the browser would send back (string-valued expires_at_ns).
        let json = format!(
            r#"{{
                "canister_id": "rrkah-fqaaa-aaaaa-aaaaq-cai",
                "chain_id": 8453,
                "address": "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23",
                "command_hash": "0x{hash}",
                "nonce": 0,
                "expires_at_ns": "{large_ns}",
                "signature": "0x{sig}"
            }}"#,
            hash = "aa".repeat(32),
            sig = "bb".repeat(65),
        );
        let proof: EvmStewardProof =
            serde_json::from_str(&json).expect("string expires_at_ns should deserialize");
        assert_eq!(proof.expires_at_ns, large_ns);

        // Also verify plain numeric form still works (Candid / non-browser callers).
        let json_numeric = format!(
            r#"{{
                "canister_id": "rrkah-fqaaa-aaaaa-aaaaq-cai",
                "chain_id": 8453,
                "address": "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23",
                "command_hash": "0x{hash}",
                "nonce": 0,
                "expires_at_ns": 1000000,
                "signature": "0x{sig}"
            }}"#,
            hash = "aa".repeat(32),
            sig = "bb".repeat(65),
        );
        let proof_numeric: EvmStewardProof =
            serde_json::from_str(&json_numeric).expect("numeric expires_at_ns should deserialize");
        assert_eq!(proof_numeric.expires_at_ns, 1_000_000);
    }

    #[test]
    fn verify_evm_steward_proof_rejects_chain_nonce_and_expiry_mismatches() {
        let key = steward_test_signing_key();
        let signer_address = steward_address_from_key(&key);
        let command_hash = format!("0x{}", "33".repeat(32));
        let canister_id = "rrkah-fqaaa-aaaaa-aaaaq-cai";
        let base_context = EvmStewardVerificationContext {
            canister_id: canister_id.to_string(),
            active_steward: Some(StewardState {
                chain_id: 8453,
                address: signer_address.clone(),
                enabled: true,
                last_used_at_ns: None,
                principal: None,
            }),
            expected_nonce: 3,
            expected_command_hash: command_hash.clone(),
            now_ns: 10_000,
        };

        let chain_mismatch = build_steward_proof(
            canister_id,
            1,
            &signer_address,
            &command_hash,
            3,
            20_000,
            &key,
        );
        let chain_error = verify_evm_steward_proof(&chain_mismatch, &base_context)
            .expect_err("chain mismatch should fail");
        assert!(chain_error.contains("proof chain_id does not match active steward"));

        let nonce_mismatch = build_steward_proof(
            canister_id,
            8453,
            &signer_address,
            &command_hash,
            4,
            20_000,
            &key,
        );
        let nonce_error = verify_evm_steward_proof(&nonce_mismatch, &base_context)
            .expect_err("nonce mismatch should fail");
        assert!(nonce_error.contains("proof nonce mismatch"));

        let expired = build_steward_proof(
            canister_id,
            8453,
            &signer_address,
            &command_hash,
            3,
            9_999,
            &key,
        );
        let expiry_error =
            verify_evm_steward_proof(&expired, &base_context).expect_err("expired proof fails");
        assert!(expiry_error.contains("proof expired"));
    }

    #[test]
    fn mock_evm_broadcaster_returns_mock_tx_hash() {
        let broadcaster = MockEvmBroadcaster;
        let result = broadcaster
            .broadcast("0xdeadbeef")
            .expect("mock broadcaster should succeed");
        assert_eq!(result.tx_hash, "0x0xdeadbeef-mock");
    }

    #[test]
    fn parse_evm_read_args_enforces_method_requirements() {
        assert!(parse_evm_read_args("{}").is_err());
        assert!(parse_evm_read_args(
            r#"{"method":"eth_call","address":"not-an-address","calldata":"0x00"}"#
        )
        .is_err());
        assert!(parse_evm_read_args(
            r#"{"method":"eth_call","address":"0x1111111111111111111111111111111111111111"}"#
        )
        .is_err());
        assert!(parse_evm_read_args(r#"{"method":"eth_getBalance"}"#).is_err());
        assert!(parse_evm_read_args(r#"{"method":"eth_getTransactionCount"}"#).is_err());
        assert!(parse_evm_read_args(r#"{"method":"eth_blockNumber"}"#).is_ok());
        assert!(parse_evm_read_args(r#"{"method":"debug_traceCall","params_json":"[]"}"#).is_err());
        assert!(
            parse_evm_read_args(r#"{"method":"eth_getCode","params_json":"[\"0x1111111111111111111111111111111111111111\",\"latest\"]"}"#).is_ok()
        );
    }

    #[test]
    fn parse_evm_read_args_rejects_non_string_address_values() {
        let numeric_error =
            match parse_evm_read_args(r#"{"method":"eth_getBalance","address":12345}"#) {
                Ok(_) => panic!("numeric address should be rejected explicitly"),
                Err(error) => error,
            };
        assert!(numeric_error.contains("invalid evm_read address"));
        assert!(numeric_error.contains("expected 0x-prefixed string"));

        let scientific_error =
            match parse_evm_read_args(r#"{"method":"eth_getBalance","address":1e21}"#) {
                Ok(_) => panic!("scientific address should be rejected explicitly"),
                Err(error) => error,
            };
        assert!(scientific_error.contains("invalid evm_read address"));
        assert!(scientific_error.contains("expected 0x-prefixed string"));
    }

    #[test]
    fn parse_evm_read_args_accepts_eth_get_balance_params_json_compat() {
        let parsed = parse_evm_read_args(
            r#"{"method":"eth_getBalance","params_json":"[\"0x1111111111111111111111111111111111111111\",\"latest\"]"}"#,
        )
        .expect("eth_getBalance should accept params_json[0] compatibility address");
        assert!(matches!(&parsed.method, EvmReadMethod::GetBalance));
        assert_eq!(
            parsed.address.as_deref(),
            Some("0x1111111111111111111111111111111111111111")
        );
        assert!(parsed.calldata.is_none());
    }

    #[test]
    fn parse_evm_read_args_accepts_eth_call_to_data_aliases() {
        let parsed = parse_evm_read_args(
            r#"{"method":"eth_call","to":"0x1111111111111111111111111111111111111111","data":"0x70A08231"}"#,
        )
        .expect("eth_call should accept to/data compatibility aliases");
        assert!(matches!(&parsed.method, EvmReadMethod::Call));
        assert_eq!(
            parsed.address.as_deref(),
            Some("0x1111111111111111111111111111111111111111")
        );
        assert_eq!(parsed.calldata.as_deref(), Some("0x70a08231"));
    }

    #[test]
    fn parse_evm_read_args_accepts_eth_call_params_json_compat() {
        let parsed = parse_evm_read_args(
            r#"{"method":"eth_call","params_json":"[{\"to\":\"0x1111111111111111111111111111111111111111\",\"data\":\"0x70A08231\"},\"latest\"]"}"#,
        )
        .expect("eth_call should accept params_json compatibility object");
        assert!(matches!(&parsed.method, EvmReadMethod::Call));
        assert_eq!(
            parsed.address.as_deref(),
            Some("0x1111111111111111111111111111111111111111")
        );
        assert_eq!(parsed.calldata.as_deref(), Some("0x70a08231"));
    }

    #[test]
    fn parse_evm_read_args_accepts_eth_get_transaction_count_params_json_compat() {
        let parsed = parse_evm_read_args(
            r#"{"method":"eth_getTransactionCount","params_json":"[\"0x1111111111111111111111111111111111111111\",\"latest\"]"}"#,
        )
        .expect("eth_getTransactionCount should accept params_json[0] compatibility address");
        assert!(matches!(&parsed.method, EvmReadMethod::GetTransactionCount));
        assert_eq!(
            parsed.address.as_deref(),
            Some("0x1111111111111111111111111111111111111111")
        );
        assert!(parsed.calldata.is_none());
    }

    #[test]
    fn parse_evm_read_args_rejects_conflicting_canonical_and_compat_values() {
        let error = match parse_evm_read_args(
            r#"{"method":"eth_getBalance","address":"0x1111111111111111111111111111111111111111","params_json":"[\"0x2222222222222222222222222222222222222222\",\"latest\"]"}"#,
        ) {
            Ok(_) => panic!("conflicting canonical and compatibility addresses must fail"),
            Err(error) => error,
        };
        assert!(error.contains("conflicting address values for eth_getBalance"));
    }

    #[test]
    fn parse_evm_read_args_rejects_non_array_params_json_for_compat() {
        let error = match parse_evm_read_args(
            r#"{"method":"eth_getBalance","params_json":"{\"address\":\"0x1111111111111111111111111111111111111111\"}"}"#,
        ) {
            Ok(_) => panic!("compatibility params_json must be a JSON array"),
            Err(error) => error,
        };
        assert!(error.contains("params_json for eth_getBalance must be a JSON array"));
    }

    #[test]
    fn poll_filter_topic_is_derived_from_agent_evm_address() {
        let topic = address_to_topic("0x1111111111111111111111111111111111111111")
            .expect("topic derivation should succeed");
        assert_eq!(
            topic,
            "0x0000000000000000000000001111111111111111111111111111111111111111"
        );
    }

    #[test]
    fn message_queued_topic_signature_matches_finalized_inbox_abi() {
        assert_eq!(
            INBOX_MESSAGE_QUEUED_EVENT_SIGNATURE,
            "MessageQueued(address,uint64,address,string,uint256,uint256)"
        );
    }

    #[test]
    fn poll_bootstrap_from_block_uses_recent_head_window_when_cursor_is_unset() {
        let cursor = EvmPollCursor {
            next_block: 0,
            ..EvmPollCursor::default()
        };
        assert_eq!(effective_from_block(&cursor, 5_000, 1_000), 4_000);
        assert_eq!(effective_from_block(&cursor, 100, 1_000), 0);
    }

    #[test]
    fn poll_bootstrap_from_block_can_start_from_current_head_when_lookback_is_zero() {
        let cursor = EvmPollCursor {
            next_block: 0,
            ..EvmPollCursor::default()
        };
        assert_eq!(effective_from_block(&cursor, 5_000, 0), 5_000);
        assert_eq!(effective_from_block(&cursor, 100, 0), 100);
    }

    #[test]
    fn poll_bootstrap_from_block_respects_existing_cursor_progress() {
        let cursor = EvmPollCursor {
            next_block: 4_321,
            ..EvmPollCursor::default()
        };
        assert_eq!(effective_from_block(&cursor, 9_999, 1_000), 4_321);
    }

    #[test]
    fn poll_bootstrap_from_block_fast_forwards_when_cursor_is_far_behind() {
        let cursor = EvmPollCursor {
            next_block: 1_000,
            ..EvmPollCursor::default()
        };
        assert_eq!(effective_from_block(&cursor, 100_000, 1_000), 99_000);
    }

    #[test]
    fn poll_bootstrap_from_block_fast_forward_uses_configured_lookback_window() {
        let cursor = EvmPollCursor {
            next_block: 1_000,
            ..EvmPollCursor::default()
        };
        assert_eq!(effective_from_block(&cursor, 100_000, 250), 99_750);
    }

    #[test]
    fn filter_route_matched_logs_requires_address_topic_and_tx_hash() {
        let cursor = EvmPollCursor {
            chain_id: 8453,
            next_block: 100,
            next_log_index: 2,
            ..EvmPollCursor::default()
        };
        let contract = "0x2222222222222222222222222222222222222222";
        let topic0 = inbox_message_queued_topic0();
        let topic1 = address_to_topic("0x1111111111111111111111111111111111111111")
            .expect("topic derivation should succeed");

        let logs = vec![
            RpcLog {
                block_number: Some("0x64".to_string()),
                log_index: Some("0x1".to_string()),
                address: contract.to_string(),
                topics: vec![topic0.clone(), topic1.clone()],
                data: "0x1234".to_string(),
                tx_hash: Some(
                    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                ),
            },
            RpcLog {
                block_number: Some("0x64".to_string()),
                log_index: Some("0x2".to_string()),
                address: "0x3333333333333333333333333333333333333333".to_string(),
                topics: vec![topic0.clone(), topic1.clone()],
                data: "0x1234".to_string(),
                tx_hash: Some(
                    "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        .to_string(),
                ),
            },
            RpcLog {
                block_number: Some("0x64".to_string()),
                log_index: Some("0x3".to_string()),
                address: contract.to_string(),
                topics: vec![topic0.clone(), topic1],
                data: "0x1234".to_string(),
                tx_hash: None,
            },
            RpcLog {
                block_number: Some("0x64".to_string()),
                log_index: Some("0x4".to_string()),
                address: contract.to_string(),
                topics: vec![
                    topic0,
                    "0x000000000000000000000000ffffffffffffffffffffffffffffffffffffffff"
                        .to_string(),
                ],
                data: "0x1234".to_string(),
                tx_hash: Some(
                    "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        .to_string(),
                ),
            },
        ];

        let filtered = filter_route_matched_logs(
            logs,
            &cursor,
            contract,
            &address_to_topic("0x1111111111111111111111111111111111111111")
                .expect("topic derivation should succeed"),
            DEFAULT_MAX_LOGS_PER_POLL,
        )
        .expect("filtering should succeed");

        assert_eq!(
            filtered.len(),
            0,
            "all logs should be dropped in this fixture"
        );
    }

    #[test]
    fn filter_route_matched_logs_orders_by_block_and_log_index() {
        let cursor = EvmPollCursor {
            chain_id: 8453,
            next_block: 100,
            next_log_index: 0,
            ..EvmPollCursor::default()
        };
        let contract = "0x2222222222222222222222222222222222222222";
        let topic0 = inbox_message_queued_topic0();
        let topic1 = address_to_topic("0x1111111111111111111111111111111111111111")
            .expect("topic derivation should succeed");

        let logs = vec![
            RpcLog {
                block_number: Some("0x65".to_string()),
                log_index: Some("0x2".to_string()),
                address: contract.to_string(),
                topics: vec![topic0.clone(), topic1.clone()],
                data: "0x1234".to_string(),
                tx_hash: Some(
                    "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                        .to_string(),
                ),
            },
            RpcLog {
                block_number: Some("0x65".to_string()),
                log_index: Some("0x1".to_string()),
                address: contract.to_string(),
                topics: vec![topic0, topic1],
                data: "0x5678".to_string(),
                tx_hash: Some(
                    "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                        .to_string(),
                ),
            },
        ];

        let filtered = filter_route_matched_logs(
            logs,
            &cursor,
            contract,
            &address_to_topic("0x1111111111111111111111111111111111111111")
                .expect("topic derivation should succeed"),
            DEFAULT_MAX_LOGS_PER_POLL,
        )
        .expect("filtering should succeed");

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].block_number, 101);
        assert_eq!(filtered[0].log_index, 1);
        assert_eq!(filtered[1].log_index, 2);
    }

    #[test]
    fn http_evm_poller_requires_inbox_contract_and_agent_address() {
        let missing_contract = RuntimeSnapshot {
            evm_rpc_url: "https://mainnet.base.org".to_string(),
            evm_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            ..RuntimeSnapshot::default()
        };
        assert!(HttpEvmPoller::from_snapshot(&missing_contract).is_err());

        let missing_agent = RuntimeSnapshot {
            evm_rpc_url: "https://mainnet.base.org".to_string(),
            inbox_contract_address: Some("0x2222222222222222222222222222222222222222".to_string()),
            ..RuntimeSnapshot::default()
        };
        assert!(HttpEvmPoller::from_snapshot(&missing_agent).is_err());
    }

    #[test]
    fn http_evm_poller_uses_snapshot_bootstrap_lookback_blocks() {
        let snapshot = RuntimeSnapshot {
            evm_rpc_url: "https://mainnet.base.org".to_string(),
            evm_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            inbox_contract_address: Some("0x2222222222222222222222222222222222222222".to_string()),
            evm_bootstrap_lookback_blocks: 0,
            ..RuntimeSnapshot::default()
        };

        let poller =
            HttpEvmPoller::from_snapshot(&snapshot).expect("poller should initialize from config");
        assert_eq!(poller.bootstrap_lookback_blocks, 0);
    }

    #[test]
    fn evm_read_tool_supports_multiple_methods_in_host_mode() {
        stable::init_storage();
        stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
            .expect("rpc url should be set");

        let call_out = block_on_with_spin(evm_read_tool(
            r#"{"method":"eth_call","address":"0x1111111111111111111111111111111111111111","calldata":"0x1234"}"#,
        ))
        .expect("evm_read should succeed in host-mode stub");
        assert_eq!(call_out, "0x");

        let balance_out = block_on_with_spin(evm_read_tool(
            r#"{"method":"eth_getBalance","address":"0x1111111111111111111111111111111111111111"}"#,
        ))
        .expect("eth_getBalance should succeed in host-mode stub");
        assert_eq!(balance_out, "0x1");

        let block_number_out = block_on_with_spin(evm_read_tool(r#"{"method":"eth_blockNumber"}"#))
            .expect("eth_blockNumber should succeed in host-mode stub");
        assert_eq!(block_number_out, "0x0");

        let nonce_out = block_on_with_spin(evm_read_tool(
            r#"{"method":"eth_getTransactionCount","address":"0x1111111111111111111111111111111111111111"}"#,
        ))
        .expect("eth_getTransactionCount should succeed in host-mode stub");
        assert_eq!(nonce_out, "0x0");

        let generic_out = block_on_with_spin(evm_read_tool(
            r#"{"method":"eth_gasPrice","params_json":"[]"}"#,
        ))
        .expect("generic eth_gasPrice should succeed in host-mode stub");
        let generic_json: Value =
            serde_json::from_str(&generic_out).expect("generic output should be valid json");
        assert_eq!(
            generic_json.get("result").and_then(Value::as_str),
            Some("0x3b9aca00")
        );
    }

    #[test]
    fn fetch_wallet_balance_sync_read_discovers_usdc_and_reads_balances() {
        let snapshot = RuntimeSnapshot {
            evm_rpc_url: "https://mainnet.base.org".to_string(),
            evm_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            inbox_contract_address: Some("0x2222222222222222222222222222222222222222".to_string()),
            wallet_balance_sync: crate::domain::types::WalletBalanceSyncConfig {
                max_response_bytes: 256,
                discover_usdc_via_inbox: true,
                ..crate::domain::types::WalletBalanceSyncConfig::default()
            },
            ..RuntimeSnapshot::default()
        };

        let read = block_on_with_spin(fetch_wallet_balance_sync_read(&snapshot))
            .expect("wallet balance sync read should succeed");
        assert_eq!(read.eth_balance_wei_hex, "0x1");
        assert_eq!(read.usdc_balance_raw_hex, "0x2a");
        assert_eq!(
            read.usdc_contract_address,
            "0x3333333333333333333333333333333333333333"
        );
    }

    #[test]
    fn fetch_wallet_balance_sync_read_prefers_explicit_usdc_contract_override() {
        let snapshot = RuntimeSnapshot {
            evm_rpc_url: "https://mainnet.base.org".to_string(),
            evm_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            wallet_balance: crate::domain::types::WalletBalanceSnapshot {
                usdc_contract_address: Some(
                    "0x4444444444444444444444444444444444444444".to_string(),
                ),
                ..crate::domain::types::WalletBalanceSnapshot::default()
            },
            wallet_balance_sync: crate::domain::types::WalletBalanceSyncConfig {
                max_response_bytes: 256,
                discover_usdc_via_inbox: false,
                ..crate::domain::types::WalletBalanceSyncConfig::default()
            },
            ..RuntimeSnapshot::default()
        };

        let read = block_on_with_spin(fetch_wallet_balance_sync_read(&snapshot))
            .expect("wallet balance sync read should succeed with explicit usdc contract");
        assert_eq!(
            read.usdc_contract_address,
            "0x4444444444444444444444444444444444444444"
        );
        assert_eq!(read.usdc_balance_raw_hex, "0x2a");
    }

    #[test]
    fn fetch_wallet_balance_sync_read_requires_usdc_source() {
        let snapshot = RuntimeSnapshot {
            evm_rpc_url: "https://mainnet.base.org".to_string(),
            evm_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            wallet_balance_sync: crate::domain::types::WalletBalanceSyncConfig {
                max_response_bytes: 256,
                discover_usdc_via_inbox: false,
                ..crate::domain::types::WalletBalanceSyncConfig::default()
            },
            ..RuntimeSnapshot::default()
        };

        let err = block_on_with_spin(fetch_wallet_balance_sync_read(&snapshot))
            .expect_err("missing usdc source should fail");
        assert!(err.contains("usdc contract address is not configured"));
    }

    #[test]
    fn control_plane_outcalls_use_safe_response_cap() {
        let snapshot = RuntimeSnapshot {
            evm_rpc_url: "https://mainnet.base.org".to_string(),
            evm_rpc_max_response_bytes: 256,
            ..RuntimeSnapshot::default()
        };
        let rpc = HttpEvmRpcClient::from_snapshot(&snapshot).expect("rpc client should build");
        assert_eq!(rpc.control_plane_max_response_bytes(), 4_096);
    }

    #[test]
    fn classify_evm_failure_maps_response_too_large_errors() {
        let failure = classify_evm_failure("Header size exceeds specified response size limit");
        assert_eq!(
            failure,
            crate::domain::types::RecoveryFailure::Outcall(crate::domain::types::OutcallFailure {
                kind: crate::domain::types::OutcallFailureKind::ResponseTooLarge,
                retry_after_secs: None,
                observed_response_bytes: None,
            })
        );
    }

    #[test]
    fn classify_evm_failure_maps_rate_limit_status_errors() {
        let failure = classify_evm_failure("evm rpc returned status 429");
        assert_eq!(
            failure,
            crate::domain::types::RecoveryFailure::Outcall(crate::domain::types::OutcallFailure {
                kind: crate::domain::types::OutcallFailureKind::RateLimited,
                retry_after_secs: None,
                observed_response_bytes: None,
            })
        );
    }

    #[test]
    fn classify_evm_failure_maps_missing_configuration_errors() {
        let failure = classify_evm_failure("evm rpc url is not configured");
        assert_eq!(
            failure,
            crate::domain::types::RecoveryFailure::Operation(
                crate::domain::types::OperationFailure {
                    kind: crate::domain::types::OperationFailureKind::MissingConfiguration,
                }
            )
        );
    }

    #[test]
    fn classify_evm_failure_maps_decode_errors_to_invalid_response() {
        let failure = classify_evm_failure("failed to parse eth_getLogs response JSON: bad json");
        assert_eq!(
            failure,
            crate::domain::types::RecoveryFailure::Outcall(crate::domain::types::OutcallFailure {
                kind: crate::domain::types::OutcallFailureKind::InvalidResponse,
                retry_after_secs: None,
                observed_response_bytes: None,
            })
        );
    }

    #[test]
    fn classify_evm_failure_maps_rpc_error_codes() {
        let rate_limited = classify_evm_failure(
            "rpc returned error for eth_getLogs: code=-32005 message=query returned more than 10000 results",
        );
        assert_eq!(
            rate_limited,
            crate::domain::types::RecoveryFailure::Outcall(crate::domain::types::OutcallFailure {
                kind: crate::domain::types::OutcallFailureKind::RateLimited,
                retry_after_secs: None,
                observed_response_bytes: None,
            })
        );

        let unavailable = classify_evm_failure(
            "rpc returned error for eth_getLogs: code=-32011 message=backend unhealthy",
        );
        assert_eq!(
            unavailable,
            crate::domain::types::RecoveryFailure::Outcall(crate::domain::types::OutcallFailure {
                kind: crate::domain::types::OutcallFailureKind::UpstreamUnavailable,
                retry_after_secs: None,
                observed_response_bytes: None,
            })
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn eth_get_logs_does_not_retry_on_status_400() {
        with_host_stub_env(
            &[(HOST_EVM_RPC_STUB_MAX_LOG_BLOCK_SPAN_ENV, Some("100"))],
            || {
                let snapshot = RuntimeSnapshot {
                    evm_rpc_url: "https://base.publicnode.com".to_string(),
                    evm_rpc_max_response_bytes: 65_536,
                    ..RuntimeSnapshot::default()
                };
                let rpc =
                    HttpEvmRpcClient::from_snapshot(&snapshot).expect("rpc client should build");

                let error = match block_on_with_spin(rpc.eth_get_logs(
                    1_000,
                    1_400,
                    None,
                    None,
                    snapshot.evm_rpc_max_response_bytes,
                )) {
                    Ok(_) => panic!("eth_getLogs should fail without in-call retries"),
                    Err(error) => error,
                };
                assert!(error.contains("status 400"));
                assert!(error.contains("exceeds max 100"));
            },
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn rpc_errors_include_http_status_body_preview() {
        with_host_stub_env(
            &[
                (HOST_EVM_RPC_STUB_FORCE_STATUS_ENV, Some("400")),
                (
                    HOST_EVM_RPC_STUB_FORCE_BODY_ENV,
                    Some("provider says invalid"),
                ),
            ],
            || {
                let snapshot = RuntimeSnapshot {
                    evm_rpc_url: "https://base.publicnode.com".to_string(),
                    ..RuntimeSnapshot::default()
                };
                let rpc =
                    HttpEvmRpcClient::from_snapshot(&snapshot).expect("rpc client should build");

                let error =
                    block_on_with_spin(rpc.eth_block_number()).expect_err("rpc should fail");
                assert!(error.contains("evm rpc returned status 400"));
                assert!(error.contains("provider says invalid"));
            },
        );
    }

    struct FixedSignatureSigner;

    #[async_trait(?Send)]
    impl SignerPort for FixedSignatureSigner {
        async fn sign_message(&self, _message_hash: &str) -> Result<String, String> {
            Ok(format!("0x{}", "11".repeat(64)))
        }
    }

    #[test]
    fn send_eth_tool_returns_tx_hash_in_host_mode() {
        stable::init_storage();
        stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
            .expect("rpc url should be set");
        stable::set_ecdsa_key_name("dfx_test_key".to_string()).expect("key should be set");
        stable::set_evm_address(Some(
            "0x1111111111111111111111111111111111111111".to_string(),
        ))
        .expect("address should be set");

        let signer = FixedSignatureSigner;
        let tx_hash = block_on_with_spin(send_eth_tool(
            r#"{"to":"0x2222222222222222222222222222222222222222","value_wei":"1"}"#,
            &signer,
        ))
        .expect("send_eth should succeed");

        assert_eq!(
            tx_hash,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn set_min_prices_rejects_unconfigured_inbox_and_invalid_amounts() {
        stable::init_storage();
        let signer = FixedSignatureSigner;
        let error = block_on_with_spin(set_min_prices_tool(
            r#"{"usdc_min_raw":"x","eth_min_wei":"1"}"#,
            &signer,
        ))
        .expect_err("invalid amount must fail before broadcast");
        assert!(error.contains("usdc_min_raw must be a decimal string"));

        let error = block_on_with_spin(set_min_prices_tool(
            r#"{"usdc_min_raw":"1000000","eth_min_wei":"1"}"#,
            &signer,
        ))
        .expect_err("missing inbox must fail");
        assert!(error.contains("inbox contract is not configured"));
    }

    #[test]
    fn set_min_prices_targets_configured_inbox_with_exact_calldata_and_broadcasts() {
        stable::init_storage();
        let inbox = "0x2222222222222222222222222222222222222222";
        stable::set_inbox_contract_address(Some(inbox.to_string()))
            .expect("inbox should be configured");
        stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
            .expect("rpc url should be set");
        stable::set_ecdsa_key_name("dfx_test_key".to_string()).expect("key should be set");
        stable::set_evm_address(Some(
            "0x1111111111111111111111111111111111111111".to_string(),
        ))
        .expect("address should be set");

        let call =
            set_min_prices_send_eth_args_json(inbox, U256::from(1_500_000u64), U256::from(7u64));
        assert_eq!(
            call,
            format!(
                r#"{{"to":"{inbox}","value_wei":"0","data":"0x{}{:064x}{:064x}"}}"#,
                function_selector_hex("setMinPrices(uint256,uint256)"),
                U256::from(1_500_000u64),
                U256::from(7u64)
            )
        );

        let hash = block_on_with_spin(set_min_prices_tool(
            r#"{"usdc_min_raw":"1500000","eth_min_wei":"7"}"#,
            &FixedSignatureSigner,
        ))
        .expect("configured price update should broadcast");
        assert_eq!(
            hash,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn strategy_call_to_send_eth_args_json_serializes_expected_payload() {
        let call = crate::domain::types::StrategyExecutionCall {
            role: "router".to_string(),
            to: "0x2222222222222222222222222222222222222222".to_string(),
            value_wei: "7".to_string(),
            data: "0xabcdef00".to_string(),
        };
        let args_json =
            strategy_call_to_send_eth_args_json(&call).expect("strategy call should serialize");
        assert_eq!(
            args_json,
            r#"{"to":"0x2222222222222222222222222222222222222222","value_wei":"7","data":"0xabcdef00"}"#
        );
    }

    #[test]
    fn classify_strategy_failure_kind_distinguishes_error_classes() {
        assert_eq!(
            classify_strategy_failure_kind("execution reverted: slippage exceeded"),
            crate::domain::types::StrategyOutcomeKind::DeterministicFailure
        );
        assert_eq!(
            classify_strategy_failure_kind("rpc timeout while broadcasting tx"),
            crate::domain::types::StrategyOutcomeKind::NondeterministicFailure
        );
    }

    #[test]
    fn execute_strategy_plan_records_deterministic_failure_evidence() {
        stable::init_storage();
        let plan = crate::domain::types::ExecutionPlan {
            key: crate::domain::types::StrategyTemplateKey {
                protocol: "erc20".to_string(),
                primitive: "transfer".to_string(),
                chain_id: 8453,
                template_id: "execute-strategy-failure".to_string(),
            },
            action_id: "transfer".to_string(),
            calls: vec![crate::domain::types::StrategyExecutionCall {
                role: "token".to_string(),
                to: "not-an-address".to_string(),
                value_wei: "0".to_string(),
                data: "0x".to_string(),
            }],
            preconditions: vec!["ok".to_string()],
            postconditions: vec!["delta".to_string()],
        };

        let signer = FixedSignatureSigner;
        let err = block_on_with_spin(execute_strategy_plan(&plan, &signer))
            .expect_err("invalid strategy call should fail");
        assert!(
            err.contains("address"),
            "expected address validation error, got {err}"
        );

        let stats = crate::storage::stable::strategy_outcome_stats(&plan.key)
            .expect("failure evidence should persist");
        assert_eq!(stats.total_runs, 1);
        assert_eq!(stats.deterministic_failures, 1);
        assert_eq!(stats.nondeterministic_failures, 0);
    }

    #[cfg(all(not(feature = "anvil_e2e"), not(target_arch = "wasm32")))]
    #[test]
    #[ignore = "Enable feature `anvil_e2e` and ensure `anvil` is installed to run this E2E test"]
    fn placeholder_http_evm_poller_e2e_against_anvil() {}

    #[cfg(all(feature = "anvil_e2e", not(target_arch = "wasm32")))]
    #[test]
    fn http_evm_poller_e2e_against_anvil() {
        let port = find_free_local_port();
        let _anvil = start_anvil(port).expect("anvil must start for E2E test");

        with_host_rpc_mode_real(|| {
            let rpc_url = format!("http://127.0.0.1:{port}");
            let rpc_snapshot = RuntimeSnapshot {
                evm_rpc_url: rpc_url,
                evm_rpc_max_response_bytes: 65_536,
                evm_cursor: EvmPollCursor {
                    chain_id: 31_337,
                    ..EvmPollCursor::default()
                },
                ..RuntimeSnapshot::default()
            };
            let rpc = HttpEvmRpcClient::from_snapshot(&rpc_snapshot)
                .expect("rpc client should initialize");
            let chain_id = block_on_with_spin(rpc.rpc_call(
                "eth_chainId",
                json!([]),
                rpc.control_plane_max_response_bytes(),
            ))
            .expect("host rpc call should reach anvil");
            assert_eq!(
                chain_id
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                "0x7a69"
            );

            let accounts = anvil_accounts(&rpc).expect("anvil accounts should be available");
            let deployer = accounts[0].clone();
            let payer = accounts[1].clone();
            let automaton = "0x1111111111111111111111111111111111111111".to_string();
            let message = "survival ping";
            let usdc_amount = U256::from(1_500_000u64);
            let eth_amount = U256::from(1_000_000_000_000_000u64);

            let mock_usdc_bytecode =
                load_contract_creation_bytecode("evm/out/MockUSDC.sol/MockUSDC.json")
                    .expect("mock usdc artifact should load");
            let mock_usdc = deploy_contract(&rpc, &deployer, &mock_usdc_bytecode)
                .expect("mock usdc should deploy");

            let inbox_base_bytecode =
                load_contract_creation_bytecode("evm/out/Inbox.sol/Inbox.json")
                    .expect("inbox artifact should load");
            let inbox_constructor_data = encode_constructor_single_address(&mock_usdc)
                .expect("inbox constructor args should encode");
            let inbox_bytecode =
                append_constructor_args(&inbox_base_bytecode, &inbox_constructor_data);
            let inbox =
                deploy_contract(&rpc, &deployer, &inbox_bytecode).expect("inbox should deploy");

            let mint_data = encode_call_address_u256("mint(address,uint256)", &payer, usdc_amount)
                .expect("mint calldata should encode");
            send_transaction(
                &rpc,
                &deployer,
                Some(mock_usdc.as_str()),
                mint_data.as_str(),
                None,
            )
            .expect("mint should succeed");

            let approve_data =
                encode_call_address_u256("approve(address,uint256)", &inbox, usdc_amount)
                    .expect("approve calldata should encode");
            send_transaction(
                &rpc,
                &payer,
                Some(mock_usdc.as_str()),
                approve_data.as_str(),
                None,
            )
            .expect("approve should succeed");

            let queue_message_data =
                encode_queue_message_calldata(&automaton, message, usdc_amount)
                    .expect("queueMessage calldata should encode");
            send_transaction(
                &rpc,
                &payer,
                Some(inbox.as_str()),
                queue_message_data.as_str(),
                Some(eth_amount),
            )
            .expect("queueMessage should succeed");

            let snapshot = RuntimeSnapshot {
                evm_rpc_url: rpc_snapshot.evm_rpc_url.clone(),
                evm_rpc_max_response_bytes: rpc_snapshot.evm_rpc_max_response_bytes,
                evm_address: Some(automaton.clone()),
                inbox_contract_address: Some(inbox.clone()),
                evm_cursor: EvmPollCursor {
                    chain_id: 31_337,
                    next_block: 0,
                    next_log_index: 0,
                    confirmation_depth: 0,
                    ..EvmPollCursor::default()
                },
                ..RuntimeSnapshot::default()
            };
            let poller = HttpEvmPoller::from_snapshot(&snapshot).expect("poller should initialize");
            let result = block_on_with_spin(poller.poll(&snapshot.evm_cursor))
                .expect("poll should succeed against anvil");
            assert!(
                result.cursor.next_block >= 1,
                "cursor should advance after polling anvil head"
            );
            assert_eq!(
                result.events.len(),
                1,
                "exactly one route-matching MessageQueued event is expected"
            );

            let event = &result.events[0];
            let decoded =
                decode_message_queued_payload(&event.payload).expect("event payload should decode");
            assert_eq!(
                decoded.sender,
                normalize_evm_address(&payer).unwrap_or_default()
            );
            assert_eq!(decoded.message, message);
            assert_eq!(decoded.usdc_amount, usdc_amount);
            assert_eq!(decoded.eth_amount, eth_amount);

            let automaton_eth_balance = parse_hex_u256(
                &block_on_with_spin(rpc.eth_get_balance(&automaton))
                    .expect("eth balance should be readable"),
                "automaton eth balance",
            )
            .expect("automaton eth balance should parse");
            assert_eq!(
                automaton_eth_balance, eth_amount,
                "automaton should receive forwarded ETH payment"
            );

            let usdc_balance_call = encode_call_address("balanceOf(address)", &automaton)
                .expect("balanceOf calldata should encode");
            let automaton_usdc_balance = parse_hex_u256(
                &block_on_with_spin(rpc.eth_call(&mock_usdc, &usdc_balance_call))
                    .expect("usdc balance call should succeed"),
                "automaton usdc balance",
            )
            .expect("automaton usdc balance should parse");
            assert_eq!(
                automaton_usdc_balance, usdc_amount,
                "automaton should receive forwarded USDC payment"
            );
        });
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    #[derive(Debug, PartialEq, Eq)]
    struct DecodedMessageQueuedPayload {
        sender: String,
        message: String,
        usdc_amount: U256,
        eth_amount: U256,
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn anvil_accounts(rpc: &HttpEvmRpcClient) -> Result<Vec<String>, String> {
        let response = block_on_with_spin(rpc.rpc_call(
            "eth_accounts",
            json!([]),
            rpc.control_plane_max_response_bytes(),
        ))?;
        let result = response
            .get("result")
            .and_then(Value::as_array)
            .ok_or_else(|| "eth_accounts result was missing".to_string())?;
        let mut accounts = Vec::with_capacity(result.len());
        for value in result {
            let account = value
                .as_str()
                .ok_or_else(|| "eth_accounts result must contain strings".to_string())?;
            accounts.push(normalize_evm_address(account)?);
        }
        if accounts.len() < 2 {
            return Err("anvil did not return enough unlocked accounts".to_string());
        }
        Ok(accounts)
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn load_contract_creation_bytecode(artifact_path: &str) -> Result<String, String> {
        let artifact_abs_path = project_root().join(artifact_path);
        let artifact_content = fs::read_to_string(&artifact_abs_path).map_err(|error| {
            format!(
                "failed to read contract artifact {}: {error}",
                artifact_abs_path.display()
            )
        })?;
        let json: Value = serde_json::from_str(&artifact_content).map_err(|error| {
            format!(
                "failed to parse contract artifact {} as JSON: {error}",
                artifact_abs_path.display()
            )
        })?;
        let bytecode = json
            .pointer("/bytecode/object")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!(
                    "contract artifact {} is missing bytecode.object",
                    artifact_abs_path.display()
                )
            })?;
        normalize_hex_blob(bytecode, "artifact bytecode")
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn deploy_contract(
        rpc: &HttpEvmRpcClient,
        from: &str,
        creation_bytecode: &str,
    ) -> Result<String, String> {
        let receipt = send_transaction(rpc, from, None, creation_bytecode, None)?;
        let contract_address = receipt
            .get("contractAddress")
            .and_then(Value::as_str)
            .ok_or_else(|| "transaction receipt missing contractAddress".to_string())?;
        normalize_evm_address(contract_address)
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn send_transaction(
        rpc: &HttpEvmRpcClient,
        from: &str,
        to: Option<&str>,
        data: &str,
        value_wei: Option<U256>,
    ) -> Result<Value, String> {
        let mut tx = Map::new();
        tx.insert(
            "from".to_string(),
            Value::String(normalize_evm_address(from)?),
        );
        if let Some(to) = to {
            tx.insert("to".to_string(), Value::String(normalize_evm_address(to)?));
        }
        tx.insert(
            "data".to_string(),
            Value::String(normalize_hex_blob(data, "transaction data")?),
        );
        if let Some(value) = value_wei {
            tx.insert("value".to_string(), Value::String(format!("0x{value:x}")));
        }

        let response = block_on_with_spin(rpc.rpc_call(
            "eth_sendTransaction",
            Value::Array(vec![Value::Object(tx)]),
            rpc.control_plane_max_response_bytes(),
        ))?;
        let tx_hash = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| "eth_sendTransaction result was missing".to_string())
            .and_then(|value| normalize_hex_blob(value, "transaction hash"))?;

        wait_for_receipt(rpc, &tx_hash)
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn wait_for_receipt(rpc: &HttpEvmRpcClient, tx_hash: &str) -> Result<Value, String> {
        for _ in 0..50 {
            let response = block_on_with_spin(rpc.rpc_call(
                "eth_getTransactionReceipt",
                json!([tx_hash]),
                rpc.control_plane_max_response_bytes(),
            ))?;
            if let Some(receipt) = response.get("result") {
                if !receipt.is_null() {
                    return Ok(receipt.clone());
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(format!(
            "transaction receipt was not mined in time for {tx_hash}"
        ))
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn encode_constructor_single_address(address: &str) -> Result<String, String> {
        Ok(format!("0x{}", encode_address_word_hex(address)?))
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn append_constructor_args(bytecode: &str, encoded_args: &str) -> String {
        format!(
            "0x{}{}",
            bytecode.trim_start_matches("0x"),
            encoded_args.trim_start_matches("0x")
        )
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn encode_call_address_u256(
        signature: &str,
        address: &str,
        amount: U256,
    ) -> Result<String, String> {
        Ok(format!(
            "0x{}{}{}",
            function_selector_hex(signature),
            encode_address_word_hex(address)?,
            encode_u256_word_hex(amount)
        ))
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn encode_call_address(signature: &str, address: &str) -> Result<String, String> {
        Ok(format!(
            "0x{}{}",
            function_selector_hex(signature),
            encode_address_word_hex(address)?
        ))
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn encode_queue_message_calldata(
        automaton: &str,
        message: &str,
        usdc_amount: U256,
    ) -> Result<String, String> {
        let message_hex = hex::encode(message.as_bytes());
        let message_padding = "0".repeat((64 - (message_hex.len() % 64)) % 64);
        Ok(format!(
            "0x{}{}{}{}{}{}",
            function_selector_hex("queueMessage(address,string,uint256)"),
            encode_address_word_hex(automaton)?,
            encode_u256_word_hex(U256::from(96u64)),
            encode_u256_word_hex(usdc_amount),
            encode_u256_word_hex(U256::from(message.len())),
            message_hex + &message_padding
        ))
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn function_selector_hex(signature: &str) -> String {
        let hash = keccak256(signature.as_bytes());
        hex::encode(&hash.as_slice()[..4])
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn encode_address_word_hex(address: &str) -> Result<String, String> {
        let normalized = normalize_evm_address(address)?;
        Ok(format!("{:0>64}", normalized.trim_start_matches("0x")))
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn encode_u256_word_hex(value: U256) -> String {
        format!("{value:064x}")
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn decode_message_queued_payload(
        payload_hex: &str,
    ) -> Result<DecodedMessageQueuedPayload, String> {
        let payload = normalize_hex_blob(payload_hex, "message queued payload")?;
        let bytes = hex::decode(payload.trim_start_matches("0x"))
            .map_err(|error| format!("failed to decode message queued payload: {error}"))?;
        if bytes.len() < 128 {
            return Err("message queued payload must be at least 128 bytes".to_string());
        }

        let sender = format!("0x{}", hex::encode(&bytes[12..32]));
        let sender = normalize_evm_address(&sender)?;
        let message_offset = read_usize_word(&bytes[32..64], "message offset")?;
        let usdc_amount = U256::from_be_slice(&bytes[64..96]);
        let eth_amount = U256::from_be_slice(&bytes[96..128]);
        if message_offset.saturating_add(32) > bytes.len() {
            return Err("message offset points outside payload".to_string());
        }
        let message_len = read_usize_word(
            &bytes[message_offset..message_offset + 32],
            "message length",
        )?;
        let message_start = message_offset + 32;
        let message_end = message_start.saturating_add(message_len);
        if message_end > bytes.len() {
            return Err("message bytes exceed payload length".to_string());
        }

        let message = std::str::from_utf8(&bytes[message_start..message_end])
            .map_err(|error| format!("message payload is not utf-8: {error}"))?
            .to_string();

        Ok(DecodedMessageQueuedPayload {
            sender,
            message,
            usdc_amount,
            eth_amount,
        })
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn read_usize_word(word: &[u8], field: &str) -> Result<usize, String> {
        if word.len() != 32 {
            return Err(format!("{field} word must be 32 bytes"));
        }
        let size = std::mem::size_of::<usize>();
        if word[..(32 - size)].iter().any(|byte| *byte != 0) {
            return Err(format!("{field} overflowed usize"));
        }
        let mut value = 0usize;
        for byte in &word[(32 - size)..] {
            value = (value << 8) | usize::from(*byte);
        }
        Ok(value)
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn project_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn find_free_local_port() -> u16 {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("ephemeral port binding should work");
        listener
            .local_addr()
            .expect("listener local addr should be available")
            .port()
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn start_anvil(port: u16) -> Result<ChildProcessGuard, String> {
        let child = Command::new("anvil")
            .args([
                "--chain-id",
                "31337",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--silent",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to start anvil process: {error}"))?;
        let mut guard = ChildProcessGuard { child };
        wait_for_anvil_rpc(port).inspect_err(|_error| {
            let _ = guard.child.kill();
        })?;
        Ok(guard)
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn wait_for_anvil_rpc(port: u16) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{port}");
        let request_body = json!({
            "jsonrpc": "2.0",
            "method": "eth_chainId",
            "params": [],
            "id": 1
        })
        .to_string();

        for _ in 0..50 {
            let response = ureq::post(&url)
                .set("content-type", "application/json")
                .send_string(&request_body);
            if let Ok(response) = response {
                if response.status() == 200 {
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_millis(100));
        }

        Err("anvil rpc did not become ready in time".to_string())
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    struct ChildProcessGuard {
        child: Child,
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    impl Drop for ChildProcessGuard {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "anvil_e2e"))]
    fn with_host_rpc_mode_real<T>(f: impl FnOnce() -> T) -> T {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("host rpc mode lock should not be poisoned");

        let previous = std::env::var_os(HOST_EVM_RPC_MODE_ENV);
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var(HOST_EVM_RPC_MODE_ENV, "real");
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

        match previous {
            Some(value) => {
                #[allow(unused_unsafe)]
                unsafe {
                    std::env::set_var(HOST_EVM_RPC_MODE_ENV, value);
                }
            }
            None => {
                #[allow(unused_unsafe)]
                unsafe {
                    std::env::remove_var(HOST_EVM_RPC_MODE_ENV);
                }
            }
        }

        match result {
            Ok(output) => output,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
