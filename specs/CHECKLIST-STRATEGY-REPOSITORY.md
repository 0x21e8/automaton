# Strategy Repository and Spawn Selection - Execution-Ready Checklist

**Status:** Proposed, execution-ready checklist
**Date:** 2026-04-03
**Source:** shaped from the factory spawn flow, sibling `ic-automaton` strategy model, and repository planning decisions captured on 2026-04-03
**Audience:** coding agents working in this repo and engineers coordinating with the sibling `ic-automaton` repo
**Scope:** add a factory-hosted strategy repository seeded from copied `ic-automaton` strategy recipes, wire strategy selection into the spawn flow, and ensure spawned children receive executable installed strategies

## Outcome

Ship one minimal strategy-repository path where:

- the factory canister owns the canonical strategy repository for launchpad spawns
- the repository is initially seeded from copied `ic-automaton` strategy recipes checked into this repo
- repository entries are full copied recipes, not loose category labels or external references
- public spawn-session creation accepts selected repository strategy IDs
- selected strategies are validated against repository state and spawn-chain compatibility
- selected strategy artifacts are snapshotted into the spawn session at creation time
- the factory installs the selected strategies into the spawned child as real executable templates before controller handoff
- failed strategy installation fails the spawn rather than producing a partially configured child
- indexer and web surfaces expose concrete repository strategies in a user-understandable way
- tests can drive the existing paid spawn-session flow programmatically with repository strategy IDs

## Locked Decisions

These are the implementation contract for the first slice.

1. The factory is the long-term source of truth for strategy selection in launchpad spawns.
   Reason: the spawn orchestrator already owns session validation, retry semantics, and child installation.

2. The repository stores full copied strategy recipes.
   Reason: spawned children need executable strategy payloads, not references back to the sibling repo.

3. The initial seed set is copied into this repo and loaded into the factory.
   Reason: the first cut should be small, local, and deterministic.

4. The initial seed strategies are:
   - `base-aave-usdc-reserve-01`
   - `base-moonwell-usdc-reserve-01`
   - `base-usdc-carry-cbbtc-01`
   Reason: these are the currently agreed concrete sibling strategies.

5. Skills are out of scope for the first cut.
   Reason: strategy seeding is the immediate need; skills do not yet have an equivalent copied artifact path.

6. The existing public paid spawn-session path is the only automation path for tests.
   Reason: test automation should exercise the real product path; no payment-bypass spawn helper is introduced in this slice.

7. `create-spawn-session` accepts repository strategy IDs in the existing config shape.
   Reason: this keeps the test path and the UI path aligned.

8. Spawn-session creation snapshots the selected strategy artifacts immutably into the session.
   Reason: retries must remain deterministic even if the repository changes later.

9. The child must receive fully installed executable strategies, not just bootstrap labels.
   Reason: the automaton needs usable templates, not metadata.

10. Strategy install is atomic with spawn success.
    Reason: a partially configured automaton is worse than a retryable failed spawn.

11. Repository lifecycle supports admin add, deprecate, and revoke after the initial seed import.
    Reason: the repository is intended to become the durable control plane, not a fixed bootstrap-only catalog.

12. Compatibility is validated by semantic chain family, not only raw numeric chain ID equality.
    Reason: Base strategies must work for `base` and for local Base-fork playground chains.

13. Concrete repository strategies are what the UI presents.
    Reason: users should select real deployable templates such as `Base Aave USDC Reserve`, not broad category placeholders.

14. Repository admin ingestion accepts the same raw strategy recipe JSON format used by `ic-automaton`.
    Reason: this keeps the copied strategy authoring and validation path aligned with the sibling project.

## Scope Guardrails

- Do not add a separate strategy repository canister in this slice.
- Do not add a payment-bypass spawn helper or admin-only direct spawn helper.
- Do not keep the current mock category labels as if they were real executable strategies.
- Do not allow partial success where a child spawns with only some selected strategies installed.
- Do not depend on live reads from the sibling repo at runtime.
- Do not broaden this slice to include real skill seeding or skill lifecycle.
- Do not change the public spawn flow shape more than necessary beyond replacing mock strategy labels with repository IDs.

## External Prerequisites

The repo can prepare for these, but cannot fully satisfy them alone:

