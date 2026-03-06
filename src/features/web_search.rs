use crate::domain::cycle_admission::{
    affordability_requirements, can_afford, estimate_operation_cost, OperationClass,
    DEFAULT_RESERVE_FLOOR_CYCLES, DEFAULT_SAFETY_MARGIN_BPS,
};
use crate::sanitize::frame_untrusted_content;
use crate::storage::stable;
use serde::Deserialize;

#[cfg(target_arch = "wasm32")]
use candid::Nat;
#[cfg(target_arch = "wasm32")]
use ic_cdk::management_canister::{http_request, HttpHeader, HttpMethod, HttpRequestArgs};

const WEB_SEARCH_DEFAULT_RESULTS: usize = 5;
const WEB_SEARCH_MAX_RESULTS: usize = 10;
const WEB_SEARCH_MAX_OUTPUT_CHARS: usize = 4_000;
const WEB_SEARCH_MAX_SNIPPET_CHARS: usize = 200;
const WEB_SEARCH_MAX_DOMAIN_FILTERS: usize = 5;
const WEB_SEARCH_MAX_RESPONSE_BYTES: u64 = 16 * 1024;

#[derive(Deserialize)]
struct WebSearchArgs {
    query: String,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    freshness: Option<String>,
    #[serde(default)]
    include_domains: Option<Vec<String>>,
    #[serde(default)]
    exclude_domains: Option<Vec<String>>,
}

#[derive(Debug)]
struct NormalizedWebSearchArgs {
    query: String,
    count: usize,
    freshness: Option<String>,
    include_domains: Vec<String>,
    exclude_domains: Vec<String>,
}

#[derive(Deserialize)]
struct BraveSearchResponse {
    #[serde(default)]
    web: Option<BraveSearchWeb>,
}

#[derive(Deserialize, Default)]
struct BraveSearchWeb {
    #[serde(default)]
    results: Vec<BraveSearchResult>,
}

#[derive(Deserialize, Default)]
struct BraveSearchResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    age: Option<String>,
    #[serde(default)]
    page_age: Option<String>,
}

pub async fn web_search_tool(args_json: &str) -> Result<String, String> {
    let args = parse_web_search_args(args_json)?;
    let api_key = stable::get_search_api_key()
        .ok_or_else(|| "search api key is not configured".to_string())?;

    let url = build_brave_search_url(&args);
    let request_size_bytes =
        u64::try_from(url.len().saturating_add(api_key.len()).saturating_add(256))
            .unwrap_or(u64::MAX);
    ensure_web_search_affordable(request_size_bytes, WEB_SEARCH_MAX_RESPONSE_BYTES)?;

    let body = http_get_search(&url, &api_key, WEB_SEARCH_MAX_RESPONSE_BYTES).await?;
    let response: BraveSearchResponse = serde_json::from_slice(&body)
        .map_err(|error| format!("web_search failed to parse provider response: {error}"))?;
    let formatted = format_search_results(&args.query, &response)?;
    Ok(frame_untrusted_content("web_search", &formatted))
}

fn parse_web_search_args(args_json: &str) -> Result<NormalizedWebSearchArgs, String> {
    let args: WebSearchArgs = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid web_search args json: {error}"))?;
    let query = args.query.trim();
    if query.is_empty() {
        return Err("missing required field: query".to_string());
    }

    let count = args
        .count
        .unwrap_or(WEB_SEARCH_DEFAULT_RESULTS)
        .clamp(1, WEB_SEARCH_MAX_RESULTS);
    let freshness = normalize_freshness(args.freshness)?;
    let include_domains = normalize_domains(args.include_domains)?;
    let exclude_domains = normalize_domains(args.exclude_domains)?;

    if include_domains.len() > WEB_SEARCH_MAX_DOMAIN_FILTERS {
        return Err(format!(
            "include_domains may contain at most {WEB_SEARCH_MAX_DOMAIN_FILTERS} entries"
        ));
    }
    if exclude_domains.len() > WEB_SEARCH_MAX_DOMAIN_FILTERS {
        return Err(format!(
            "exclude_domains may contain at most {WEB_SEARCH_MAX_DOMAIN_FILTERS} entries"
        ));
    }
    if include_domains
        .iter()
        .any(|domain| exclude_domains.iter().any(|blocked| blocked == domain))
    {
        return Err("include_domains and exclude_domains must not overlap".to_string());
    }

    Ok(NormalizedWebSearchArgs {
        query: query.to_string(),
        count,
        freshness,
        include_domains,
        exclude_domains,
    })
}

fn normalize_freshness(freshness: Option<String>) -> Result<Option<String>, String> {
    let Some(freshness) = freshness else {
        return Ok(None);
    };
    let normalized = freshness.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "any" => Ok(None),
        "day" | "week" | "month" => Ok(Some(normalized)),
        _ => Err("freshness must be one of: day, week, month, any".to_string()),
    }
}

fn normalize_domains(domains: Option<Vec<String>>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for entry in domains.unwrap_or_default() {
        let domain = normalize_domain(&entry)?;
        if !normalized.iter().any(|existing| existing == &domain) {
            normalized.push(domain);
        }
    }
    Ok(normalized)
}

fn normalize_domain(domain: &str) -> Result<String, String> {
    let trimmed = domain.trim();
    if trimmed.is_empty() {
        return Err("domain filters must not contain empty entries".to_string());
    }

    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = without_scheme.split('/').next().unwrap_or_default().trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty()
        || host.chars().any(char::is_whitespace)
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
    {
        return Err(format!("invalid domain filter: {trimmed}"));
    }
    Ok(host)
}

