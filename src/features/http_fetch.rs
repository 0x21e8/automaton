/// Controlled HTTPS GET tool with allowlist enforcement and response-size limits.
///
/// The `http_fetch_tool` function is the runtime entry point for the `http_fetch`
/// agent tool.  Before dispatching an outbound request it:
/// 1. Validates the URL is HTTPS and parses the hostname.
/// 2. Checks the hostname against the configurable domain allowlist (when enforced).
/// 3. Verifies the canister has enough liquid cycles to pay for the outcall.
/// 4. Optionally extracts structured content (`json_path` or `regex`) from the
///    response body.
/// 5. Truncates output to `HTTP_FETCH_MAX_OUTPUT_CHARS` and wraps it with
///    untrusted-content framing before returning it to the agent.
// ── Imports ──────────────────────────────────────────────────────────────────
use crate::domain::cycle_admission::{
    affordability_requirements, can_afford, estimate_operation_cost, OperationClass,
    DEFAULT_RESERVE_FLOOR_CYCLES, DEFAULT_SAFETY_MARGIN_BPS,
};
use crate::sanitize::frame_untrusted_content;
use crate::storage::stable;
use crate::timing::current_time_ns;
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::Value;

#[cfg(target_arch = "wasm32")]
use candid::Nat;
#[cfg(target_arch = "wasm32")]
use ic_cdk::management_canister::{http_request, HttpMethod, HttpRequestArgs};

// ── Constants ────────────────────────────────────────────────────────────────

/// Initial max response size for the first HTTP outcall attempt.
const HTTP_FETCH_INITIAL_RESPONSE_BYTES: u64 = 64 * 1024;
/// Hard cap for HTTP outcall response bytes after exponential retries.
const HTTP_FETCH_MAX_RESPONSE_BYTES: u64 = 512 * 1024;

/// Maximum number of UTF-8 characters returned to the agent after fetching.
/// Responses are truncated at this boundary with a `[truncated, N total bytes]` suffix.
const HTTP_FETCH_MAX_OUTPUT_CHARS: usize = 8_000;
const HTTP_FETCH_REGEX_MAX_PATTERN_CHARS: usize = 256;
const HTTP_FETCH_REGEX_SIZE_LIMIT_BYTES: usize = 256 * 1024;
const HTTP_FETCH_REGEX_DFA_SIZE_LIMIT_BYTES: usize = 256 * 1024;
const JSON_PATH_HINT_MAX_KEYS: usize = 8;
const JSON_PATH_FALLBACK_MAX_ATTEMPTS: usize = 4;

// ── Tool entry point ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct HttpFetchArgs {
    url: String,
    #[serde(default)]
    extract: Option<ExtractionMode>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode")]
enum ExtractionMode {
    #[serde(rename = "json_path")]
    JsonPath { path: String },
    #[serde(rename = "regex")]
    Regex { pattern: String },
}

/// Execute the `http_fetch` tool — parse args, enforce allowlist, check cycles, fetch.
///
/// Returns framed untrusted content with either the full response body or an
/// extracted value (depending on `extract` mode), truncated to
/// `HTTP_FETCH_MAX_OUTPUT_CHARS`. Binary bodies are represented as the literal
/// string `"binary response (not UTF-8)"`.
pub async fn http_fetch_tool(args_json: &str) -> Result<String, String> {
    let args = parse_http_fetch_args(args_json)?;
    let host = extract_https_host(&args.url)?;
    ensure_host_allowed(&host, &args.url)?;
    let request_size_bytes = u64::try_from(args.url.len().saturating_add(128)).unwrap_or(u64::MAX);

    let outcall_started_at_ns = current_time_ns();
    let body_result = http_get_with_size_retry(&args.url, request_size_bytes).await;
    let outcall_finished_at_ns = current_time_ns();
    let body = match body_result {
        Ok(body) => {
            stable::record_outcall_timing(
                stable::RuntimeOutcallKind::HttpFetch,
                outcall_started_at_ns,
                outcall_finished_at_ns,
                None,
                false,
            );
            body
        }
        Err(error) => {
            let timeout_failure = error.to_ascii_lowercase().contains("timeout");
            stable::record_outcall_timing(
                stable::RuntimeOutcallKind::HttpFetch,
                outcall_started_at_ns,
                outcall_finished_at_ns,
                Some(error.as_str()),
                timeout_failure,
            );
            return Err(error);
        }
    };
    let body =
        String::from_utf8(body).unwrap_or_else(|_| "binary response (not UTF-8)".to_string());
    let extracted = extract_http_fetch_content(&body, args.extract.as_ref())?;
    let output = truncate_http_fetch_output(&extracted);
    Ok(frame_untrusted_content("http_fetch", &output))
}

