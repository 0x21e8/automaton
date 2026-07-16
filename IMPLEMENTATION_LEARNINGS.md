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
