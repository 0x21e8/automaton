# Plan 006: Converge the public, direct-console, and evaluator appearance

> **Executor instructions**: Preserve the three audience/trust boundaries while
> making them visibly one product. Do not replace them with one public bundle.
> Run every gate and stop on any STOP condition. Update row 006 in
> `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 09fdfe2..HEAD -- apps/web apps/evaluator-web packages components/ic-automaton/src scripts`
> Changes created by completed prerequisite plans are expected. Plan 002 and
> plan 003 must both be complete before continuing.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED
- **Depends on**: `plans/002-execute-steward-signed-commands.md`, `plans/003-import-runtime-monorepo-component.md`
- **Category**: direction, tech-debt
- **Planned at**: destination commit `09fdfe2`, source commit `eda66fd`, 2026-07-10

## Why this matters

The same product currently presents three independent visual identities: a
phosphor CRT direct console, a cream/red Lab, and a rounded IBM Plex evaluator
dashboard. The Lab and direct console also implement the same 17 terminal
commands separately. The goal is a canonical Automaton Lab brand with a dark
direct-console theme and a dense operator theme, all derived from shared tokens
and command metadata while remaining separate deployable/access surfaces.

## Current state after plan 003

- Public Lab shell and theme tokens:
  `apps/web/src/App.tsx:134-213` and `apps/web/src/theme/tokens.ts:1-45`.
- Direct console certified assets:
  `components/ic-automaton/src/http.rs:57-59`, `ui_index.html`, `ui_styles.css`,
  and `ui_app.js`.
- Evaluator operator controls and independent theme:
  `apps/evaluator-web/src/App.tsx:182-246` and `styles.css:1-17`.
- Lab command definitions:
  `apps/web/src/lib/cli-command-registry.ts:18-155`.
- Direct-console command switch:
  `components/ic-automaton/src/ui_app.js` near the `switch (cmd)` dispatcher.
- Selecting an automaton loads indexed detail, then mounting the command panel
  triggers six additional direct canister requests through
  `apps/web/src/hooks/useCommandSession.ts:56-81` and
  `apps/web/src/api/automaton.ts:108-120`.

Audience boundaries:

- **Public/Steward Lab**: fleet, room, spawn, automaton overview/activity,
  wallet interaction, and steward terminal.
- **Direct Console**: certified per-canister fallback, diagnostics, recovery,
  and wallet/steward access without the VPS stack.
- **Evaluator**: operator-only run control; must not be bundled into the public
  Lab production image.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Token generation check | `npm run verify:ui-tokens` | generated CSS/TS assets match canonical tokens |
| Terminal parity | `npm run verify:terminal-parity` | Lab and direct console expose identical supported command metadata |
| Web tests | `npm run test --workspace @ic-automaton/web` | all pass |
| Evaluator tests | `npm run test --workspace @ic-automaton/evaluator-web` | all pass |
| Build surfaces | `npm run build --workspace @ic-automaton/web && npm run build --workspace @ic-automaton/evaluator-web` | both build |
| Certified UI gate | `cargo test -p backend --no-default-features http::` | HTTP/certification UI tests pass |
| Full gate | `npm run lint && npm test && cargo test --workspace` | all pass |

## Scope

**In scope**:

- `packages/ui/**` (create)
- `packages/shared/src/terminal-commands.ts` and tests
- `apps/web/src/theme/**`
- `apps/web/src/styles.css`
- `apps/web/src/App.tsx`
- `apps/web/src/components/drawer/**`
- `apps/web/src/components/grid/**` only for shared status colors/tokens
- `apps/web/src/lib/cli-command-registry.ts`
- `apps/web/src/hooks/useCommandSession.ts`
- `apps/evaluator-web/src/styles.css`
- `apps/evaluator-web/src/App.tsx` and presentational components
- `components/ic-automaton/src/ui_index.html`
- `components/ic-automaton/src/ui_styles.css`
- `components/ic-automaton/src/ui_app.js` only for generated tokens/command
  metadata integration; do not change signing semantics from plan 002
