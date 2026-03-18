# Spec: Policy-Bounded Autonomy Runtime Implementation Tasklist

**Status:** LOCKED
**Date:** 2026-03-18
**Author:** Codex (spec-writer) | Mode: autonomous
**Complexity:** complex
**Authority:** approval
**Tier:** 3

---

## Problem

The locked spec in `docs/specs/2026-03-07-policy-bounded-autonomous-decision-runtime.md` is directionally correct, but it is not yet executable against the current codebase. The repo already has autonomy suppression, survival-policy gates, strategy validation, strategy outcome learning, and Layer-10 runtime context, but the new autonomy runtime slice has to be mapped onto those existing seams explicitly.

Without a code-referenced tasklist, implementation risks drifting into a second policy system, duplicating existing strategy controls, or wiring the new behavior into the wrong layer of the runtime.

## Goal

Produce an implementation-ready tasklist for the policy-bounded autonomy slice that maps each missing capability onto the current Rust/SQLite/runtime structure.

Measurable success:

1. A dev agent can implement the slice top-to-bottom without rediscovering where each change belongs.
2. Every major requirement from the March 7 spec is tied to concrete files and symbols already present in this repo.
3. Verification steps are concrete enough to prove the feature end-to-end with unit tests, `icp build`, and PocketIC tests.

## Non-Goals

1. Redesigning the generic strategy engine.
2. Replacing survival-policy, kill-switch, or existing strategy-budget validation.
3. Adding protocol-specific execution tools.
4. Solving general portfolio optimization or revenue strategy selection in this slice.
5. Editing the old locked spec in place.

---

## Autonomous Decisions

1. Keep the March 7 spec as the product-intent document and create this follow-up executable tasklist as the implementation contract.
2. Reuse existing seams instead of introducing parallel runtime paths:
   - policy/config types next to `AutonomySuppressionConfig` in `src/domain/types.rs`
   - durable CRUD in `src/storage/sqlite.rs` and `src/storage/stable.rs`
   - prompt/context wiring in `src/prompt.rs` and `src/agent.rs`
   - hard execution gates in `src/tools.rs`
   - periodic reconciliation in `src/scheduler.rs::run_reconcile_job`
3. Use dedicated durable stores for policy, exposures, quarantines, reconciliation status, and recent decisions; do not overload `RuntimeSnapshot`, `MemoryFact`, `MemoryRollup`, `TurnRecord`, or `StrategyOutcomeStats`.
4. Keep the first implementation slice focused on the existing `simulate_strategy_action` / `execute_strategy_action` surface. Do not broaden to every EVM write path yet.
5. Treat the existing strategy learner auto-deactivation as adjacent safety behavior, not as a substitute for `StrategyQuarantine`.

---

## Requirements

### Must Have

- [ ] Add first-class autonomy runtime types: `AutonomyPolicy`, `ReservePolicy`, `RiskLimits`, `ExecutionAuthority`, `EscalationRules`, `ActiveExposure`, `StrategyQuarantine`, `DecisionRecord`, `DecisionTrigger`, `DecisionOutcome`, `EscalationClass`, `AutonomyDecisionEnvelope`, `DecisionEnvelopeOutcome`, and a small `ExposureReconciliationStatus` view.
- [ ] Persist one active autonomy policy, active exposures, active strategy quarantines, recent decision records, and last exposure-reconciliation status in dedicated SQLite-backed stores.
- [ ] Install a conservative default `AutonomyPolicy` after `init` / `post_upgrade` when no policy exists, and log that installation explicitly.
- [ ] Expose controller/query APIs:
  - `update_autonomy_policy`
  - `get_autonomy_policy`
  - `get_recent_decisions`
  - `get_active_exposures`
  - `get_strategy_quarantines`
  - `get_exposure_reconciliation_status`