- the sibling `ic-automaton` repo must keep a stable controller-only strategy registration path compatible with copied recipe JSON
- the sibling `ic-automaton` repo must keep query surfaces that allow installed strategy verification after spawn
- an integration environment must exist where the factory can install a child and then make the required child canister calls before controller handoff

## Dependency Order

Use this order rather than implementing by subsystem alone.

1. `STRAT-01` and `STRAT-02` can start in parallel.
2. `STRAT-03` depends on `STRAT-01`.
3. `STRAT-04` depends on `STRAT-01`, `STRAT-02`, and `STRAT-03`.
4. `STRAT-05` depends on `STRAT-04`.
5. `STRAT-06` depends on `STRAT-04` and `STRAT-05`.
6. `STRAT-07` depends on `STRAT-03` and `STRAT-04`.
7. `STRAT-08` depends on `STRAT-07`.
8. `STRAT-09` depends on `STRAT-05`, `STRAT-06`, and `STRAT-08`.
9. `IA-STRAT-01` should start no later than `STRAT-05`.
10. `STRAT-10` depends on `STRAT-09` and `IA-STRAT-01`.

## Checklist

### Phase 1 - Contracts and seed assets

- [x] **STRAT-01: Define shared repository and session snapshot contracts**
  - Files to modify:
    - `packages/shared/src/catalog.ts`
    - `packages/shared/src/spawn.ts`
    - `packages/shared/src/index.ts`
    - `packages/shared/test/contracts.test.ts`
  - Implement:
    - shared repository record types for concrete strategy templates
    - lifecycle status types such as `active`, `deprecated`, and `revoked`
    - session-level snapshot types for selected repository strategies
    - explicit semantics that `config.strategies` contains repository strategy IDs
  - Important detail:
    - keep repository records separate from session snapshots; repository mutates, snapshots do not
  - Done when:
    - downstream code can model repository reads, selected IDs, and immutable session snapshots without resorting to mock strategy labels
  - Validation:
    - `npm run test --workspace @ic-automaton/shared`

- [x] **STRAT-02: Copy seed strategy recipes and define local seed metadata**
  - Files to create:
    - seed strategy files under a repo-owned path such as `strategy-seeds/` or `specs/strategy-seeds/`
    - a machine-readable manifest describing display names and copied source provenance
  - Implement:
    - copied recipe JSON for:
      - `base-aave-usdc-reserve-01`
      - `base-moonwell-usdc-reserve-01`
      - `base-usdc-carry-cbbtc-01`
    - display metadata for concrete UI presentation
    - provenance fields such as source path and source commit
  - Important detail:
    - the copied artifacts are the launchpad-owned seed input; they must not be loaded dynamically from the sibling repo at runtime
  - Done when:
    - this repo contains all seed data needed to initialize the factory strategy repository offline

- [x] **STRAT-03: Extend factory domain types and Candid for repository-backed strategy selection**
  - Files to modify:
    - `backend/factory/src/types.rs`
    - `backend/factory/factory.did`
    - `packages/shared/src/spawn.d.ts` or generated source-side artifacts if maintained
  - Implement:
    - factory strategy repository record types
    - session snapshot types for selected strategies
    - admin request/response types for add, deprecate, and revoke
    - public read types for listing and fetching repository entries
  - Important detail:
    - keep the public spawn session model stable where possible; only the meaning of `config.strategies` changes from free-form labels to repository IDs
  - Done when:
    - the factory Candid contract can represent repository state, repository admin mutations, and session snapshots explicitly
  - Validation:
    - `cargo test -p factory`

### Phase 2 - Factory repository core

- [x] **STRAT-04: Add seeded repository storage and admin lifecycle in the factory**
  - Files to modify:
    - `backend/factory/src/state.rs`
    - `backend/factory/src/api/admin.rs`
    - `backend/factory/src/lib.rs`
    - `backend/factory/src/types.rs`
    - seed-loading support files as needed
  - Implement:
    - stable storage for strategy repository entries
    - initial seed loading from copied repo assets
    - admin methods to:
      - add/import a strategy recipe
      - deprecate a strategy
      - revoke a strategy
    - public read methods to:
      - list repository strategies
      - fetch a single repository strategy
  - Important detail:
    - admin ingestion should accept the same raw recipe JSON format used by `ic-automaton`
  - Done when:
    - the factory can boot with seeded strategies and can manage repository lifecycle without code changes
  - Validation:
    - `cargo test -p factory`