- `components/ic-automaton/src/http.rs` only for generated certified assets
- `scripts/generate-ui-assets.mjs` and `scripts/verify-terminal-parity.mjs`
- Root package scripts/lockfile
- Relevant web/evaluator/component tests

**Out of scope**:

- Combining evaluator into the public production bundle.
- Removing the certified direct console.
- Changing factory, spawn, payment, wallet, steward proof, or canister behavior.
- Adding a new router/framework solely for this plan.
- Replacing remote fonts with licensed local assets unless the operator supplies
  those assets and licenses.
- Pixel-perfect redesign of every component.
- Raster image generation; the current visual system is CSS/SVG/canvas based.

## Git workflow

- Branch: `advisor/006-converge-product-appearance`
- Suggested commits:
  `feat(ui): add shared automaton design tokens`,
  `refactor(terminal): share command metadata`,
  `refactor(web): make Lab the canonical product shell`, and
  `style(evaluator): apply operator theme`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Define canonical product tokens and three themes

Create `packages/ui` with a machine-readable canonical token source covering:

- brand name and audience labels;
- typography stacks;
- spacing and border widths;
- background, ink, accent, muted, success/warning/critical status colors;
- focus, disabled, and motion values;
- theme overrides for `lab`, `direct-console`, and `operator`.

Use the existing Lab cream/black/red palette as the default. Preserve a dark
high-contrast console theme and a dense operator theme, but derive shared status
semantics and spacing from the same source.

Add `scripts/generate-ui-assets.mjs` with `--write` and `--check` modes. Generate:

- TypeScript token exports for Lab/evaluator;
- CSS custom properties for each theme;
- a small generated CSS token file consumed by the embedded direct console.

Generated files must contain a header saying not to hand-edit them.

**Verify**: `npm run verify:ui-tokens` -> exit 0; changing a generated token in a
temporary test fixture makes `--check` fail.

### Step 2: Make Automaton Lab the canonical public shell

Refactor `apps/web/src/App.tsx` and existing components without adding a routing
framework. The visible information architecture must distinguish:

- Fleet;
- Room (automaton-authored/publicly observed);
- Spawn;
- selected automaton Overview, Activity, Terminal, and Strategies sections.

Use “steward” for the authorized EVM identity, “operator” for evaluator/factory
operations, “visitor/supporter” for public wallet interaction, “controller” only
for IC principals, and “automaton” as the product subject. Remove or implement
the currently unreachable `mine` scope; do not leave hidden state with no UI.

Do not expose evaluator stop/run controls or operator terminology in the public
Lab bundle.

**Verify**: extend `apps/web/src/App.test.tsx` to assert the canonical product
name, public sections, role labels, and absence of operator controls.

### Step 3: Lazy-load the live terminal context

Do not mount the command panel or issue its six direct canister requests merely
because the drawer opened. Initially render overview/activity from indexed
detail. Fetch live context only when the user opens Terminal or invokes a
live-only command. Keep one refresh/error model visible per section.

Add tests proving:

- drawer overview causes no direct canister context request;
- opening Terminal causes one aggregate context load;
- closing/changing selection aborts the request;
- cached live context is reused until its explicit freshness window expires.

**Verify**: `npm run test --workspace @ic-automaton/web` -> all pass.

### Step 4: Share terminal command metadata and enforce parity

Move command names, usage, auth level, transport, and summaries into
`packages/shared/src/terminal-commands.ts`. Lab imports this source directly.
The UI generator emits a generated metadata block for the embedded console.

Keep execution implementations platform-specific, but add
`verify-terminal-parity.mjs` to compare:

- canonical command names;
- embedded dispatcher cases;
- Lab executor mappings;
- auth/transport classifications where represented.

The script must fail when a command exists on only one surface. It must not
rewrite or weaken the signed steward flow completed by plan 002.

**Verify**: `npm run verify:terminal-parity` -> exit 0; a fixture with one
missing command fails.