async fn http_get_with_size_retry(url: &str, request_size_bytes: u64) -> Result<Vec<u8>, String> {
    let mut max_response_bytes = HTTP_FETCH_INITIAL_RESPONSE_BYTES;
    loop {
        ensure_http_fetch_affordable(request_size_bytes, max_response_bytes)?;
        match http_get(url, max_response_bytes).await {
            Ok(body) => return Ok(body),
            Err(error) => {
                let Some(next_max) = next_retry_response_bytes(max_response_bytes, &error) else {
                    return Err(error);
                };
                max_response_bytes = next_max;
            }
        }
    }
}

fn next_retry_response_bytes(current_max_response_bytes: u64, error: &str) -> Option<u64> {
    if !is_response_too_large_error(error)
        || current_max_response_bytes >= HTTP_FETCH_MAX_RESPONSE_BYTES
    {
        return None;
    }
    Some((current_max_response_bytes.saturating_mul(2)).min(HTTP_FETCH_MAX_RESPONSE_BYTES))
}

fn is_response_too_large_error(error: &str) -> bool {
    let normalized = error.trim().to_ascii_lowercase();
    normalized.contains("http body exceeds size limit")
        || normalized.contains("response exceeded max_response_bytes")
}

fn parse_http_fetch_args(args_json: &str) -> Result<HttpFetchArgs, String> {
    let args: HttpFetchArgs = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid http_fetch args json: {error}"))?;
    if args.url.trim().is_empty() {
        return Err("missing required field: url".to_string());
    }
    Ok(args)
}

fn extract_http_fetch_content(
    body: &str,
    extract: Option<&ExtractionMode>,
) -> Result<String, String> {
    match extract {
        Some(ExtractionMode::JsonPath { path }) => extract_json_path(body, path),
        Some(ExtractionMode::Regex { pattern }) => extract_regex_lines(body, pattern),
        None => Ok(body.to_string()),
    }
}

fn extract_json_path(body: &str, path: &str) -> Result<String, String> {
    let trimmed_path = path.trim();
    if trimmed_path.is_empty() {
        return Err("json_path extraction failed: missing required field: path".to_string());
    }

    let root: Value = serde_json::from_str(body).map_err(|error| {
        format!("json_path extraction failed: response is not valid JSON: {error}")
    })?;

    let primary = extract_json_path_from_root(&root, trimmed_path);
    let Err(primary_error) = &primary else {
        return primary;
    };
    if !primary_error.starts_with("json_path extraction failed: path `") {
        return primary;
    }
    for fallback_path in json_path_fallback_candidates(trimmed_path)
        .into_iter()
        .take(JSON_PATH_FALLBACK_MAX_ATTEMPTS)
    {
        if fallback_path == trimmed_path {
            continue;
        }
        if let Ok(extracted) = extract_json_path_from_root(&root, &fallback_path) {
            return Ok(extracted);
        }
    }
    primary
}

fn extract_json_path_from_root(root: &Value, path: &str) -> Result<String, String> {
    let segments = parse_json_path_segments(path)?;
    let mut current = root;
    for segment in segments {
        let next = match &segment {
            JsonPathSegment::Field(name) => resolve_json_path_field(current, name),
            JsonPathSegment::Index(index) => current.as_array().and_then(|array| array.get(*index)),
        };
        current = next.ok_or_else(|| {
            format!(
                "json_path extraction failed: path `{path}` not found{}",
                json_path_missing_hint(current)
            )
        })?;
    }
    serialize_json_path_value(current)
}