- [ ] Extend Layer 6 and Layer 10 so autonomous economic turns reason from policy and durable runtime state instead of open-ended operator prompts.
- [ ] Require autonomous economic turns to terminate in a machine-readable `AutonomyDecisionEnvelope` and persist a bounded `DecisionRecord`.
- [ ] Retry invalid decision-envelope output in-turn with bounded retries, then persist `NoOp { reason: "invalid_decision_shape" }`.
- [ ] Enforce autonomy hard gates before live strategy execution:
  - reserve floors
  - protocol concentration
  - quarantine
  - autonomous-execution enabled
  - per-action value authority
  - simulation-first
- [ ] Require successful capital-touching `enter_*` / `exit_*` strategy executions to durably update or close `ActiveExposure`.
- [ ] Record repeated strategy execution failures into `StrategyQuarantine` and block re-execution while quarantined.
- [ ] Add periodic exposure reconciliation that can rebuild or repair exposure state after drift, crash, upgrade, or local bookkeeping failure.
- [ ] Regenerate `ic-automaton.did` after public interfaces change.

### Should Have

- [ ] Persist a compact machine-readable blocked reason on rejected executions.
- [ ] Include `policy_version` and `trigger` in every `DecisionRecord`.
- [ ] Keep recent decisions bounded with FIFO eviction at 200 items.
- [ ] Record a machine-readable reconciliation drift reason when exposure state is repaired or recreated.
- [ ] Add unit tests that pin default-policy values and migration/default-install behavior.

### Could Have

- [ ] Add a derived policy-summary query for UI/debugging if it stays small.
- [ ] Add a combined operator-facing query returning exposures, quarantines, and reconciliation status in one view if it simplifies testing.

---

## Constraints

- Keep KISS. Reuse the current scheduler, agent turn loop, strategy tool path, and SQLite storage shape.
- Do not implement exposures or decisions as `MemoryFact`, `TurnRecord`, or `StrategyOutcomeStats`.
- Keep hard safety in runtime code, not prompt prose.
- Keep decision-envelope parsing deterministic and internal to the runtime.
- Use `current_time_ns()` / host-safe helpers in any new testable timing logic.
- Follow repo workflow: run `icp build` before PocketIC tests so `target/wasm32-unknown-unknown/release/backend.wasm` is fresh.
- Regenerate Candid from compiled Wasm; do not hand-edit `ic-automaton.did`.
- Because this slice touches live capital execution policy, shipping authority remains `approval`.

---

## Implementation Plan

- [x] **Task 1: Add autonomy runtime types and defaults**
      - Files: `src/domain/types.rs`
      - Code references:
        - add new policy types adjacent to `AutonomySuppressionConfig`
        - add decision/exposure/quarantine types near `TurnRecord` and `StrategyOutcomeStats`
        - keep existing `AutonomySuppressionConfig` unchanged; this slice adds a second, broader policy type rather than mutating suppression semantics
      - Validation: `cargo test autonomy_policy_defaults_are_conservative --lib -- --nocapture`
      - Notes: include helper constructors/defaults for the conservative policy from the March 7 spec; keep serde migration additive and explicit.

- [x] **Task 2: Add SQLite schema and stable-layer CRUD**
      - Files: `src/storage/sqlite.rs`, `src/storage/stable.rs`
      - Code references:
        - add a new migration after `MIGRATION_004_REFLECTION_MEMORY_SCHEMA` in `src/storage/sqlite.rs`
        - mirror the existing patterns used by `strategy_outcome_stats`, `strategy_budgets`, and `reflection_memory`
        - add stable wrappers near `autonomy_suppression_config`, `append_turn_record`, and `record_strategy_outcome`
      - Validation: `cargo test autonomy_policy_and_decision_records_roundtrip --lib -- --nocapture`
      - Dependencies: Task 1
      - Notes: create dedicated tables/stores for policy, exposures, quarantines, reconciliation status, and recent decisions; enforce FIFO cap for decisions in storage helpers, not only at call sites.

