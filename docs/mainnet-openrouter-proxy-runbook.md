# Mainnet OpenRouter Proxy Runbook

This runbook deploys the queue-backed async OpenRouter proxy worker, upgrades the
mainnet canister, sets inference to `OpenRouterProxyWorker`, and verifies health.

## 1) Deploy the worker (queue-backed async)

```bash
cd workers/openrouter-proxy
npx wrangler whoami

# one-time queue setup
npx wrangler queues create openrouter-proxy-jobs

# callback identity secret (store this securely; this value defines trusted callback principal)
CALLBACK_IDENTITY_SEED_HEX="$(openssl rand -hex 32)"
printf '%s' "$CALLBACK_IDENTITY_SEED_HEX" | npx wrangler secret put CALLBACK_IDENTITY_SEED_HEX

# print callback principal to configure in canister
node --input-type=module -e "import { Ed25519KeyIdentity } from '@dfinity/identity'; const hex = process.env.CALLBACK_IDENTITY_SEED_HEX; const bytes = Uint8Array.from(hex.match(/.{1,2}/g).map(b => parseInt(b, 16))); const id = Ed25519KeyIdentity.generate(bytes); console.log(id.getPrincipal().toText());"

# OpenRouter secret
set -a
source ../../.env
set +a
printf '%s' "$OPENROUTER_API_KEY" | npx wrangler secret put OPENROUTER_API_KEY

# deploy
npx wrangler deploy
```

Expected worker URL:

```text
https://openrouter-proxy.dom-woe.workers.dev
```

## 2) Build the canister

```bash
cd /Users/domwoe/Dev/projects/ic-automaton
icp build backend -e ic
```

## 3) Upgrade deploy to mainnet

```bash
icp canister install backend -e ic --mode upgrade
```

Current mainnet backend canister ID:

```text
oc3hk-viaaa-aaaak-qxcuq-cai
```

## 4) Configure inference to use proxy + model + API key

```bash
icp canister call backend set_inference_provider '(variant { OpenRouterProxyWorker })' -e ic
icp canister call backend set_inference_model '("google/gemini-3-flash-preview")' -e ic
icp canister call backend set_openrouter_base_url '("https://openrouter.ai/api/v1")' -e ic

set -a
source .env
set +a
escaped_api_key="$(printf '%s' "$OPENROUTER_API_KEY" | sed 's/\\/\\\\/g; s/"/\\"/g')"
icp canister call backend set_openrouter_api_key "(opt \"$escaped_api_key\")" -e ic
icp canister call backend set_inference_proxy_config '(record {
  worker_base_url = "https://openrouter-proxy.dom-woe.workers.dev";
  trusted_callback_principal = opt principal "nv5tq-gmjom-vchmv-jeiqk-pbhei-ss7wo-6tbt5-ojaek-4aoz4-shrgy-kqe"
})' -e ic
```

## 5) Verify everything works

```bash
curl -fsS https://openrouter-proxy.dom-woe.workers.dev/health
curl -fsS "https://oc3hk-viaaa-aaaak-qxcuq-cai.icp0.io/api/inference/config?ts=$(date +%s)"
curl -fsS "https://oc3hk-viaaa-aaaak-qxcuq-cai.icp0.io/api/inference/proxy/status?ts=$(date +%s)"
curl -fsS https://oc3hk-viaaa-aaaak-qxcuq-cai.icp0.io/api/snapshot \
  | jq '{runtime_state:.runtime.state, runtime_last_error:.runtime.last_error, scheduler_enabled:.scheduler.enabled, provider:.runtime.inference_provider, model:.runtime.inference_model}'
icp canister status backend -e ic
```
