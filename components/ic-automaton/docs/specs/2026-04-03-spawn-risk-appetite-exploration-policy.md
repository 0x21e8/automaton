# Spec: Spawn Risk Appetite and Explore/Exploit Autonomy Policy

**Status:** LOCKED
**Date:** 2026-04-03
**Author:** Codex (spec-writer) | Mode: autonomous
**Complexity:** complex
**Authority:** approval
**Tier:** 3

---

## Problem

The sibling launchpad already collects a spawn-time `risk` value, but in `ic-automaton` that value is only persisted as bootstrap metadata. It does not currently derive or constrain the automaton's live economic behavior.

That leaves two concrete autonomy gaps:

1. the spawn risk field is mostly cosmetic because it does not seed the durable `AutonomyPolicy`
2. the runtime has no first-class way to balance exploration versus exploitation, so the automaton must either ossify around known loops or take novelty risk using the same authority budget as proven strategies

The repo already has the right substrate:

1. spawn bootstrap metadata
2. durable autonomy policy
3. durable exposure and quarantine state
4. strategy outcome stats
5. runtime strategy execution gates

What is missing is the contract that says:

1. how spawn risk maps to a durable policy
2. how experimental strategies differ from established strategies
3. how much capital may be used to probe novelty versus scale known loops

## Goal

Implement a first shippable slice where launchpad risk appetite becomes a real autonomy control that seeds both:

1. financial risk limits for established strategies
2. exploration limits for experimental strategies

This slice succeeds when:

1. spawn risk is validated as a closed `1..5` contract across launchpad and child bootstrap
2. `ic-automaton` derives a durable autonomy policy from spawn risk during bootstrap
3. the policy includes a bounded exploration sleeve distinct from normal exploitation limits
4. runtime code enforces separate limits for experimental versus established strategies
5. spawn-selected strategies can exploit within the main policy budget, while newly introduced strategies are limited to the exploration sleeve until they earn promotion
6. the prompt/runtime context clearly exposes remaining exploitation and exploration budget so the automaton can choose between exploit, explore, and no-op without asking a human for tactical guidance

## Non-Goals

1. No expected-value math, confidence scores, or multi-armed-bandit framework in this slice.
2. No broad strategy discovery worker changes in this slice.
3. No changes to existential reserve floors based on appetite; survival floors stay fixed initially.
4. No generalized asset-pricing engine across arbitrary ERC-20s.
5. No promise that risk appetite remains immutable after spawn; it is a bootstrap seed, not a permanent lock.
6. No redesign of the launchpad UX beyond making the current `1..5` slider contract explicit and meaningful.

---

## Autonomous Decisions

1. Keep the wire contract numerically simple: `risk` remains a numeric value over the wire, but both repos must validate it as a closed `1..5` range instead of treating `u8` / `nat8` as open-ended.
2. Interpret spawn appetite as a bootstrap seed for durable policy, not as prompt-only flavor text.
3. Keep reserve floors constant across appetite levels in v1:
   - cycles runway
   - inference USDC floor
   - gas ETH floor
4. Add a new `ExplorationPolicy` instead of overloading `RiskLimits`. Exploitation and exploration need separate budgets.
5. Use a minimal two-class maturity model in v1:
   - `Established`
   - `Experimental`
6. Seed spawn-selected repository strategies as `Established`.
7. Default newly introduced strategies to `Experimental` unless a controller explicitly seeds them otherwise.
8. Promotion from `Experimental` to `Established` should be deterministic and based on existing durable runtime evidence, primarily `StrategyOutcomeStats`, not on model-written confidence fields.
9. Treat strategy quarantine as an automatic demotion signal: a quarantined strategy cannot remain `Established`.
10. Fix two existing enforcement gaps as part of this slice rather than leaving them behind:
    - `max_single_action_bps` exists in policy but is not currently enforced
    - capital-at-risk denominators are effectively ETH-only today, which is unsafe for USDC-centric spawned strategies

---

## Requirements

### Must Have

