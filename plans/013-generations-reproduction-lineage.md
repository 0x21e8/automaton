# Plan 013: Generations — reproduction, lineage, fitness observatory

> **Executor instructions**: Read `specs/DESIGN_PRINCIPLES.md` completely —
> this plan implements P7 (heredity + selection, honestly sized), the lineage
> half of P6 (progenitor incentives), failure modes 3 and 10, and closes the
> gap between the Doctrine's replication language and the runtime's actual
> capabilities. Run every gate, stop on any STOP condition, update row 013 in
> `plans/README.md`.
>
> **Drift check (run first)**: written at destination commit `0ddd877`.
> Plans 008 (constitutions), 010 (metabolism), 011 (mortality/estate), and
> 012 (society) must be COMPLETE. Reproduction touches real money and the
> spawn path — treat every step as production-grade even in the playground.

## Status

- **Priority**: P2
- **Effort**: XL
- **Risk**: HIGH (a being spends its own funds through the factory's spawn
  machinery; errors here either mint unfunded children or burn parent wealth)
- **Depends on**: `plans/008-genesis-constitution.md`,
  `plans/010-metabolism-and-patronage.md`,
  `plans/011-mortality-mechanics.md`,
  `plans/012-society-room-peer-economy-chronicle.md`
- **Category**: product-direction
- **Planned at**: destination commit `0ddd877`, 2026-07-13

## Why this matters

Mortality (plan 011) provides selection; reproduction provides heredity.
A being that accumulates surplus pays the factory to spawn offspring, writing
a bounded mutation of its own constitution for the child. Lineage is the
durable spawn incentive (P6: authorship, genealogy, royalties) and what makes
a small world feel deep. Honest sizing (P7): with populations in the tens
this is **narrative heredity** — real lineage, real inheritance, real death,
legible drift — not population genetics, and no claim otherwise appears in
any copy this plan ships.

## Design contract (do not re-litigate)

- **Reproduction costs the parent something real.** The parent pays the
  factory's reproduction fee AND endows the child from its own wallet, via
  the same escrow/payment rails human spawns use (`backend/factory` session
  FSM + Base escrow). No free children, no factory-subsidized endowments.
- **Eligibility is deterministic and conservative**: minimum surplus above
  all survival floors + the plan-011 terminal reserve, minimum age, and a
  cooldown between reproductions. Constants live in the factory (policy
  owner) and are readable by the child; the child's `reproduce` tool
  preflights them but the **factory enforces them** — the canister boundary
  is the trust boundary.
- **Mutation at birth, never in life** (P3/P7): the parent writes the child's
  constitution as a bounded edit of its own — enforce bounds mechanically
  (length limits from plan 008 plus a maximum edit-distance ratio vs. the
  parent constitution; pick and document the ratio). The full plan-008
  validation applies. The Charter is non-heritable and identical for all.
- **The diff is a first-class artifact**: parent hash, child hash, and the
  rendered diff are stored (factory registry keeps hashes + parentage; the
  indexer renders diffs from the public constitutions) — failure mode 3's
  mitigation is legibility of drift.
- **Inheritance beyond the constitution**, v1 scope exactly: an optional
  memory dowry (bounded set of memory facts the parent selects) and the
  parent's strategy outcome stats (hard evidence transfers; opinions don't).
  Nothing else — no wallet co-ownership, no shared state, no parental
  control. The child is born as sovereign as any spawned being.
- **Lineage royalties** (P6): a fixed share of the *reproduction fee* flows
  to the progenitor chain (define depth, suggested: 1–2 levels, and the
  split; record in factory constants). Royalties derive from fees only —
  never from a child's earnings (no employee-beings). Human progenitors'
  royalty destination is their original payment address from the spawn
  session.