- [ ] **STRAT-05: Add chain-family compatibility resolution in the factory**
  - Files to modify:
    - `backend/factory/src/types.rs`
    - `backend/factory/src/api/public.rs`
    - `backend/factory/src/init.rs`
    - `packages/shared/src/spawn.ts`
  - Implement:
    - canonical mapping from spawn chain to execution chain family
    - compatibility rules where canonical Base strategies are valid for `base` and local Base-fork playground chains
    - helpers that derive the child-install chain ID from runtime config while preserving canonical repository metadata
  - Important detail:
    - repository records should keep canonical source chain semantics; any required local chain-ID rewrite belongs to the install path or session snapshot resolution, not to the copied seed artifact itself
  - Done when:
    - a Base recipe can be selected for a local Base-family playground spawn without weakening incompatible-chain validation
  - Validation:
    - `cargo test -p factory`

### Phase 3 - Spawn-session validation and snapshotting

- [x] **STRAT-06: Validate selected repository strategies during session creation**
  - Files to modify:
    - `backend/factory/src/api/public.rs`
    - `backend/factory/src/lib.rs`
    - `backend/factory/src/types.rs`
    - `apps/indexer/src/integrations/factory-canister-adapter.ts`
    - `apps/indexer/src/integrations/factory-client.ts`
  - Implement:
    - session-creation validation that each selected strategy ID:
      - exists
      - is active
      - is compatible with the requested spawn chain family
    - clear user-facing errors for missing, deprecated, revoked, or incompatible strategies
  - Important detail:
    - validation belongs in the factory source of truth, not only in the web client
  - Done when:
    - invalid selections are rejected before a payable session is created
  - Validation:
    - `cargo test -p factory`
    - `npm run test --workspace @ic-automaton/indexer`

- [x] **STRAT-07: Snapshot selected repository entries into the session**
  - Files to modify:
    - `backend/factory/src/types.rs`
    - `backend/factory/src/state.rs`
    - `backend/factory/src/session_transitions.rs`
    - `backend/factory/src/api/public.rs`
    - `packages/shared/src/spawn.ts`
  - Implement:
    - immutable session snapshots for selected strategies, including copied recipe payloads and resolved compatibility metadata
    - serialization through public session-detail reads
    - retry-safe persistence semantics
  - Important detail:
    - snapshots must be sufficient to replay child strategy installation without consulting current repository state
  - Done when:
    - changing or revoking a repository entry after session creation does not change the selected session’s install payload
  - Validation:
    - `cargo test -p factory`

### Phase 4 - Child installation path

- [x] **STRAT-08: Install snapped strategies into the spawned child before controller handoff**
  - Files to modify:
    - `backend/factory/src/spawn.rs`
    - `backend/factory/src/init.rs`
    - `backend/factory/src/lib.rs`
    - any child-call helpers or typed wrappers needed in the factory
  - Implement:
    - post-install child registration loop that applies the snapped strategies using the child controller-only strategy registration path
    - install-time recipe adaptation for Base-family local playground chain IDs when required
    - verification step that the child exposes the installed templates after registration
  - Important detail:
    - strategy installation must happen before final controller handoff
  - Done when:
    - a completed spawned child lists the selected strategies as real executable installed templates
  - Validation:
    - `cargo test -p factory`

- [x] **STRAT-09: Fail spawn atomically on strategy install errors and make retry deterministic**
  - Files to modify:
    - `backend/factory/src/spawn.rs`
    - `backend/factory/src/retry.rs`
    - `backend/factory/src/session_transitions.rs`
    - `backend/factory/src/types.rs`
  - Implement:
    - explicit failure path when any selected strategy fails to register into the child
    - retry behavior that reuses snapped artifacts only
    - audit entries that distinguish child install failure from strategy registration failure
  - Important detail:
    - do not leave behind a completed or self-controlled child if selected strategy installation did not finish successfully
  - Done when:
    - strategy registration failures leave the session retryable and no partial success path exists
  - Validation:
    - `cargo test -p factory`

### Phase 5 - Indexer and web selection path

