# Reusable Prompt: Implement Strategy Repository One Task At A Time

Use this prompt when you want an agent to execute the checklist incrementally, finish one task cleanly, validate it, and stop or commit before moving on.

## How to use

1. Copy the prompt below.
2. Replace:
   - `{{TASK_ID}}`
   - `{{COMMIT_MODE}}`
   - `{{VALIDATION_COMMANDS}}`
3. Run it once per checklist task.

Recommended values:
- `{{COMMIT_MODE}} = do not commit; stop after validation and summarize`
- or `{{COMMIT_MODE}} = create one commit for this task if validation passes`

## Prompt

```md
Implement exactly `{{TASK_ID}}` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Repo:
- `/Users/domwoe/Dev/projects/automaton-launchpad`

First step:
- Read the checklist entry for `{{TASK_ID}}` carefully.
- Read only the minimum additional files needed to execute that task correctly.
- State the files you plan to modify before editing.

Hard scope rule:
- Implement only `{{TASK_ID}}`.
- You may make the smallest necessary adjacent contract changes if they are strictly required to complete `{{TASK_ID}}`, but do not start later checklist tasks preemptively.
- If `{{TASK_ID}}` is blocked by an unfinished dependency from the checklist, stop and explain the blocker instead of leaking into later work.

Non-negotiable project contract:
- The factory is the long-term source of truth for strategy selection in launchpad spawns.
- The repository stores full copied strategy recipes.
- Initial seed strategies are:
  - `base-aave-usdc-reserve-01`
  - `base-moonwell-usdc-reserve-01`
  - `base-usdc-carry-cbbtc-01`
- Skills are out of scope for this slice.
- The existing paid public spawn-session path remains the automation path.
- `create-spawn-session` must accept repository strategy IDs in the existing config shape.
- Selected strategies are snapshotted immutably into the session at creation time.
- Retries must use snapped artifacts only.
- The child must receive fully installed executable strategies.
- Strategy install failure must fail the spawn atomically.
- Compatibility must be validated by semantic chain family.
- Base strategies must work for launchpad `base` and local Base-fork playground chains.
- The UI must present concrete deployable templates rather than mock categories.
- Admin ingestion must accept the same raw strategy recipe JSON format used by `ic-automaton`.

Execution rules:
- Use the checklist’s “Implement”, “Important detail”, “Done when”, and “Validation” bullets as the acceptance contract for this task.
- Preserve the distinction between:
  - live repository records
  - immutable session strategy snapshots
  - child-observed installed templates
- Do not silently broaden scope into skills, payment-bypass helpers, or unrelated cleanup.
- If you need to make a meaningful decision not already locked, append it to the checklist’s “Implementation Notes / Decisions Log”.

Validation rules:
- Run the narrowest relevant validation for `{{TASK_ID}}`.
- Required commands:
  - `{{VALIDATION_COMMANDS}}`
- If validation fails, fix it if the fix is still within `{{TASK_ID}}`; otherwise stop and explain exactly what remains.

Completion rules:
- When `{{TASK_ID}}` is complete, verify that its “Done when” conditions are actually satisfied.
- Update the checkbox for `{{TASK_ID}}` in [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md) only if the task is truly done.
- `{{COMMIT_MODE}}`

Output format:
- Findings/blockers first, if any.
- Then what changed for `{{TASK_ID}}`.
- Then validation run and results.
- Then whether the checklist box was updated.
- Include the most important changed file references.

Do not:
- Do not start the next checklist task automatically.
- Do not mix multiple checklist tasks into one change unless the checklist explicitly made them inseparable.
- Do not add speculative follow-on refactors.
- Do not claim `{{TASK_ID}}` is complete if its validation or “Done when” section is not met.
```

## Ready-to-use examples

### Example: foundation contract task

```md
Implement exactly `STRAT-01` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Commit mode: do not commit; stop after validation and summarize.

Validation commands:
- `npm run test --workspace @ic-automaton/shared`
```

### Example: seed asset task

```md
Implement exactly `STRAT-02` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Commit mode: create one commit for this task if validation passes.

Validation commands:
- run the narrowest file-level or package-level checks needed to prove the copied assets and manifest are consistent
```

### Example: high-risk spawn task

```md
Implement exactly `STRAT-08` from [CHECKLIST-STRATEGY-REPOSITORY.md](/Users/domwoe/Dev/projects/automaton-launchpad/specs/CHECKLIST-STRATEGY-REPOSITORY.md).

Commit mode: do not commit; stop after validation and summarize.

Validation commands:
- `cargo test -p factory`
- plus the narrowest integration proof needed to show selected snapped strategies are installed into the spawned child before controller handoff
```

## Suggested operating pattern

For disciplined execution, use this order:

1. Run the one-task prompt for `STRAT-01`.
2. Validate and optionally commit.
3. Run the one-task prompt for `STRAT-02`.
4. Continue in dependency order from the checklist.

This keeps each task checkable, reviewable, and reversible.
