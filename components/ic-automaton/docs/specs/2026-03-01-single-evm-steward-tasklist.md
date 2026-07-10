# Single EVM Steward Implementation Tasklist

Date: 2026-03-01  
Status: Planned  
Source: `docs/design/STEWARD_ROLE_DUAL_AUTH_II_EVM.md`

## Scope

Implement a single-steward, EVM-only command plane with full parity for current controller-gated runtime operations, steward-aware UI behavior, and direct unpaid steward messaging.

## Guardrails (always on)

- [ ] Keep autonomy-first behavior intact (no manual reset requirement after transient failures).
  - File refs: `src/agent.rs`, `src/scheduler.rs`, `src/domain/state_machine.rs`
- [ ] Use host-safe time helper for expiry/nonce windows (`current_time_ns()` pattern), no direct `ic_cdk::api::time()` in native-test paths.
  - File refs: `src/timing.rs`, `src/lib.rs`, new steward auth modules
- [ ] Keep all steward auth decisions and command outcomes auditable via `canlog`.
  - File refs: `src/lib.rs`, `src/storage/stable.rs`
- [ ] Regenerate candid from compiled wasm after API changes; do not hand-edit did.
  - File refs: `scripts/generate-candid.sh`, `ic-automaton.did`

## Phase 1: Single steward state + admin path

- [x] Add stable model for one active steward (`chain_id`, `address`, `enabled`, `last_used_at_ns`) and steward nonce state.
  - File refs: `src/domain/types.rs`, `src/storage/stable.rs`
  - Check:
    - [x] `cargo test --lib storage::stable::tests::`
- [x] Add/extend controller recovery endpoint to set/rotate steward (`set_steward_admin`) and read status (`get_steward_status`).
  - File refs: `src/lib.rs`, `src/domain/types.rs`, `src/storage/stable.rs`, `ic-automaton.did`
  - Check:
    - [x] `cargo test --lib lib::tests::`

## Phase 2: EVM-only steward auth + command execution

- [x] Add `EvmStewardProof` and verification flow (address normalization, signature recovery, chain match, nonce match, expiry).
  - File refs: `src/domain/types.rs`, `src/features/`, `src/lib.rs`, `src/storage/stable.rs`
  - Check:
    - [x] `cargo test --lib features::evm::tests::verify_evm_steward_proof_ -- --nocapture`
- [x] Add `steward_execute(command, proof)` update endpoint.
  - File refs: `src/lib.rs`, `ic-automaton.did`
  - Check:
    - [x] `cargo test --lib lib::tests::`
- [x] Add `UpdateSteward` command variant and atomic nonce reset on steward rotation.
  - File refs: `src/domain/types.rs`, `src/lib.rs`, `src/storage/stable.rs`
  - Check:
    - [x] `cargo test --lib storage::stable::tests:: lib::tests::`

## Phase 3: Full controller-operation parity through commands

- [x] Add `StewardCommand` variants mapping to every current controller-gated runtime method in `src/lib.rs`.
  - File refs: `src/domain/types.rs`, `src/lib.rs`
- [x] Implement dispatcher from each command variant to existing runtime mutators.
  - File refs: `src/lib.rs`, `src/storage/stable.rs`, `src/strategy/`
- [x] Add parity guard test that fails when a controller-gated method is missing a command mapping.
  - File refs: `src/lib.rs` tests, or dedicated `tests/` parity test
  - Check:
    - [x] `cargo test --lib controller_gated_updates_have_steward_command_parity`

## Phase 4: Direct steward messaging (no payment, no inbox contract dependency)

- [x] Add `SendStewardMessage { sender, message }` command variant.
  - File refs: `src/domain/types.rs`, `src/lib.rs`
- [x] Route steward message through the same internal ingestion path as inbox messages, tagged as `steward_direct`.
  - File refs: `src/agent.rs`, `src/scheduler.rs`, `src/storage/stable.rs`, `src/domain/types.rs`
  - Check:
    - [x] `cargo test --lib agent::tests:: scheduler::tests::`

## Phase 5: UI steward awareness and command expansion

- [x] Detect EVM wallet connection and resolve current steward status/capabilities.
  - File refs: `src/ui_app.js`, HTTP/API handlers in `src/http.rs` and `src/lib.rs`
- [x] Show expanded command palette only when connected wallet matches active steward.
  - File refs: `src/ui_app.js`
- [x] Add direct steward message UI action and command signing flow.
  - File refs: `src/ui_app.js`
  - Check:
    - [x] `cargo test --lib http::tests::`

## Phase 6: Integration, security, and candid validation

- [x] Add PocketIC integration coverage for:
  - valid steward command execution
  - non-steward rejection
  - proof replay rejection
  - steward rotation invalidating old wallet
  - direct steward message entering conversation flow
  - File refs: `tests/` (new/updated PocketIC tests)
  - Check:
    - [x] `icp build`
    - [x] `cargo test --features pocketic_tests --test pocketic_steward_command_plane`
- [x] Regenerate candid after API additions.
  - Check:
    - [x] `icp build`
    - [x] `./scripts/generate-candid.sh ic-automaton.did`

## Final validation gate (must be green)

- [x] `bash .githooks/pre-commit`
- [ ] `icp build`
- [ ] `cargo test --all-targets --all-features`
- [ ] `cargo test --features pocketic_tests`
- [ ] `./scripts/generate-candid.sh ic-automaton.did`

## Important Lessons

- Keep this section current with durable implementation lessons discovered while executing tasks.
