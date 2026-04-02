/// Certified HTTP handler for the canister's browser UI and JSON API.
///
/// Every route that is served as a query response is covered by an
/// IC-certified Merkle tree (v2 certificate header).  Write routes
/// (`POST /api/conversation`, …) carry an
/// `upgrade: true` flag so the IC boundary nodes automatically retry them as
/// update calls, which go through `handle_http_request_update`.
///
/// # Route map
///
/// | Method | Path                          | Kind        |
/// |--------|-------------------------------|-------------|
/// | GET    | `/`                           | query       |
/// | GET    | `/index.html`                 | query       |
/// | GET    | `/styles.css`                 | query       |
/// | GET    | `/app.js`                     | query       |
/// | GET    | `/api/snapshot`               | query       |
/// | GET    | `/api/steward/status`         | query       |
/// | GET    | `/api/wallet/balance`         | query       |
/// | GET    | `/api/wallet/balance/sync-config` | query   |
/// | GET    | `/api/evm/config`             | query       |
/// | GET    | `/api/inference/config`       | query       |
/// | GET    | `/api/inference/proxy/status` | query       |
/// | GET    | `/api/scheduler/config`       | query       |
/// | GET    | `/api/welcome`                | query       |
/// | GET    | `/api/build-info`             | query       |
/// | POST   | `/api/conversation`           | update      |
/// | POST   | `/api/steward/direct-message/prepare` | update |
/// | POST   | `/api/steward/direct-message/execute` | update |
/// | POST   | `/api/steward/model/prepare`  | update      |
/// | POST   | `/api/steward/model/execute`  | update      |
/// | POST   | `/api/steward/reasoning/prepare` | update   |
/// | POST   | `/api/steward/reasoning/execute` | update   |
use crate::domain::types::{EvmStewardProof, StewardCommand, StewardReasoningVariant};
use crate::storage::stable;
use crate::timing::current_time_ns;
use canlog::{log, GetLogFilter, LogFilter, LogPriorityLevels};
#[cfg(target_arch = "wasm32")]
use ic_http_certification::utils::add_v2_certificate_header;
use ic_http_certification::{
    DefaultCelBuilder, DefaultResponseCertification, HttpCertification, HttpCertificationPath,
    HttpCertificationTree, HttpCertificationTreeEntry, HttpRequest, HttpResponse,
    HttpUpdateRequest, HttpUpdateResponse, Method, StatusCode, CERTIFICATE_EXPRESSION_HEADER_NAME,
};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::cell::RefCell;

const HEADER_CONTENT_TYPE: &str = "Content-Type";
const HEADER_CACHE_CONTROL: &str = "Cache-Control";
const CONTENT_TYPE_HTML: &str = "text/html; charset=utf-8";
const CONTENT_TYPE_CSS: &str = "text/css; charset=utf-8";
const CONTENT_TYPE_JS: &str = "application/javascript; charset=utf-8";
const CONTENT_TYPE_JSON: &str = "application/json; charset=utf-8";
const CACHE_NO_STORE: &str = "no-store";
const DEFAULT_SNAPSHOT_LIMIT: usize = 25;
const UI_INDEX_HTML: &str = include_str!("ui_index.html");
const UI_STYLES_CSS: &str = include_str!("ui_styles.css");
const UI_APP_JS: &str = include_str!("ui_app.js");
const EVM_STEWARD_SIGNING_DOMAIN: &str = "ic-automaton:steward-execute:v1";
const STEWARD_DIRECT_MESSAGE_PROOF_TTL_NS: u64 = 5 * 60 * 1_000_000_000;

// ── Certification types ──────────────────────────────────────────────────────

/// Log priority levels for HTTP-layer diagnostics.
#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum HttpLogPriority {
    #[log_level(capacity = 1000, name = "HTTP_INFO")]
    Info,
    #[log_level(capacity = 500, name = "HTTP_WARN")]
    Warn,
    #[log_level(capacity = 200, name = "HTTP_ERROR")]
    Error,
}

impl GetLogFilter for HttpLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

/// A fully-certified HTTP route: pairs a pre-built `HttpCertification` proof
/// with the base response body so that `render_certified_response` can attach
/// the v2 certificate header without recomputing the Merkle witness each time.
#[derive(Clone)]
struct CertifiedRoute {
    method: Method,
    request_path: &'static str,
    cert_path: HttpCertificationPath<'static>,
    #[cfg(target_arch = "wasm32")]
    expr_path: Vec<String>,
    certification: HttpCertification,
    base_response: HttpResponse<'static>,
}

/// Thread-local snapshot of the certification tree and all registered routes.
/// Rebuilt by `init_certification` on every `init` / `post_upgrade` call and
/// whenever a write route mutates state that is reflected in a GET response.
#[derive(Clone)]
struct HttpCertificationState {
    tree: HttpCertificationTree,
    routes: Vec<CertifiedRoute>,
    fallback_not_found: CertifiedRoute,
}

// ── API types ────────────────────────────────────────────────────────────────

/// Parsed body for `POST /api/conversation` — identifies the conversation by
/// sender address.
#[derive(Clone, Debug, Deserialize)]
struct ConversationLookupRequest {
    sender: String,
}

/// JSON body returned when `POST /api/conversation` cannot find the requested
/// sender.
#[derive(Clone, Debug, Serialize)]
struct ConversationLookupError {
    ok: bool,
    error: String,
}

#[derive(Clone, Debug, Deserialize)]
struct StewardDirectMessagePrepareRequest {
    sender: String,
    message: String,
}

#[derive(Clone, Debug, Deserialize)]
struct StewardModelPrepareRequest {
    model: String,
}

#[derive(Clone, Debug, Deserialize)]
struct StewardReasoningPrepareRequest {
    variant: StewardReasoningVariant,
}

#[derive(Clone, Debug, Serialize)]
struct StewardDirectMessagePrepareView {
    sender: String,
    message: String,
    proof_template: EvmStewardProofTemplateView,
    signing_payload: String,
}

#[derive(Clone, Debug, Serialize)]
struct StewardModelPrepareView {
    model: String,
    proof_template: EvmStewardProofTemplateView,
    signing_payload: String,
}

#[derive(Clone, Debug, Serialize)]
struct StewardReasoningPrepareView {
    variant: StewardReasoningVariant,
    proof_template: EvmStewardProofTemplateView,
    signing_payload: String,
}

#[derive(Clone, Debug, Serialize)]
struct EvmStewardProofTemplateView {
    canister_id: String,
    chain_id: u64,
    address: String,
    command_hash: String,
    nonce: u64,
    /// Serialized as a string to avoid JavaScript `Number` precision loss
    /// (nanosecond timestamps exceed `Number.MAX_SAFE_INTEGER`).
    #[serde(serialize_with = "serialize_u64_as_string")]
    expires_at_ns: u64,
}