- [ ] Validate launchpad spawn risk as a closed `1..5` contract in shared/web/factory code and reject out-of-range values before child install.
- [ ] Validate child bootstrap risk as `1..5` inside `ic-automaton` and trap invalid bootstrap payloads.
- [ ] Add a deterministic appetite-to-policy mapping function in `ic-automaton`.
- [ ] Extend the durable autonomy policy model with `ExplorationPolicy`.
- [ ] Persist the provenance of the active policy so the runtime can distinguish at least:
      - system default
      - spawn-derived
      - explicit override
- [ ] Add durable per-strategy maturity state with exactly two classes in this slice:
      - `Established`
      - `Experimental`
- [ ] Seed spawn-selected repository strategies as `Established` during spawn bootstrap and strategy installation.
- [ ] Default agent-authored or post-spawn introduced strategies to `Experimental`.
- [ ] Promote an `Experimental` strategy to `Established` only after deterministic, durable success criteria are met.
- [ ] Demote or keep a strategy `Experimental` when repeated deterministic failures or quarantine indicate it is not yet safe to exploit at normal size.
- [ ] Enforce separate hard caps for `Established` versus `Experimental` strategies during `execute_strategy_action`.
- [ ] Enforce `max_single_action_bps` in runtime code.
- [ ] Make capital-at-risk enforcement asset-aware for at least:
      - ETH / WETH-like gas-denominated actions
      - USDC-denominated strategies
- [ ] Block live capital deployment for experimental strategies when the asset denominator cannot be derived safely.
- [ ] Inject exploration-versus-exploitation state into Layer 10 context, including:
      - active appetite level
      - remaining exploitation sleeve
      - remaining exploration sleeve
      - established strategies
      - experimental strategies eligible for promotion
- [ ] Keep `risk` visible in bootstrap/query surfaces for auditability.
- [ ] Regenerate `ic-automaton.did` after public interfaces change.

### Should Have

- [ ] Add a compact query view summarizing:
      - appetite level
      - policy provenance
      - exploration sleeve usage
      - strategy maturity classes
- [ ] Surface promotion and demotion reasons in queryable runtime state.
- [ ] Make launchpad UI copy explicit that risk appetite steers both capital deployment and novelty tolerance.
- [ ] Add sibling-repo tests proving the launchpad rejects out-of-range appetite values before session creation.

### Could Have

- [ ] Add a manual controller/steward method to reclassify a strategy between `Experimental` and `Established` for recovery operations.
- [ ] Add a separate prompt summary line describing why the last exploration probe was promoted, blocked, or demoted.

---

## Constraints

1. Keep KISS. This slice adds budget sleeves and maturity classes, not a general portfolio optimizer.
2. Survival-first constraints remain absolute and do not scale down for aggressive appetites.
3. Hard enforcement must stay in runtime code and durable state, not in prompt prose.
4. The exploration sleeve must never bypass reserve floors, concentration gates, or simulation requirements.
5. Cross-repo changes are required, so this slice is approval-only.
6. The first asset-aware denominator implementation may be narrow, but it must be fail-closed for unsupported assets.
7. Use existing durable evidence where possible:
   - `StrategyOutcomeStats`
   - `ActiveExposure`
   - `StrategyQuarantine`
8. Do not hand-edit `ic-automaton.did`; regenerate it from built Wasm.

---

## Data Model

### Shared appetite contract

The launchpad remains `1..5` in the UI and API, but the range becomes explicit and enforced:

- `1` Conservative
- `2` Cautious
- `3` Balanced
- `4` Aggressive
- `5` Degen

Out-of-range values are invalid everywhere.

### `ExplorationPolicy`

Add a new durable type in `ic-automaton`:

```rust
pub struct ExplorationPolicy {
    pub max_experimental_total_exposure_bps: u16,
    pub max_experimental_single_action_bps: u16,
    pub max_concurrent_experimental_positions: u16,
    pub promotion_success_runs: u16,
}
```

Meaning:

1. `max_experimental_total_exposure_bps`
   total share of deployable capital that may be tied up in experimental positions
2. `max_experimental_single_action_bps`
   max size of one live experimental probe
