# OpenRouter Proxy Worker (Async Callback)

This crate contains the core proxy logic for the async inference path used by
`InferenceProvider::OpenRouterProxyWorker`.

## Responsibilities

- Accept `POST /v1/inference/jobs` submit payloads.
- Resolve tenant configuration by `canister_id` (multi-tenant support).
- Require bearer auth per tenant.
- Require `x-openrouter-api-key` header for pass-through inference calls.
- Persist pending jobs without storing API keys.
- Build callback payloads for canister method `submit_inference_result`.
- Use a single persistent callback principal identity (`callback_identity`).

## Security constraints

- API keys must not be written to job state.
- API keys must not be logged.
- Callback identity principal must be stable and explicitly configured.
- Callback payload size is bounded by `max_callback_payload_bytes`.

## Local checks

```bash
cargo test --manifest-path workers/openrouter-proxy/Cargo.toml -- --nocapture
cargo check --manifest-path workers/openrouter-proxy/Cargo.toml --target wasm32-unknown-unknown
```