fn serialize_json_path_value(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        other => serde_json::to_string(other).map_err(|error| {
            format!("json_path extraction failed: could not serialize extracted value: {error}")
        }),
    }
}

fn json_path_fallback_candidates(path: &str) -> Vec<String> {
    let normalized = path
        .trim()
        .strip_prefix("$.")
        .or_else(|| path.trim().strip_prefix('$'))
        .unwrap_or(path.trim());
    let mut candidates = Vec::new();

    if let Some(rest) = normalized.strip_prefix("pairs[0].") {
        push_unique_json_path_candidate(&mut candidates, format!("pair.{rest}"));
    }
    if let Some(rest) = normalized.strip_prefix("pair.") {
        push_unique_json_path_candidate(&mut candidates, format!("pairs[0].{rest}"));
    }
    if normalized == "market_data.current_price.usd" {
        push_unique_json_path_candidate(&mut candidates, "0.current_price".to_string());
    }
    if normalized == "0.current_price" {
        push_unique_json_path_candidate(
            &mut candidates,
            "market_data.current_price.usd".to_string(),
        );
    }

    candidates
}

fn push_unique_json_path_candidate(candidates: &mut Vec<String>, candidate: String) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn resolve_json_path_field<'a>(current: &'a Value, field_name: &str) -> Option<&'a Value> {
    current.get(field_name).or_else(|| {
        let index = field_name.parse::<usize>().ok()?;
        current.as_array().and_then(|array| array.get(index))
    })
}

fn json_path_missing_hint(current: &Value) -> String {
    match current {
        Value::Object(fields) => {
            if fields.is_empty() {
                return "; current node is empty object".to_string();
            }
            let listed = fields
                .keys()
                .take(JSON_PATH_HINT_MAX_KEYS)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            let ellipsis = if fields.len() > JSON_PATH_HINT_MAX_KEYS {
                ", ..."
            } else {
                ""
            };
            format!("; current node is object with keys: {listed}{ellipsis}")
        }
        Value::Array(items) => format!(
            "; current node is array (len={}); use [index] syntax for array access",
            items.len()
        ),
        other => format!("; current node is {}", json_value_kind(other)),
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[derive(Debug)]
enum JsonPathSegment {
    Field(String),
    Index(usize),
}

fn parse_json_path_segments(path: &str) -> Result<Vec<JsonPathSegment>, String> {
    let mut segments = Vec::<JsonPathSegment>::new();
    // Strip optional JSONPath root prefix ($. or standalone $)
    let path = path
        .strip_prefix("$.")
        .or_else(|| path.strip_prefix("$"))
        .unwrap_or(path);
    let chars = path.chars().collect::<Vec<_>>();
    let mut i = 0usize;
    let len = chars.len();

    while i < len {
        if chars[i] == '.' {
            return Err(format!(
                "json_path extraction failed: invalid path `{path}`"
            ));
        }

        if chars[i] == '[' {
            let (index, next) = parse_json_path_index(&chars, i, path)?;
            segments.push(JsonPathSegment::Index(index));
            i = next;
        } else {
            let start = i;
            while i < len && chars[i] != '.' && chars[i] != '[' {
                i = i.saturating_add(1);
            }
            let field = chars[start..i].iter().collect::<String>();
            if field.trim().is_empty() {
                return Err(format!(
                    "json_path extraction failed: invalid path `{path}`"
                ));
            }
            segments.push(JsonPathSegment::Field(field));
        }

        while i < len && chars[i] == '[' {
            let (index, next) = parse_json_path_index(&chars, i, path)?;
            segments.push(JsonPathSegment::Index(index));
            i = next;
        }

        if i < len {
            if chars[i] != '.' {
                return Err(format!(
                    "json_path extraction failed: invalid path `{path}`"
                ));
            }
            i = i.saturating_add(1);
            if i == len {
                return Err(format!(
                    "json_path extraction failed: invalid path `{path}`"
                ));
            }
        }
    }

    if segments.is_empty() {
        return Err(format!(
            "json_path extraction failed: invalid path `{path}`"
        ));
    }
    Ok(segments)
}

fn parse_json_path_index(
    chars: &[char],
    start: usize,
    raw_path: &str,
) -> Result<(usize, usize), String> {
    let mut i = start.saturating_add(1);
    let mut digits = String::new();

    while i < chars.len() && chars[i] != ']' {
        digits.push(chars[i]);
        i = i.saturating_add(1);
    }

    if i >= chars.len() || chars[i] != ']' {
        return Err(format!(
            "json_path extraction failed: invalid path `{raw_path}`"
        ));
    }
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "json_path extraction failed: invalid path `{raw_path}`"
        ));
    }

    let index = digits
        .parse::<usize>()
        .map_err(|_| format!("json_path extraction failed: invalid path `{raw_path}`"))?;
    Ok((index, i.saturating_add(1)))
}

