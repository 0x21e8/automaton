# Plan 005: Build and deploy one atomic release from one revision

> **Executor instructions**: This plan changes CI and deployment orchestration.
> It does not authorize a real deployment. Implement and test workflow/script
> behavior locally and in non-deploying CI only unless the operator separately
> authorizes an environment action. Stop on every STOP condition. Update row
> 005 in `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 09fdfe2..HEAD -- .github/workflows scripts ops package.json components/ic-automaton`
> Changes created by completed prerequisite plans are expected. Compare them
> with this plan's current-state description and stop if plan 004 is incomplete.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `plans/004-canonicalize-boundary-contracts.md`
- **Category**: migration, dx
- **Planned at**: destination commit `09fdfe2`, source commit `eda66fd`, 2026-07-10

## Why this matters

Current launchpad images are built from the launchpad commit, while child Wasm
URL, SHA-256, and source commit come from three independent secrets. The child
tag workflow runs in another repository. The VPS executes deploy scripts from a
checkout whose revision is not compared with the manifest. The monorepo should
produce one immutable manifest from one clean CI checkout while retaining exact
per-artifact digests and explicit rollout actions.

## Current state after plans 003-004

- `components/ic-automaton/.github/workflows/publish-wasm.yml` is historical
  imported configuration and is not active at repository root.
- Root `.github/workflows/publish-playground-images.yml` builds three images and
  writes their digests.
- `.github/workflows/deploy-soft.yml:32-41` currently reads child URL, digest,
  and commit from environment secrets.
- `scripts/deploy-playground-release.sh:155-264` validates manifest fields and
  child bytes, then `:360-392` chooses soft or hard-reset behavior.
- Soft deploy updates containers but does not admit the child artifact to the
  factory. Hard reset uses playground bootstrap/reset behavior.
- The runner executes scripts from the VPS checkout path without proving it is
  the same source revision as the manifest.
- Plan 004 adds `npm run verify:child-contract` and a built-Wasm integration
  gate; those must run before publishing.

## Target release contract

Schema version 2 must include:

```json
{
  "schemaVersion": 2,
  "release": {
    "sourceCommit": "40 lowercase hex characters",
    "environmentVersion": "display value",
    "createdAt": "ISO-8601"
  },
  "images": {
    "web": { "ref": "repository@sha256:...", "digest": "sha256:..." },
    "indexer": { "ref": "repository@sha256:...", "digest": "sha256:..." },
    "rpcGateway": { "ref": "repository@sha256:...", "digest": "sha256:..." }
  },
  "artifacts": {
    "automatonWasm": { "fileName": "...", "sha256": "64 lowercase hex", "sourceCommit": "..." },
    "factoryWasm": { "fileName": "...", "sha256": "64 lowercase hex", "sourceCommit": "..." }
  },
  "ops": { "sourceCommit": "same release commit" }
}
```

All source-commit fields default to the same monorepo commit, but remain
explicit to support reviewed backports. Exact byte digests are mandatory.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Validate release code | `npm run test --workspace @ic-automaton/shared -- test/release-manifest.test.ts` | schema tests pass |
| Full validation | `npm run lint && npm test && cargo test --workspace` | all pass |
| Contract gate | `npm run verify:child-contract && npm run verify:strategies` | both pass |
| Build child | `./components/ic-automaton/scripts/build-backend-wasm.sh dist/automaton.wasm` | exact output exists |
| Build factory | `./scripts/build-factory-wasm.sh dist/factory.wasm` | exact output exists and Candid metadata matches |
| Workflow syntax | `actionlint` | exit 0 if installed; otherwise CI dry-run/lint job validates YAML |
| Manifest dry run | `node scripts/render-release-manifest.mjs --mode dry-run --output tmp/release-manifest.json` | schema-valid manifest written without deploying |

## Scope

**In scope**:

- `.github/workflows/publish-playground-images.yml` (replace or refactor into a
  reusable build-release workflow)