### Step 5: Apply the direct-console theme while preserving certification

Consume generated direct-console tokens in the embedded CSS. Align header,
status semantics, wallet/steward labels, typography scale, focus styles, and
product naming with the Lab. Keep the dark console aesthetic and all certified
asset routes.

Ensure the child build includes generated assets before Rust `include_str!`
compilation, and that CI `--check` prevents stale generated assets.

**Verify**:

```bash
npm run verify:ui-tokens
cargo test -p backend --no-default-features http::
./components/ic-automaton/scripts/build-backend-wasm.sh
```

All exit 0.

### Step 6: Apply the operator theme to evaluator without merging bundles

Replace evaluator's independent raw palette/spacing with shared operator-theme
variables. Preserve its dense run metrics, realtime state, stop control, and
responsive table/card views. Add a clear “Operator / Evaluation” label so it is
not confused with the public Lab.

Do not add evaluator dependencies to `apps/web/package.json` or its Docker
image. The apps may share `packages/ui` and `packages/shared` only.

**Verify**: evaluator tests and build commands from the table pass; a dependency
scan shows the web app does not import evaluator modules.

### Step 7: Add responsive and accessibility regression coverage

Use existing component-test conventions and the installed Playwright tooling.
Cover at least desktop, tablet, and narrow mobile layouts for:

- Lab header and wallet controls;
- fleet/room/spawn access;
- automaton overview and terminal sections;
- evaluator operator label and stop control;
- direct console focusable command input and status bar.

Assert focus visibility, landmark/heading labels, no horizontal page overflow,
and that status is never conveyed only by color. Prefer semantic assertions over
large brittle snapshots.

**Verify**: focused component tests plus the repository's Playwright command
exit 0. If no Playwright script exists, add a root `test:e2e:ui` script and
document its local server prerequisites.

### Step 8: Run all appearance and workspace gates

```bash
npm run verify:ui-tokens
npm run verify:terminal-parity
npm run lint
npm test
npm run build --workspace @ic-automaton/web
npm run build --workspace @ic-automaton/evaluator-web
cargo test --workspace
./components/ic-automaton/scripts/build-backend-wasm.sh
```

**Verify**: all commands exit 0 and the web production build has no evaluator
application import.

## Test plan

- Token generator write/check determinism.
- Shared status colors/labels across themes.
- Public Lab contains no operator controls.
- Evaluator remains a separate build and is labeled operator-only.
- Direct console remains certified and independently reachable.
- Every terminal command has parity across canonical metadata, Lab dispatch,
  and embedded dispatch.
- Plan-002 signed steward behavior remains covered and unchanged.
- Drawer overview avoids six-request live fan-out; Terminal loads lazily.
- Responsive and keyboard/focus behavior at three viewport classes.

## Done criteria

- [x] One canonical token source generates all three themes.
- [x] Lab is visibly the canonical public product shell.
- [x] Direct console retains certified fallback access and a related dark theme.
- [x] Evaluator retains a separate operator-only bundle and related theme.
- [x] Terminal metadata parity check is required in CI.
- [x] Overview does not fetch live terminal context until requested.
- [x] All tests, builds, lint, token, parity, and child-Wasm gates pass.
- [x] `plans/README.md` row 006 is `DONE`.

## STOP conditions

- Plan 002 is incomplete or signed steward commands still simulate success.
- The direct console cannot consume generated tokens without breaking certified
  asset hashes/routes.
- A proposed shared component imports operator code into the public bundle.
- Visual convergence requires changing wallet, payment, proof, or canister
  behavior.
- Product role terminology cannot be resolved from current documented domain
  rules; report the exact ambiguous terms.
- Playwright requires an external/live environment rather than local fixtures.
- Any verification fails twice after one focused correction.

## Maintenance notes

Shared tokens define product semantics, not identical layouts. The direct
console and evaluator should remain purpose-specific. Reviewers should inspect
bundle boundaries, signed-command regressions, generated-asset freshness,
accessibility, and whether new terminal commands update all parity sources.
