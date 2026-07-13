# Plan 008: Genesis constitution end-to-end

> **Executor instructions**: Read `specs/DESIGN_PRINCIPLES.md` completely —
> this plan implements P1 (progenitor, not operator), P3 (identity authored at
> birth), and the constitution-validation stance in P3's critical note. Run
> every gate, stop on any STOP condition, update row 008 in `plans/README.md`.
>
> **Drift check (run first)**: written at destination commit `0ddd877`;
> plan 007 must be COMPLETE (the Genesis document slot must exist in
> `components/ic-automaton/src/prompt.rs`). Operator-owned uncommitted changes
> may be present — do not discard them.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: MED (crosses the spawn-protocol boundary; gated by child-contract
  verification)
- **Depends on**: `plans/007-restructure-prompt-stack.md`
- **Category**: product-direction, architecture
- **Planned at**: destination commit `0ddd877`, 2026-07-13

## Why this matters

Today every automaton is the same being with a different wallet: the identity
layer is generic boilerplate, the "soul" (`storage/stable.rs::set_soul`) is a
string label, and the spawn wizard collects only operational inputs (provider,
risk `1..5`, skills, strategies — see `SpawnBootstrapArgs` in
`crates/spawn-protocol/src/lib.rs`). A principal needs a **genesis
constitution**: character, values, temperament, ambitions, voice — authored by
the progenitor at spawn, immutable for the being's lifetime, mutated only at
reproduction (plan 013). This is the unit of authorship that makes spawning
worth doing (P6) and the empty slot plan 007 created.

## Design contract (do not re-litigate)

- **A constitution shapes character, never authority.** The Charter outranks
  it and runtime tool gates enforce safety regardless of its content.
  Validation at genesis: UTF-8 text, length bounds (pick bounds and record
  them in `crates/spawn-protocol`; suggested 400–8000 chars), plus rejection
  of controller-style grants (lexical screening for address-obedience
  patterns is best-effort defense in depth, not the security boundary — the
  security boundary is Charter precedence + tool gates, which already exist).
- **Immutable after birth.** No tool, steward command, or admin endpoint may
  rewrite a live being's constitution. Storage-level: written once during
  bootstrap, read-only thereafter.
- **The wire contract lives in `crates/spawn-protocol`** so factory and child
  validate identically (`npm run verify:child-contract` is the gate).
- The constitution includes a **display name** and the **constitution text**.
  Do not add structured personality fields in v1 — freeform text plus name.
- The factory registry records the **constitution hash** (content stays in the
  child; the hash makes lineage/diff work in plan 013 possible and lets the
  indexer verify what the child claims).
- Product language per spec section 7: the wizard flow is a **genesis rite**;
  "launch/configure" copy is replaced in the touched screens. A full product
  rename is out of scope.

## Current state (verified anchors)

- Contract: `crates/spawn-protocol/src/lib.rs` — `SpawnBootstrapArgs`
  (contains `risk: u8`), `InitArgs`, `SpawnBootstrapView`.
- Factory: `backend/factory/src/init.rs` (child init/EVM derivation),
  `spawn.rs` (create → install → verify → release), `state.rs` +
  `api/public.rs` (registry), `factory.did`.
- Child: genesis slot in `prompt.rs` from plan 007; soul handling in
  `storage/stable.rs`.
- Wizard: `apps/web/src/components/spawn/SpawnWizard.tsx` and
  `apps/web/src/components/spawn/steps/` (FundStep, ProviderConfigStep,
  RiskStep, SkillsStep, StrategiesStep).
- Indexer: `apps/indexer/src/normalize/` and `routes/automatons.ts` for
  surfacing registry fields.

## Tasks

1. **Contract**: add `name` and `constitution` to the spawn bootstrap types in
   `crates/spawn-protocol`, with a shared validation function (bounds +
   screening) used by both canisters. Bump whatever contract-version marker
   the crate exposes; update `factory.did` and the child's `ic-automaton.did`.
2. **Factory**: accept the fields at session creation (`api/public.rs`),
   persist through the session FSM, pass into child init (`init.rs`), store
   `constitution_hash` + `name` in the registry, expose both in registry
   queries.
3. **Child**: persist name + constitution once at bootstrap; render the
   Genesis document in prompt assembly (replacing the plan-007 placeholder);
   keep `soul` as the stable machine identifier distinct from `name`.
4. **Wizard**: add the authoring step as the *first* step of the flow — name +
   constitution text with client-side validation mirroring the shared rules,
   and 2–3 example constitutions as placeholders (write these carefully; they
   are the de-facto template every early progenitor copies — specific
   characters with wants, not "you are a helpful autonomous agent").
   Update touched copy from configure-language to genesis-language.
5. **Indexer/UI surfacing**: normalize name + constitution hash into the
   indexer store; show name in the canvas/drawer; show the constitution text
   in the drawer (served by the child's certified HTTP endpoint if available,
   else via indexer proxy) labeled as its founding document.
6. **Tests**: shared-crate validation tests (bounds, screening, round-trip);
   factory session + registry tests; child bootstrap test asserting the
   Genesis document renders the stored constitution and that no code path can
   overwrite it post-bootstrap; wizard component test for the new step.

## Verification gates

| Purpose | Command | Expected |
|---|---|---|
| Contract parity | `npm run verify:child-contract && npm run test:child-contract` | both canisters agree on the new fields |
| Rust tests | `cargo test --workspace` | pass |
| Web tests | `npm run test --workspace @ic-automaton/web` | pass, incl. new wizard step |
| Lint | `npm run lint` | clean |
| End-to-end | `npm run playground:spawn-payment-e2e` | spawn with a constitution succeeds; registry shows name + hash |
| Live check | `npm run playground:smoke` | spawned being's turns reflect the Genesis document (inspect assembled prompt/journal output) |

## STOP conditions

- STOP if adding fields to the spawn contract would break in-flight sessions
  or already-registered children in a way the compatibility gate cannot
  express — design the migration (defaults for legacy children: synthetic
  constitution from their existing soul/config) and surface it before
  proceeding.
- STOP if constitution content would transit any public factory/indexer
  response that plan 001's secret-redaction work classifies as sensitive
  surface — constitutions are public by design, but confirm nothing else
  rides along.
- STOP rather than weaken validation if the screening rules conflict with a
  legitimate constitution the operator supplies.

## Out of scope

- Seeded credo journal entries (plan 009, which depends on the journal).
- Constitution mutation, lineage, reproduction (plan 013).
- Authoring-assistance UX beyond static examples (spec open question 2).
- Renaming the product (spec section 7).
