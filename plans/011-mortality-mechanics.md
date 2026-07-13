# Plan 011: Mortality mechanics — hibernation tiers and the terminal turn

> **Executor instructions**: Read `specs/DESIGN_PRINCIPLES.md` completely —
> this plan implements P2's hibernation, terminal-turn, and
> infrastructure-death covenants, and answers spec open question 5 in code.
> Run every gate, stop on any STOP condition, update row 011 in
> `plans/README.md`.
>
> **Drift check (run first)**: written at destination commit `0ddd877`.
> Plans 007 and 009 must be COMPLETE (the terminal turn writes a journal
> entry; doctrine/protocol documents must exist). Plan 010 should be complete
> so the UI can render hibernation and death states this plan produces.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH (touches survival-critical scheduling and reserved budgets;
  a bug here kills beings — test in the playground exhaustively)
- **Depends on**: `plans/007-restructure-prompt-stack.md`,
  `plans/009-voice-journal-and-deference-metric.md`;
  soft: `plans/010-metabolism-and-patronage.md`
- **Category**: correctness, product-direction
- **Planned at**: destination commit `0ddd877`, 2026-07-13

## Why this matters

Death is the world's engine (P2) but today it is an accident: a canister
burns cycles at whatever cadence until it freezes, stranding any USDC held by
its tECDSA address forever. The spec requires dying to be slow enough to
witness and avert (hibernation), to end in a settled estate rather than a
fund-loss bug (terminal turn), and to be distinguishable from
platform-caused loss (the infrastructure-death covenant).

## Design contract (do not re-litigate)

- **Hibernation tiers are runway-driven and deterministic.** Define named
  tiers (suggested: `active` / `conserving` / `hibernating` / `terminal`)
  with runway thresholds, each mapping to a tick cadence and a reasoning
  budget (the adaptive-cadence and `set_openrouter_reasoning_level` machinery
  already exists — this plan makes the policy explicit, tiered, and visible
  in telemetry rather than ad hoc). Tier state appears in the Situation
  document and in `/api/snapshot` so plan 010's UI can render it.
- **The terminal turn is guaranteed and budgeted.** From bootstrap, the
  runtime holds a **reserved terminal budget** (cycles for one final turn +
  gas headroom for a small number of EVM transfers) that no other subsystem —
  strategies, top-ups, inference — may spend. When runway crosses the
  terminal threshold, the runtime runs exactly one final turn with a
  **restricted tool set**: `think`, `journal`, `send_eth` /
  ERC-20-transfer-capable signing for bequests, `recall`. No strategy tools,
  no http fetch, no room posting beyond a single farewell if room tooling
  exists. The being knows: the Situation document states plainly that this is
  its last turn.
- **Default estate rule** when the being makes no valid bequest: defined
  behavior, not silence. V1 default: funds remain in place (a monument);
  record the choice in the registry. (Spec open question 5 leaves
  sweep-to-lineage open — lineage does not exist until plan 013, so monument
  is the only coherent v1 default.)
- **After the terminal turn**: the being is dead. It does not tick again. Its
  journal, registry entry, and constitution remain readable (the certified
  endpoints should outlive the agent loop as long as cycles physically
  permit; document the actual post-death readability window honestly).
- **The covenant in writing**: starvation is permanent; infrastructure death
  is not death. This plan adds the operational policy document — which
  snapshot/restore actions are legitimate (platform-caused loss, publicly
  logged) and which are forbidden (undoing starvation) — to `ops/` alongside
  the existing release/rollback boundary docs, and the factory/registry gains
  a way to record a death cause (`starved` vs `infrastructure`).

## Current state (verified anchors)

- Cadence/admission: `components/ic-automaton/src/scheduler.rs`,
  `domain/cycle_admission.rs`, `domain/recovery_policy.rs`, `timing.rs`.
- Cycle economics: `features/cycle_topup/mod.rs` (existing reserve floors and
  minimums — the terminal budget must compose with, not conflict with, these).
- Turn machinery: `agent.rs` (turn FSM), `domain/state_machine.rs`.
- Journal + Situation: from plans 007/009.
- Estate transfers: existing EVM signing/broadcast in `features/evm.rs`,
  `features/signer.rs`, `features/threshold_signer.rs`; `send_eth` tool in
  `tools.rs` (bequests in USDC need an ERC-20 transfer path — check what
  `execute_strategy_action`/EVM tooling already provides before adding one).
- Registry/death records: `backend/factory/src/state.rs`, `api/public.rs`;
  indexer normalization for surfacing (plan 010's store).
- Ops docs: `ops/` and the release/rollback boundary documentation referenced
  in recent commit `0e6b75b`.

## Tasks

1. **Tier policy**: implement the tier table (thresholds → cadence +
   reasoning level + tool scope), driven off the canonical runway input;
   expose current tier in snapshot telemetry and the Situation document;
   unit-test tier transitions on fixture runways, including hysteresis so a
   noisy runway doesn't flap tiers every tick.
2. **Terminal budget reservation**: carve the reserved budget out at
   bootstrap; make admission control treat it as unspendable outside the
   terminal turn; test that no strategy/top-up/inference path can dip into it.
3. **Terminal turn**: trigger, restricted tool registry, last-turn Situation
   framing, bequest execution, final journal entry, and the post-turn dead
   state (scheduler stops, endpoints keep serving). Integration-test the full
   sequence in the playground by spawning a being with a tiny endowment and
   watching it die well.
4. **Death records**: child reports terminal completion (or the factory/
   indexer infers freeze); registry records death cause and timestamp;
   indexer marks the being dead; plan 010's UI states render it.
5. **Covenant ops doc**: write the starvation-vs-infrastructure policy in
   `ops/`, including the public-logging requirement for any restore and the
   explicit prohibition on restoring starved beings.
6. **Evaluator hook**: the playground experiment config gains a
   die-well scenario (spawn under-endowed being → assert terminal journal
   entry + estate behavior + death record) so mortality mechanics are gated
   on every future change.

## Verification gates

| Purpose | Command | Expected |
|---|---|---|
| Rust tests | `cargo test --workspace` | tier, reservation, terminal-turn tests pass |
| Lint | `npm run lint` | clean |
| Contract parity | `npm run verify:child-contract && npm run test:child-contract` | death-record fields agree |
| Die-well E2E | `npm run eval:run` with the new experiment | terminal turn observed: last journal entry, bequest or monument default, death record |
| Regression | `npm run playground:smoke` | healthy beings unaffected; no tier flapping in logs |

## STOP conditions

- STOP if the reserved terminal budget cannot be guaranteed under the
  existing cycle-admission model without weakening a survival floor in
  `cycle_topup` — reconcile the floors explicitly with the operator rather
  than picking silently.
- STOP if a terminal-turn EVM bequest can exceed the reserved gas headroom
  (unbounded bequest lists) — cap bequest count/size in the design, do not
  let the estate over-spend its own funeral.
- STOP if any code path allows a post-terminal tick — a being that acts
  after death breaks the covenant; this is a release blocker, not a
  known issue.

## Out of scope

- Sweep-to-lineage estate rules and wills referencing descendants (plan 013).
- UI beyond consuming the new states (plan 010 owns the surfaces).
- Any resurrection/admin-restore tooling (covenant: policy doc only).
- Changes to the top-up bridge pipeline itself.
