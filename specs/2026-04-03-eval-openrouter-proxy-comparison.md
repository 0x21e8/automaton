# Spec: Evaluation Harness OpenRouter Direct vs Proxy Comparison

**Status:** LOCKED
**Date:** 2026-04-03
**Author:** Codex (spec-writer) | Mode: interactive
**Complexity:** complex
**Authority:** approval
**Tier:** 3

---

## Problem
The evaluation harness can currently vary only the per-automaton OpenRouter model identifier. It cannot run spawned automatons through the `OpenRouterProxyWorker` path, cannot set OpenRouter reasoning effort, and cannot express an apples-to-apples comparison between direct OpenRouter inference and proxy-backed OpenRouter inference in the same evaluation run.

The child canister already supports these runtime knobs, but the launchpad evaluator does not pass them through spawn/bootstrap. Because the factory hands each child off to self-control, the evaluator cannot reliably patch these settings after spawn without adding new steward-signing machinery.

## Goal
Allow one evaluation experiment to launch automatons that use either direct OpenRouter or OpenRouter via proxy worker, with explicit model and reasoning settings, and produce artifacts that make those configurations visible and comparable.

Success means a single experiment YAML can define at least two otherwise-equivalent automatons that differ only by transport (`openrouter_direct` vs `openrouter_proxy_worker`), and the resulting evaluator summary/report/dashboard clearly records which path each automaton used.

## Non-Goals
- Add post-spawn evaluator control-plane writes to child canisters.
- Add steward proof generation or wallet signing to the evaluator.
- Redesign the spawn/session contract beyond the minimal fields required for inference transport and reasoning.
- Add support for providers other than OpenRouter in this change.
- Build or deploy the proxy worker itself from the launchpad repo.

---

## Autonomous Decisions
- The spec uses per-automaton `transport` and `reasoningLevel` fields rather than experiment-global fields so one run can compare direct vs proxy under otherwise identical settings.
- Proxy worker deployment details remain environment/runtime configuration, not experiment YAML. The experiment should select behavior; the environment should provide infrastructure endpoints and secrets.
- The implementation path should extend bootstrap-time config propagation rather than introducing post-spawn mutation, because spawned children become self-controlled after factory handoff.
- The launchpad repo should record inference transport and reasoning in evaluator artifacts even if provider inference counts remain unavailable for now.
- This spec assumes the sibling `ic-automaton` repo will be changed in the same delivery to extend spawn bootstrap args and apply those values during init.

---

## Requirements

### Must Have
- [ ] Extend the evaluation experiment schema to support per-automaton inference transport selection.
- [ ] Extend the evaluation experiment schema to support per-automaton OpenRouter reasoning level selection.
- [ ] Preserve existing per-automaton `model` selection.
- [ ] Allow one experiment to compare direct OpenRouter calling with proxy-worker-backed OpenRouter by defining multiple automatons with the same strategies/model but different transport values.
- [ ] Carry the new transport and reasoning fields from evaluation experiment parsing through evaluator spawn request construction.
- [ ] Extend the shared spawn contract and factory bootstrap path so the child receives transport and reasoning during install/init, before controller handoff.
- [ ] Extend `ic-automaton` spawn bootstrap args so init can set:
  - inference provider to `OpenRouter` or `OpenRouterProxyWorker`
  - inference model
  - OpenRouter reasoning level
  - proxy worker config when proxy transport is selected
- [ ] Keep direct OpenRouter behavior backward-compatible when transport is omitted in older flows or defaults are used.
- [ ] Add evaluator artifact fields that expose the actual intended inference configuration per automaton:
  - transport
  - model
  - reasoning level
- [ ] Sample and persist child inference config evidence so reports can show configured runtime state, not only requested config.
- [ ] For proxy-backed automatons, sample and persist proxy status evidence from the child.
- [ ] Update docs and example experiment YAML to show how to compare direct vs proxy in one run.