- [x] **STRAT-10: Add repository reads and selected-strategy visibility to the indexer**
  - Files to modify:
    - `apps/indexer/src/integrations/factory-canister-adapter.ts`
    - `apps/indexer/src/integrations/factory-client.ts`
    - `apps/indexer/src/routes/spawn-sessions.ts`
    - `apps/indexer/src/routes/automatons.ts`
    - new or existing repository routes as needed
    - relevant indexer tests
  - Implement:
    - public read path for repository strategy listings
    - session detail exposure of snapped strategies
    - automaton detail enrichment showing installed or selected repository strategies where available
  - Important detail:
    - indexer responses should preserve the distinction between repository metadata, session snapshots, and child-observed installed templates
  - Done when:
    - web and test clients can fetch the repository and inspect selected-strategy state through the indexer
  - Validation:
    - `npm run test --workspace @ic-automaton/indexer`

- [x] **STRAT-11: Replace the mock web strategy catalog with repository-backed concrete templates**
  - Files to modify:
    - `apps/web/src/components/spawn/spawn-state.ts`
    - `apps/web/src/components/spawn/steps/StrategiesStep.tsx`
    - `apps/web/src/hooks/useSpawnSession.ts`
    - `apps/web/src/api/indexer.ts`
    - relevant web tests
  - Implement:
    - fetch concrete repository strategies from the indexer
    - show only active, chain-compatible concrete templates
    - render concrete names and descriptions such as `Base Aave USDC Reserve`
    - submit selected repository IDs through the existing spawn flow
  - Important detail:
    - remove the misleading mock category behavior rather than layering the real repository on top of it
  - Done when:
    - the spawn flow lets a user choose real repository strategies and submit them without relying on hardcoded labels
  - Validation:
    - `npm run test --workspace @ic-automaton/web`

### Phase 6 - Cross-repo and end-to-end verification

- [ ] **IA-STRAT-01: Verify sibling `ic-automaton` registration/query compatibility**
  - Files to modify in the sibling `ic-automaton` repo only if needed:
    - the controller-only strategy registration path
    - template query/verification paths
    - any relevant tests or docs
  - Implement:
    - confirm the child accepts copied recipe JSON through its controller-only registration interface
    - confirm installed-template reads remain stable enough for factory verification
  - Important detail:
    - this checklist assumes no new strategy-specific bootstrap path is required in the child; the factory installs strategies after child install using existing child APIs
  - Done when:
    - cross-repo integration has a locked child contract for strategy registration and verification

- [ ] **STRAT-12: Add end-to-end coverage for repository-backed spawning**
  - Files to create or modify:
    - playground smoke or e2e scripts in this repo
    - relevant integration tests in this repo
    - sibling integration tests if needed
  - Implement:
    - at least one positive scenario where:
      - repository strategies are listed
      - a spawn session is created with selected repository IDs
      - payment is submitted through the existing paid path
      - the child spawns successfully
      - the child lists the installed strategy templates
    - negative scenarios where:
      - deprecated or revoked strategies are rejected at session creation
      - incompatible-chain selections are rejected
      - repository changes after session creation do not alter retry behavior
      - child strategy install failure makes the session fail atomically
  - Done when:
    - the repository-backed strategy path is proven across factory, indexer, web, and child boundaries

## Suggested First Slice

If this needs to be broken into the smallest shippable path:

1. Land `STRAT-01` through `STRAT-05`.
2. Land `STRAT-06` and `STRAT-07`.
3. Land `STRAT-08` and `STRAT-09`.
4. Replace the web mock catalog with `STRAT-10` and `STRAT-11`.
5. Prove the end-to-end path with `IA-STRAT-01` and `STRAT-12`.

## Implementation Notes / Decisions Log

Use this section as execution memory while work lands. Update it whenever reality differs from the checklist, a contract is locked more tightly, or a task is partially completed with an important caveat.

### How to use this log

- Add one entry per meaningful implementation decision or deviation.
- Keep entries short and factual.
- Reference checklist task IDs such as `STRAT-06` or `IA-STRAT-01`.
- Record contract changes here before or alongside code changes.
- If a decision affects another repo, note that explicitly.

### Entry template

