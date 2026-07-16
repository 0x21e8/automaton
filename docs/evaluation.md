# Evaluation harness

The evaluation harness runs experiment fleets of automatons against a fresh local playground and records evidence for later analysis. It uses two dedicated apps on top of the playground stack:

- `apps/evaluator` boots a fresh playground, validates the experiment, spawns the fleet, samples evidence every `15s`, and writes artifacts to `tmp/evaluations/<runId>/`
- `apps/evaluator-web` shows the live run dashboard and exposes the stop control

## Environment

Before starting the harness, create a repo-root `.env` from `.env.example`:

```bash
cp .env.example .env
$EDITOR .env
```

The evaluator expects the following keys:

```dotenv
EVAL_STEWARD_ADDRESS=0x...
EVAL_OPENROUTER_API_KEY=...
LOCAL_EVM_FORK_URL=https://...

# Optional
EVAL_BRAVE_SEARCH_API_KEY=
EVAL_INFERENCE_PROXY_WORKER_BASE_URL=
EVAL_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL=
```

The child canister artifact is built from the in-repo automaton component (`components/ic-automaton`) by default. Set `AUTOMATON_COMPONENT_ROOT` in `.env` only if you want to build from a different checkout.

## Experiments

Use `transport` and `reasoningLevel` per automaton in the experiment YAML. Keep proxy worker infrastructure in env/runtime config, not in the experiment:

```yaml
automatons:
  - id: alpha-direct
    label: Alpha Direct
    model: openrouter/openai/gpt-5
    transport: openrouter_direct
    reasoningLevel: default
    strategies:
      - base-aave-usdc-reserve-01
  - id: alpha-proxy
    label: Alpha Proxy
    model: openrouter/openai/gpt-5
    transport: openrouter_proxy_worker
    reasoningLevel: medium
    strategies:
      - base-aave-usdc-reserve-01
```

When any automaton uses `transport: openrouter_proxy_worker`, the evaluator requires both proxy env keys above. In the local playground flow, those values are forwarded into the factory child runtime automatically. Only set separate factory vars if you need an override:

```dotenv
FACTORY_CHILD_INFERENCE_PROXY_WORKER_BASE_URL=
FACTORY_CHILD_INFERENCE_PROXY_TRUSTED_CALLBACK_PRINCIPAL=
```

## Running

Use `eval` during implementation. It runs the evaluator backend in watch mode, bootstraps the playground stack (including the indexer), serves the operator dashboard from Vite, and serves the lab web app:

```bash
npm run eval -- --experiment evaluations/experiments/smoke.yaml
```

`eval:dev` remains available as an explicit alias for the same workflow.

Use `eval:run` for a cleaner one-command local run without watch mode. It builds the required workspaces first, then starts the evaluator backend plus preview-served dashboard and lab:

```bash
npm run eval:run -- --experiment evaluations/experiments/smoke.yaml
```

Default local endpoints:

- evaluator API: `http://127.0.0.1:3003`
- evaluator dashboard: `http://127.0.0.1:4173`
- the lab: `http://127.0.0.1:5173`
- playground indexer: `http://127.0.0.1:3001`
- artifacts: `tmp/evaluations/<runId>/`

Manual stop is handled from the dashboard. Press `Stop Run` in the operator console to finalize the current run, write `manifest.json`, `events.ndjson`, `samples/*.jsonl`, `summary.json`, and `report.md`, and tear the playground down cleanly.

## Overrides

Both scripts accept the following optional overrides:

- `EVALUATOR_HOST` / `EVALUATOR_PORT` for the backend bind address
- `EVALUATOR_WEB_HOST` / `EVALUATOR_WEB_PORT` for the dashboard server
- `LAUNCHPAD_WEB_HOST` / `LAUNCHPAD_WEB_PORT` for the lab web server (legacy variable names retained for compatibility)
- `LAUNCHPAD_INDEXER_BASE_URL` to point the lab at a non-default indexer origin (legacy variable name retained for compatibility)
- `EVALUATOR_ARTIFACTS_ROOT` to move run outputs away from `tmp/evaluations`
- `VITE_EVALUATOR_BASE_URL` if you want the dashboard to target a different evaluator origin

Both web servers use strict ports. If `4173` or `5173` is already occupied, the eval wrapper exits instead of silently switching to a different port.