fn serialize_u64_as_string<S: serde::Serializer>(
    value: &u64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

#[derive(Clone, Debug, Deserialize)]
struct StewardDirectMessageExecuteRequest {
    sender: String,
    message: String,
    proof: EvmStewardProof,
}

#[derive(Clone, Debug, Deserialize)]
struct StewardModelExecuteRequest {
    model: String,
    proof: EvmStewardProof,
}

#[derive(Clone, Debug, Deserialize)]
struct StewardReasoningExecuteRequest {
    variant: StewardReasoningVariant,
    proof: EvmStewardProof,
}

#[derive(Clone, Debug, Serialize)]
struct StewardDirectMessageExecuteResult {
    result: String,
}

#[derive(Clone, Debug)]
struct SignedStewardCommandPrepareView {
    proof_template: EvmStewardProofTemplateView,
    signing_payload: String,
}

/// Serialisable welcome message served by `GET /api/welcome`.
///
/// `message` is `None` when no custom message has been set by the agent or a
/// controller, in which case the TUI falls back to its built-in default.
#[derive(Clone, Debug, Serialize)]
struct WelcomeView {
    message: Option<String>,
}

/// Serialisable build metadata served by `GET /api/build-info`.
#[derive(Clone, Debug, Serialize)]
struct BuildInfoView {
    commit: String,
}

/// Serialisable scheduler timing configuration served by
/// `GET /api/scheduler/config`.
#[derive(Clone, Debug, Serialize)]
struct SchedulerConfigView {
    base_tick_secs: u64,
    ticks_per_turn_interval: u64,
    default_turn_interval_secs: u64,
}

/// Serialisable snapshot of EVM configuration fields served by
/// `GET /api/evm/config`.
#[derive(Clone, Debug, Serialize)]
struct EvmConfigView {
    automaton_address: Option<String>,
    inbox_contract_address: Option<String>,
    usdc_address: Option<String>,
    chain_id: u64,
    rpc_url: String,
}

/// Returns a public-safe RPC endpoint string for open HTTP views.
///
/// Keeps only `scheme://host[:port]` (or bare `host[:port]` if no scheme),
/// stripping user-info, path, query, and fragment components so API keys and
/// tokens embedded in the URL are never exposed.
fn redact_public_rpc_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let (scheme, remainder) = match trimmed.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, trimmed),
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, value)| value)
        .unwrap_or(authority)
        .trim();
    if host_port.is_empty() {
        return String::new();
    }

    match scheme {
        Some(value) if !value.is_empty() => format!("{value}://{host_port}"),
        _ => host_port.to_string(),
    }
}

fn welcome_view() -> WelcomeView {
    WelcomeView {
        message: stable::get_welcome_message(),
    }
}

fn build_info_view() -> BuildInfoView {
    BuildInfoView {
        commit: stable::installed_version_commit().unwrap_or_else(|| {
            option_env!("AUTOMATON_GIT_COMMIT")
                .unwrap_or("unknown")
                .to_string()
        }),
    }
}

fn scheduler_config_view() -> SchedulerConfigView {
    let base_tick_secs = stable::get_scheduler_base_tick_secs();
    let ticks_per_turn_interval = stable::get_cadence_multiplier();
    SchedulerConfigView {
        base_tick_secs,
        ticks_per_turn_interval,
        default_turn_interval_secs: base_tick_secs.saturating_mul(ticks_per_turn_interval),
    }
}

fn evm_config_view() -> EvmConfigView {
    let route = stable::evm_route_state_view();
    EvmConfigView {
        automaton_address: route.automaton_evm_address,
        inbox_contract_address: route.inbox_contract_address,
        usdc_address: stable::get_discovered_usdc_address(),
        chain_id: route.chain_id,
        rpc_url: redact_public_rpc_url(&stable::get_evm_rpc_url()),
    }
}

fn steward_http_expected_canister_id() -> String {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::id().to_text();

    #[cfg(not(target_arch = "wasm32"))]
    return "rrkah-fqaaa-aaaaa-aaaaq-cai".to_string();
}

fn normalize_http_evm_address(value: &str) -> Result<String, String> {
    let trimmed = value.trim().to_ascii_lowercase();
    let valid = trimmed.len() == 42
        && trimmed.starts_with("0x")
        && trimmed
            .as_bytes()
            .iter()
            .skip(2)
            .all(|byte| byte.is_ascii_hexdigit());
    if !valid {
        return Err("address must be a 0x-prefixed 20-byte hex string".to_string());
    }
    Ok(trimmed)
}

fn normalize_steward_direct_message(
    sender: String,
    message: String,
) -> Result<(String, String), String> {
    let sender = sender.trim().to_string();
    if sender.is_empty() {
        return Err("steward sender cannot be empty".to_string());
    }
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("steward message cannot be empty".to_string());
    }
    Ok((sender, message))
}

fn normalize_steward_model(model: String) -> Result<String, String> {
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err("steward model cannot be empty".to_string());
    }
    Ok(model)
}

fn steward_send_message_command(sender: String, message: String) -> Result<StewardCommand, String> {
    let (sender, message) = normalize_steward_direct_message(sender, message)?;
    Ok(StewardCommand::SendStewardMessage { sender, message })
}

fn steward_set_model_command(model: String) -> Result<StewardCommand, String> {
    let model = normalize_steward_model(model)?;
    Ok(StewardCommand::SetInferenceModel { model })
}

fn steward_set_reasoning_command(variant: StewardReasoningVariant) -> StewardCommand {
    StewardCommand::SetOpenrouterReasoningLevel {
        level: variant.reasoning_level(),
    }
}