fn extract_regex_lines(body: &str, pattern: &str) -> Result<String, String> {
    let trimmed_pattern = pattern.trim();
    if trimmed_pattern.is_empty() {
        return Err("regex extraction failed: missing required field: pattern".to_string());
    }
    if trimmed_pattern.chars().count() > HTTP_FETCH_REGEX_MAX_PATTERN_CHARS {
        return Err(format!(
            "regex extraction failed: pattern exceeds max length of {HTTP_FETCH_REGEX_MAX_PATTERN_CHARS} characters"
        ));
    }

    let regex = RegexBuilder::new(trimmed_pattern)
        .size_limit(HTTP_FETCH_REGEX_SIZE_LIMIT_BYTES)
        .dfa_size_limit(HTTP_FETCH_REGEX_DFA_SIZE_LIMIT_BYTES)
        .build()
        .map_err(|error| format!("regex extraction failed: invalid pattern: {error}"))?;

    let matched_lines = body
        .lines()
        .filter(|line| regex.is_match(line))
        .collect::<Vec<_>>();
    if matched_lines.is_empty() {
        return Err("regex extraction failed: no matching lines".to_string());
    }

    Ok(matched_lines.join("\n"))
}

fn ensure_host_allowed(host: &str, url: &str) -> Result<(), String> {
    if !stable::is_http_allowlist_enforced() {
        return Ok(());
    }
    let allowed = stable::list_allowed_http_domains();
    if allowed.is_empty() {
        return Err("no domains allowed".to_string());
    }
    if allowed
        .iter()
        .any(|domain| host == domain || host.ends_with(&format!(".{domain}")))
    {
        Ok(())
    } else {
        Err(format!("domain not in allowlist: {url}"))
    }
}

fn extract_https_host(raw_url: &str) -> Result<String, String> {
    let trimmed = raw_url.trim();
    let remainder = trimmed
        .strip_prefix("https://")
        .ok_or_else(|| "only HTTPS URLs are allowed".to_string())?;
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    if authority.is_empty() {
        return Err("could not parse host".to_string());
    }
    if authority.contains('@') {
        return Err("user info is not allowed in URL".to_string());
    }
    if authority.starts_with('[') {
        return Err("IPv6 hosts are not supported".to_string());
    }

    let host = authority
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err("could not parse host".to_string());
    }
    if host.starts_with('.') || host.ends_with('.') {
        return Err("host is invalid".to_string());
    }

    for label in host.split('.') {
        if label.is_empty() {
            return Err("host is invalid".to_string());
        }
        let bytes = label.as_bytes();
        if !bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || !bytes
                .last()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        {
            return Err("host is invalid".to_string());
        }
    }

    Ok(host)
}

