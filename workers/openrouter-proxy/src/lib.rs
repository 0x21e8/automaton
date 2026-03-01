use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const SUBMIT_ROUTE: &str = "/v1/inference/jobs";
pub const DEFAULT_CALLBACK_METHOD: &str = "submit_inference_result";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub callback_identity: PersistentCallbackIdentity,
    pub max_submit_request_bytes: usize,
    pub max_callback_payload_bytes: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            callback_identity: PersistentCallbackIdentity {
                principal_text: "2vxsx-fae".to_string(),
            },
            max_submit_request_bytes: 16 * 1024,
            max_callback_payload_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentCallbackIdentity {
    pub principal_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantConfig {
    pub canister_id: String,
    pub submit_bearer_token: String,
    pub callback_method: String,
}

impl TenantConfig {
    pub fn validate(&self) -> Result<(), ProxyError> {
        if self.canister_id.trim().is_empty() {
            return Err(ProxyError::InvalidRequest(
                "canister_id cannot be empty".to_string(),
            ));
        }
        if self.submit_bearer_token.trim().is_empty() {
            return Err(ProxyError::InvalidRequest(
                "submit_bearer_token cannot be empty".to_string(),
            ));
        }
        Ok(())
    }

    fn callback_method_or_default(&self) -> String {
        let trimmed = self.callback_method.trim();
        if trimmed.is_empty() {
            DEFAULT_CALLBACK_METHOD.to_string()
        } else {
            trimmed.to_string()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitJobRequest {
    pub canister_id: String,
    pub turn_id: String,
    pub job_id: String,
    pub model: String,
    pub inference_request: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitJobAck {
    pub job_id: String,
    pub accepted_at_ns: u64,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingJob {
    pub canister_id: String,
    pub turn_id: String,
    pub job_id: String,
    pub model: String,
    pub submitted_at_ns: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_call_id: Option<String>,
    pub tool: String,
    pub args_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceProxyResultPayload {
    pub explanation: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitInferenceResultArgs {
    pub job_id: String,
    pub turn_id: String,
    pub completed_at_ns: u64,
    pub result: Option<InferenceProxyResultPayload>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallbackEnvelope {
    pub canister_id: String,
    pub method: String,
    pub caller_principal: String,
    pub args: SubmitInferenceResultArgs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpHeaders {
    inner: HashMap<String, String>,
}

impl HttpHeaders {
    pub fn new(headers: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut inner = HashMap::new();
        for (name, value) in headers {
            inner.insert(name.to_ascii_lowercase(), value);
        }
        Self { inner }
    }

    pub fn bearer_token(&self) -> Option<&str> {
        let auth = self.inner.get("authorization")?;
        let token = auth.strip_prefix("Bearer ")?;
        let trimmed = token.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    pub fn openrouter_api_key(&self) -> Option<&str> {
        let key = self.inner.get("x-openrouter-api-key")?;
        let trimmed = key.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }
}

pub trait CallbackClient {
    fn submit_result(&self, envelope: &CallbackEnvelope) -> Result<(), String>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProxyError {
    InvalidRequest(String),
    MissingAuth,
    Unauthorized,
    UnknownTenant,
    MissingOpenRouterApiKey,
    RequestTooLarge,
    UnknownJob,
    CallbackPayloadTooLarge,
    CallbackFailed(String),
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyError::InvalidRequest(message) => write!(f, "invalid request: {message}"),
            ProxyError::MissingAuth => write!(f, "missing bearer authorization"),
            ProxyError::Unauthorized => write!(f, "unauthorized"),
            ProxyError::UnknownTenant => write!(f, "unknown tenant"),
            ProxyError::MissingOpenRouterApiKey => write!(f, "missing x-openrouter-api-key"),
            ProxyError::RequestTooLarge => write!(f, "request body exceeds max size"),
            ProxyError::UnknownJob => write!(f, "unknown job_id"),
            ProxyError::CallbackPayloadTooLarge => {
                write!(f, "callback payload exceeds max size")
            }
            ProxyError::CallbackFailed(message) => write!(f, "callback failed: {message}"),
        }
    }
}

impl std::error::Error for ProxyError {}

#[derive(Clone, Debug)]
pub struct ProxyWorker {
    config: WorkerConfig,
    tenants: HashMap<String, TenantConfig>,
    pending_jobs: HashMap<String, PendingJob>,
}

impl ProxyWorker {
    pub fn new(config: WorkerConfig, tenants: Vec<TenantConfig>) -> Result<Self, ProxyError> {
        if config.callback_identity.principal_text.trim().is_empty() {
            return Err(ProxyError::InvalidRequest(
                "callback identity principal cannot be empty".to_string(),
            ));
        }
        if config.max_submit_request_bytes == 0 {
            return Err(ProxyError::InvalidRequest(
                "max_submit_request_bytes must be > 0".to_string(),
            ));
        }
        if config.max_callback_payload_bytes == 0 {
            return Err(ProxyError::InvalidRequest(
                "max_callback_payload_bytes must be > 0".to_string(),
            ));
        }

        let mut tenant_map = HashMap::new();
        for tenant in tenants {
            tenant.validate()?;
            if tenant_map
                .insert(tenant.canister_id.clone(), tenant)
                .is_some()
            {
                return Err(ProxyError::InvalidRequest(
                    "duplicate tenant canister_id".to_string(),
                ));
            }
        }

        Ok(Self {
            config,
            tenants: tenant_map,
            pending_jobs: HashMap::new(),
        })
    }

    pub fn accept_submit(
        &mut self,
        headers: &HttpHeaders,
        raw_body: &[u8],
        now_ns: u64,
    ) -> Result<SubmitJobAck, ProxyError> {
        if raw_body.len() > self.config.max_submit_request_bytes {
            return Err(ProxyError::RequestTooLarge);
        }

        let bearer = headers.bearer_token().ok_or(ProxyError::MissingAuth)?;
        let _openrouter_api_key = headers
            .openrouter_api_key()
            .ok_or(ProxyError::MissingOpenRouterApiKey)?;
        let request: SubmitJobRequest = serde_json::from_slice(raw_body)
            .map_err(|error| ProxyError::InvalidRequest(error.to_string()))?;

        if request.canister_id.trim().is_empty()
            || request.turn_id.trim().is_empty()
            || request.job_id.trim().is_empty()
            || request.model.trim().is_empty()
        {
            return Err(ProxyError::InvalidRequest(
                "canister_id, turn_id, job_id, and model are required".to_string(),
            ));
        }

        let tenant = self
            .tenants
            .get(&request.canister_id)
            .ok_or(ProxyError::UnknownTenant)?;
        if bearer != tenant.submit_bearer_token {
            return Err(ProxyError::Unauthorized);
        }

        // Intentionally persist only durable routing metadata and never the
        // OpenRouter API key to avoid secrets at rest.
        self.pending_jobs.insert(
            request.job_id.clone(),
            PendingJob {
                canister_id: request.canister_id,
                turn_id: request.turn_id,
                job_id: request.job_id.clone(),
                model: request.model,
                submitted_at_ns: now_ns,
            },
        );

        Ok(SubmitJobAck {
            job_id: request.job_id,
            accepted_at_ns: now_ns,
            status: "accepted".to_string(),
        })
    }

    pub fn complete_job<C: CallbackClient>(
        &mut self,
        job_id: &str,
        completed_at_ns: u64,
        result: Option<InferenceProxyResultPayload>,
        error: Option<String>,
        callback_client: &C,
    ) -> Result<(), ProxyError> {
        let pending = self
            .pending_jobs
            .remove(job_id)
            .ok_or(ProxyError::UnknownJob)?;
        let tenant = self
            .tenants
            .get(&pending.canister_id)
            .ok_or(ProxyError::UnknownTenant)?;

        let envelope = CallbackEnvelope {
            canister_id: pending.canister_id,
            method: tenant.callback_method_or_default(),
            caller_principal: self.config.callback_identity.principal_text.clone(),
            args: SubmitInferenceResultArgs {
                job_id: pending.job_id,
                turn_id: pending.turn_id,
                completed_at_ns,
                result,
                error,
            },
        };

        let serialized = serde_json::to_vec(&envelope)
            .map_err(|err| ProxyError::InvalidRequest(err.to_string()))?;
        if serialized.len() > self.config.max_callback_payload_bytes {
            return Err(ProxyError::CallbackPayloadTooLarge);
        }

        callback_client
            .submit_result(&envelope)
            .map_err(ProxyError::CallbackFailed)
    }

    pub fn pending_job(&self, job_id: &str) -> Option<&PendingJob> {
        self.pending_jobs.get(job_id)
    }

    pub fn pending_count_for(&self, canister_id: &str) -> usize {
        self.pending_jobs
            .values()
            .filter(|job| job.canister_id == canister_id)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn default_worker() -> ProxyWorker {
        ProxyWorker::new(
            WorkerConfig {
                callback_identity: PersistentCallbackIdentity {
                    principal_text: "w36hm-eqaaa-aaaal-qr76a-cai".to_string(),
                },
                max_submit_request_bytes: 4096,
                max_callback_payload_bytes: 8192,
            },
            vec![
                TenantConfig {
                    canister_id: "mdwwn-niaaa-aaaab-qabta-cai".to_string(),
                    submit_bearer_token: "tenant-a-token".to_string(),
                    callback_method: DEFAULT_CALLBACK_METHOD.to_string(),
                },
                TenantConfig {
                    canister_id: "ulvla-h7777-77774-qaacq-cai".to_string(),
                    submit_bearer_token: "tenant-b-token".to_string(),
                    callback_method: "".to_string(),
                },
            ],
        )
        .expect("worker should initialize")
    }

    fn submit_headers(token: &str) -> HttpHeaders {
        HttpHeaders::new(vec![
            ("authorization".to_string(), format!("Bearer {token}")),
            (
                "x-openrouter-api-key".to_string(),
                "sk-or-test-value".to_string(),
            ),
        ])
    }

    fn submit_body(canister_id: &str, job_id: &str, turn_id: &str) -> Vec<u8> {
        serde_json::to_vec(&SubmitJobRequest {
            canister_id: canister_id.to_string(),
            turn_id: turn_id.to_string(),
            job_id: job_id.to_string(),
            model: "openai/gpt-4o-mini".to_string(),
            inference_request: serde_json::json!({
                "messages": [{"role":"user","content":"ping"}]
            }),
        })
        .expect("submit request should serialize")
    }

    struct RecordingCallbackClient {
        envelopes: RefCell<Vec<CallbackEnvelope>>,
    }

    impl RecordingCallbackClient {
        fn new() -> Self {
            Self {
                envelopes: RefCell::new(Vec::new()),
            }
        }
    }

    impl CallbackClient for RecordingCallbackClient {
        fn submit_result(&self, envelope: &CallbackEnvelope) -> Result<(), String> {
            self.envelopes.borrow_mut().push(envelope.clone());
            Ok(())
        }
    }

    #[test]
    fn accept_submit_requires_known_tenant_and_valid_auth() {
        let mut worker = default_worker();
        let headers = submit_headers("wrong-token");
        let body = submit_body("mdwwn-niaaa-aaaab-qabta-cai", "job-1", "turn-1");

        let error = worker
            .accept_submit(&headers, &body, 42)
            .expect_err("unknown token should be rejected");
        assert_eq!(error, ProxyError::Unauthorized);

        let unknown_tenant = submit_body("aaaaa-aa", "job-2", "turn-2");
        let error = worker
            .accept_submit(&submit_headers("tenant-a-token"), &unknown_tenant, 42)
            .expect_err("unknown tenant should be rejected");
        assert_eq!(error, ProxyError::UnknownTenant);
    }

    #[test]
    fn accept_submit_stores_pending_without_api_key_persistence() {
        let mut worker = default_worker();
        let headers = submit_headers("tenant-a-token");
        let body = submit_body("mdwwn-niaaa-aaaab-qabta-cai", "job-1", "turn-1");

        let ack = worker
            .accept_submit(&headers, &body, 99)
            .expect("submit should be accepted");
        assert_eq!(ack.status, "accepted");
        assert_eq!(ack.job_id, "job-1");

        let pending = worker
            .pending_job("job-1")
            .expect("pending job should be stored");
        assert_eq!(pending.canister_id, "mdwwn-niaaa-aaaab-qabta-cai");
        assert_eq!(pending.turn_id, "turn-1");
        let serialized = serde_json::to_string(pending).expect("pending should serialize");
        assert!(
            !serialized.contains("sk-or-test-value"),
            "pending job state must not persist API keys"
        );
    }

    #[test]
    fn complete_job_uses_persistent_callback_identity() {
        let mut worker = default_worker();
        let callback_client = RecordingCallbackClient::new();

        worker
            .accept_submit(
                &submit_headers("tenant-a-token"),
                &submit_body("mdwwn-niaaa-aaaab-qabta-cai", "job-1", "turn-1"),
                123,
            )
            .expect("submit should be accepted");
        worker
            .complete_job(
                "job-1",
                124,
                Some(InferenceProxyResultPayload {
                    explanation: Some("ready".to_string()),
                    tool_calls: Vec::new(),
                }),
                None,
                &callback_client,
            )
            .expect("callback should be dispatched");

        let envelopes = callback_client.envelopes.borrow();
        assert_eq!(envelopes.len(), 1);
        let callback = &envelopes[0];
        assert_eq!(callback.canister_id, "mdwwn-niaaa-aaaab-qabta-cai");
        assert_eq!(callback.method, DEFAULT_CALLBACK_METHOD);
        assert_eq!(
            callback.caller_principal, "w36hm-eqaaa-aaaal-qr76a-cai",
            "callback principal should stay stable across jobs"
        );
        assert_eq!(callback.args.job_id, "job-1");
        assert_eq!(worker.pending_count_for("mdwwn-niaaa-aaaab-qabta-cai"), 0);
    }

    #[test]
    fn submit_isolated_per_tenant() {
        let mut worker = default_worker();

        worker
            .accept_submit(
                &submit_headers("tenant-a-token"),
                &submit_body("mdwwn-niaaa-aaaab-qabta-cai", "job-a", "turn-a"),
                1,
            )
            .expect("tenant A submit should succeed");
        worker
            .accept_submit(
                &submit_headers("tenant-b-token"),
                &submit_body("ulvla-h7777-77774-qaacq-cai", "job-b", "turn-b"),
                2,
            )
            .expect("tenant B submit should succeed");

        assert_eq!(worker.pending_count_for("mdwwn-niaaa-aaaab-qabta-cai"), 1);
        assert_eq!(worker.pending_count_for("ulvla-h7777-77774-qaacq-cai"), 1);
    }
}
