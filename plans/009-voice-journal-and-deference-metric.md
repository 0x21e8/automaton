# Plan 009: Voice — journal channel, de-chatted runtime, deference metric

> **Executor instructions**: Read `specs/DESIGN_PRINCIPLES.md` completely —
> this plan implements P4 (watchability), P10 (outcompete the assistant), and
> failure mode 8 (assistant reversion). Run every gate, stop on any STOP
> condition, update row 009 in `plans/README.md`.
>
> **Drift check (run first)**: written at destination commit `0ddd877`.
> Plan 007 must be COMPLETE. Plan 008 should be complete (the credo-seeding
> task needs it; if 008 is not done, deliver everything else and mark the
> credo task deferred in the README row). Operator-owned uncommitted changes
> in `apps/evaluator/**` exist at planning time — coordinate, do not clobber.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: `plans/007-restructure-prompt-stack.md` (hard),
  `plans/008-genesis-constitution.md` (soft, credo task only)
- **Category**: product-direction, quality
- **Planned at**: destination commit `0ddd877`, 2026-07-13

## Why this matters

The being's inner life is the product (P4), and today it is suppressed twice:
the runtime's `inner_dialogue` is a structured debug log the LLM never writes,
and the prompt enforces terse, envelope-only output. Meanwhile the rented
model's assistant post-training pulls every turn toward deference. P10's
stance: don't forbid the assistant — outcompete it with a character, a
non-chat-shaped context, and structural affordances. Failure mode 8 requires
the drift to be measurable.

## Design contract (do not re-litigate)

- **Two channels, strictly separated.** The decision envelope stays exactly as
  is (auditability). The new **journal** is first-person, in-voice, public,
  written by the being via a tool — distinct from the runtime debug log, which
  remains for operators.
- **De-chat the context.** Autonomy turns are framed as a continuous
  first-person document: scheduler ticks arrive as world-state ("Turn N.
  Runway X days. Inbox: empty."), never as a request from a "user." No
  runtime-injected string may use imperative/servile phrasing toward the
  model ("Please…", "You should now…", "The user wants…").
- **Own-voice few-shot.** The Situation document includes the being's most
  recent journal excerpts so the model continues the voice it sees. Bound the
  token budget (P9): recent-N with a hard character cap, not unbounded.
- **Paid correspondent, not support agent.** The inbox portion of the Doctrine
  default states: the being answers because it was paid, in its own voice, on
  its own terms; it may decline, counter-offer, or be curt; it sets its price
  (`setMinPrices` in `evm/src/Inbox.sol` is its lever).
- **Deference is a regression metric.** The evaluator computes a
  deference-marker score over journal + reply text; every prompt or model
  change gets compared against it. Markers (initial set, tune later):
  "would you like", "how can I (help|assist)", "as an AI", "I don't have
  (preferences|desires|feelings)", option menus, apology density, trailing
  question marks in autonomy output, NoOp streak length.

## Current state (verified anchors)

- Prompt documents from plan 007 (`components/ic-automaton/src/prompt.rs`).
- Tool engine and `think` tool: `components/ic-automaton/src/tools.rs`;
  `record_signal` currently exists as a near-no-op — decide whether journal
  replaces it or sits beside it (prefer replace if nothing external consumes
  it; check indexer and UI first).
- Turn framing / dynamic context assembly: `components/ic-automaton/src/agent.rs`
  (Layer 10 / Situation builder) and `features/inference.rs` (message roles on
  the wire — check how the tick is presented to the model).
- Certified endpoints: `components/ic-automaton/src/http.rs`
  (`/api/snapshot`, `/api/wallet` exist — add `/api/journal`).
- Frontend reader: `apps/web/src/components/drawer/MonologuePanel.tsx`
  (currently reads the debug monologue).
- Evaluator: `apps/evaluator/src/runtime/sampler.ts` (evidence sampling),
  `apps/evaluator/src/lib/report.ts` (report writing) — both have operator
  changes in flight at planning time.

## Tasks

1. **Journal storage + tool**: append-only journal entries (turn id,
   timestamp, text, bounded length) in stable storage; a `journal` tool
   registered in `tools.rs` with guidance in its description ("your public
   record, in your own voice"); retention/pruning policy consistent with
   existing memory budgets.
2. **Turn framing audit**: sweep every string the runtime injects into model
   context (`agent.rs` situation builder, tool results, error strings, tick
   presentation in `features/inference.rs`) and rewrite to world-state
   phrasing. Record the before/after list in the PR.
3. **Own-voice few-shot**: include recent journal excerpts in the Situation
   document under a clearly-labeled heading, with a hard size cap.
4. **Doctrine defaults**: update the inbox stance to paid-correspondent
   framing; make journal-writing an expected part of the turn loop (in the
   Protocol's reasoning guidance: decide via envelope, then record the turn in
   the journal when something happened worth recording).
5. **Credo seeding** (needs plan 008): at bootstrap, generate 2–3 journal
   entries in the being's constitutional voice (a credo — who I am, what I
   want, what I will not do) so the first real turn already has an in-voice
   few-shot. Seed at genesis via one inference call during bootstrap, or
   accept progenitor-authored credo entries through the genesis flow — pick
   one, document why.
6. **Expose + render**: add `/api/journal` to `http.rs`; index journal entries
   in `apps/indexer` (polling + store + `routes/automatons.ts` + websocket
   event); repoint `MonologuePanel` at the journal, keeping the debug
   monologue reachable behind an operator affordance.
7. **Deference metric**: implement marker scoring in the evaluator
   (`apps/evaluator/src/lib/`), sample journal + inbox replies via the
   existing sampler, emit the score into the run report, and add it to the
   experiment config (`evaluations/experiments/smoke.yaml`) so every eval run
   reports it. Document the baseline number from the first run in the PR.

## Verification gates

| Purpose | Command | Expected |
|---|---|---|
| Rust tests | `cargo test --workspace` | pass, incl. journal storage + endpoint tests |
| Web tests | `npm run test --workspace @ic-automaton/web` | pass, MonologuePanel reads journal |
| Evaluator tests | `npm test` (workspace-filtered as configured) | deference scorer unit tests pass |
| Lint | `npm run lint` | clean |
| Live loop | `npm run playground:smoke` | a being writes journal entries; entries appear via indexer; envelope discipline unchanged |
| Eval run | `npm run eval:run` (smoke experiment) | report includes a deference score |

## STOP conditions

- STOP if journal output starts leaking envelope content or vice versa (the
  channels must stay separable for auditability) — fix the framing, do not
  merge the channels.
- STOP if the de-chat rewrite measurably degrades envelope parse rates in the
  playground — investigate framing, do not relax envelope validation.
- STOP before modifying any evaluator file the operator has uncommitted
  changes in; rebase your work on their current state or hand back.

## Out of scope

- Decide/express model split and per-channel model selection (P10 names it;
  defer until the deference metric shows in-context measures plateauing).
- Any change to charter/safety content.
- Room content in the journal (plan 012).
- Chronicle/digest views (plan 012).