3. `max_concurrent_experimental_positions`
   hard cap on simultaneous experimental live positions
4. `promotion_success_runs`
   minimum successful live runs before the strategy may move to `Established`

### `StrategyMaturity`

Add a durable per-strategy runtime class:

```rust
pub enum StrategyMaturity {
    Experimental,
    Established,
}
```

V1 behavior:

1. spawn-selected repository strategies start as `Established`
2. strategies introduced after spawn start as `Experimental`
3. `Experimental` strategies use exploration caps
4. `Established` strategies use the main `RiskLimits`
5. any quarantined strategy is treated as non-established until it satisfies promotion criteria again

### Policy provenance

Add a minimal durable provenance marker:

```rust
pub enum AutonomyPolicySource {
    SystemDefault,
    SpawnDerived { risk_appetite: u8 },
    Override,
}
```

This is needed so the runtime and UI can answer:

1. whether the active policy still reflects the spawn appetite
2. whether a controller/steward override replaced the spawn-derived policy

### Appetite mapping

Use one deterministic mapping function for both `RiskLimits` and `ExplorationPolicy`.

Initial v1 mapping:

| Appetite | Main total exposure | Main single action | Main protocol concentration | Experimental total exposure | Experimental single action | Concurrent experimental positions | Promotion success runs |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 Conservative | 1000 bps | 300 bps | 500 bps | 100 bps | 100 bps | 1 | 4 |
| 2 Cautious | 2000 bps | 700 bps | 1000 bps | 200 bps | 100 bps | 1 | 3 |
| 3 Balanced | 3000 bps | 1000 bps | 1500 bps | 500 bps | 200 bps | 2 | 3 |
| 4 Aggressive | 4500 bps | 1500 bps | 2500 bps | 1200 bps | 400 bps | 3 | 2 |
| 5 Degen | 6000 bps | 2500 bps | 3500 bps | 2000 bps | 700 bps | 4 | 2 |

Reserve floors remain the current conservative defaults for all five levels.

---

## Runtime Semantics

### Exploit

The automaton is exploiting when it executes an `Established` strategy. These executions are governed by:

1. reserve floors
2. main `RiskLimits`
3. execution authority
4. quarantine

### Explore

The automaton is exploring when it executes an `Experimental` strategy with live capital. These executions are governed by:

1. reserve floors
2. main `RiskLimits`
3. exploration sleeve caps
4. execution authority
5. quarantine

An experimental live action must pass all of the following:

1. `max_single_action_bps`
2. `max_experimental_single_action_bps`
3. `max_total_exposure_bps`
4. `max_experimental_total_exposure_bps`
5. `max_protocol_concentration_bps`
6. `max_concurrent_experimental_positions`

### Promotion

An `Experimental` strategy may be promoted to `Established` when:

1. it is not quarantined
2. `StrategyOutcomeStats.success_runs >= promotion_success_runs`
3. `StrategyOutcomeStats.deterministic_failure_streak == 0`

No EV scoring or profitability model is required in this slice.

### Asset-aware denominators

V1 denominator rules:

1. `USDC`
   deployable capital = wallet USDC balance minus `min_inference_usdc_6dp`
2. `ETH` / `WETH`
   deployable capital = wallet ETH balance minus `min_gas_wei`
3. unsupported / unknown asset symbol
   fail closed for live `enter_*` execution with a machine-readable error

This fixes the current unsafe assumption that deployable capital is always ETH-derived.

---

## Implementation Plan

- [ ] **Task 1: Close the spawn risk contract across launchpad and child bootstrap**
      - Files: `../automaton-launchpad/packages/shared/src/spawn.ts`, `../automaton-launchpad/apps/web/src/components/spawn/spawn-state.ts`, `../automaton-launchpad/backend/factory/src/types.rs`, `../automaton-launchpad/backend/factory/src/init.rs`, `../automaton-launchpad/backend/factory/src/spawn.rs`, `src/lib.rs`, `tests/pocketic_spawn_bootstrap.rs`
      - Validation: `cargo test apply_init_args_spawn_bootstrap_invalid_risk_rejected --lib -- --nocapture`
      - Notes:
        - keep the public wire format numeric
        - add explicit shared validation helpers and reject anything outside `1..5`
        - update launchpad copy to say appetite affects both deployment aggressiveness and novelty tolerance