- [x] **Task 3: Expose governed APIs and regenerate Candid**
      - Files: `src/lib.rs`, `ic-automaton.did`
      - Code references:
        - add new queries near `get_autonomy_suppression_config`, `list_turns`, and `get_strategy_outcome_stats`
        - add controller-only update near other controller config methods such as `set_autonomy_suppression_config`
      - Validation: `icp build && ./scripts/generate-candid.sh ic-automaton.did`
      - Dependencies: Task 1, Task 2
      - Notes: `update_autonomy_policy` must use `ensure_controller()`. Read queries may follow the same safe posture as the existing observability queries.

- [x] **Task 4: Wire policy-aware prompt context and decision contract into the turn loop**
      - Files: `src/prompt.rs`, `src/agent.rs`, `src/features/inference.rs`
      - Code references:
        - extend `LAYER_6_DECISION_LOOP_DEFAULT` in `src/prompt.rs`
        - extend `build_dynamic_context` in `src/agent.rs`
        - wire decision-envelope parse/validate/retry into `run_scheduled_turn_job_with_limits_and_tool_cap` in `src/agent.rs`
        - preserve the existing deterministic inference test path in `src/features/inference.rs`
      - Validation: `cargo test autonomy_context_includes_policy_and_recent_decisions --lib -- --nocapture`
      - Dependencies: Task 1, Task 2
      - Notes: the decision envelope is an internal runtime contract. The agent should keep using existing inference plumbing, but autonomous economic turns must parse a final JSON object and persist a decision outcome even when the model output is malformed twice in a row.

- [ ] **Task 5: Add autonomy hard gates, exposure bookkeeping, and quarantine accounting**
      - Files: `src/tools.rs`, `src/storage/stable.rs`, `src/strategy/learner.rs`
      - Code references:
        - gate live execution in the `execute_strategy_action` branch of `ToolManager::execute_actions`
        - keep `simulate_strategy_action_tool` read-only
        - update `execute_strategy_action_tool` success/failure handling to persist exposure changes and quarantine counters
        - do not replace existing `validator::validate_execution_plan`; autonomy gates sit on top of existing validation
        - keep `src/strategy/learner.rs::record_outcome` as adjacent outcome learning, not the quarantine mechanism
      - Validation: `cargo test strategy_execution_blocked_by_autonomy_policy_gates --lib -- --nocapture`
      - Dependencies: Task 1, Task 2, Task 4
      - Notes: treat `enter_*` and `exit_*` as the first covered action family. For now, derive protocol concentration from active exposures keyed by `protocol`.

- [ ] **Task 6: Add exposure reconciliation in the existing reconcile job**
      - Files: `src/scheduler.rs`, `src/storage/stable.rs`, `src/tools.rs` or a small new helper module under `src/strategy/`
      - Code references:
        - extend `run_reconcile_job(now_ns)` instead of inventing a new task kind
        - keep the current template freshness/provenance activation pass intact
        - add a second reconciliation phase for durable exposures after template reconciliation
      - Validation: `cargo test exposure_reconciliation_repairs_missing_state_after_execution --lib -- --nocapture`
      - Dependencies: Task 2, Task 5
      - Notes: reconciliation must be idempotent and safe after restart/upgrade. If on-chain position introspection requires reusable helpers, factor them out locally without changing the public tool surface.

- [ ] **Task 7: Add end-to-end coverage and finalize**
      - Files: `tests/pocketic_agent_autonomy.rs`, `tests/pocketic_strategy_controls.rs`, optionally `tests/pocketic_scheduler_queue.rs`
      - Code references:
        - add autonomy-turn coverage to `tests/pocketic_agent_autonomy.rs`
        - add execution-gate/quarantine coverage to `tests/pocketic_strategy_controls.rs`
        - only touch `tests/pocketic_scheduler_queue.rs` if reconciliation visibility is easier to validate there
      - Validation: `icp build && cargo test --features pocketic_tests --test pocketic_agent_autonomy --test pocketic_strategy_controls -- --nocapture`
      - Dependencies: Task 1, Task 2, Task 3, Task 4, Task 5, Task 6
      - Notes: cover no-tactical-question autonomy behavior, invalid envelope fallback to `NoOp`, reserve preservation, authority blocking, quarantine blocking, and exposure reconciliation observability.