### Should Have
- [ ] Add validation that proxy transport is not accepted unless proxy worker runtime config is available in the evaluator environment/factory child runtime.
- [ ] Surface clear evaluator startup or spawn-time errors when proxy transport is requested but proxy worker base URL or trusted callback principal is missing.
- [ ] Include the configured transport/reasoning in evaluator markdown reports and dashboard cards/tables.
- [ ] Keep the new API shape explicit and typed in both TS and Rust rather than smuggling transport choices through overloaded model strings.
- [ ] Add test coverage in launchpad for experiment parsing, spawn request mapping, and evaluator reporting.
- [ ] Add test coverage in `ic-automaton` for spawn bootstrap applying direct vs proxy transport and reasoning.

### Could Have
- [ ] Add evaluator-side grouping or summary rollups by inference transport in reports.
- [ ] Add provider inference count collection later if the child/runtime exposes a trustworthy metric for direct vs proxy requests.
- [ ] Add a dedicated evaluation example fixture that compares the same model and strategy across `default`, `low`, `medium`, and `high` reasoning levels.

---

## Constraints
- The evaluator cannot depend on post-spawn controller-only mutation because the factory hands the child off to self-control.
- The existing launchpad spawn contract currently carries only `openRouterApiKey`, `model`, and `braveSearchApiKey` in provider config.
- The child already supports runtime `InferenceProvider`, `OpenRouterReasoningLevel`, and `OpenRouterProxyWorkerConfig`; the launchpad/factory path does not yet pass those through bootstrap.
- Proxy worker infrastructure is owned in the sibling `ic-automaton` repo and should not be duplicated inside launchpad.
- Existing evaluation YAML should continue to parse after a compatibility/defaulting pass.
- This change spans two repositories:
  - launchpad
  - sibling `ic-automaton`

---

## Success Criteria
- An experiment YAML can define:
  - one automaton with `transport: openrouter_direct`
  - another automaton with `transport: openrouter_proxy_worker`
  - both with the same model and strategies
  - optional per-automaton `reasoningLevel`
- The evaluator creates spawn sessions successfully for both modes in a local harness run when proxy worker env/config is present.
- The resulting `summary.json` and `report.md` include the selected transport and reasoning level for each automaton.
- The evaluator samples child `/api/inference/config` for every spawned automaton and `/api/inference/proxy/status` for proxy-backed automatons.
- The child runtime for direct mode reports provider `OpenRouter`.
- The child runtime for proxy mode reports provider `OpenRouterProxyWorker`.
- Existing evaluation experiments without explicit transport/reasoning continue to work and default to direct OpenRouter with default reasoning.

---

## Recommended Approach
Use bootstrap-time propagation, not post-spawn mutation.

Reasoning:
- Launchpad evaluator already controls session creation and the factory already injects spawn bootstrap args into child init.
- The child applies provider bootstrap during init today, but only for model, OpenRouter API key, and Brave key.
- Extending the bootstrap payload is smaller, safer, and more testable than teaching the evaluator to impersonate the steward or gain controller authority after handoff.

Configuration boundary:
- Experiment YAML:
  - selects `model`
  - selects `transport`
  - selects `reasoningLevel`
- Evaluator environment / factory child runtime:
  - provides proxy worker base URL
  - provides trusted callback principal

This keeps experiments portable while still allowing infrastructure-specific proxy settings.

---

## Implementation Plan