- `.github/workflows/deploy-soft.yml`
- `.github/workflows/deploy-hard-reset.yml`
- `.github/workflows/publish-wasm.yml` at repository root (create/migrate)
- `.github/workflows/admit-child-artifact.yml` (create)
- `.github/workflows/upgrade-automaton.yml` (create as explicit manual workflow)
- `scripts/build-factory-wasm.sh` (create)
- `scripts/render-release-manifest.mjs` (create)
- `scripts/deploy-playground-release.sh`
- `scripts/upload-factory-artifact.mjs`
- `scripts/playground-bootstrap.sh`, reset, smoke, and status scripts only where
  needed for schema-v2 artifacts
- `scripts/lib/release-manifest.mjs` and tests (create)
- `packages/shared/src/release.ts` and `packages/shared/test/release-manifest.test.ts`
- `ops/playground/release-manifest.example.json`
- `ops/playground/README.md` and `VPS-SETUP.md`
- Root `package.json` scripts

**Out of scope**:

- Automatically upgrading existing child automatons.
- Deploying Cloudflare Workers; record their selected identifiers only in a
  later manifest extension.
- Changing factory/child business logic or stable state.
- Replacing Docker Compose, VPS hosting, Tailscale, GHCR, or ICP.
- Running a real deploy from the implementation branch.
- Storing secret values in a manifest, artifact, test fixture, or log.

## Git workflow

- Branch: `advisor/005-atomic-monorepo-release`
- Suggested commits:
  `build: package factory and automaton wasm artifacts`,
  `ci: publish atomic automaton release manifest`,
  `deploy: separate artifact admission and playground reset`, and
  `docs: document release and rollback boundaries`.
- Do not push, publish, or deploy unless separately instructed.

## Steps

### Step 1: Define and test release manifest schema v2

Create a pure parser/validator used by Node scripts and mirrored with exported
TypeScript types in `packages/shared`. Validate:

- exact schema version;
- full lowercase source SHAs;
- immutable image digest refs;
- artifact file names and SHA-256 values;
- equal default source revisions, while allowing explicit reviewed overrides;
- no unknown release mode hidden inside the immutable build manifest.

Keep deployment action (`soft`, `hard-reset`, `admit-child`, `upgrade-named`)
outside the build manifest or as an explicit command-line selection. The same
release can be used by multiple actions without mutating it.

Add tests for valid manifests and every invalid field class. Never include real
URLs, keys, hostnames, or credentials in fixtures.

**Verify**: focused shared-package test command from the table -> all pass.

### Step 2: Build deterministic factory and child artifacts in CI

1. Keep the existing child build script as the source of canister-ready child
   bytes; direct it to `dist/automaton.wasm`.
2. Add `scripts/build-factory-wasm.sh`, following the repository's existing
   ICP Rust build recipe. It must produce `dist/factory.wasm`, attach Candid
   metadata, and compare extracted Candid with `backend/factory/factory.did`.
3. Use deterministic gzip (`gzip -n -9`) only for transport; record hashes for
   the exact transported bytes and name that fact clearly.
4. Write SHA files and run the plan-004 built-Wasm compatibility test before
   publishing anything.

**Verify**:

```bash
mkdir -p dist
./components/ic-automaton/scripts/build-backend-wasm.sh dist/automaton.wasm
./scripts/build-factory-wasm.sh dist/factory.wasm
sha256sum dist/automaton.wasm dist/factory.wasm
npm run verify:child-contract
```

All exit 0; both files are non-empty.

### Step 3: Produce one reusable build-release workflow

Refactor the current image workflow into a reusable workflow that, from one
checkout and one `GITHUB_SHA`:

1. installs locked Node, Rust, Foundry, and Wasm tools;
2. runs lint, JS tests, Rust workspace tests, EVM tests, strategy check, Candid
   check, and the built-Wasm integration test;
3. builds child and factory Wasm artifacts;
4. builds/pushes the three digest-addressed images;
5. computes exact artifact and image digests;
6. renders manifest schema v2;
7. uploads manifest, both Wasms, checksums, and an archived source/ops bundle as
   one workflow artifact set.

Do not source child metadata from repository secrets. Secrets may authenticate
registries or deployment transport only.

**Verify**: trigger the reusable workflow in build-only mode. It publishes
workflow artifacts and images but performs no environment deployment.

### Step 4: Make the VPS run the release's orchestration revision

Package a source/ops bundle from the exact release commit. On the VPS, stage it
under a release-specific directory, verify its archive hash, install locked npm
dependencies if required by deployment scripts, and execute the deploy runner
from that staged revision. Keep mutable state and environment files outside the
release directory.