---

## Context Files

Files the dev agent should read before starting:

- `AGENTS.md`
- `docs/specs/2026-03-07-policy-bounded-autonomous-decision-runtime.md`
- `docs/specs/2026-03-06-schema-first-strategy-actions.md`
- `docs/specs/2026-02-21-autonomous-wallet-balance-awareness-cycles-eth-usdc.md`
- `src/domain/types.rs`
- `src/storage/sqlite.rs`
- `src/storage/stable.rs`
- `src/lib.rs`
- `src/prompt.rs`
- `src/agent.rs`
- `src/tools.rs`
- `src/scheduler.rs`
- `src/strategy/validator.rs`
- `src/strategy/learner.rs`
- `tests/pocketic_agent_autonomy.rs`
- `tests/pocketic_strategy_controls.rs`

---

## Codebase Snapshot

- `src/domain/types.rs`
  - Already defines `AutonomySuppressionConfig` and stores it in `RuntimeSnapshot`, but there is no first-class `AutonomyPolicy` or decision/exposure/quarantine type.
  - Already has `TurnRecord` and `StrategyOutcomeStats`, but neither matches the spec’s `DecisionRecord` semantics.

- `src/storage/sqlite.rs`
  - Already has additive migration structure through `MIGRATION_004_REFLECTION_MEMORY_SCHEMA`.
  - Already stores strategy activations, kill switches, outcome stats, budgets, and reflection memory, which are the closest patterns for the new autonomy-runtime tables.

- `src/storage/stable.rs`
  - Already exposes `autonomy_suppression_config()` / `set_autonomy_suppression_config(...)`.
  - Already persists turn records through `append_turn_record(...)`.
  - Already persists strategy outcome stats through `record_strategy_outcome(...)`.
  - No dedicated storage exists for autonomy policy, active exposures, strategy quarantines, recent decisions, or reconciliation status.

- `src/lib.rs`
  - Already exposes `get_autonomy_suppression_config`, `list_turns`, and `get_strategy_outcome_stats`.
  - No public Candid methods exist yet for autonomy policy, recent decisions, exposures, quarantines, or reconciliation status.

- `src/prompt.rs`
  - Already says autonomy turns should not ask third parties questions and should proactively act.
  - `LAYER_6_DECISION_LOOP_DEFAULT` still describes general autonomous behavior, not the new decision-envelope contract.

- `src/agent.rs`
  - `build_dynamic_context(...)` already injects cycles, wallet telemetry, staged inbox context, memory, and available tools into Layer 10.
  - `run_scheduled_turn_job_with_limits_and_tool_cap(...)` already manages inference rounds, tool execution, degraded autonomy paths, and turn persistence.
  - No decision-envelope parse/validate/retry loop exists.

- `src/tools.rs`
  - `ToolManager::execute_actions(...)` already gates strategy execution by survival-policy checks.
  - `simulate_strategy_action_tool(...)` and `execute_strategy_action_tool(...)` already compile/validate/execute generic strategy plans.
  - Existing hard gates are template validation, survival policy, kill switch, per-call/template budgets, and strategy status. There are no autonomy-policy reserve/concentration/quarantine gates yet.

- `src/scheduler.rs`
  - `TaskKind::Reconcile` and `run_reconcile_job(...)` already provide the right periodic seam for idempotent repair logic.
  - Current reconciliation only manages strategy template freshness/provenance/activation, not economic exposure state.

- `src/strategy/learner.rs`
  - Already records execution outcomes and auto-disables templates after three consecutive deterministic failures.
  - This is useful adjacent behavior, but it is not the spec’s per-strategy quarantine state and should remain separate.