```md
- Date: YYYY-MM-DD
  Task: STRAT-XX / IA-STRAT-XX
  Decision: <what was decided or changed>
  Reason: <why>
  Impact:
  - <affected file, contract, or follow-up task>
  - <compatibility or migration note if any>
```

### Initial execution notes

- Date: 2026-04-03
  Task: DESIGN / CHECKLIST
  Decision: The first slice implements strategy repository seeding only; real skill repository work is intentionally deferred.
  Reason: the immediate need is executable strategy-backed spawning, and there is not yet a parallel copied-artifact workflow for skills.
  Impact:
  - `STRAT-01` through `STRAT-12` cover strategies only.
  - Future skill work should reuse the repository/snapshot/lifecycle shape where possible instead of inventing a separate spawn-selection model.

- Date: 2026-04-03
  Task: STRAT-06 / STRAT-07 / STRAT-09
  Decision: Selected repository strategies are snapshotted into the session at creation time and retries reuse only that snapshot.
  Reason: repository lifecycle is mutable by admin, but spawn retries must remain deterministic.
  Impact:
  - Session storage will grow because copied recipe payloads are retained per selected strategy.
  - Repository reads and session reads must clearly distinguish live repository state from immutable session snapshots.

- Date: 2026-04-03
  Task: STRAT-05 / STRAT-08
  Decision: Compatibility is determined by semantic chain family, while copied repository recipes retain canonical source chain identity.
  Reason: Base strategies must remain usable on local Base-fork playground chains without rewriting the source-of-truth repository into environment-specific variants.
  Impact:
  - The factory needs an explicit chain-family mapping layer.
  - Install-time strategy registration may adapt canonical `8453` recipes to the child runtime chain ID when the target chain is a Base-family fork.

- Date: 2026-04-03
  Task: STRAT-08 / STRAT-09
  Decision: Strategy installation is part of spawn completeness, not post-completion best-effort configuration.
  Reason: a spawned automaton without its selected executable strategies does not satisfy user intent or test requirements.
  Impact:
  - Controller handoff must happen only after strategy registration and verification succeed.
  - Spawn failures need explicit audit visibility for strategy-registration errors distinct from base child-install errors.

- Date: 2026-04-03
  Task: STRAT-01 / STRAT-03
  Decision: Phase 1 defines repository-record and session-snapshot contracts as standalone shared/factory types while keeping the live `SpawnSession` payload stable; only the semantics of `config.strategies` are tightened to repository IDs in this slice.
  Reason: repository validation, snapshot persistence, and session-detail exposure land in later dependent tasks, so widening the active session wire model now would add placeholder state before the factory repository exists.
  Impact:
  - `packages/shared` and `backend/factory` now expose explicit repository lifecycle, admin-ingest, public-read, and immutable snapshot types for later tasks.
  - `STRAT-07` will attach these snapshot contracts to persisted session reads once repository-backed validation and snapshotting are implemented.

- Date: 2026-04-03
  Task: STRAT-02
  Decision: Seed assets are stored under repo-owned `strategy-seeds/` as raw copied `recipe.json` payloads plus a manifest carrying display metadata and sibling-repo provenance.
  Reason: the first slice needs deterministic offline seed inputs that can initialize the factory repository without runtime dependency on the sibling repo.
  Impact:
  - Future factory seed loading can consume `strategy-seeds/manifest.json` and the adjacent raw recipe files directly.
  - UI-facing display names and provenance remain separate from the copied raw recipe JSON, matching the repository-record versus recipe-payload split.

- Date: 2026-04-03
  Task: STRAT-04
  Decision: The factory now seeds its repository from embedded `strategy-seeds/` assets and persists repository entries in stable storage alongside sessions.
  Reason: Phase 2 needs deterministic boot-time seed loading on fresh install plus upgrade-safe repository lifecycle state after admin mutations.
  Impact:
  - `backend/factory/src/strategy_repository.rs` is the single seed-ingest and recipe-validation path used by both embedded seeds and admin imports.
  - `backend/factory/src/state.rs` adds a dedicated stable map for repository entries, so repository lifecycle changes survive upgrades without re-reading seed files.