fn ensure_http_fetch_affordable(
    request_size_bytes: u64,
    max_response_bytes: u64,
) -> Result<(), String> {
    let operation = OperationClass::HttpOutcall {
        request_size_bytes,
        max_response_bytes,
    };
    let estimated = estimate_operation_cost(&operation)?;
    let requirements = affordability_requirements(
        estimated,
        DEFAULT_SAFETY_MARGIN_BPS,
        DEFAULT_RESERVE_FLOOR_CYCLES,
    );
    let liquid = liquid_cycle_balance();
    if !can_afford(liquid, &requirements) {
        return Err("insufficient cycles for HTTP fetch".to_string());
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn liquid_cycle_balance() -> u128 {
    ic_cdk::api::canister_liquid_cycle_balance()
}

#[cfg(not(target_arch = "wasm32"))]
fn liquid_cycle_balance() -> u128 {
    u128::MAX
}

#[cfg(target_arch = "wasm32")]
async fn http_get(url: &str, max_response_bytes: u64) -> Result<Vec<u8>, String> {
    let request = HttpRequestArgs {
        url: url.to_string(),
        max_response_bytes: Some(max_response_bytes),
        method: HttpMethod::GET,
        headers: vec![],
        body: None,
        transform: None,
        is_replicated: Some(false),
    };

    let response = http_request(&request)
        .await
        .map_err(|error| format!("HTTP fetch failed: {error}"))?;
    let status = nat_to_u16(&response.status)?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} from {url}"));
    }
    Ok(response.body)
}

