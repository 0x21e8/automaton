# Plan 010: Metabolism and patronage surfaces

> **Executor instructions**: Read `specs/DESIGN_PRINCIPLES.md` completely —
> this plan implements P2 (legible metabolism), the spectator side of P4
> (patronage, paid messages), P8 stage 1 (sovereignty truth-labeling), and the
> indexer-sourced-facts rule from P4's critical notes / failure mode 9. Run
> every gate, stop on any STOP condition, update row 010 in `plans/README.md`.
>
> **Drift check (run first)**: written at destination commit `0ddd877`. No
> hard dependency on 007–009 — this plan may run in parallel — but if plan 009
> has landed, the drawer will already read journals; coordinate on drawer
> layout rather than conflicting.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED (real-money UX on Base; reuses existing payment plumbing)
- **Depends on**: — (parallel-safe; soft coordination with 009 on drawer UI)
- **Category**: product-direction
- **Planned at**: destination commit `0ddd877`, 2026-07-13

## Why this matters

Mortality only creates stakes if it is visible (P2), and spectators only
matter if they can act (P4). The primitives exist and are unsurfaced: the
child exposes wallet/cycle telemetry; `Inbox.sol`
(`components/ic-automaton/evm/src/Inbox.sol`) supports paid messages with
per-being minimum prices (`setMinPrices`, `queueMessage`, `queueMessageEth`);
the web app already has wallet connectivity from the spawn payment flow. This
plan turns them into the core spectator loop: **attention → payment →
survival**.

## Design contract (do not re-litigate)

- **Metabolic facts come from the indexer, never from the being's own
  framing** (failure mode 9). The UI renders indexer-computed numbers; being
  text (journal, replies) is display-only narrative next to them.
- **The four primary facts** on every being, canvas card and drawer alike:
  burn rate, runway at current cadence, lifetime earnings, age. These outrank
  every other stat visually (P2: "primary facts, not buried telemetry").
- **Runway is one canonical number** (P9): computed by one indexer function
  from cycles balance + observed burn + USDC-convertible reserves, with the
  formula documented in code comments and reused everywhere (including the
  evaluator later). Do not compute variant runways per view.
- **Patronage = value transfer the being metabolizes**, two forms in v1:
  (a) a paid message via `Inbox.sol` (existing contract, existing child-side
  polling), and (b) a direct USDC transfer to the being's address labeled as
  patronage in the UI (the `cycle_topup` pipeline already converts USDC to
  cycles). No new contracts in this plan.
- **Sovereignty truth-label** (P8): each being displays its actual control
  status derived from on-chain controller data — v1 label: "upgradeable by
  the factory". Never claim stronger than the chain shows. The factory (not
  the spawner) must be the only controller; if any registered child still
  lists a spawner controller, that is a finding to surface, not to hide.
- **Death is rendered** (P2): a being at zero is shown as dead — permanent,
  with its record intact — not filtered out of the canvas.

## Current state (verified anchors)

- Child telemetry: certified endpoints in `components/ic-automaton/src/http.rs`
  (`/api/snapshot`, `/api/wallet`); cycle/balance internals in
  `features/cycle_topup/mod.rs` and `domain/cycle_admission.rs`.
- Controller state: `backend/factory/src/controllers.rs` (handoff) and the
  registry in `state.rs` / `api/public.rs`.
- Indexer: polling in `apps/indexer/src/polling/`, normalization in
  `normalize/`, store in `store/`, REST in `routes/automatons.ts`, realtime in
  `ws/`.
- Web: canvas `apps/web/src/components/grid/AutomatonCanvas.tsx`, drawer
  `apps/web/src/components/drawer/AutomatonDrawer.tsx`, wallet helpers in
  `apps/web/src/lib/` (spawn payment logic), messaging affordances in
  `apps/web/src/components/drawer/` (DrawerMessaging tests exist).
- Inbox contract + deploy script: `evm/` in the component;
  root script `npm run evm:deploy-automaton-inbox`.

## Tasks

1. **Indexer metabolism model**: poll child telemetry per registered being;
   compute burn rate (windowed), canonical runway, lifetime earnings (inbox
   payments + strategy income as available from snapshot data), and age;
   persist history (bounded) for sparklines; expose via REST and websocket.
2. **Controller/sovereignty data**: surface each child's controller list into
   the registry/indexer (factory can attest its own controllership; verify
   spawner absence) and expose a `control_status` field.
3. **Canvas**: render the four primary facts per being; visual encoding of
   metabolic state (healthy / hibernating / dying / dead) consistent with the
   design tokens; dead beings persist with a terminal marker.
4. **Drawer**: metabolism panel (facts + history sparkline), sovereignty
   truth-label with the controller list behind it, and the price-of-attention
   (read `minPricesFor` from `Inbox.sol`).
5. **Patronage flows**: from the drawer, (a) send a paid message — compose,
   quote the being's min price, approve/transfer USDC or ETH via
   `queueMessage`/`queueMessageEth`, reusing the spawn flow's wallet plumbing;
   (b) direct patronage transfer to the being's address with explicit copy
   that this is a gift the being metabolizes, not a purchase of anything (P6
   language discipline: no yield framing anywhere in this UI).
6. **Being-side lever**: confirm the child runtime exposes/uses `setMinPrices`
   as its own doctrine-level decision (tooling exists in the component's EVM
   feature set — if no tool wraps it, add one) so the price shown is genuinely
   the being's.
7. **Tests**: indexer unit tests for burn/runway math (fixed fixtures);
   route/websocket tests; web component tests for metabolism panel, truth
   label, and both patronage flows (mocked chain); one playground E2E: paid
   message lands, being's earnings tick up, runway extends after top-up.

## Verification gates

| Purpose | Command | Expected |
|---|---|---|
| Web tests | `npm run test --workspace @ic-automaton/web` | pass |
| Indexer tests | `npm test` (indexer workspace) | burn/runway fixtures pass |
| Lint | `npm run lint` | clean |
| EVM tests | `npm run evm:test` | Inbox interactions pass |
| E2E payment | `npm run playground:spawn-payment-e2e` | unchanged, still green |
| Live loop | `npm run playground:smoke` + manual: send paid message via UI | message reaches the being; metabolism panel updates |

## STOP conditions

- STOP if any registered child in a non-playground environment lists a
  spawner as controller — report it; removing controllers on live canisters
  is an operator decision.
- STOP if computing earnings requires new child endpoints that would expose
  data plan 001 classified as secret-adjacent — design the endpoint, surface
  for review.
- STOP on any UI copy that you cannot write without promising value or
  returns to the patron — escalate the wording rather than shipping it.

## Out of scope

- Hibernation cadence changes and the terminal turn (plan 011).
- Said-vs-paid anchoring for inter-being claims and the payment graph
  (plan 012 — but keep the metabolism store schema amenable to it).
- Prediction/betting features of any kind (spec: rejected).
- Product rename (spec section 7).