- [ ] **Task 1:** Extend shared evaluation contracts for transport and reasoning.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/src/evaluation.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/test/evaluation.test.ts`
      - Changes:
        - Add `EvaluationInferenceTransport = "openrouter_direct" | "openrouter_proxy_worker"`.
        - Add `EvaluationOpenRouterReasoningLevel = "default" | "low" | "medium" | "high"`.
        - Extend `EvaluationAutomatonConfig` with `transport` and `reasoningLevel`.
        - Default omitted values to `openrouter_direct` and `default` in parsing.
        - Validate only the supported enum strings.
      - Validation: `npm run test --workspace @ic-automaton/shared`

- [ ] **Task 2:** Extend evaluator runtime and reporting types to carry the new fields.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/types.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/lib/report.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/src/evaluation.ts`
      - Changes:
        - Include `transport` and `reasoningLevel` in dashboard/summary-facing automaton records.
        - Add raw sampled inference config/proxy status evidence fields to evaluation samples.
      - Validation: `npm run test --workspace @ic-automaton/evaluator`

- [ ] **Task 3:** Extend evaluator environment loading for proxy runtime configuration.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/lib/env.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/types.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/.env.example`, `/Users/domwoe/Dev/projects/automaton-launchpad/README.md`
      - Changes:
        - Add env fields for proxy worker base URL and trusted callback principal.
        - Keep them optional unless a run requests proxy transport.
        - Document exact required env keys for proxy-backed evaluation.
      - Validation: `npm run test --workspace @ic-automaton/evaluator`

- [ ] **Task 4:** Extend launchpad shared spawn contract to carry inference transport and reasoning.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/src/spawn.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/apps/indexer/src/integrations/factory-canister-adapter.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/test/contracts.test.ts`
      - Changes:
        - Add typed provider fields for:
          - inference transport / provider
          - OpenRouter reasoning level
        - Update Candid mapping in the indexer’s factory adapter.
      - Validation: `npm run test --workspace @ic-automaton/shared && npm run test --workspace @ic-automaton/indexer`

- [ ] **Task 5:** Extend evaluator spawn request construction to use the new transport and reasoning.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/runtime/run-controller.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/test/run-controller.test.ts`
      - Changes:
        - Populate the extended provider config for each automaton.
        - If any automaton requests proxy transport, require proxy worker env/config before spawn begins.
        - Fail fast with a clear error when proxy transport is requested but infra config is missing.
      - Validation: `npm run test --workspace @ic-automaton/evaluator`

- [ ] **Task 6:** Extend evaluator sampling to capture child inference config and proxy status.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/lib/automaton-client.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/runtime/sampler.ts`, `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/test/run-controller.test.ts`
      - Changes:
        - Read `/api/inference/config`.
        - Read `/api/inference/proxy/status`.
        - Persist both into sample raw evidence.
        - Update report/dashboard builders to expose configuration fields.
      - Validation: `npm run test --workspace @ic-automaton/evaluator`

- [ ] **Task 7:** Extend factory-side provider/bootstrap structs in launchpad Rust code.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/types.rs`, `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/init.rs`, `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/lib.rs`, `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/spawn.rs`
      - Changes:
        - Add transport/provider and reasoning fields to Rust `ProviderConfig`.
        - Encode them into `AutomatonSpawnBootstrapArgs`.
        - Preserve current secret-clearing behavior for API keys.
      - Validation: `cargo test -p factory`

- [ ] **Task 8:** Extend child spawn bootstrap args and init application in `ic-automaton`.
      - Files: `/Users/domwoe/Dev/projects/ic-automaton/src/lib.rs`, `/Users/domwoe/Dev/projects/ic-automaton/src/domain/types.rs`, `/Users/domwoe/Dev/projects/ic-automaton/ic-automaton.did`, `/Users/domwoe/Dev/projects/ic-automaton/tests/pocketic_spawn_bootstrap.rs`
      - Changes:
        - Add transport/provider and reasoning fields to spawn bootstrap provider args.
        - When bootstrap is applied:
          - set inference model if present
          - set OpenRouter API key if present
          - set reasoning level
          - set provider to `OpenRouter` or `OpenRouterProxyWorker`
          - if proxy transport is selected, install proxy worker config from init/bootstrap inputs
        - Keep default behavior backward-compatible for omitted fields.
      - Validation: `cargo test --manifest-path /Users/domwoe/Dev/projects/ic-automaton/Cargo.toml pocketic_spawn_bootstrap --features pocketic_tests -- --nocapture`

- [ ] **Task 9:** Extend factory child runtime configuration to include proxy worker runtime fields.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/types.rs`, `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/init.rs`, `/Users/domwoe/Dev/projects/automaton-launchpad/scripts/render-factory-local-init-args.mjs`, `/Users/domwoe/Dev/projects/automaton-launchpad/scripts/playground-bootstrap.sh`, `/Users/domwoe/Dev/projects/automaton-launchpad/playground.local.env.example`
      - Changes:
        - Add child runtime fields for proxy worker base URL and trusted callback principal.
        - Render them into local factory child runtime config.
        - Ensure local playground bootstrap can provision them from env.
      - Validation: `node /Users/domwoe/Dev/projects/automaton-launchpad/scripts/render-factory-local-init-args.mjs`