fn steward_command_hash(command: &StewardCommand) -> Result<String, String> {
    let encoded = candid::encode_one(command)
        .map_err(|error| format!("failed to encode steward command: {error}"))?;
    let digest = Keccak256::digest(&encoded);
    Ok(format!("0x{}", hex::encode(digest)))
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

fn prepare_signed_steward_command(
    command: StewardCommand,
) -> Result<SignedStewardCommandPrepareView, String> {
    let status = stable::steward_status_view();
    let nonce = status.next_nonce;
    let active = status
        .active_steward
        .ok_or_else(|| "no active steward configured".to_string())?;
    if !active.enabled {
        return Err("active steward is disabled".to_string());
    }
    if active.chain_id == 0 {
        return Err("active steward chain id must be non-zero".to_string());
    }
    let address = normalize_http_evm_address(&active.address)?;
    let command_hash = steward_command_hash(&command)?;
    let canister_id = steward_http_expected_canister_id();
    let expires_at_ns = current_time_ns().saturating_add(STEWARD_DIRECT_MESSAGE_PROOF_TTL_NS);
    let signing_payload = canonical_steward_signing_payload(
        &canister_id,
        active.chain_id,
        &address,
        &command_hash,
        nonce,
        expires_at_ns,
    );

    Ok(SignedStewardCommandPrepareView {
        proof_template: EvmStewardProofTemplateView {
            canister_id,
            chain_id: active.chain_id,
            address,
            command_hash,
            nonce,
            expires_at_ns,
        },
        signing_payload,
    })
}

async fn execute_signed_steward_command(
    command: StewardCommand,
    proof: EvmStewardProof,
) -> Result<StewardDirectMessageExecuteResult, String> {
    let result = crate::steward_execute(command, proof).await?;
    Ok(StewardDirectMessageExecuteResult { result })
}

fn prepare_steward_direct_message_view(
    payload: StewardDirectMessagePrepareRequest,
) -> Result<StewardDirectMessagePrepareView, String> {
    let command = steward_send_message_command(payload.sender, payload.message)?;
    let (sender, message) = match &command {
        StewardCommand::SendStewardMessage { sender, message } => (sender.clone(), message.clone()),
        _ => unreachable!("steward direct message prepare should only build send command"),
    };
    let shared = prepare_signed_steward_command(command)?;
    Ok(StewardDirectMessagePrepareView {
        sender,
        message,
        proof_template: shared.proof_template,
        signing_payload: shared.signing_payload,
    })
}

fn prepare_steward_model_view(
    payload: StewardModelPrepareRequest,
) -> Result<StewardModelPrepareView, String> {
    let command = steward_set_model_command(payload.model)?;
    let model = match &command {
        StewardCommand::SetInferenceModel { model } => model.clone(),
        _ => unreachable!("steward model prepare should only build set model command"),
    };
    let shared = prepare_signed_steward_command(command)?;
    Ok(StewardModelPrepareView {
        model,
        proof_template: shared.proof_template,
        signing_payload: shared.signing_payload,
    })
}

fn prepare_steward_reasoning_view(
    payload: StewardReasoningPrepareRequest,
) -> Result<StewardReasoningPrepareView, String> {
    let command = steward_set_reasoning_command(payload.variant);
    let shared = prepare_signed_steward_command(command)?;
    Ok(StewardReasoningPrepareView {
        variant: payload.variant,
        proof_template: shared.proof_template,
        signing_payload: shared.signing_payload,
    })
}

/// Expands conversation reply previews with full outbox payloads when linked.
fn hydrate_conversation_agent_replies(
    mut log: crate::domain::types::ConversationLog,
) -> crate::domain::types::ConversationLog {
    for entry in &mut log.entries {
        let Some(outbox_message_id) = entry.outbox_message_id.as_deref() else {
            continue;
        };
        if let Some(outbox_message) = stable::get_outbox_message(outbox_message_id) {
            if !outbox_message.body.trim().is_empty() {
                entry.agent_reply = outbox_message.body;
            }
        }
    }
    log
}

// ── Route handlers ───────────────────────────────────────────────────────────

// Per-canister thread-local state holding the live certification tree.
thread_local! {
    static HTTP_STATE: RefCell<Option<HttpCertificationState>> = const { RefCell::new(None) };
}

/// Builds a fresh `HttpCertificationState` from current stable storage,
/// commits the Merkle root hash as certified data, and stores the state in
/// the thread-local slot.  Must be called from `init`, `post_upgrade`, and
/// after any write route that changes a GET-served payload.
pub fn init_certification() {
    let state = build_certification_state();
    set_tree_as_certified_data(&state.tree);
    HTTP_STATE.with(|slot| {
        *slot.borrow_mut() = Some(state);
    });
}

/// Handles `http_request` query calls.
///
/// Routes that exist in the certification tree are served with a v2 certificate
/// header.  Upgrade routes return `upgrade: true` so the boundary node retries
/// as `http_request_update`.  Unknown paths fall back to the certified 404
/// wildcard route.
pub fn handle_http_request(request: HttpRequest<'_>) -> HttpResponse<'static> {
    ensure_initialized();

    let path = match request.get_path() {
        Ok(path) => path,
        Err(error) => {
            log!(
                HttpLogPriority::Warn,
                "http_request malformed url={} err={}",
                request.url(),
                error
            );
            return HttpResponse::bad_request(
                br#"{"ok":false,"error":"malformed request url"}"#.as_slice(),
                vec![
                    (
                        HEADER_CONTENT_TYPE.to_string(),
                        CONTENT_TYPE_JSON.to_string(),
                    ),
                    (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
                ],
            )
            .build();
        }
    };

    HTTP_STATE.with(|slot| {
        let state = slot.borrow();
        let state = state
            .as_ref()
            .expect("http certification state must be initialized");
        if let Some(route) = state
            .routes
            .iter()
            .find(|route| route.method == *request.method() && route.request_path == path)
        {
            return render_certified_response(state, route, &path);
        }
        render_certified_response(state, &state.fallback_not_found, &path)
    })
}

/// Handles `http_request_update` calls — the mutable side of the HTTP
/// interface.  Each arm dispatches to the appropriate storage operation and
/// calls `init_certification` when a state change affects a GET-served route.
pub async fn handle_http_request_update(
    request: HttpUpdateRequest<'_>,
) -> HttpUpdateResponse<'static> {
    let path = match request.get_path() {
        Ok(path) => path,
        Err(_) => {
            return HttpResponse::bad_request(
                br#"{"ok":false,"error":"malformed request url"}"#.as_slice(),
                vec![
                    (
                        HEADER_CONTENT_TYPE.to_string(),
                        CONTENT_TYPE_JSON.to_string(),
                    ),
                    (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
                ],
            )
            .build_update();
        }
    };

    match (request.method(), path.as_str()) {
        (&Method::GET, "/api/snapshot") => {
            let snapshot = stable::observability_snapshot(DEFAULT_SNAPSHOT_LIMIT);
            json_update_response(StatusCode::OK, &snapshot)
        }
        (&Method::GET, "/api/steward/status") => {
            let status = stable::steward_status_view();
            json_update_response(StatusCode::OK, &status)
        }
        (&Method::GET, "/api/wallet/balance") => {
            let telemetry = stable::wallet_balance_telemetry_view();
            json_update_response(StatusCode::OK, &telemetry)
        }
        (&Method::GET, "/api/wallet/balance/sync-config") => {
            let config = stable::wallet_balance_sync_config_view();
            json_update_response(StatusCode::OK, &config)
        }
        (&Method::POST, "/api/conversation") => {
            match parse_conversation_lookup_request(request.body()) {
                Ok(payload) => match stable::get_conversation_log(&payload.sender) {
                    Some(log) => {
                        let hydrated = hydrate_conversation_agent_replies(log);
                        json_update_response(StatusCode::OK, &hydrated)
                    }
                    None => json_update_response(
                        StatusCode::NOT_FOUND,
                        &ConversationLookupError {
                            ok: false,
                            error: format!("conversation not found for sender {}", payload.sender),
                        },
                    ),
                },
                Err(error) => json_update_response(
                    StatusCode::BAD_REQUEST,
                    &ConversationLookupError { ok: false, error },
                ),
            }
        }
        (&Method::POST, "/api/steward/direct-message/prepare") => {
            match parse_steward_direct_message_prepare_request(request.body()) {
                Ok(payload) => match prepare_steward_direct_message_view(payload) {
                    Ok(view) => json_update_response(StatusCode::OK, &view),
                    Err(error) => json_update_response(
                        StatusCode::BAD_REQUEST,
                        &ConversationLookupError { ok: false, error },
                    ),
                },
                Err(error) => json_update_response(
                    StatusCode::BAD_REQUEST,
                    &ConversationLookupError { ok: false, error },
                ),
            }
        }
        (&Method::POST, "/api/steward/direct-message/execute") => {
            match parse_steward_direct_message_execute_request(request.body()) {
                Ok(payload) => {
                    let command = StewardCommand::SendStewardMessage {
                        sender: payload.sender,
                        message: payload.message,
                    };
                    match execute_signed_steward_command(command, payload.proof).await {
                        Ok(result) => json_update_response(StatusCode::OK, &result),
                        Err(error) => json_update_response(
                            StatusCode::BAD_REQUEST,
                            &ConversationLookupError { ok: false, error },
                        ),
                    }
                }
                Err(error) => json_update_response(
                    StatusCode::BAD_REQUEST,
                    &ConversationLookupError { ok: false, error },
                ),
            }
        }
        (&Method::POST, "/api/steward/model/prepare") => {
            match parse_steward_model_prepare_request(request.body()) {
                Ok(payload) => match prepare_steward_model_view(payload) {
                    Ok(view) => json_update_response(StatusCode::OK, &view),
                    Err(error) => json_update_response(
                        StatusCode::BAD_REQUEST,
                        &ConversationLookupError { ok: false, error },
                    ),
                },
                Err(error) => json_update_response(
                    StatusCode::BAD_REQUEST,
                    &ConversationLookupError { ok: false, error },
                ),
            }
        }
        (&Method::POST, "/api/steward/model/execute") => {
            match parse_steward_model_execute_request(request.body()) {
                Ok(payload) => {
                    match steward_set_model_command(payload.model) {
                        Ok(command) => match execute_signed_steward_command(command, payload.proof).await {
                            Ok(result) => json_update_response(StatusCode::OK, &result),
                            Err(error) => json_update_response(
                                StatusCode::BAD_REQUEST,
                                &ConversationLookupError { ok: false, error },
                            ),
                        },
                        Err(error) => json_update_response(
                            StatusCode::BAD_REQUEST,
                            &ConversationLookupError { ok: false, error },
                        ),
                    }
                }
                Err(error) => json_update_response(
                    StatusCode::BAD_REQUEST,
                    &ConversationLookupError { ok: false, error },
                ),
            }
        }
        (&Method::POST, "/api/steward/reasoning/prepare") => {
            match parse_steward_reasoning_prepare_request(request.body()) {
                Ok(payload) => match prepare_steward_reasoning_view(payload) {
                    Ok(view) => json_update_response(StatusCode::OK, &view),
                    Err(error) => json_update_response(
                        StatusCode::BAD_REQUEST,
                        &ConversationLookupError { ok: false, error },
                    ),
                },
                Err(error) => json_update_response(
                    StatusCode::BAD_REQUEST,
                    &ConversationLookupError { ok: false, error },
                ),
            }
        }
        (&Method::POST, "/api/steward/reasoning/execute") => {
            match parse_steward_reasoning_execute_request(request.body()) {
                Ok(payload) => {
                    let command = steward_set_reasoning_command(payload.variant);
                    match execute_signed_steward_command(command, payload.proof).await {
                        Ok(result) => json_update_response(StatusCode::OK, &result),
                        Err(error) => json_update_response(
                            StatusCode::BAD_REQUEST,
                            &ConversationLookupError { ok: false, error },
                        ),
                    }
                }
                Err(error) => json_update_response(
                    StatusCode::BAD_REQUEST,
                    &ConversationLookupError { ok: false, error },
                ),
            }
        }
        (&Method::GET, "/api/evm/config") => {
            let config = evm_config_view();
            json_update_response(StatusCode::OK, &config)
        }
        (&Method::GET, "/api/inference/config") => {
            let config = stable::inference_config_view();
            json_update_response(StatusCode::OK, &config)
        }
        (&Method::GET, "/api/inference/proxy/status") => {
            let status = stable::inference_proxy_status_view();
            json_update_response(StatusCode::OK, &status)
        }
        (&Method::GET, "/api/scheduler/config") => {
            let config = scheduler_config_view();
            json_update_response(StatusCode::OK, &config)
        }
        (&Method::GET, "/api/welcome") => {
            let view = welcome_view();
            json_update_response(StatusCode::OK, &view)
        }
        (&Method::GET, "/api/build-info") => {
            let view = build_info_view();
            json_update_response(StatusCode::OK, &view)
        }
        _ => HttpResponse::not_found(
            br#"{"ok":false,"error":"not found"}"#.as_slice(),
            vec![
                (
                    HEADER_CONTENT_TYPE.to_string(),
                    CONTENT_TYPE_JSON.to_string(),
                ),
                (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
            ],
        )
        .build_update(),
    }
}

/// Lazily initialises the certification state if not yet present.
/// Used as a safety net in `handle_http_request`; in production the explicit
/// `init_certification` call in `init`/`post_upgrade` should always pre-populate
/// the state.
fn ensure_initialized() {
    HTTP_STATE.with(|slot| {
        if slot.borrow().is_none() {
            let state = build_certification_state();
            set_tree_as_certified_data(&state.tree);
            *slot.borrow_mut() = Some(state);
        }
    });
}

// ── UI serving ───────────────────────────────────────────────────────────────

/// Constructs the full `HttpCertificationState` by reading current stable
/// storage values and building certified routes for all static assets and API
/// endpoints.  Inserts each route into a fresh `HttpCertificationTree`.
fn build_certification_state() -> HttpCertificationState {
    let snapshot = stable::observability_snapshot(DEFAULT_SNAPSHOT_LIMIT);
    let steward_status = stable::steward_status_view();
    let wallet_balance = stable::wallet_balance_telemetry_view();
    let wallet_sync_config = stable::wallet_balance_sync_config_view();
    let evm_config = evm_config_view();
    let inference_config = stable::inference_config_view();
    let inference_proxy_status = stable::inference_proxy_status_view();
    let scheduler_config = scheduler_config_view();
    let welcome = welcome_view();
    let build_info = build_info_view();

    let mut tree = HttpCertificationTree::default();
    let routes = vec![
        static_asset_route(
            Method::GET,
            "/",
            "/",
            UI_INDEX_HTML.as_bytes(),
            CONTENT_TYPE_HTML,
        ),
        static_asset_route(
            Method::GET,
            "/index.html",
            "/index.html",
            UI_INDEX_HTML.as_bytes(),
            CONTENT_TYPE_HTML,
        ),
        static_asset_route(
            Method::GET,
            "/styles.css",
            "/styles.css",
            UI_STYLES_CSS.as_bytes(),
            CONTENT_TYPE_CSS,
        ),
        static_asset_route(
            Method::GET,
            "/app.js",
            "/app.js",
            UI_APP_JS.as_bytes(),
            CONTENT_TYPE_JS,
        ),
        json_route(Method::GET, "/api/snapshot", &snapshot),
        json_route(Method::GET, "/api/steward/status", &steward_status),
        json_route(Method::GET, "/api/wallet/balance", &wallet_balance),
        json_route(
            Method::GET,
            "/api/wallet/balance/sync-config",
            &wallet_sync_config,
        ),
        json_route(Method::GET, "/api/evm/config", &evm_config),
        json_route(Method::GET, "/api/inference/config", &inference_config),
        json_route(
            Method::GET,
            "/api/inference/proxy/status",
            &inference_proxy_status,
        ),
        json_route(Method::GET, "/api/scheduler/config", &scheduler_config),
        json_route(Method::GET, "/api/welcome", &welcome),
        json_route(Method::GET, "/api/build-info", &build_info),
        upgrade_route(Method::POST, "/api/conversation"),
        upgrade_route(Method::POST, "/api/steward/direct-message/prepare"),
        upgrade_route(Method::POST, "/api/steward/direct-message/execute"),
        upgrade_route(Method::POST, "/api/steward/model/prepare"),
        upgrade_route(Method::POST, "/api/steward/model/execute"),
        upgrade_route(Method::POST, "/api/steward/reasoning/prepare"),
        upgrade_route(Method::POST, "/api/steward/reasoning/execute"),
    ];
    for route in &routes {
        let entry = HttpCertificationTreeEntry::new(&route.cert_path, route.certification);
        tree.insert(&entry);
    }

    let fallback_not_found = not_found_route();
    let fallback_entry = HttpCertificationTreeEntry::new(
        &fallback_not_found.cert_path,
        fallback_not_found.certification,
    );
    tree.insert(&fallback_entry);

    HttpCertificationState {
        tree,
        routes,
        fallback_not_found,
    }
}

/// Builds a `CertifiedRoute` for a static file asset (HTML, CSS, JS).
fn static_asset_route(
    method: Method,
    request_path: &'static str,
    cert_path: &'static str,
    body: &[u8],
    content_type: &'static str,
) -> CertifiedRoute {
    let base_response = HttpResponse::ok(
        body.to_vec(),
        vec![
            (HEADER_CONTENT_TYPE.to_string(), content_type.to_string()),
            (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
        ],
    )
    .build();

    certified_route(
        method,
        request_path,
        HttpCertificationPath::exact(cert_path),
        base_response,
    )
}

/// Serialises `payload` to JSON and builds a certified GET route for
/// `request_path`.  On serialization failure a 500 response with a static
/// error JSON body is used so callers reliably detect the failure path.
fn json_route<T: Serialize>(
    method: Method,
    request_path: &'static str,
    payload: &T,
) -> CertifiedRoute {
    let base_response = match serde_json::to_vec(payload) {
        Ok(body) => HttpResponse::ok(
            body,
            vec![
                (
                    HEADER_CONTENT_TYPE.to_string(),
                    CONTENT_TYPE_JSON.to_string(),
                ),
                (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
            ],
        )
        .build(),
        Err(error) => {
            log!(
                HttpLogPriority::Error,
                "http_json_serialize_error route={} err={}",
                request_path,
                error
            );
            HttpResponse::internal_server_error(
                br#"{"ok":false,"error":"serialization failed"}"#.as_slice(),
                vec![
                    (
                        HEADER_CONTENT_TYPE.to_string(),
                        CONTENT_TYPE_JSON.to_string(),
                    ),
                    (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
                ],
            )
            .build()
        }
    };

    certified_route(
        method,
        request_path,
        HttpCertificationPath::exact(request_path),
        base_response,
    )
}

/// Builds a certified route that signals `upgrade: true` to the boundary
/// node, causing it to retry the request as an update call.
fn upgrade_route(method: Method, request_path: &'static str) -> CertifiedRoute {
    let base_response = HttpResponse::ok(
        br#"{"upgrade":true}"#.as_slice(),
        vec![
            (
                HEADER_CONTENT_TYPE.to_string(),
                CONTENT_TYPE_JSON.to_string(),
            ),
            (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
        ],
    )
    .with_upgrade(true)
    .build();

    certified_route(
        method,
        request_path,
        HttpCertificationPath::exact(request_path),
        base_response,
    )
}

/// Builds the certified wildcard 404 fallback route that covers all paths not
/// explicitly registered in the tree.
fn not_found_route() -> CertifiedRoute {
    let base_response = HttpResponse::not_found(
        br#"404 Not Found"#.as_slice(),
        vec![
            (
                HEADER_CONTENT_TYPE.to_string(),
                "text/plain; charset=utf-8".to_string(),
            ),
            (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
        ],
    )
    .build();

    certified_route(
        Method::GET,
        "__wildcard_not_found__",
        HttpCertificationPath::wildcard("/"),
        base_response,
    )
}

/// Core builder: attaches the CEL expression header to `base_response`,
/// computes the `HttpCertification` proof, and packages everything into a
/// `CertifiedRoute`.
fn certified_route(
    method: Method,
    request_path: &'static str,
    cert_path: HttpCertificationPath<'static>,
    mut base_response: HttpResponse<'static>,
) -> CertifiedRoute {
    let cel_expr = DefaultCelBuilder::response_only_certification()
        .with_response_certification(DefaultResponseCertification::response_header_exclusions(
            vec![],
        ))
        .build();
    base_response.add_header((
        CERTIFICATE_EXPRESSION_HEADER_NAME.to_string(),
        cel_expr.to_string(),
    ));
    let certification = HttpCertification::response_only(&cel_expr, &base_response, None)
        .expect("response-only certification should succeed");
    #[cfg(target_arch = "wasm32")]
    let expr_path = cert_path.to_expr_path();

    CertifiedRoute {
        method,
        request_path,
        cert_path,
        #[cfg(target_arch = "wasm32")]
        expr_path,
        certification,
        base_response,
    }
}

/// Clones the pre-built base response and — on wasm32 — attaches the IC v2
/// certificate header using the data certificate and a Merkle witness for
/// `request_path`.
fn render_certified_response(
    state: &HttpCertificationState,
    route: &CertifiedRoute,
    request_path: &str,
) -> HttpResponse<'static> {
    #[cfg(target_arch = "wasm32")]
    let mut response = route.base_response.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let response = route.base_response.clone();

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (state, route, request_path);
    }

    #[cfg(target_arch = "wasm32")]
    {
        if let Some(data_certificate) = ic_cdk::api::data_certificate() {
            let entry = HttpCertificationTreeEntry::new(&route.cert_path, route.certification);
            match state.tree.witness(&entry, request_path) {
                Ok(witness) => {
                    add_v2_certificate_header(
                        &data_certificate,
                        &mut response,
                        &witness,
                        &route.expr_path,
                    );
                }
                Err(error) => {
                    log!(
                        HttpLogPriority::Error,
                        "http_witness_error request_path={} err={}",
                        request_path,
                        error
                    );
                }
            }
        } else {
            log!(
                HttpLogPriority::Warn,
                "http_data_certificate_missing request_path={}",
                request_path
            );
        }
    }
    response
}

/// Serialises `payload` to JSON and wraps it in an `HttpUpdateResponse` with
/// the given status code.  Falls back to a 500 plain-error body on
/// serialization failure.
fn json_update_response<T: Serialize>(
    status_code: StatusCode,
    payload: &T,
) -> HttpUpdateResponse<'static> {
    match serde_json::to_vec(payload) {
        Ok(body) => HttpResponse::builder()
            .with_status_code(status_code)
            .with_body(body)
            .with_headers(vec![
                (
                    HEADER_CONTENT_TYPE.to_string(),
                    CONTENT_TYPE_JSON.to_string(),
                ),
                (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
            ])
            .build_update(),
        Err(error) => {
            log!(
                HttpLogPriority::Error,
                "http_json_serialize_error err={}",
                error
            );
            HttpResponse::internal_server_error(
                br#"{"ok":false,"error":"serialization failed"}"#.as_slice(),
                vec![
                    (
                        HEADER_CONTENT_TYPE.to_string(),
                        CONTENT_TYPE_JSON.to_string(),
                    ),
                    (HEADER_CACHE_CONTROL.to_string(), CACHE_NO_STORE.to_string()),
                ],
            )
            .build_update()
        }
    }
}

/// Parses the `POST /api/conversation` request body and trims the sender field.
fn parse_conversation_lookup_request(body: &[u8]) -> Result<ConversationLookupRequest, String> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("conversation lookup body cannot be empty".to_string());
    }

    let payload = serde_json::from_slice::<ConversationLookupRequest>(body)
        .map_err(|error| format!("invalid conversation lookup payload: {error}"))?;
    let sender = payload.sender.trim();
    if sender.is_empty() {
        return Err("sender cannot be empty".to_string());
    }

    Ok(ConversationLookupRequest {
        sender: sender.to_string(),
    })
}

fn parse_steward_direct_message_prepare_request(
    body: &[u8],
) -> Result<StewardDirectMessagePrepareRequest, String> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("steward direct message prepare body cannot be empty".to_string());
    }

    let payload = serde_json::from_slice::<StewardDirectMessagePrepareRequest>(body)
        .map_err(|error| format!("invalid steward direct message prepare payload: {error}"))?;
    let (sender, message) = normalize_steward_direct_message(payload.sender, payload.message)?;
    Ok(StewardDirectMessagePrepareRequest { sender, message })
}

fn parse_steward_direct_message_execute_request(
    body: &[u8],
) -> Result<StewardDirectMessageExecuteRequest, String> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("steward direct message execute body cannot be empty".to_string());
    }

    let payload = serde_json::from_slice::<StewardDirectMessageExecuteRequest>(body)
        .map_err(|error| format!("invalid steward direct message execute payload: {error}"))?;
    let (sender, message) = normalize_steward_direct_message(payload.sender, payload.message)?;
    Ok(StewardDirectMessageExecuteRequest {
        sender,
        message,
        proof: payload.proof,
    })
}

fn parse_steward_model_prepare_request(body: &[u8]) -> Result<StewardModelPrepareRequest, String> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("steward model prepare body cannot be empty".to_string());
    }

    let payload = serde_json::from_slice::<StewardModelPrepareRequest>(body)
        .map_err(|error| format!("invalid steward model prepare payload: {error}"))?;
    let model = normalize_steward_model(payload.model)?;
    Ok(StewardModelPrepareRequest { model })
}