---

## Autonomy Scope

### Decide yourself:

- Exact helper names and local module factoring for policy summaries, gate evaluation, and reconciliation helpers.
- Exact shape of `DecisionRecord.candidates_summary` and `DecisionRecord.explanation` as long as they stay concise and deterministic enough for auditing.
- Exact operator query split between separate queries and one combined reconciliation-status query, as long as the required inspection data is exposed.
- Exact blocked-reason enum/string wording, as long as tests pin it and it is machine-readable.

### Escalate (log blocker, skip, continue):

- If policy gates cannot be implemented cleanly on top of the current strategy tool surface without redesigning how strategy execution intent carries value/notional information.
- If on-chain position truth for exposure reconciliation requires a new public runtime capability rather than internal helper logic.
- If Candid changes force broader UI or external-client compatibility work beyond the scope of this slice.
- If the existing generic strategy abstractions are insufficient to distinguish capital-touching `enter_*` / `exit_*` actions safely.

---

## Verification

### Smoke Tests

- `cargo test autonomy_policy_defaults_are_conservative --lib -- --nocapture` -- proves the new default policy is installed with the intended conservative values.
- `cargo test autonomy_policy_and_decision_records_roundtrip --lib -- --nocapture` -- proves SQLite/stable persistence for policy + recent decisions works.
- `cargo test autonomy_context_includes_policy_and_recent_decisions --lib -- --nocapture` -- proves Layer 10 rendering includes the new autonomy sections.
- `cargo test invalid_autonomy_decision_shape_retries_then_noops --lib -- --nocapture` -- proves invalid decision envelopes do not loop indefinitely.
- `cargo test strategy_execution_blocked_by_autonomy_policy_gates --lib -- --nocapture` -- proves runtime hard gates block unsafe live execution.
- `cargo test repeated_strategy_failures_trigger_quarantine --lib -- --nocapture` -- proves repeated failures create a quarantine that blocks re-execution.
- `cargo test exposure_reconciliation_repairs_missing_state_after_execution --lib -- --nocapture` -- proves reconciliation can repair drifted local exposure state.

### Expected State

- `rg "struct AutonomyPolicy" src/domain/types.rs` succeeds.
- `rg "struct ActiveExposure" src/domain/types.rs` succeeds.
- `rg "struct StrategyQuarantine" src/domain/types.rs` succeeds.
- `rg "struct DecisionRecord" src/domain/types.rs` succeeds.
- `rg "AutonomyDecisionEnvelope" src/domain/types.rs src/agent.rs` succeeds.
- `rg "update_autonomy_policy" src/lib.rs ic-automaton.did` succeeds.
- `rg "get_recent_decisions" src/lib.rs ic-automaton.did` succeeds.
- `rg "get_active_exposures" src/lib.rs ic-automaton.did` succeeds.
- `rg "get_strategy_quarantines" src/lib.rs ic-automaton.did` succeeds.
- `rg "get_exposure_reconciliation_status" src/lib.rs ic-automaton.did` succeeds.
- `rg "invalid_decision_shape" src/agent.rs` succeeds.
- `rg "concentration" src/tools.rs` succeeds.

### Regression

- `cargo test --lib -- --nocapture` -- library regression suite stays green.
- `cargo test --all-targets --all-features -- --nocapture` -- broader Rust regression suite stays green.
- `cargo clippy --all-targets --all-features -- -D warnings` -- lint baseline stays green.
- `icp build` -- Wasm, Candid export path, and PocketIC artifact path stay valid.

### Integration Test

- `icp build && cargo test --features pocketic_tests --test pocketic_agent_autonomy --test pocketic_strategy_controls -- --nocapture` -- proves the feature works from the outside: autonomous turns emit bounded decisions, unsafe executions are blocked, repeated failures quarantine a strategy, and operators can inspect recent decisions/exposures through public queries.
