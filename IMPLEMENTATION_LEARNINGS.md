# Implementation Learnings

## Plan 005 — Hermetic release-manifest tests (2026-07-15)

- Repository convention: production release inputs such as git state, environment, clock, and artifact files stay at the CLI boundary; manifest construction is a pure validated function with explicit inputs.
- Constraint: hermetic tests must not depend on ignored `dist/` Wasm files or mutate the working repository. A temporary git repository can safely exercise the production CLI's trusted dirty-state path.
- ICP/Candid/stable storage: not applicable; this plan changed release tooling only and preserved the manifest schema and output fields.
- Effective gates: the focused Node suite caught clean/dirty and publish semantics; the shared schema suite protected consumer compatibility; full `npm test` and `npm run lint` restored the root gate.
- Review: one plan-required P2 was accepted and fixed by adding production-CLI integration coverage for dirty publish rejection. No findings were deferred or rejected, and the second review cleared all P0/P1/implementation-caused P2 findings.
- Follow-up risk: future release-manifest fields should remain injectable and receive deterministic fixture coverage without introducing CLI or environment overrides for trusted source commit or dirty state.

## Plan 001 — Asset-aware strategy preflight (2026-07-16)

- Repository convention: one canonical `(chain_id, asset_address/native, decimals)` identity must drive compilation, reserve and concentration policy, persistence, and reconciliation. `ActiveExposure` is also constructed in storage round-trip tests, so the user authorized narrowly scoped test-literal updates in `storage/sqlite.rs` and `storage/stable.rs` for the additive fields.
- Constraint: declared effects are trustworthy only when bound to the compiled calldata or token target. Multiple effects must be grouped with checked `U256` arithmetic; split effects, declaration order, chain changes, or decimal changes must never alter policy meaning.
- ICP/Candid/stable storage: additive optional exposure fields regenerate to optional Candid fields and retain serde decoding compatibility. Historical execution effects are deltas, so reconciliation replays them oldest-first, preserves partial exits, closes only at zero, and is idempotent.
- Effective gates: focused strategy, tools, storage, reconciliation, and hermetic RPC tests; full backend; factory seed tests; fmt; warning-free clippy; recipe verification; Wasm generation; Candid regeneration; and child-contract parity all passed. A zero-match exact-filter attempt was rejected and rerun with one matching test.
- Review findings accepted and fixed: calldata-bound asset proofs; aggregate per-asset limits; chain/native identity; approval checks; checked `U256` persistence; order-independent reserve projection; static evaluator coverage; consistent fixtures; full Base-USDC identity; delta-folding reconciliation; and distinct transport-failure coverage. No findings were deferred or rejected, and the final authorized review cleared every P0/P1/implementation-caused P2.
- Follow-up risk for Plan 002: receipt confirmation must consume these exact compiled asset effects and apply durable budget/exposure changes once, only after confirmed success. It must not introduce a second amount model or replay already-folded deltas.

## Plan 002 — Receipt-backed strategy execution (2026-07-16)

- Repository convention: success and terminal-failure bookkeeping must be committed with the durable execution transition in guarded SQLite transactions. Any state copied across an RPC `await` is stale-capable and must use compare-and-swap or reload/no-op semantics before persistence.
- Constraint: confirmation and failure are competing terminal transitions. Both exactly-once guards and the current lifecycle/version must be checked symmetrically; unconditional progress updates can otherwise resurrect terminal records and erase guards.
- ICP/Candid/stable storage: the additive schema-v10 JSON table preserves ordered call evidence, Plan 001 effects, and reload state. Confirmed-history reconstruction requires deterministic keyset pagination rather than a fixed oldest-first limit.
- Effective gates: focused strategy execution, scheduler, storage, atomic failure classification/idempotency, pagination boundary, full backend, fmt, and clippy all passed. Passing isolated gates did not expose stale-copy interleavings, which required adversarial review.
- Review cycle 1: accepted and fixed atomic failure bookkeeping, paginated history, exactly-once submission failures, deterministic-vs-nondeterministic classification, and explicit unattempted-call recovery.
- Review cycle 2: identified and then, with explicit user authorization for a narrow additional cycle, fixed stale nonterminal overwrites, asymmetric competing-terminal guards, and missing end-to-end malformed/RPC durable backoff coverage.
- Final review: payload compare-and-swap fences all stale progress writes crossing RPC awaits; success and failure transactions symmetrically require durable `Pending` with both guards clear; adversarial conflict-direction tests and the real scheduler error path passed. No P0/P1/implementation-caused P2 findings remain, and no findings were deferred or rejected.
- Process improvement: durable async work now requires a written transition table, stale-snapshot fencing after every `await`, symmetric terminal guards, atomic side effects, adversarial interleaving tests, end-to-end malformed/RPC persistence tests, pagination-boundary tests, and a state-machine review checkpoint before full gates. High-risk plans must be split or receive an explicit design-review budget rather than using implementation review to discover the state model.
- Follow-up risk for Plan 004: reuse the same confirmation-count convention and monotonic terminal-transition discipline for factory finality; never use an unconditional stale write after an RPC await.