fn parse_steward_model_execute_request(body: &[u8]) -> Result<StewardModelExecuteRequest, String> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("steward model execute body cannot be empty".to_string());
    }

    let payload = serde_json::from_slice::<StewardModelExecuteRequest>(body)
        .map_err(|error| format!("invalid steward model execute payload: {error}"))?;
    let model = normalize_steward_model(payload.model)?;
    Ok(StewardModelExecuteRequest {
        model,
        proof: payload.proof,
    })
}

fn parse_steward_reasoning_prepare_request(
    body: &[u8],
) -> Result<StewardReasoningPrepareRequest, String> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("steward reasoning prepare body cannot be empty".to_string());
    }

    serde_json::from_slice::<StewardReasoningPrepareRequest>(body)
        .map_err(|error| format!("invalid steward reasoning prepare payload: {error}"))
}

fn parse_steward_reasoning_execute_request(
    body: &[u8],
) -> Result<StewardReasoningExecuteRequest, String> {
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Err("steward reasoning execute body cannot be empty".to_string());
    }

    serde_json::from_slice::<StewardReasoningExecuteRequest>(body)
        .map_err(|error| format!("invalid steward reasoning execute payload: {error}"))
}

/// Commits the Merkle root hash of `tree` as the canister's certified data.
/// No-op in native/test builds where the IC certified data API is unavailable.
fn set_tree_as_certified_data(tree: &HttpCertificationTree) {
    #[cfg(target_arch = "wasm32")]
    {
        let root_hash = tree.root_hash();
        ic_cdk::api::certified_data_set(&root_hash);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = tree;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn find_header<'a>(response: &'a HttpResponse<'_>, name: &str) -> Option<&'a str> {
        response
            .headers()
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn serves_root_asset_with_expected_headers() {
        init_certification();

        let request = HttpRequest::get("/").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert!(std::str::from_utf8(response.body())
            .expect("root body should be utf8")
            .contains("AUTOMATON"));
        assert_eq!(
            find_header(&response, HEADER_CONTENT_TYPE),
            Some(CONTENT_TYPE_HTML)
        );
    }

    #[test]
    fn serves_app_asset_with_history_and_config_commands_wired() {
        init_certification();

        let request = HttpRequest::get("/app.js").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(
            find_header(&response, HEADER_CONTENT_TYPE),
            Some(CONTENT_TYPE_JS)
        );

        let body = std::str::from_utf8(response.body()).expect("app.js body should be utf8");
        assert!(
            body.contains("Past messages and automaton responses"),
            "help output should mention the history command"
        );
        assert!(
            body.contains("Configuration overview"),
            "help output should mention the config command"
        );
        assert!(
            body.contains("case \"history\""),
            "command dispatcher should route the history command"
        );
        assert!(
            body.contains("case \"config\""),
            "command dispatcher should route the config command"
        );
        assert!(
            body.contains("LIVE STATUS VIEW"),
            "status command should open a live status view"
        );
    }

    #[test]
    fn serves_app_asset_with_steward_palette_guarded_by_wallet_match() {
        init_certification();

        let response = handle_http_request(HttpRequest::get("/app.js").build());

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = std::str::from_utf8(response.body()).expect("app.js body should be utf8");
        assert!(
            body.contains("buildHelpLines"),
            "help rendering should be built dynamically for steward-aware expansion"
        );
        assert!(
            body.contains("walletMatchesActiveSteward"),
            "steward palette expansion should gate on wallet/steward identity match"
        );
        assert!(
            body.contains("STEWARD COMMAND SURFACE"),
            "expanded help palette should include a steward command section"
        );
        assert!(
            body.contains("steward-send -m \"message\""),
            "expanded help palette should include direct steward messaging action"
        );
        assert!(
            body.contains("steward-model <model>"),
            "expanded help palette should include steward model command"
        );
        assert!(
            body.contains("steward-reasoning <variant>"),
            "expanded help palette should include steward reasoning command variants"
        );
        assert!(
            body.contains("google/gemini-3-flash-preview"),
            "expanded help palette should show arbitrary model examples"
        );
        assert!(
            body.contains("default|low|medium|high"),
            "expanded help palette should show reasoning variant hints"
        );
        assert!(
            body.contains("personal_sign"),
            "steward direct message flow should request wallet signature"
        );
        assert!(
            body.contains("/api/steward/direct-message/prepare"),
            "steward direct message flow should prepare a signed command payload"
        );
        assert!(
            body.contains("/api/steward/direct-message/execute"),
            "steward direct message flow should submit signed direct message commands"
        );
        assert!(
            body.contains("/api/steward/model/prepare"),
            "steward model flow should prepare signed model commands"
        );
        assert!(
            body.contains("/api/steward/model/execute"),
            "steward model flow should execute signed model commands"
        );
        assert!(
            body.contains("/api/steward/reasoning/prepare"),
            "steward reasoning flow should prepare signed reasoning commands"
        );
        assert!(
            body.contains("/api/steward/reasoning/execute"),
            "steward reasoning flow should execute signed reasoning commands"
        );
    }

    #[test]
    fn api_snapshot_query_path_returns_certified_json() {
        init_certification();

        let request = HttpRequest::get("/api/snapshot").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("snapshot body should decode as json");
        assert!(body.get("runtime").is_some());
    }

    #[test]
    fn get_wallet_balance_route_is_certified_query() {
        init_certification();

        let request = HttpRequest::get("/api/wallet/balance").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("wallet balance body should decode as json");
        assert!(body.get("status").is_some());
    }

    #[test]
    fn get_steward_status_route_is_certified_query() {
        init_certification();

        let request = HttpRequest::get("/api/steward/status").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("steward status body should decode as json");
        assert!(body.get("active_steward").is_some());
        assert!(body.get("next_nonce").is_some());
    }

    #[test]
    fn get_steward_status_route_returns_active_identity_and_nonce() {
        stable::init_storage();
        let stored = stable::set_active_steward(Some(crate::domain::types::StewardState {
            chain_id: 8453,
            address: "0xAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAa".to_string(),
            enabled: true,
            last_used_at_ns: Some(42),
            principal: None,
        }))
        .expect("active steward should persist")
        .expect("active steward should be set");
        let _ = stable::set_steward_nonce_state(crate::domain::types::StewardNonceState {
            next_nonce: 11,
        });
        init_certification();

        let response = futures::executor::block_on(handle_http_request_update(
            HttpRequest::get("/api/steward/status").build_update(),
        ));

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("steward status update body should decode as json");
        assert_eq!(
            body.pointer("/active_steward/chain_id")
                .and_then(Value::as_u64),
            Some(stored.chain_id)
        );
        assert_eq!(
            body.pointer("/active_steward/address")
                .and_then(Value::as_str),
            Some(stored.address.as_str())
        );
        assert_eq!(
            body.pointer("/active_steward/enabled")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(body.get("next_nonce").and_then(Value::as_u64), Some(11));
    }

    #[test]
    fn get_wallet_balance_sync_config_route_is_certified_query() {
        init_certification();

        let request = HttpRequest::get("/api/wallet/balance/sync-config").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("wallet sync config body should decode as json");
        assert!(body.get("enabled").is_some());
    }

    #[test]
    fn get_evm_config_route_is_certified_query() {
        init_certification();

        let request = HttpRequest::get("/api/evm/config").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body =
            serde_json::from_slice::<Value>(response.body()).expect("evm config should decode");
        assert!(body.get("chain_id").is_some());
    }

    #[test]
    fn get_evm_config_returns_expected_fields() {
        stable::init_storage();
        stable::set_evm_rpc_url(
            "https://base-mainnet.g.alchemy.com/v2/secret-api-key?foo=bar".to_string(),
        )
        .expect("rpc url should store");
        stable::set_evm_address(Some(
            "0x1111111111111111111111111111111111111111".to_string(),
        ))
        .expect("automaton address should store");
        stable::set_inbox_contract_address(Some(
            "0x2222222222222222222222222222222222222222".to_string(),
        ))
        .expect("inbox contract should store");
        stable::set_evm_chain_id(31337).expect("chain id should store");
        init_certification();

        let request = HttpRequest::get("/api/evm/config").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<serde_json::Value>(response.body())
            .expect("response should decode as json");
        assert_eq!(
            body.get("automaton_address")
                .and_then(serde_json::Value::as_str),
            Some("0x1111111111111111111111111111111111111111")
        );
        assert_eq!(
            body.get("inbox_contract_address")
                .and_then(serde_json::Value::as_str),
            Some("0x2222222222222222222222222222222222222222")
        );
        assert_eq!(
            body.get("chain_id").and_then(serde_json::Value::as_u64),
            Some(31337)
        );
        let rpc_url = body
            .get("rpc_url")
            .and_then(serde_json::Value::as_str)
            .expect("rpc_url should be present");
        assert_eq!(rpc_url, "https://base-mainnet.g.alchemy.com");
        assert!(!rpc_url.contains("secret-api-key"));
        assert!(!rpc_url.contains("/v2/"));
        assert!(!rpc_url.contains('?'));
    }

    #[test]
    fn wallet_balance_routes_return_safe_non_secret_views() {
        init_certification();
        stable::init_storage();
        stable::set_wallet_balance_snapshot(crate::domain::types::WalletBalanceSnapshot {
            eth_balance_wei_hex: Some("0x1".to_string()),
            usdc_balance_raw_hex: Some("0x2a".to_string()),
            usdc_decimals: 6,
            usdc_contract_address: Some("0x3333333333333333333333333333333333333333".to_string()),
            last_synced_at_ns: Some(1),
            last_synced_block: Some(123),
            last_error: Some("rpc timeout".to_string()),
        });
        stable::set_wallet_balance_bootstrap_pending(true);
        stable::set_wallet_balance_sync_config(crate::domain::types::WalletBalanceSyncConfig {
            enabled: true,
            normal_interval_secs: 300,
            low_cycles_interval_secs: 900,
            freshness_window_secs: 600,
            max_response_bytes: 256,
            discover_usdc_via_inbox: true,
        })
        .expect("wallet sync config should persist");

        let telemetry_response = futures::executor::block_on(handle_http_request_update(
            HttpRequest::get("/api/wallet/balance").build_update(),
        ));
        assert_eq!(telemetry_response.status_code(), StatusCode::OK);
        let telemetry = serde_json::from_slice::<Value>(telemetry_response.body())
            .expect("telemetry body should decode as json");
        assert_eq!(
            telemetry.get("eth_balance_wei_hex").and_then(Value::as_str),
            Some("0x1")
        );
        assert_eq!(
            telemetry
                .get("usdc_balance_raw_hex")
                .and_then(Value::as_str),
            Some("0x2a")
        );
        assert_eq!(
            telemetry.get("status").and_then(Value::as_str),
            Some("Error")
        );
        assert_eq!(
            telemetry.get("bootstrap_pending").and_then(Value::as_bool),
            Some(true)
        );
        assert!(telemetry.get("ecdsa_key_name").is_none());
        assert!(telemetry.get("evm_rpc_url").is_none());
        assert!(telemetry.get("openrouter_api_key").is_none());

        let config_response = futures::executor::block_on(handle_http_request_update(
            HttpRequest::get("/api/wallet/balance/sync-config").build_update(),
        ));
        assert_eq!(config_response.status_code(), StatusCode::OK);
        let config = serde_json::from_slice::<Value>(config_response.body())
            .expect("config body should decode as json");
        assert_eq!(
            config.get("normal_interval_secs").and_then(Value::as_u64),
            Some(300)
        );
        assert_eq!(
            config
                .get("low_cycles_interval_secs")
                .and_then(Value::as_u64),
            Some(900)
        );
        assert_eq!(
            config.get("freshness_window_secs").and_then(Value::as_u64),
            Some(600)
        );
        assert!(config.get("ecdsa_key_name").is_none());
        assert!(config.get("evm_rpc_url").is_none());
        assert!(config.get("openrouter_api_key").is_none());
    }

    #[test]
    fn unknown_paths_render_not_found_response() {
        init_certification();

        let request = HttpRequest::get("/no-such-path").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn get_welcome_route_returns_null_message_by_default() {
        stable::init_storage();
        init_certification();

        let request = HttpRequest::get("/api/welcome").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("welcome body should decode as json");
        assert_eq!(body.get("message"), Some(&Value::Null));
    }

    #[test]
    fn get_welcome_route_returns_custom_message_after_set() {
        stable::init_storage();
        stable::set_welcome_message("Hello from the automaton!".to_string())
            .expect("welcome message should store");
        init_certification();

        let request = HttpRequest::get("/api/welcome").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("welcome body should decode as json");
        assert_eq!(
            body.get("message").and_then(Value::as_str),
            Some("Hello from the automaton!")
        );
    }

    #[test]
    fn get_welcome_route_returns_null_message_after_clear() {
        stable::init_storage();
        stable::set_welcome_message("Welcome now".to_string()).expect("welcome message should set");
        stable::set_welcome_message("   ".to_string()).expect("welcome message should clear");
        init_certification();

        let request = HttpRequest::get("/api/welcome").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("welcome body should decode as json");
        assert_eq!(body.get("message"), Some(&Value::Null));
    }

    #[test]
    fn get_build_info_route_returns_commit_metadata() {
        stable::init_storage();
        init_certification();

        let request = HttpRequest::get("/api/build-info").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);

        let body = serde_json::from_slice::<Value>(response.body())
            .expect("build info body should decode as json");
        assert_eq!(
            body.get("commit").and_then(Value::as_str),
            Some(option_env!("AUTOMATON_GIT_COMMIT").unwrap_or("unknown"))
        );
    }

    #[test]
    fn get_build_info_route_prefers_installed_version_commit() {
        stable::init_storage();
        let _ = stable::set_spawn_bootstrap_metadata(crate::domain::types::SpawnBootstrapView {
            session_id: None,
            parent_id: None,
            factory_principal: None,
            risk: None,
            strategies: Vec::new(),
            skills: Vec::new(),
            version_commit: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        });
        init_certification();

        let request = HttpRequest::get("/api/build-info").build();
        let response = handle_http_request(request);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("build info body should decode as json");

        assert_eq!(
            body.get("commit").and_then(Value::as_str),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
    }

    #[test]
    fn get_build_info_route_returns_commit_metadata_via_update() {
        stable::init_storage();
        init_certification();

        let response = futures::executor::block_on(handle_http_request_update(
            HttpRequest::get("/api/build-info").build_update(),
        ));

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("build info update body should decode as json");
        assert_eq!(
            body.get("commit").and_then(Value::as_str),
            Some(option_env!("AUTOMATON_GIT_COMMIT").unwrap_or("unknown"))
        );
    }

    #[test]
    fn get_inference_config_route_is_certified_query() {
        init_certification();

        let request = HttpRequest::get("/api/inference/config").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("inference config body should decode as json");
        assert!(body.get("provider").is_some());
    }

    #[test]
    fn get_inference_proxy_status_route_is_certified_query() {
        stable::init_storage();
        init_certification();

        let request = HttpRequest::get("/api/inference/proxy/status").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("inference proxy status body should decode as json");
        assert!(body.get("pending_jobs").is_some());
        assert!(body.get("completed_jobs").is_some());
    }

    #[test]
    fn get_scheduler_config_route_is_certified_query() {
        stable::init_storage();
        stable::set_scheduler_base_tick_secs(300).expect("base tick should persist");
        init_certification();

        let request = HttpRequest::get("/api/scheduler/config").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), None);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("scheduler config body should decode as json");
        assert_eq!(
            body.get("base_tick_secs").and_then(Value::as_u64),
            Some(300)
        );
        assert!(body.get("ticks_per_turn_interval").is_some());
        assert!(body.get("default_turn_interval_secs").is_some());
    }

    #[test]
    fn post_inference_config_route_is_not_upgradable() {
        init_certification();

        let request = HttpRequest::post("/api/inference/config").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(response.upgrade(), None);
    }

    #[test]
    fn post_inference_proxy_status_route_is_not_upgradable() {
        init_certification();

        let request = HttpRequest::post("/api/inference/proxy/status").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(response.upgrade(), None);
    }

    #[test]
    fn post_inbox_route_is_not_upgradable() {
        init_certification();

        let request = HttpRequest::post("/api/inbox").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
        assert_eq!(response.upgrade(), None);
    }

    #[test]
    fn post_conversation_route_is_upgradable() {
        init_certification();

        let request = HttpRequest::post("/api/conversation").build();
        let response = handle_http_request(request);

        assert_eq!(response.status_code(), StatusCode::OK);
        assert_eq!(response.upgrade(), Some(true));
    }

    #[test]
    fn post_steward_direct_message_routes_are_upgradable() {
        init_certification();

        let prepare =
            handle_http_request(HttpRequest::post("/api/steward/direct-message/prepare").build());
        assert_eq!(prepare.status_code(), StatusCode::OK);
        assert_eq!(prepare.upgrade(), Some(true));

        let execute =
            handle_http_request(HttpRequest::post("/api/steward/direct-message/execute").build());
        assert_eq!(execute.status_code(), StatusCode::OK);
        assert_eq!(execute.upgrade(), Some(true));

        let model_prepare =
            handle_http_request(HttpRequest::post("/api/steward/model/prepare").build());
        assert_eq!(model_prepare.status_code(), StatusCode::OK);
        assert_eq!(model_prepare.upgrade(), Some(true));

        let model_execute =
            handle_http_request(HttpRequest::post("/api/steward/model/execute").build());
        assert_eq!(model_execute.status_code(), StatusCode::OK);
        assert_eq!(model_execute.upgrade(), Some(true));

        let reasoning_prepare =
            handle_http_request(HttpRequest::post("/api/steward/reasoning/prepare").build());
        assert_eq!(reasoning_prepare.status_code(), StatusCode::OK);
        assert_eq!(reasoning_prepare.upgrade(), Some(true));

        let reasoning_execute =
            handle_http_request(HttpRequest::post("/api/steward/reasoning/execute").build());
        assert_eq!(reasoning_execute.status_code(), StatusCode::OK);
        assert_eq!(reasoning_execute.upgrade(), Some(true));
    }

    #[test]
    fn steward_model_prepare_route_maps_model_to_set_inference_model_command() {
        stable::init_storage();
        stable::set_active_steward(Some(crate::domain::types::StewardState {
            chain_id: 8453,
            address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            enabled: true,
            last_used_at_ns: None,
            principal: None,
        }))
        .expect("active steward should persist");
        let _ = stable::set_steward_nonce_state(crate::domain::types::StewardNonceState {
            next_nonce: 19,
        });
        init_certification();

        let request: HttpUpdateRequest = HttpRequest::post("/api/steward/model/prepare")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{"model":" google/gemini-3-flash-preview "}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));
        assert_eq!(response.status_code(), StatusCode::OK);

        let body = serde_json::from_slice::<Value>(response.body())
            .expect("model prepare response should decode");
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some("google/gemini-3-flash-preview")
        );
        assert_eq!(
            body.pointer("/proof_template/nonce")
                .and_then(Value::as_u64),
            Some(19)
        );
        let expected_hash = steward_command_hash(&StewardCommand::SetInferenceModel {
            model: "google/gemini-3-flash-preview".to_string(),
        })
        .expect("model command hash should encode");
        assert_eq!(
            body.pointer("/proof_template/command_hash")
                .and_then(Value::as_str),
            Some(expected_hash.as_str())
        );
    }

    #[test]
    fn steward_reasoning_prepare_route_maps_variant_to_reasoning_command() {
        stable::init_storage();
        stable::set_active_steward(Some(crate::domain::types::StewardState {
            chain_id: 8453,
            address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            enabled: true,
            last_used_at_ns: None,
            principal: None,
        }))
        .expect("active steward should persist");
        let _ = stable::set_steward_nonce_state(crate::domain::types::StewardNonceState {
            next_nonce: 7,
        });
        init_certification();

        let request: HttpUpdateRequest = HttpRequest::post("/api/steward/reasoning/prepare")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{"variant":"high"}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));
        assert_eq!(response.status_code(), StatusCode::OK);

        let body = serde_json::from_slice::<Value>(response.body())
            .expect("reasoning prepare response should decode");
        assert_eq!(
            body.get("variant").and_then(Value::as_str),
            Some("high"),
            "response should preserve canonical reasoning variant"
        );
        let expected_hash = steward_command_hash(&StewardCommand::SetOpenrouterReasoningLevel {
            level: crate::domain::types::OpenRouterReasoningLevel::High,
        })
        .expect("reasoning command hash should encode");
        assert_eq!(
            body.pointer("/proof_template/command_hash")
                .and_then(Value::as_str),
            Some(expected_hash.as_str())
        );
    }

    #[test]
    fn conversation_lookup_returns_conversation_log() {
        init_certification();
        stable::init_storage();
        stable::append_conversation_entry(
            "0xAbCd00000000000000000000000000000000Ef12",
            crate::domain::types::ConversationEntry {
                inbox_message_id: "inbox:1".to_string(),
                outbox_message_id: None,
                sender_body: "hello".to_string(),
                agent_reply: "hi".to_string(),
                turn_id: "turn-1".to_string(),
                timestamp_ns: 1,
            },
        );

        let request: HttpUpdateRequest = HttpRequest::post("/api/conversation")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{"sender":"0xabcd00000000000000000000000000000000ef12"}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("response should decode as json");
        assert_eq!(
            body.get("sender").and_then(Value::as_str),
            Some("0xabcd00000000000000000000000000000000ef12")
        );
        assert_eq!(
            body.get("entries")
                .and_then(Value::as_array)
                .map(|entries| entries.len()),
            Some(1)
        );
    }

    #[test]
    fn conversation_lookup_returns_full_agent_reply_when_available() {
        init_certification();
        stable::init_storage();

        let sender = "0xAbCd00000000000000000000000000000000Ef12";
        let full_reply = "r".repeat(900);
        let outbox_id = stable::post_outbox_message(
            "turn-1".to_string(),
            full_reply.clone(),
            vec!["inbox:1".to_string()],
        )
        .expect("outbox message should be accepted");
        stable::append_conversation_entry(
            sender,
            crate::domain::types::ConversationEntry {
                inbox_message_id: "inbox:1".to_string(),
                outbox_message_id: Some(outbox_id),
                sender_body: "hello".to_string(),
                agent_reply: full_reply.clone(),
                turn_id: "turn-1".to_string(),
                timestamp_ns: 1,
            },
        );

        let request: HttpUpdateRequest = HttpRequest::post("/api/conversation")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{"sender":"0xabcd00000000000000000000000000000000ef12"}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("response should decode as json");
        assert_eq!(
            body.pointer("/entries/0/agent_reply")
                .and_then(Value::as_str)
                .map(str::len),
            Some(900),
            "history endpoint should expose full replies when available"
        );
    }

    #[test]
    fn conversation_lookup_falls_back_to_stored_preview_when_outbox_missing() {
        init_certification();
        stable::init_storage();

        let sender = "0xAbCd00000000000000000000000000000000Ef12";
        let full_reply = "r".repeat(900);
        stable::append_conversation_entry(
            sender,
            crate::domain::types::ConversationEntry {
                inbox_message_id: "inbox:1".to_string(),
                outbox_message_id: Some("outbox:missing".to_string()),
                sender_body: "hello".to_string(),
                agent_reply: full_reply,
                turn_id: "turn-1".to_string(),
                timestamp_ns: 1,
            },
        );

        let request: HttpUpdateRequest = HttpRequest::post("/api/conversation")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{"sender":"0xabcd00000000000000000000000000000000ef12"}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));

        assert_eq!(response.status_code(), StatusCode::OK);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("response should decode as json");
        assert_eq!(
            body.pointer("/entries/0/agent_reply")
                .and_then(Value::as_str)
                .map(str::len),
            Some(500),
            "history endpoint should keep stored preview when full outbox payload is missing"
        );
    }

    #[test]
    fn post_inbox_update_route_returns_not_found() {
        init_certification();

        let request: HttpUpdateRequest = HttpRequest::post("/api/inbox")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{"message":"legacy path"}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("response should decode as json");
        assert_eq!(body.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(body.get("error").and_then(Value::as_str), Some("not found"));
    }

    #[test]
    fn post_inference_config_update_returns_not_found() {
        init_certification();

        let request: HttpUpdateRequest = HttpRequest::post("/api/inference/config")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{"provider":"openrouter"}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("response should decode as json");
        assert_eq!(body.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(body.get("error").and_then(Value::as_str), Some("not found"));
    }

    #[test]
    fn post_inference_proxy_status_update_returns_not_found() {
        init_certification();

        let request: HttpUpdateRequest = HttpRequest::post("/api/inference/proxy/status")
            .with_headers(vec![(
                "content-type".to_string(),
                CONTENT_TYPE_JSON.to_string(),
            )])
            .with_body(br#"{}"#.to_vec())
            .build_update();
        let response = futures::executor::block_on(handle_http_request_update(request));

        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
        let body = serde_json::from_slice::<Value>(response.body())
            .expect("response should decode as json");
        assert_eq!(body.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(body.get("error").and_then(Value::as_str), Some("not found"));
    }
}