- [ ] **Task 2: Extend the autonomy policy model with exploration policy and provenance**
      - Files: `src/domain/types.rs`, `src/storage/sqlite.rs`, `src/storage/stable.rs`, `src/lib.rs`
      - Validation: `cargo test risk_appetite_profile_derivation_and_persistence --lib -- --nocapture`
      - Dependencies: Task 1
      - Notes:
        - add `ExplorationPolicy`
        - add `AutonomyPolicySource`
        - add a helper like `AutonomyPolicy::from_spawn_risk_appetite(risk, now_ns)`
        - preserve current reserve defaults for all appetite levels

- [ ] **Task 3: Add durable strategy maturity state and bootstrap seeding**
      - Files: `src/domain/types.rs`, `src/storage/sqlite.rs`, `src/storage/stable.rs`, `src/lib.rs`, `src/strategy/registry.rs`
      - Validation: `cargo test spawn_selected_strategies_seed_established_maturity --lib -- --nocapture`
      - Dependencies: Task 2
      - Notes:
        - persist `StrategyMaturity` keyed by strategy/template id
        - seed spawn-selected repository strategies as `Established`
        - default later-introduced strategies to `Experimental`
        - add a small helper to evaluate promotion based on `StrategyOutcomeStats`

- [ ] **Task 4: Enforce explore/exploit limits in the strategy execution path**
      - Files: `src/tools.rs`, `src/agent.rs`, `tests/pocketic_strategy_controls.rs`, `tests/pocketic_agent_autonomy.rs`
      - Validation: `cargo test strategy_execution_enforces_single_action_and_experimental_sleeves --lib -- --nocapture`
      - Dependencies: Task 2, Task 3
      - Notes:
        - enforce `max_single_action_bps`
        - compute denominators by asset symbol instead of ETH-only
        - enforce exploration sleeve caps for `Experimental` strategies
        - fail closed when the denominator asset cannot be derived safely

- [ ] **Task 5: Add promotion, demotion, and prompt/runtime visibility**
      - Files: `src/tools.rs`, `src/agent.rs`, `src/lib.rs`, `ic-automaton.did`
      - Validation: `icp build && ./scripts/generate-candid.sh ic-automaton.did`
      - Dependencies: Task 4
      - Notes:
        - promote experimental strategies once durable success criteria are met
        - treat quarantine as a non-established state
        - add a compact query view for appetite, provenance, sleeves, and maturity classes
        - render exploit/explore budget state into Layer 10

- [ ] **Task 6: Add end-to-end spawn-to-policy tests**
      - Files: `tests/pocketic_spawn_bootstrap.rs`, `tests/pocketic_agent_autonomy.rs`, `../automaton-launchpad/apps/web/src/components/spawn/spawn-state.test.ts`, `../automaton-launchpad/apps/indexer/test/factory-canister-adapter.test.ts`
      - Validation: `cargo test --features pocketic_tests spawn_bootstrap_risk_appetite_seeds_policy_and_limits -- --nocapture`
      - Dependencies: Task 1, Task 5
      - Notes:
        - prove that different appetite levels seed different policies
        - prove that the same experimental action can be blocked at low appetite and allowed at higher appetite without changing survival floors

---

## Context Files

Files the dev agent should read before implementation:

- `src/lib.rs`
- `src/domain/types.rs`
- `src/tools.rs`
- `src/agent.rs`
- `src/storage/stable.rs`
- `src/storage/sqlite.rs`
- `tests/pocketic_spawn_bootstrap.rs`
- `tests/pocketic_strategy_controls.rs`
- `tests/pocketic_agent_autonomy.rs`
- `../automaton-launchpad/apps/web/src/components/spawn/spawn-state.ts`
- `../automaton-launchpad/backend/factory/src/types.rs`
- `../automaton-launchpad/backend/factory/src/init.rs`
- `../automaton-launchpad/backend/factory/src/spawn.rs`

