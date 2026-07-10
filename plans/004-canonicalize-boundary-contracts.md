# Plan 004: Canonicalize boundary contracts and add a built-Wasm compatibility gate

> **Executor instructions**: Read this plan fully before editing. It changes the
> factory/child wire boundary after both live in one workspace. Follow each
> verification gate and stop instead of inventing compatibility behavior.
> Update row 004 in `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 09fdfe2..HEAD -- backend/factory components/ic-automaton packages strategy-seeds strategies tests scripts`
> Changes created by completed prerequisite plans are expected. Compare them
> with this plan's current-state description. If plan 003 is not complete, stop.

## Status

- **Status**: DONE
- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: `plans/003-import-runtime-monorepo-component.md`
- **Category**: tech-debt, tests
- **Planned at**: destination commit `09fdfe2`, source commit `eda66fd`, 2026-07-10

## Why this matters

The child init/bootstrap contract is independently declared in child Rust,
factory Rust, factory response mirrors, handwritten TypeScript IDL, and shared
UI types. The current artifact check only searches Wasm bytes for six method
names. Strategy recipes are also copied from the runtime repository into
launchpad seed files with pinned provenance. A monorepo removes path coupling,
but deployment becomes reliable only after these boundary contracts and assets
have one source and one built-Wasm verification gate.

## Current state after plan 003

- Child init types originate at
  `components/ic-automaton/src/lib.rs:139-199`.
- Factory duplicates them at `backend/factory/src/types.rs:96-178` and encodes
  them in `backend/factory/src/init.rs:238-272`.
- Factory child response mirrors exist in `backend/factory/src/spawn.rs` near
  the current `AutomatonBootstrapEvidence` conversion code.
- Indexer manually defines automaton wire and HTTP types in
  `apps/indexer/src/integrations/automaton-client.ts:12-269`.
- Web repeats HTTP response types in `apps/web/src/api/automaton.ts:1-85`.
- `scripts/validate-child-canister-interface.mjs:9-95` scans export labels but
  does not compare record/variant field types.
- Factory embeds copied strategy files from `strategy-seeds` in
  `backend/factory/src/strategy_repository.rs:11-17`.
- Canonical runtime recipes currently live under
  `components/ic-automaton/docs/strategies`.
- Child installation uses `child_runtime.evm_chain_id`, while bootstrap
  verification currently compares another independently mutable release-chain
  setting. They must become one validated invariant.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Workspace tests | `cargo test --workspace` | all Rust tests pass |
| JS tests | `npm test` | all workspace tests pass |
| Child build | `./components/ic-automaton/scripts/build-backend-wasm.sh` | canister-ready Wasm built |
| Candid generation | `./components/ic-automaton/scripts/generate-candid.sh` | exit 0; no unexpected DID diff |
| Boundary check | `npm run verify:child-contract` | exact Candid metadata and required HTTP schema checks pass |
| Integration | `AUTOMATON_WASM_PATH=components/ic-automaton/target/wasm32-wasip1/release/backend_nowasi.wasm cargo test -p integration-tests spawn_contract` | built-Wasm contract test passes |
| Strategy check | `npm run verify:strategies` | manifest and canonical recipes match factory inputs |

## Scope

**In scope**:

- `crates/spawn-protocol/**` (create)
- Root `Cargo.toml` and `Cargo.lock`
- `components/ic-automaton/Cargo.toml`
- `components/ic-automaton/src/lib.rs`
- `components/ic-automaton/src/domain/types.rs` only for boundary-type imports
- `components/ic-automaton/ic-automaton.did`
- `backend/factory/Cargo.toml`
- `backend/factory/src/types.rs`
- `backend/factory/src/init.rs`
- `backend/factory/src/spawn.rs`
- `backend/factory/src/api/admin.rs`
- `backend/factory/factory.did`
- `packages/canister-clients/**` (create)
- `apps/indexer/src/integrations/automaton-client.ts`
- `apps/web/src/api/automaton.ts`
- `strategies/**` (create as canonical assets)
- `strategy-seeds/**` (remove only after all consumers switch)
- `backend/factory/src/strategy_repository.rs`
- `scripts/validate-child-canister-interface.mjs`
- `scripts/verify-strategies.mjs` (create)
- `tests/integration/**` (create)
- Root `package.json` and lockfile

**Out of scope**:

- Sharing stable-state types or storage layouts between canisters.
- Sharing scheduler, EVM polling, signing, inference, or business logic.
- Changing public command semantics or adding strategies.
- Upgrading existing child automatons.
- Broad dependency upgrades.
- Generating frontend domain models directly from stable-state structs.

## Git workflow

- Branch: `advisor/004-canonicalize-boundary-contracts`
- Commit in this order:
  `refactor(protocol): share factory child wire types`,
  `refactor(clients): centralize automaton contracts`,
  `refactor(strategies): use canonical workspace assets`, and
  `test(integration): verify factory child wasm boundary`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Create a narrow Rust spawn-protocol crate

Create `crates/spawn-protocol` with Candid/Serde types limited to the wire
boundary:

- inference transport and OpenRouter reasoning variants;
- child init args;
- spawn provider bootstrap args;
- spawn bootstrap args and public bootstrap view;
- the factory-facing child status/evidence records required for verification.

Use field names and variant labels exactly as they exist in the generated child
DID. Do not put factory stable state, provider-secret storage, scheduler state,
or child runtime snapshots in this crate.

Add the crate to the root workspace and use it from both child and factory.
Remove duplicate Rust definitions only after both crates compile against the
shared type.

**Verify**:

```bash
cargo check -p spawn-protocol
cargo test -p factory
cargo test -p backend --lib --no-default-features
```

All exit 0.

### Step 2: Lock the EVM chain deployment invariant

