# Implementation Learnings

## Plan 005 — Hermetic release-manifest tests (2026-07-15)

- Repository convention: production release inputs such as git state, environment, clock, and artifact files stay at the CLI boundary; manifest construction is a pure validated function with explicit inputs.
- Constraint: hermetic tests must not depend on ignored `dist/` Wasm files or mutate the working repository. A temporary git repository can safely exercise the production CLI's trusted dirty-state path.
- ICP/Candid/stable storage: not applicable; this plan changed release tooling only and preserved the manifest schema and output fields.
- Effective gates: the focused Node suite caught clean/dirty and publish semantics; the shared schema suite protected consumer compatibility; full `npm test` and `npm run lint` restored the root gate.
- Review: one plan-required P2 was accepted and fixed by adding production-CLI integration coverage for dirty publish rejection. No findings were deferred or rejected, and the second review cleared all P0/P1/implementation-caused P2 findings.
- Follow-up risk: future release-manifest fields should remain injectable and receive deterministic fixture coverage without introducing CLI or environment overrides for trusted source commit or dirty state.
