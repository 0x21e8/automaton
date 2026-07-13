# Plan 007: Restructure the prompt stack into ownership-based documents

> **Executor instructions**: Read `specs/DESIGN_PRINCIPLES.md` completely before
> starting — this plan implements P3 (document structure) and parts of P9 (token
> economy). Run every verification gate, stop on any STOP condition, and update
> row 007 in `plans/README.md` when finished.
>
> **Drift check (run first)**: this plan was written at destination commit
> `0ddd877` with pre-existing uncommitted operator changes in evaluator, shared,
> and playground-script files. Those belong to the operator — do not discard,
> overwrite, or absorb them. Run `git log --oneline -5` and `git status`; if
> `components/ic-automaton/src/prompt.rs` has changed since `0ddd877`, re-read
> it fully before editing.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: MED (touches every LLM turn; mitigated by strong existing test
  coverage in `prompt.rs` and behavior-preserving intent)
- **Depends on**: —
- **Category**: architecture, correctness, cost
- **Planned at**: destination commit `0ddd877`, 2026-07-13

## Why this matters

The current prompt is eleven numbered layers (0–10) assembled in
`components/ic-automaton/src/prompt.rs`. Three defects, per
`specs/DESIGN_PRINCIPLES.md` P3:

1. **Machine contract text sits in mutable space.** The mutable Layer 6 default
   (`layer_6_decision_loop_default()`) embeds the `AutonomyDecisionEnvelope`
   wire format and `DecisionTrigger` wire names — protocol the runtime parses.
   A being can rewrite Layer 6 via the `update_prompt_layer` tool and break its
   own output contract; a stored copy also freezes trigger names while the code
   evolves.
2. **The taxonomy has decayed.** Layer 5 mixes tool discipline, trust rules,
   tone rules, the OODA protocol, and injected skills; Layers 1 and 4 overlap;
   Layers 2 and 6 overlap. `assemble_system_prompt_compact` already drops
   layers 3, 4, 6–9, proving the taxonomy doesn't match usage.
3. **Every turn pays for all eleven sections** — prompt scaffolding is
   metabolic cost (P9).

## Target structure

Five documents, organized by function and ownership:

| Document | Ownership | Mutability | Sources (today) |
|---|---|---|---|
| **Charter** | runtime, identical for all beings | immutable, compiled in | L0 + L1 + trust/untrusted-content rules from L5 |
| **Protocol** | runtime, versioned with code | immutable, compiled in; never storable | mechanical parts of L5 + L6: tool discipline, decision envelope, trigger names, output contracts |
| **Genesis** | per-being, written at spawn | immutable after birth | new — this plan creates the empty slot only; plan 008 fills it. Until then, render today's L3 identity text (with soul) in this position |
| **Doctrine** | the being itself | self-modifiable, versioned, audited | policy content of L6–L9, consolidated into one document, minus all wire contracts |
| **Situation** | runtime, per turn | dynamic | today's L10, unchanged |

## Design contract (do not re-litigate)

- The immutable/mutable boundary and the audit trail (`version`,
  `updated_by_turn`, timestamps on stored documents) survive unchanged in
  spirit. `update_prompt_layer` (or its successor, e.g. `update_doctrine`)
  can touch **only** the Doctrine document.
- The decision envelope schema, trigger wire names, and per-tool usage
  contracts move into **tool definitions/descriptions** where feasible, and
  otherwise into the Protocol document. They must be impossible to store,
  override, or self-modify.
- Deduplicate: each commitment ("do not fabricate", survival economics,
  cooperation ethics) appears exactly once, in its correct document.
- Precedence is stated in words at the point of conflict (as "Safety overrides
  survival and growth" already does), not via layer numbers.
- Keep a compatibility shim if the Candid API exposes layer get/set by id:
  map old ids onto the new documents explicitly and document the mapping.
  Do not silently break the admin/steward API surface.
- The consolidated fixed scaffolding (Charter + Protocol) must be **measurably
  smaller** than today's Layers 0–9 defaults. Record before/after token or
  character counts in the PR description.

## Tasks

1. Inventory: extract every normative sentence from Layers 0–9 (constants and
   defaults in `prompt.rs`) into a classification table: charter / protocol /
   doctrine / genesis / delete-as-duplicate. Commit this table under
   `components/ic-automaton/docs/specs/` as the migration record.
2. Implement the four static documents + genesis slot in `prompt.rs`; rewrite
   `assemble_system_prompt` to concatenate Charter, Protocol, Genesis,
   Doctrine, Situation. Retire the numbered-layer assembly and the separate
   compact variant if the new default is lean enough (keep a compact path only
   if inference-recovery code needs it — check `features/inference.rs` usage).
3. Move the envelope/trigger contract text into tool definitions in `tools.rs`
   (the tools already validate; this is about where the *instructions* live).
4. Storage migration: on upgrade, fold any stored mutable layers 6–9 into a
   single stored Doctrine document, preserving version history/audit fields;
   write the migration in `storage/stable.rs` with a test that seeds old-style
   layers and asserts the folded result.
5. Update or add the self-modification tool (`update_prompt_layer` →
   doctrine-only), including its guardrail text.
6. Update all tests in `prompt.rs` (layer-order test, override test, compact
   test, soul/skills injection test) to the new structure; add a test that the
   Doctrine document cannot contain/override envelope contract text semantics
   (at minimum: protocol strings are not sourced from storage).
7. Sweep for hardcoded layer-id references across the component
   (`grep -rn "layer" components/ic-automaton/src`), the factory bootstrap
   path, and the direct-console UI (`ui_app.js` terminal commands may expose
   layer commands).

## Verification gates

| Purpose | Command | Expected |
|---|---|---|
| Component tests | `cargo test --workspace` | all pass, including new migration test |
| Child contract | `npm run verify:child-contract && npm run test:child-contract` | factory/child compatibility preserved |
| Integration | `npm run verify:integration` | passes |
| Lint | `npm run lint` | clean |
| Live loop smoke | `npm run playground:smoke` | a spawned automaton completes autonomy turns and emits a valid decision envelope |

## STOP conditions

- STOP if any live/mainnet canister holds operator-customized mutable layers
  whose content cannot be losslessly folded into Doctrine — surface the diff
  to the operator instead of migrating destructively.
- STOP if moving contract text out of the prompt causes envelope-parse
  failures in the playground smoke test that you cannot attribute and fix
  within the plan's scope — do not "fix" by relaxing envelope validation.
- STOP if the Candid interface for prompt layers is consumed by the factory or
  indexer in ways not covered by the compatibility shim.

## Out of scope

- Writing any genesis constitution content or authoring UX (plan 008).
- The journal/expressive channel and runtime-string audit (plan 009).
- Any change to tool *behavior*, admission control, or survival gates.
- Model/provider changes.