Before any action, the runner must assert:

- its packaged source revision equals `release.sourceCommit` and
  `ops.sourceCommit`;
- downloaded/copied artifact hashes match the manifest;
- image refs are immutable digest refs;
- the target mode is explicit.

Never run new images with a stale unverified Compose/script revision.

**Verify**: a dry-run fixture succeeds with matching revisions and fails when
the ops revision, artifact hash, or image digest is altered.

### Step 5: Separate four release actions

Implement distinct entrypoints:

1. **Soft playground deploy**: update web/indexer/gateway images only; do not
   upload child Wasm, reinstall factory, reset chain state, or upgrade children.
2. **Hard-reset playground**: explicit manual workflow/environment approval;
   use manifest factory/child artifacts, recreate ephemeral state, bootstrap,
   and smoke test.
3. **Admit child artifact**: explicit workflow uploads the manifest-selected
   child Wasm to the existing factory for future spawns and verifies factory
   health reports the selected digest/revision. It does not upgrade children.
4. **Upgrade named automaton**: explicit manual workflow requires canister ID,
   selected release, environment approval, pre-upgrade health snapshot, and
   post-upgrade verification. It must use upgrade mode, never reinstall, unless
   a separate destructive approval is added later.

Names, summaries, and documentation must use these exact distinctions.

**Verify**: script/workflow tests assert that each mode invokes only its allowed
operations. A soft-deploy test must fail if artifact-upload, install, upgrade,
or reset commands are observed.

### Step 6: Preserve rollback and provenance

Record every applied manifest immutably under the existing release-state
directory and keep `current.json`. Add a rollback command that selects a prior
manifest and repeats only the requested action. It must not infer that rolling
back images also rolls back the factory artifact or existing children.

Mark local dirty builds explicitly as dirty and prevent them from being
published as a clean commit. In CI, fail if tracked files are dirty after build
or generation.

**Verify**: manifest/provenance tests prove a dirty marker is required locally
and prohibited in published production artifacts.

### Step 7: Update deployment documentation and run all non-deploying gates

Document:

- release creation;
- each explicit action;
- component-level rollback;
- which secret store owns authentication versus runtime configuration;
- why existing children are not automatically upgraded.

Run every command in the command table. No real deployment is part of this plan.

**Verify**: every non-deploying command exits 0, build-only workflow artifacts
are complete, and no environment deployment job ran.

## Test plan

- Schema-v2 valid and invalid fixtures.
- Deterministic artifact build and digest recording.
- One revision feeds all image and Wasm metadata.
- Dirty-source publication rejected.
- Stale ops runner rejected.
- Soft mode cannot upload/install/reset/upgrade.
- Hard reset consumes manifest-selected factory and child bytes.
- Artifact admission verifies factory health digest/revision.
- Named upgrade requires explicit canister and approval inputs.
- Rollback selects components explicitly rather than assuming all move together.

## Done criteria

- [x] Build-only workflow emits one schema-v2 manifest and all artifacts.
- [x] No child URL/SHA/commit deployment secret remains.
- [x] Exact image and Wasm digests are in the manifest.
- [x] Plan-004 contract/integration gates precede publication.
- [x] VPS runner revision must match the manifest.
- [x] Soft, hard-reset, admit-child, and upgrade-named are separate actions.
- [x] Existing children are never upgraded implicitly.
- [x] All non-deploying tests/lint pass.
- [x] `plans/README.md` row 005 is `DONE`.

## STOP conditions

- Plan 004's built-Wasm gate is not green.
- Factory Wasm cannot be built reproducibly with checked Candid metadata.
- The implementation would require embedding a secret in the manifest.
- The VPS cannot stage or verify the release's ops/source revision.
- A “soft” path invokes any destructive or artifact-admission operation.
- A named upgrade cannot preserve stable state with upgrade mode.
- Completing validation would require a real deployment not separately
  authorized by the operator.
- Any verification fails twice after one focused correction.

## Maintenance notes

One Git revision is the coordination anchor; per-artifact digests remain the
byte-level truth. Reviewers should inspect mode separation, stale-runner guards,
and absence of secret material. Adding a new deployable component means adding
its digest, build gate, and explicit rollout policy to the schema.