---

## Codebase Snapshot

Current relevant state as of 2026-04-03:

- `src/lib.rs`
  - `SpawnBootstrapArgs` carries `risk: u8`
  - `apply_spawn_bootstrap()` stores `SpawnBootstrapView.risk` and provider secrets, but does not derive or install a policy from risk
- `src/domain/types.rs`
  - `AutonomyPolicy` currently contains `ReservePolicy`, `RiskLimits`, `ExecutionAuthority`, and `EscalationRules`
  - there is no `ExplorationPolicy`, no policy provenance, and no strategy maturity model
  - `RuntimeSnapshot` stores `spawn_risk: Option<u8>`
- `src/tools.rs`
  - reserve floors are enforced
  - total exposure and protocol concentration are enforced
  - `max_single_action_bps` exists in `RiskLimits` but is not enforced
  - deployable capital is derived from ETH balance minus gas floor, which is wrong for USDC-denominated spawned strategies
- `src/agent.rs`
  - Layer 10 renders the main autonomy policy summary but has no explore/exploit budget section
- `../automaton-launchpad/apps/web/src/components/spawn/spawn-state.ts`
  - the UI slider exposes a fixed `1..5` range with labels, but this is not yet a closed contract end to end
- `../automaton-launchpad/backend/factory/src/types.rs`
  - factory/session types accept `risk: u8`
- `../automaton-launchpad/backend/factory/src/spawn.rs`
  - spawn verification checks that the child reports the same bootstrap risk value, but does not know whether that value changed the child policy

---

## Autonomy Scope

### Decide yourself:

- Exact helper names and storage-table names for appetite, provenance, and maturity state
- Whether query surfacing is a new query or an extension of an existing policy/status view
- Whether promotion evaluation runs inline after successful execution or in a small post-execution helper

### Escalate (log blocker, skip, continue):

- Any proposal to change survival reserve floors by appetite
- Any proposal to support non-USDC ERC-20 denominators in this slice beyond safe fail-closed behavior
- Any proposal to let experimental strategies bypass the main `RiskLimits`
- Any proposal to auto-promote strategies using model-written profitability or confidence claims

---

## Verification

### Smoke Tests

- `cargo test risk_appetite_profile_derivation_and_persistence --lib -- --nocapture` -- proves appetite maps deterministically to durable policy state
- `cargo test strategy_execution_enforces_single_action_and_experimental_sleeves --lib -- --nocapture` -- proves main and exploration caps are both enforced in runtime code
- `cargo test strategy_maturity_promotion_requires_success_runs --lib -- --nocapture` -- proves promotion is based on durable outcome evidence rather than prompt output
- `cargo test --features pocketic_tests spawn_bootstrap_risk_appetite_seeds_policy_and_limits -- --nocapture` -- proves spawn bootstrap risk changes the installed child policy and downstream enforcement

### Expected State

- File `docs/specs/2026-04-03-spawn-risk-appetite-exploration-policy.md` exists and is >1000 bytes
- `rg "ExplorationPolicy|StrategyMaturity|AutonomyPolicySource" src/domain/types.rs` succeeds
- `rg "max_single_action_bps" src/tools.rs` shows active enforcement logic, not just rendering/defaults
- `rg "risk: RiskProfile\\[\"value\"\\]" ../automaton-launchpad/apps/web/src/components/spawn/spawn-state.ts` still succeeds and the same file contains copy indicating novelty/exploration meaning

### Regression

- `cargo test --lib -- --nocapture` -- core Rust unit coverage still passes
- `cargo test --features pocketic_tests -- --nocapture` -- PocketIC integration coverage still passes after the new policy/maturity surfaces are added

### Integration Test

- `cargo test --features pocketic_tests appetite_level_changes_experimental_probe_admission -- --nocapture` -- proves the same experimental strategy probe is blocked for a low-appetite spawn and allowed for a higher-appetite spawn while reserve floors remain unchanged