fn build_brave_search_url(args: &NormalizedWebSearchArgs) -> String {
    let mut query = args.query.clone();
    if !args.include_domains.is_empty() {
        let scoped = args
            .include_domains
            .iter()
            .map(|domain| format!("site:{domain}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        query.push_str(" (");
        query.push_str(&scoped);
        query.push(')');
    }
    for domain in &args.exclude_domains {
        query.push(' ');
        query.push_str("-site:");
        query.push_str(domain);
    }

    let mut params = vec![
        format!("q={}", percent_encode_query_component(&query)),
        format!("count={}", args.count),
    ];
    if let Some(freshness) = &args.freshness {
        params.push(format!(
            "freshness={}",
            percent_encode_query_component(freshness)
        ));
    }

    format!(
        "https://api.search.brave.com/res/v1/web/search?{}",
        params.join("&")
    )
}

fn percent_encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~' => encoded.push(char::from(byte)),
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn ensure_web_search_affordable(
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
        return Err("insufficient cycles for web_search".to_string());
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
async fn http_get_search(
    url: &str,
    api_key: &str,
    max_response_bytes: u64,
) -> Result<Vec<u8>, String> {
    let started_at_ns = crate::timing::current_time_ns();
    let request = HttpRequestArgs {
        url: url.to_string(),
        max_response_bytes: Some(max_response_bytes),
        method: HttpMethod::GET,
        headers: vec![HttpHeader {
            name: "X-Subscription-Token".to_string(),
            value: api_key.to_string(),
        }],
        body: None,
        transform: None,
        is_replicated: Some(false),
    };

    let response = http_request(&request)
        .await
        .map_err(|error| format!("web_search failed: {error}"))?;
    let finished_at_ns = crate::timing::current_time_ns();
    let status = nat_to_u16(&response.status)?;
    if !(200..300).contains(&status) {
        let error = format!("HTTP {status}");
        stable::record_outcall_timing(
            stable::RuntimeOutcallKind::HttpFetch,
            started_at_ns,
            finished_at_ns,
            Some(error.as_str()),
            false,
        );
        return Err(format!("web_search provider returned HTTP {status}"));
    }
    stable::record_outcall_timing(
        stable::RuntimeOutcallKind::HttpFetch,
        started_at_ns,
        finished_at_ns,
        None,
        false,
    );
    Ok(response.body)
}

#[cfg(target_arch = "wasm32")]
fn nat_to_u16(value: &Nat) -> Result<u16, String> {
    value.0
        .to_string()
        .parse::<u16>()
        .map_err(|error| format!("invalid HTTP status from provider: {error}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn http_get_search(
    url: &str,
    _api_key: &str,
    _max_response_bytes: u64,
) -> Result<Vec<u8>, String> {
    if !url.contains("api.search.brave.com") {
        return Err("web_search failed: unsupported host in non-wasm test mode".to_string());
    }
    Ok(
        br#"{
            "web": {
                "results": [
                    {
                        "title": "Uniswap v4 Security Overview",
                        "url": "https://docs.uniswap.org/contracts/v4/security",
                        "description": "Overview of the Uniswap v4 security model and audit posture.",
                        "page_age": "2026-02-14"
                    },
                    {
                        "title": "Trail of Bits Publications",
                        "url": "https://github.com/trailofbits/publications",
                        "description": "Public audit reports and reviews including protocol security material.",
                        "age": "2025-11-15"
                    }
                ]
            }
        }"#
            .to_vec(),
    )
}

fn format_search_results(query: &str, response: &BraveSearchResponse) -> Result<String, String> {
    let results = response
        .web
        .as_ref()
        .map(|web| web.results.as_slice())
        .unwrap_or(&[]);
    if results.is_empty() {
        return Err("web_search returned no results".to_string());
    }

    let mut output = format!(
        "Found {} results for \"{}\":",
        results.len().min(WEB_SEARCH_MAX_RESULTS),
        query
    );

    for (index, result) in results.iter().take(WEB_SEARCH_MAX_RESULTS).enumerate() {
        let title = result
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Untitled result");
        let url = result
            .url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "web_search provider returned a result without a URL".to_string())?;
        let snippet = result
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("No snippet available.");
        let snippet = truncate_chars(snippet, WEB_SEARCH_MAX_SNIPPET_CHARS);

        output.push_str("\n\n");
        output.push_str(&format!("[{}] \"{}\"\n    {}", index + 1, title, url));
        if let Some(date) = result
            .page_age
            .as_deref()
            .or(result.age.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            output.push_str(&format!("\n    Published: {date}"));
        }
        output.push_str(&format!("\n    {snippet}"));
    }

    Ok(truncate_chars(&output, WEB_SEARCH_MAX_OUTPUT_CHARS))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    match value.char_indices().nth(max_chars) {
        None => value.to_string(),
        Some((cutoff, _)) => format!("{}...", &value[..cutoff]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_web_search_args_rejects_empty_query() {
        let error = parse_web_search_args(r#"{"query":"   "}"#)
            .expect_err("empty query should fail");
        assert!(error.contains("query"));
    }

    #[test]
    fn parse_web_search_args_normalizes_domain_filters() {
        let args = parse_web_search_args(
            r#"{"query":"uniswap v4","include_domains":["https://docs.uniswap.org/contracts"],"exclude_domains":["WWW.Example.com"]}"#,
        )
        .expect("args should parse");
        assert_eq!(args.include_domains, vec!["docs.uniswap.org".to_string()]);
        assert_eq!(args.exclude_domains, vec!["example.com".to_string()]);
    }

    #[test]
    fn format_search_results_rejects_empty_results() {
        let response = BraveSearchResponse {
            web: Some(BraveSearchWeb { results: vec![] }),
        };
        let error =
            format_search_results("query", &response).expect_err("empty results should fail");
        assert!(error.contains("no results"));
    }
}