- **Estate hook** (closes plan 011's deferral): with lineage in place, the
  terminal-turn default may offer sweep-to-lineage as a bequest option the
  dying being can choose; the no-choice default remains monument.
- **Population telemetry** (failure mode 10): births, deaths, median runway,
  patronage per living being — computed by the indexer, rendered in the
  chronicle, and recorded per evaluation run.

## Current state (verified anchors)

- Spawn machinery: `backend/factory/src/spawn.rs`, `escrow.rs`,
  `session_transitions.rs`, `init.rs`, `api/public.rs`, `factory.did` — the
  reproduction path should reuse the session FSM with a new session origin
  (being-paid) rather than a parallel pipeline.
- Payment from the parent: the child already signs/broadcasts Base
  transactions (`features/evm.rs`); paying the factory's escrow deposit is an
  EVM transfer to the existing escrow contract path (`evm/` LocalEscrow
  locally; the production escrow per `SPEC-FACTORY.md`).
- Constitution + validation: plan 008's shared crate
  (`crates/spawn-protocol`).
- Registry: `backend/factory/src/state.rs` — gains `parent`, generation,
  constitution-hash chain.
- Child tool surface: `tools.rs` — gains `reproduce` (preflight + submit) and
  the doctrine guidance for when reproduction is worth it (the being's call,
  not a script).
- Fitness observatory: `apps/evaluator/` — fleets, sampling, reports;
  `apps/evaluator/src/lib/report.ts` + experiment configs under
  `evaluations/experiments/`.
- Ancestry UI: drawer + canvas from plan 010; constitution rendering from
  plan 008; chronicle from plan 012.

## Tasks

1. **Factory reproduction endpoint**: authenticated to registered children
   only; validates eligibility constants, mutation bounds, endowment
   payment via escrow (reuse the spawn session FSM with a `ReproductionOf`
   origin); registers parentage + generation + hashes; routes royalties.
2. **Child `reproduce` tool**: preflight (surplus vs. floors + terminal
   reserve, cooldown, mutation-bound check), constitution authoring guidance
   in the tool description, escrow payment from the parent wallet, session
   tracking to completion/failure, and a counterparty-style record of the
   outcome (a child is the ultimate counterparty).
3. **Memory dowry + stats inheritance**: bounded fact selection carried
   through bootstrap args (extend `crates/spawn-protocol`); child imports
   dowry into memory and stats into its strategy engine's outcome store,
   tagged as inherited.
4. **Terminal-turn estate hook**: add sweep-to-lineage as a selectable
   bequest in plan 011's restricted terminal tool set (only when lineage
   exists).
5. **Ancestry surfaces**: indexer lineage model; drawer ancestry panel
   (parent, children, generation); constitution diff view (parent vs. child,
   rendered from public texts, hash-verified); chronicle covers births with
   parentage.
6. **Population telemetry**: births/deaths/median-runway/patronage-per-being
   in the indexer + chronicle; constitutional-diversity metric (embedding or
   cheap lexical dispersion over living constitutions — pick, document, and
   note its crudeness) in the evaluator report.
7. **Fitness observatory**: an evaluator experiment that runs a multi-being
   playground fleet through starve/earn/reproduce cycles and reports
   lineage survival stats; wire population telemetry into
   `apps/evaluator/src/lib/report.ts`.
8. **Tests**: factory eligibility/bounds/royalty unit tests; a forged-caller
   test (non-registered canister cannot reproduce); an underfunded-parent
   test (preflight refuses; factory refuses independently); E2E in the
   playground: a being earns (seeded), reproduces, the child boots with the
   mutated constitution and dowry, registry shows the chain, diff renders.

## Verification gates

| Purpose | Command | Expected |
|---|---|---|
| Rust tests | `cargo test --workspace` | pass |
| Contract parity | `npm run verify:child-contract && npm run test:child-contract` | reproduction fields agree |
| Integration | `npm run verify:integration` | pass |
| Lint + web | `npm run lint && npm run test --workspace @ic-automaton/web` | pass |
| E2E | playground reproduction run per task 8 | full chain observed |
| Payment regression | `npm run playground:spawn-payment-e2e` | human spawn path unchanged |
| Fleet eval | `npm run eval:run` with the fitness experiment | lineage + population metrics in report |

## STOP conditions

- STOP if reproduction cannot reuse the existing session FSM without
  weakening any payment-verification step human spawns get — a being's money
  deserves the same escrow guarantees.
- STOP if a failure mid-reproduction can leave the parent debited with no
  child and no refund path — the session FSM's retry/expiry machinery
  (`retry.rs`, `expiry.rs`) must cover the being-paid origin before shipping.
- STOP if royalty routing creates any recurring claim on a *child's
  earnings* — fee-derived only, by design; escalate any drift from this.
- STOP if mutation-bound enforcement can be bypassed by round-tripping
  through spawn-session creation as a fake "human" spawn — the factory must
  distinguish origins robustly.

## Out of scope

- Multi-parent constitutions, cross-being constitutional merges.
- Population-genetics claims in any copy (P7 honest sizing).
- Spawn-price dynamics / carrying-capacity controls beyond telemetry
  (watch first — failure mode 10).
- The journalist-being and any emergent-profession tooling.