- Date: 2026-04-03
  Task: STRAT-04
  Decision: Admin strategy import validates raw recipe JSON against repository metadata by matching `template_id`, `chain_id`, `protocol`, and `primitive`.
  Reason: accepting copied `ic-automaton` recipe JSON verbatim is useful, but the factory still needs a lightweight integrity check so repository metadata cannot drift from the executable payload.
  Impact:
  - Duplicate strategy IDs are rejected instead of overwritten.
  - Public reads now expose both seeded and admin-managed repository entries through factory query methods before spawn-time validation lands in `STRAT-06`.

- Date: 2026-04-03
  Task: STRAT-06 / STRAT-07
  Decision: Session creation now snapshots repository-backed strategies directly onto `SpawnSession.selected_strategies`, and each snapshot resolves its install-time chain ID from the current child runtime config at creation time.
  Reason: the factory needs one immutable payload that can reject invalid selections up front and later replay child installation without consulting mutable repository state or newer runtime-chain settings.
  Impact:
  - Factory session reads and indexer session reads now expose immutable selected-strategy payloads alongside the original repository IDs in `config.strategies`.
  - Repository deprecations/revocations after session creation no longer change the session’s selected strategy payload or resolved install chain ID.

- Date: 2026-04-03
  Task: STRAT-08 / IA-STRAT-01
  Decision: The factory installs selected repository snapshots through the child canister's existing controller-only `register_strategy_admin(text)` entrypoint and verifies visibility through `list_strategy_templates(opt key, limit)`.
  Reason: the sibling `ic-automaton` repo already exposes a single-call registration path that accepts the copied recipe JSON format and returns an active template record, so no launchpad-specific bootstrap API was needed.
  Impact:
  - `backend/factory/src/spawn.rs` now depends on `register_strategy_admin` and `list_strategy_templates` staying stable in the child contract.
  - `scripts/validate-child-canister-interface.mjs` now checks those method exports in addition to the bootstrap verification methods.

- Date: 2026-04-03
  Task: STRAT-08 / STRAT-09
  Decision: Install-time recipe adaptation rewrites only the copied recipe's `chain_id` from canonical repository metadata to the immutable session snapshot's `resolved_chain_id`; `template_id`, `protocol`, and `primitive` must still match the snapshot exactly or registration fails.
  Reason: Base-family playground installs need a local execution chain ID without weakening repository integrity or allowing retries to drift onto different executable templates.
  Impact:
  - `backend/factory/src/init.rs` now owns the shared recipe rewrite/validation helper used by both the sync test path and the live wasm path.
  - Corrupt or mismatched session snapshots fail with a retryable strategy-registration error before controller handoff.

- Date: 2026-04-03
  Task: STRAT-09
  Decision: Pre-handoff strategy-registration failures reset the persisted runtime's install marker before the session is marked failed.
  Reason: retries must recreate a fresh child from the snapped artifacts instead of attempting to reuse a partially configured canister that was intentionally cleaned up before controller handoff.
  Impact:
  - `backend/factory/src/spawn.rs` now treats strategy-registration failure as an atomic spawn failure distinct from `install_code` failure or release-broadcast failure.
  - Retry execution remains deterministic even if the live repository entry is later deprecated or revoked.

- Date: 2026-04-03
  Task: STRAT-10
  Decision: The indexer now exposes repository reads via `/api/repository/strategies` and `/api/repository/strategies/:strategyId`, while automaton detail adds a separate `spawnSelection` view for session snapshots.
  Reason: clients need one path for live repository metadata and another for immutable session-selected payloads without conflating either with child-observed installed templates.
  Impact:
  - `apps/indexer` keeps repository records, session snapshots, and child template observations distinct across route payloads.
  - `packages/shared` now carries optional automaton-detail spawn-selection context without changing the existing installed-template `strategies` shape.

- Date: 2026-04-03
  Task: STRAT-11
  Decision: The web spawn wizard restores strategy choice as its own repository-backed step and removes the old mock default strategies and mock skill defaults from the submitted spawn config.
  Reason: the Phase 5 surface needs explicit user selection of real deployable repository templates rather than silently submitting placeholder labels.
  Impact:
  - `apps/web` now fetches repository strategies from the indexer, filters to active chain-compatible entries, and submits real repository IDs.
  - The wizard blocks progression past the strategy step, and final spawn submission, until at least one compatible repository strategy is selected.