- [ ] **Task 10:** Update evaluator docs and add a comparison experiment fixture.
      - Files: `/Users/domwoe/Dev/projects/automaton-launchpad/README.md`, `/Users/domwoe/Dev/projects/automaton-launchpad/evaluations/experiments/smoke.yaml` or a new fixture under `/Users/domwoe/Dev/projects/automaton-launchpad/evaluations/experiments/`
      - Changes:
        - Document new YAML fields.
        - Show a direct-vs-proxy comparison example.
        - Document extra env required for proxy-backed evaluation runs.
      - Validation: `npm run test --workspace @ic-automaton/shared`

---

## Cross-Repo API Shape

Launchpad shared evaluation YAML shape:

```yaml
automatons:
  - id: alpha-direct
    label: Alpha Direct
    model: google/gemma-4-31b-it
    transport: openrouter_direct
    reasoningLevel: medium
    strategies:
      - base-aave-usdc-reserve-01
  - id: alpha-proxy
    label: Alpha Proxy
    model: google/gemma-4-31b-it
    transport: openrouter_proxy_worker
    reasoningLevel: medium
    strategies:
      - base-aave-usdc-reserve-01
```

Launchpad shared spawn/provider shape after extension:

```ts
interface ProviderConfig {
  openRouterApiKey: string | null;
  model: string | null;
  braveSearchApiKey: string | null;
  inferenceTransport: "openrouter_direct" | "openrouter_proxy_worker";
  openRouterReasoningLevel: "default" | "low" | "medium" | "high";
}
```

Factory child runtime shape after extension:

```ts
interface AutomatonChildRuntimeConfig {
  // existing fields...
  inferenceProxyWorkerBaseUrl?: string | null;
  inferenceProxyTrustedCallbackPrincipal?: string | null;
}
```

Child bootstrap application behavior:

- `openrouter_direct`
  - set provider `OpenRouter`
  - set model
  - set reasoning
  - do not require proxy config
- `openrouter_proxy_worker`
  - set provider `OpenRouterProxyWorker`
  - set model
  - set reasoning
  - require proxy worker base URL
  - require trusted callback principal

---

## Risks

- Backward-compatibility risk if shared spawn/provider structs change without aligned updates in:
  - TS shared package
  - indexer Candid adapter
  - factory Rust Candid types
  - child spawn bootstrap decoding
- Local harness fragility if proxy transport is requested but the worker is unreachable.
- Misleading comparison results if evaluator reports only requested config and not observed child config.
- Secret-handling regressions if new provider/runtime fields are added near existing API-key paths without preserving the “clear secrets after install” behavior.

Mitigations:
- Make the evaluator sample `/api/inference/config` as evidence.
- Keep proxy worker URL/principal in non-secret child runtime config and keep API keys on existing secret path.
- Add fail-fast validation before spawn for proxy-backed experiments.

---

## Verification

