# Reusable Prompt: Implement Strategy Repository and Spawn Selection

Use this prompt with Codex or another coding agent when implementing part or all of the strategy repository work.

## How to use

1. Copy the prompt below.
2. Replace the placeholders:
   - `{{TASK_IDS}}`
   - `{{SCOPE_NOTE}}`
   - `{{VALIDATION_SCOPE}}`
3. If you want the full feature, set:
   - `{{TASK_IDS}} = STRAT-01 through STRAT-12, plus IA-STRAT-01 as an external dependency`
   - `{{SCOPE_NOTE}} = implement the full checklist`
   - `{{VALIDATION_SCOPE}} = run all relevant factory/shared/indexer/web tests you touch`

## Prompt

```md
Implement `{{TASK_IDS}}` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Context:
- Repo: `/Users/domwoe/Dev/projects/automaton-launchpad`
- This work adds a factory-hosted strategy repository seeded from copied `ic-automaton` strategy recipes, wires repository-backed strategy selection into the spawn flow, and installs selected strategies into spawned children as real executable templates.
- `{{SCOPE_NOTE}}`

Read first:
- [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md)
- [SPEC-FACTORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/SPEC-FACTORY.md)
- launchpad spawn strategy UI placeholder catalog in [spawn-state.ts](/Users/domwoe/Dev/projects/automaton-launchpad/apps/web/src/components/spawn/spawn-state.ts)
- sibling reference model in:
  - `/Users/domwoe/Dev/projects/ic-automaton/src/lib.rs`
  - `/Users/domwoe/Dev/projects/ic-automaton/src/domain/types.rs`
  - `/Users/domwoe/Dev/projects/ic-automaton/docs/strategies/`
  - `/Users/domwoe/Dev/projects/ic-automaton/justfile`

Non-negotiable implementation contract:
- The factory is the long-term source of truth for strategy selection in launchpad spawns.
- The repository stores full copied strategy recipes, not external references.
- Initial seed strategies are:
  - `base-aave-usdc-reserve-01`
  - `base-moonwell-usdc-reserve-01`
  - `base-usdc-carry-cbbtc-01`
- Skills are out of scope for this slice.
- Use the existing paid public spawn-session path; do not add a payment-bypass spawn helper.
- `create-spawn-session` must accept repository strategy IDs in the existing config shape.
- Selected strategies must be snapshotted immutably into the session at creation time.
- Spawn retries must use the snapped artifacts, not current repository state.
- The child must receive fully installed executable strategies, not just bootstrap labels.
- Any strategy install failure must fail the spawn atomically.
- Repository lifecycle must support admin add, deprecate, and revoke later.
- Compatibility must be validated by semantic chain family.
- Canonical Base strategies must work for launchpad `base` and for local Base-fork playground chains.
- The UI must present concrete deployable templates, not mock category labels.
- Admin ingestion must accept the same raw strategy recipe JSON format used by `ic-automaton`.

Execution requirements:
- Start by mapping the requested task IDs to concrete files and dependencies before editing.
- Keep changes tightly scoped to `{{TASK_IDS}}`.
- Reuse existing project patterns and naming unless the checklist explicitly requires a new contract.
- Update contracts consistently across factory, shared, indexer, and web where needed.
- If a contract change affects generated or checked-in source-side artifacts, update them too.
- Treat the sibling `ic-automaton` repo as an interface dependency, not a place to make unplanned design changes.
- If a sibling dependency is required but out of scope here, call it out explicitly and do not fake completeness.

Implementation guidance:
- Replace the current mock strategy selection path with repository-backed concrete templates.
- Distinguish clearly between:
  - live repository records
  - immutable session strategy snapshots
  - child-observed installed templates
- Preserve canonical copied recipe provenance while allowing Base-family local install adaptation where needed.
- Put validation in the factory source of truth, not only in the client.
- Ensure controller handoff happens only after selected strategies are installed and verified in the child.
- Add or update end-to-end tests where the checklist requires proof across boundaries.

Expected deliverables:
- Code changes implementing `{{TASK_IDS}}`
- Updated tests for all touched layers
- Any new seed files or manifests required by the checklist
- A brief implementation note appended to the checklist’s “Implementation Notes / Decisions Log” if you needed to lock a meaningful decision or deviation

Validation:
- `{{VALIDATION_SCOPE}}`
- Also run the narrowest relevant tests for every touched package or crate.
- If any required test cannot be run, say exactly why.

Output format:
- First provide the findings or blockers, if any.
- Then summarize what changed.
- Then list validation run and results.
- Include file references for the most important changed files.

Do not:
- Do not add a separate strategy repository canister.
- Do not add real skill repository seeding.
- Do not preserve the hardcoded mock strategy labels as the primary selection model.
- Do not introduce partial-success spawn behavior.
- Do not silently weaken chain-compatibility checks.
- Do not rely on live runtime reads from the sibling repo for seed loading.
```

## Suggested prompt variants

### Small slice

Use this for one phase at a time:

```md
Implement `STRAT-01` through `STRAT-03` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Scope note: implement only the contracts and seed-assets foundation. Do not start child installation or web UI work.

Validation scope: run the relevant `factory` and `@ic-automaton/shared` tests for changed files.
```

### Spawn-core slice

Use this for the high-risk backend path:

```md
Implement `STRAT-04` through `STRAT-09` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Scope note: implement the factory repository, chain-family validation, session snapshotting, child installation, and retry-safe failure semantics. Do not start the web replacement work unless required by a contract change.

Validation scope: run all relevant `factory` tests and any integration coverage needed to prove selected strategies are installed into spawned children.
```

### UI and indexer slice

Use this after backend contracts are stable:

```md
Implement `STRAT-10` and `STRAT-11` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Scope note: replace the mock strategy catalog with repository-backed concrete templates and expose the repository/session-selected state through indexer and web reads.

Validation scope: run all relevant `@ic-automaton/indexer` and `@ic-automaton/web` tests for changed files.
```