Ensure the exact chain ID encoded into child install args is the chain ID used
for bootstrap verification. Either store one canonical deployment-chain field
or make both admin setters validate/update atomically. Separate factory release
transaction gas settings may remain separate, but they cannot silently select a
different chain.

Add tests proving:

- mismatched configuration is rejected before a paid spawn starts;
- changing the canonical chain updates both install and verification behavior;
- local init rendering cannot emit mismatched values.

**Verify**: `cargo test -p factory` -> all pass.

### Step 3: Centralize frontend canister clients

Create `packages/canister-clients` and move, without semantic changes:

- automaton Candid actor types and IDL construction from the indexer;
- certified HTTP response types currently duplicated by indexer and web;
- request helpers that are runtime-neutral and do not depend on React/Fastify.

Keep normalized UI types in `packages/shared`; wire types belong in
`canister-clients`. Update indexer and web imports, then delete their duplicate
wire declarations.

Add a generation/check script that extracts Candid from the freshly built child
Wasm using the repository's existing `candid-extractor`, compares it to the
checked `ic-automaton.did`, and then validates that the centralized actor exposes
every method the indexer/factory consumes. Replace export-string scanning as the
primary gate; it may remain as a quick diagnostic only.

**Verify**:

```bash
npm run lint
npm test
npm run verify:child-contract
```

All exit 0; an intentionally altered DID in a temporary test fixture makes the
contract test fail.

### Step 4: Establish one canonical strategy directory

Create root `strategies/<strategy-id>/` directories containing canonical
recipe JSON plus display/provenance metadata. Start from runtime recipes and
merge launchpad display metadata without changing executable semantics.

Update factory `include_str!` paths, runtime documentation/tests, README links,
and the strategy manifest to consume root assets. Add
`scripts/verify-strategies.mjs` that:

1. parses every recipe;
2. validates ID/protocol/primitive/chain against metadata;
3. rejects duplicate IDs;
4. hashes canonical JSON serialization so formatting-only changes do not alter
   semantic provenance;
5. proves every factory seed references an existing canonical recipe.

Remove `strategy-seeds` only after `rg` shows no consumers.

**Verify**:

```bash
npm run verify:strategies
rg -n 'strategy-seeds' backend components apps packages scripts README.md
```

The verification exits 0 and the search returns no stale consumer paths.

### Step 5: Add a built-Wasm factory/child contract integration crate

Create `tests/integration` as workspace package `integration-tests`. Model its
PocketIC setup after
`components/ic-automaton/tests/pocketic_spawn_bootstrap.rs`.

The `spawn_contract` test must:

1. read the exact canister-ready child Wasm from `AUTOMATON_WASM_PATH`;
2. install the real factory Wasm in PocketIC with valid local init args;
3. upload the exact child bytes through factory artifact upload methods and
   verify the factory-reported SHA/version;
4. construct child init args through the factory's production encoder;
5. install a separate child canister with those args;
6. query bootstrap, steward, EVM address, skills, and strategies through the
   centralized contract definitions;
7. assert direct and proxy bootstrap variants decode and match expected values;
8. assert malformed or drifted args fail deterministically before release logic.

Do not mock the child response types. The test must decode real replies from the
built child Wasm. It need not execute the Base payment/release flow; existing
factory tests retain that coverage.

**Verify**: build both Wasms, run the integration command from the table, and
confirm all cases pass.

### Step 6: Run every contract and workspace gate

```bash
./components/ic-automaton/scripts/build-backend-wasm.sh
./components/ic-automaton/scripts/generate-candid.sh
npm run verify:child-contract
npm run verify:strategies
cargo test --workspace
npm run lint
npm test
AUTOMATON_WASM_PATH=components/ic-automaton/target/wasm32-wasip1/release/backend_nowasi.wasm cargo test -p integration-tests spawn_contract
```

**Verify**: every command exits 0; `git diff` shows no unrelated runtime or
stable-state refactor.

## Test plan

- Shared Rust types encode/decode identically from both canisters.
- Checked DID exactly matches built Wasm metadata.
- Centralized TS client covers every consumed automaton method.
- HTTP response types are imported from one package by web and indexer.
- Mismatched chain configuration fails before spawning.
- Strategy metadata and recipes validate semantically.
- Real built child Wasm accepts factory-produced direct and proxy init args.
- Factory artifact admission reports the exact bytes' digest and source version.

## Done criteria

- [ ] No duplicated child init/bootstrap Rust record remains in factory or child.
- [ ] Web and indexer no longer duplicate automaton wire/HTTP response types.
- [ ] Built-Wasm Candid drift fails CI.
- [ ] One canonical strategy directory serves runtime and factory.
- [ ] Chain install/verification mismatch is impossible through public setters.
- [ ] Built-Wasm integration test passes for direct and proxy bootstrap.
- [ ] All workspace tests and lint pass.
- [ ] `plans/README.md` row 004 is `DONE`.

## STOP conditions

- Plan 003 is incomplete or the runtime is not at `components/ic-automaton`.
- Shared types would require coupling stable-memory layout between canisters.
- Generated child DID differs before any intended boundary change; investigate
  and report the pre-existing drift first.
- A strategy copy is semantically different and no product decision identifies
  the canonical version.
- PocketIC cannot install the canister-ready Wasm produced by the existing build
  script.
- Completing the test requires bypassing production auth in a production Wasm.
- Any verification fails twice after one focused correction.

## Maintenance notes

The shared protocol crate is a wire-contract crate, not a general shared-domain
dumping ground. Review every future addition for whether both canisters truly
own the concept. Always build Wasm and compare generated Candid before accepting
a boundary change. Strategy hashes should represent canonical semantics, while
artifact SHA-256 values must continue representing exact bytes.