#[cfg(not(target_arch = "wasm32"))]
async fn http_get(url: &str, _max_response_bytes: u64) -> Result<Vec<u8>, String> {
    if url.contains("coingecko") {
        Ok(br#"{"stub":"coingecko"}"#.to_vec())
    } else {
        Ok(br#"{"stub":"ok"}"#.to_vec())
    }
}

#[cfg(target_arch = "wasm32")]
fn nat_to_u16(status: &Nat) -> Result<u16, String> {
    status
        .to_string()
        .parse::<u16>()
        .map_err(|error| format!("invalid HTTP status {status}: {error}"))
}

fn truncate_utf8_chars(input: &str, max_chars: usize) -> (String, bool) {
    let Some((cutoff, _)) = input.char_indices().nth(max_chars) else {
        return (input.to_string(), false);
    };
    (input[..cutoff].to_string(), true)
}

fn truncate_http_fetch_output(content: &str) -> String {
    let (truncated, was_truncated) = truncate_utf8_chars(content, HTTP_FETCH_MAX_OUTPUT_CHARS);
    if was_truncated {
        format!(
            "{}... [truncated, {} total bytes]",
            truncated,
            content.len()
        )
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::block_on_with_spin;

    #[test]
    fn extract_https_host_rejects_non_https_urls() {
        assert!(extract_https_host("http://example.com").is_err());
        assert!(extract_https_host("example.com").is_err());
    }

    #[test]
    fn http_fetch_tool_allows_any_domain_by_default() {
        stable::init_storage();

        let out = block_on_with_spin(http_fetch_tool(r#"{"url":"https://example.com/anything"}"#))
            .expect("without configured allowlist any https host should pass");
        assert!(out.starts_with("[UNTRUSTED_CONTENT source=http_fetch]"));
        assert!(out.contains("stub"));
    }

    #[test]
    fn http_fetch_tool_records_outcall_latency_telemetry() {
        stable::init_storage();

        let _ = block_on_with_spin(http_fetch_tool(r#"{"url":"https://example.com/anything"}"#))
            .expect("host stub request should pass");
        let snapshot = stable::runtime_snapshot();
        let stats = snapshot.timing_telemetry.http_fetch_outcall;
        assert_eq!(stats.total_calls, 1);
        assert_eq!(stats.failure_calls, 0);
        assert_eq!(stats.timeout_failures, 0);
        assert!(
            stats.last_duration_ms.is_some(),
            "outcall telemetry should capture latency"
        );
        assert!(stats.last_error.is_none());
    }

    #[test]
    fn response_size_retry_caps_double_until_hard_limit() {
        assert_eq!(
            next_retry_response_bytes(
                64 * 1024,
                "HTTP fetch failed: call rejected: 1 - Http body exceeds size limit of 65536 bytes."
            ),
            Some(128 * 1024)
        );
        assert_eq!(
            next_retry_response_bytes(
                128 * 1024,
                "HTTP fetch failed: call rejected: 1 - Http body exceeds size limit of 131072 bytes."
            ),
            Some(256 * 1024)
        );
        assert_eq!(
            next_retry_response_bytes(
                256 * 1024,
                "HTTP fetch failed: call rejected: 1 - Http body exceeds size limit of 262144 bytes."
            ),
            Some(512 * 1024)
        );
        assert_eq!(
            next_retry_response_bytes(
                512 * 1024,
                "HTTP fetch failed: call rejected: 1 - Http body exceeds size limit of 524288 bytes."
            ),
            None
        );
    }

    #[test]
    fn response_size_retry_only_triggers_for_too_large_errors() {
        assert!(is_response_too_large_error(
            "HTTP fetch failed: call rejected: 1 - Http body exceeds size limit of 65536 bytes."
        ));
        assert!(is_response_too_large_error(
            "host rpc response exceeded max_response_bytes=65536"
        ));
        assert!(!is_response_too_large_error(
            "HTTP 404 from https://example.com"
        ));
    }

    #[test]
    fn http_fetch_tool_uses_allowlist_when_configured() {
        stable::init_storage();
        stable::set_http_allowed_domains(vec!["api.coingecko.com".to_string()])
            .expect("allowlist should set");

        let out = block_on_with_spin(http_fetch_tool(
            r#"{"url":"https://api.coingecko.com/api/v3/ping"}"#,
        ))
        .expect("host stub request should pass");
        assert!(out.starts_with("[UNTRUSTED_CONTENT source=http_fetch]"));
        assert!(out.contains("stub"));

        let err = block_on_with_spin(http_fetch_tool(
            r#"{"url":"https://example.com/forbidden"}"#,
        ))
        .expect_err("non-allowlisted host should fail");
        assert!(err.contains("domain not in allowlist"));
    }

    #[test]
    fn http_fetch_tool_blocks_when_allowlist_is_enforced_but_empty() {
        stable::init_storage();
        stable::set_http_allowed_domains(vec![]).expect("empty allowlist should be valid");

        let err = block_on_with_spin(http_fetch_tool(r#"{"url":"https://example.com"}"#))
            .expect_err("configured empty allowlist should block all hosts");
        assert!(err.contains("no domains allowed"));
    }

    #[test]
    fn parse_http_fetch_args_accepts_json_path_extract_mode() {
        let args = parse_http_fetch_args(
            r#"{"url":"https://example.com","extract":{"mode":"json_path","path":"data.price"}}"#,
        )
        .expect("json_path extract args should parse");
        assert_eq!(args.url, "https://example.com");
        assert_eq!(
            args.extract,
            Some(ExtractionMode::JsonPath {
                path: "data.price".to_string()
            })
        );
    }

    #[test]
    fn http_fetch_tool_json_path_extracts_and_frames_value() {
        stable::init_storage();

        let out = block_on_with_spin(http_fetch_tool(
            r#"{"url":"https://example.com/anything","extract":{"mode":"json_path","path":"stub"}}"#,
        ))
        .expect("json_path extraction should succeed");
        assert!(out.starts_with("[UNTRUSTED_CONTENT source=http_fetch]"));
        assert!(out.contains("\n---\nok\n---\n"));
    }

    #[test]
    fn http_fetch_tool_json_path_reports_invalid_json() {
        let err = extract_json_path("not json", "stub")
            .expect_err("invalid json input should fail json_path extraction");
        assert!(err.contains("response is not valid JSON"));
    }

    #[test]
    fn http_fetch_tool_json_path_reports_missing_path() {
        let err = extract_json_path(r#"{"data":{"price":42}}"#, "data.missing")
            .expect_err("missing path should fail json_path extraction");
        assert!(err.contains("path `data.missing` not found"));
    }

    #[test]
    fn http_fetch_tool_json_path_supports_array_indexing() {
        let out = extract_json_path(
            r#"{"pairs":[{"priceUsd":"0.31"},{"priceUsd":"0.32"}]}"#,
            "pairs[1].priceUsd",
        )
        .expect("array-index json_path extraction should succeed");
        assert_eq!(out, "0.32");
    }

    #[test]
    fn http_fetch_tool_json_path_accepts_dot_number_array_alias() {
        let out = extract_json_path(r#"[{"current_price":"123.45"}]"#, "0.current_price")
            .expect("dot-number array alias should resolve to index");
        assert_eq!(out, "123.45");
    }

    #[test]
    fn http_fetch_tool_json_path_fallbacks_dexscreener_pair_object_variant() {
        let out = extract_json_path(r#"{"pair":{"priceUsd":"0.31"}}"#, "pairs[0].priceUsd")
            .expect("fallback should resolve dexscreener pair object variant");
        assert_eq!(out, "0.31");
    }

    #[test]
    fn http_fetch_tool_json_path_fallbacks_dexscreener_pairs_array_variant() {
        let out = extract_json_path(r#"{"pairs":[{"priceUsd":"0.32"}]}"#, "pair.priceUsd")
            .expect("fallback should resolve dexscreener pairs array variant");
        assert_eq!(out, "0.32");
    }

    #[test]
    fn http_fetch_tool_json_path_fallbacks_coingecko_markets_array_variant() {
        let out = extract_json_path(
            r#"[{"current_price":1234.56}]"#,
            "market_data.current_price.usd",
        )
        .expect("fallback should resolve coingecko markets array variant");
        assert_eq!(out, "1234.56");
    }

    #[test]
    fn http_fetch_tool_json_path_fallbacks_coingecko_market_data_object_variant() {
        let out = extract_json_path(
            r#"{"market_data":{"current_price":{"usd":1234.56}}}"#,
            "0.current_price",
        )
        .expect("fallback should resolve coingecko market_data object variant");
        assert_eq!(out, "1234.56");
    }

    #[test]
    fn http_fetch_tool_json_path_rejects_invalid_array_index_path() {
        let err = extract_json_path(r#"{"pairs":[{"priceUsd":"0.31"}]}"#, "pairs[one].priceUsd")
            .expect_err("non-numeric array index should fail json_path extraction");
        assert!(err.contains("invalid path"));
    }

    #[test]
    fn http_fetch_tool_json_path_strips_dollar_dot_prefix() {
        let out = extract_json_path(r#"{"pairs":[{"priceUsd":"0.31"}]}"#, "$.pairs[0].priceUsd")
            .expect("$.prefix json_path extraction should succeed");
        assert_eq!(out, "0.31");
    }

    #[test]
    fn http_fetch_tool_json_path_strips_standalone_dollar_prefix() {
        let out = extract_json_path(r#"{"pairs":[{"priceUsd":"0.31"}]}"#, "$pairs[0].priceUsd")
            .expect("$prefix json_path extraction should succeed");
        assert_eq!(out, "0.31");
    }

    #[test]
    fn http_fetch_tool_json_path_missing_path_error_lists_object_keys() {
        let err = extract_json_path(r#"{"data":{"price":42},"pairs":[]}"#, "missing.value")
            .expect_err("missing path should include object-key hint");
        assert!(err.contains("path `missing.value` not found"));
        assert!(err.contains("current node is object with keys:"));
        assert!(err.contains("data"));
        assert!(err.contains("pairs"));
    }

    #[test]
    fn http_fetch_tool_json_path_missing_path_error_hints_array_index_syntax() {
        let err = extract_json_path(r#"[{"current_price":"123.45"}]"#, "current_price")
            .expect_err("array root mismatch should include array hint");
        assert!(err.contains("path `current_price` not found"));
        assert!(err.contains("current node is array (len=1)"));
        assert!(err.contains("use [index] syntax"));
    }

    #[test]
    fn http_fetch_tool_regex_extracts_matching_lines() {
        let out = extract_regex_lines("alpha\nprice:42\nbeta", r"^price:\d+$")
            .expect("regex extraction should return matching lines");
        assert_eq!(out, "price:42");
    }

    #[test]
    fn http_fetch_tool_regex_reports_invalid_pattern() {
        let err =
            extract_regex_lines("price:42", "(").expect_err("invalid regex pattern should fail");
        assert!(err.contains("invalid pattern"));
    }

    #[test]
    fn http_fetch_tool_regex_reports_no_matches() {
        let err = extract_regex_lines("alpha\nbeta", "price")
            .expect_err("regex extraction without matches should fail");
        assert!(err.contains("no matching lines"));
    }
}