### Smoke Tests
- `npm run test --workspace @ic-automaton/shared` -- proves experiment schema and contract parsing accept transport/reasoning defaults and explicit values.
- `npm run test --workspace @ic-automaton/indexer` -- proves the indexer factory adapter maps the extended provider config correctly.
- `npm run test --workspace @ic-automaton/evaluator` -- proves evaluator spawn construction, sampling, and reporting handle direct vs proxy config.
- `cargo test -p factory` -- proves Rust factory/provider/bootstrap contract changes remain coherent.
- `cargo test --manifest-path /Users/domwoe/Dev/projects/ic-automaton/Cargo.toml pocketic_spawn_bootstrap --features pocketic_tests -- --nocapture` -- proves child bootstrap applies provider, model, and reasoning correctly.

### Expected State
- File `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/src/evaluation.ts` defines transport and reasoning enums/types.
- File `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/src/spawn.ts` contains explicit provider fields for transport and reasoning.
- File `/Users/domwoe/Dev/projects/automaton-launchpad/evaluations/experiments/` contains at least one direct-vs-proxy example fixture.
- File `/Users/domwoe/Dev/projects/automaton-launchpad/tmp/evaluations/<runId>/summary.json` contains `transport` and `reasoningLevel` for each automaton result.
- File `/Users/domwoe/Dev/projects/automaton-launchpad/tmp/evaluations/<runId>/samples/<automatonId>.jsonl` contains sampled `raw.inferenceConfig`.
- For proxy-backed automatons, sampled evidence contains `raw.inferenceProxyStatus`.

### Regression
- `npm test`
- `cargo test -p factory`

### Integration Test
- Start a local evaluation run with an experiment that defines two otherwise-identical automatons, one `openrouter_direct` and one `openrouter_proxy_worker`, with the same model and strategies.
- Verify:
  - both sessions spawn successfully
  - direct automaton sampled `/api/inference/config` reports provider `OpenRouter`
  - proxy automaton sampled `/api/inference/config` reports provider `OpenRouterProxyWorker`
  - proxy automaton sampled `/api/inference/proxy/status` reports configured worker metadata
  - `report.md` and `summary.json` clearly differentiate the two automatons by transport and reasoning

---

## Codebase Snapshot

Relevant launchpad files verified to exist:
- `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/src/evaluation.ts`
- `/Users/domwoe/Dev/projects/automaton-launchpad/packages/shared/src/spawn.ts`
- `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/runtime/run-controller.ts`
- `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/runtime/sampler.ts`
- `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/lib/automaton-client.ts`
- `/Users/domwoe/Dev/projects/automaton-launchpad/apps/evaluator/src/lib/env.ts`
- `/Users/domwoe/Dev/projects/automaton-launchpad/apps/indexer/src/integrations/factory-canister-adapter.ts`
- `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/types.rs`
- `/Users/domwoe/Dev/projects/automaton-launchpad/backend/factory/src/init.rs`
- `/Users/domwoe/Dev/projects/automaton-launchpad/scripts/render-factory-local-init-args.mjs`
- `/Users/domwoe/Dev/projects/automaton-launchpad/README.md`

Relevant sibling `ic-automaton` files verified to exist:
- `/Users/domwoe/Dev/projects/ic-automaton/src/lib.rs`
- `/Users/domwoe/Dev/projects/ic-automaton/src/domain/types.rs`
- `/Users/domwoe/Dev/projects/ic-automaton/src/features/inference.rs`
- `/Users/domwoe/Dev/projects/ic-automaton/src/storage/stable.rs`
- `/Users/domwoe/Dev/projects/ic-automaton/ic-automaton.did`
- `/Users/domwoe/Dev/projects/ic-automaton/tests/pocketic_spawn_bootstrap.rs`
- `/Users/domwoe/Dev/projects/ic-automaton/workers/openrouter-proxy/src/worker.js`
- `/Users/domwoe/Dev/projects/ic-automaton/docs/mainnet-openrouter-proxy-runbook.md`
