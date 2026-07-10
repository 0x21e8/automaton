# Plan 001: Remove provider secrets from public spawn-session data

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If a STOP condition occurs, stop and report; do not improvise.
> When done, update this plan's row in `plans/README.md`.
>
> **Drift check (run first)**:
> `git diff --stat 09fdfe2..HEAD -- backend/factory packages/shared apps/indexer apps/web`
> Compare the current-state excerpts below with live code. Any semantic mismatch
> in provider configuration, session storage, or spawn responses is a STOP
> condition.

## Status

- **Status**: DONE
- **Priority**: P0
- **Effort**: M
- **Risk**: HIGH
- **Depends on**: none
- **Category**: security
- **Planned at**: destination commit `09fdfe2`, 2026-07-10

## Why this matters

`ProviderConfig` currently contains OpenRouter and Brave credentials, and the
complete provider config is nested in `SpawnSession`. The public factory query,
indexer persistence, REST list/detail responses, and realtime messages all use
that session representation. Credentials can therefore leave the narrow spawn
installation path before the existing cleanup code clears them. Fix this before
the repositories or deployment surfaces are consolidated.

## Current state

- `backend/factory/src/types.rs:121-130` defines `ProviderConfig` with both
  credential fields and non-secret model/transport fields.
- `backend/factory/src/types.rs:497-527` stores `SpawnConfig`, including that
  provider object, inside `SpawnSession`.
- `backend/factory/src/lib.rs:174-177` exports `get_spawn_session` as a public
  query without caller authorization.
- `apps/indexer/src/routes/spawn-sessions.ts:54-103` persists the complete
  factory session and includes it in realtime events.
- `apps/indexer/src/store/schema.sql:58-71` stores the complete session JSON.
- `apps/indexer/src/integrations/factory-canister-adapter.ts:894-909` maps both
  credential values from a factory response.
- `backend/factory/src/state.rs:711-715` clears keys only after they have already
  lived in the public session record.

Current load-bearing shape:

```rust
// backend/factory/src/types.rs:121-130
pub struct ProviderConfig {
    pub open_router_api_key: Option<String>,
    pub model: Option<String>,
    pub brave_search_api_key: Option<String>,
    pub inference_transport: InferenceTransport,
    pub open_router_reasoning_level: OpenRouterReasoningLevel,
}
```

Factory storage uses stable-memory IDs declared at
`backend/factory/src/state.rs:20-33`. Never reuse an existing ID. Match the
existing Candid `Storable` pattern at `state.rs:37-54` for any new stable type.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Factory tests | `cargo test -p factory` | 80 existing tests plus new tests pass |
| JS tests | `npm test` | all workspace tests pass |
| Lint | `npm run lint` | exit 0, no TypeScript/Rust lint errors |
| EVM tests | `npm run evm:test` | all Foundry tests pass |
| Secret-shape scan | `rg -n 'openRouterApiKey|braveSearchApiKey|open_router_api_key|brave_search_api_key' apps/indexer/src/routes apps/indexer/src/store` | no response/persistence mapping matches; request-only handling may remain elsewhere |

## Scope

**In scope**:

- `backend/factory/src/types.rs`
- `backend/factory/src/state.rs`
- `backend/factory/src/init.rs`
- `backend/factory/src/spawn.rs`
- `backend/factory/src/escrow.rs`
- `backend/factory/src/expiry.rs`
- `backend/factory/src/api/public.rs`
- `backend/factory/src/lib.rs`
- `backend/factory/factory.did`
- `packages/shared/src/spawn.ts`
- `packages/shared/test/contracts.test.ts`
- `apps/indexer/src/integrations/factory-canister-adapter.ts`
- `apps/indexer/src/integrations/factory-client.ts`
- `apps/indexer/src/routes/spawn-sessions.ts`
- `apps/indexer/src/store/schema.sql`
- `apps/indexer/src/store/sqlite.ts`
- Existing factory/indexer/web tests that construct spawn requests or sessions

**Out of scope**:

- Changing which provider fields the spawn wizard accepts.
- Changing the child automaton's `SpawnProviderBootstrapArgs` wire shape.
- Encrypting secrets at rest. This plan prevents public projection and
  off-canister persistence; encryption is a separate design decision.
- Logging, returning, or placing example credential values in snapshots.
- Repository moves or deployment workflow changes.

## Git workflow

- Branch: `advisor/001-redact-spawn-provider-secrets`
- Use small conventional commits, for example
  `fix(factory): isolate spawn provider secrets` and
  `test(indexer): reject provider secrets in projections`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Split public provider configuration from private spawn material

In `backend/factory/src/types.rs`:

1. Remove credential fields from the provider type stored in `SpawnConfig`.
2. Add a separate `SpawnProviderSecrets` record containing only the two optional
   credential fields.
3. Add `provider_secrets` to `CreateSpawnSessionRequest`, not to
   `SpawnSession`, `SpawnSessionStatusResponse`, registry records, audit records,
   or public config snapshots.
4. Keep model, transport, and reasoning level in the public provider config.
5. Update the TypeScript request type in `packages/shared/src/spawn.ts` to match.
   Use distinct request and response provider types; do not reuse one type for
   both directions.

**Verify**: `cargo test -p factory types:: -- --nocapture` -> exit 0.

### Step 2: Store secrets in a private stable map keyed by session ID

In `backend/factory/src/state.rs`:

1. Allocate the next unused stable-memory ID for a
   `StableBTreeMap<String, SpawnProviderSecrets, _>`.
2. Add narrowly scoped functions: insert at session creation, read only during
   child install/retry, and delete on successful install, refund, and terminal
   expiry/abandonment.
3. Never add this map to `FactoryStateSnapshot`, health responses, config
   responses, debug formatting, audit entries, or room messages.
4. Preserve secrets for retryable paid spawn failures until retry succeeds or
   the session becomes terminal.
5. If the storage schema version changes, use a new version and a deterministic
   empty-map initialization. Do not reuse a memory ID.

**Verify**: add a factory test that snapshots/reloads state and proves the public
session is redacted while the private secret map remains available for retry;
then run `cargo test -p factory state:: -- --nocapture` -> all pass.

### Step 3: Feed private secrets into child install args without public projection

Update session creation and `build_automaton_install_args` call sites:

1. Strip `provider_secrets` before constructing the stored `SpawnSession`.
2. Persist the private value under the new session ID.
3. At child install time, read the private value and construct the existing
   child bootstrap provider record expected by `ic-automaton`.
4. Delete the private value only after child install/bootstrap has consumed it
   successfully, or when refund/expiry makes retry impossible.
5. Ensure error strings contain field names only, never credential contents.

Model tests after `backend/factory/src/init.rs` tests that decode real child init
args. Add cases for direct transport, proxy transport, retry, success cleanup,
refund cleanup, and expiry cleanup.

**Verify**: `cargo test -p factory` -> all tests pass.

### Step 4: Redact all factory, indexer, SQLite, realtime, and web responses

1. Regenerate or manually update `backend/factory/factory.did` from Rust so the
   create request accepts private material but returned sessions do not contain
   credential fields.
2. Split request and response Candid types in
   `factory-canister-adapter.ts`; the create encoder may accept secrets, while
   response mappers must not expose them.
3. Ensure `session_json`, REST list/detail responses, and realtime events use
   the public response type only.
4. Update shared TS contracts and fixtures accordingly.
5. Add an indexer regression test that submits recognizable sentinel strings as
   credentials, then asserts those strings are absent from serialized session,
   SQLite `session_json`, list/detail REST JSON, and realtime event JSON.

**Verify**: `npm test` -> all tests pass, including the sentinel non-disclosure
test.

### Step 5: Run the complete validation gate

Run, in order:

```bash
npm run lint
npm test
cargo test -p factory
npm run evm:test
```

**Verify**: each command exits 0. Inspect `git diff` and confirm no credential
example or runtime secret was introduced.

## Test plan

- Factory request accepts both optional secret fields.
- Public create and get responses contain no credential fields or values.
- Private secrets survive a retryable spawn failure and state reload.
- Successful install, refund, and terminal expiry delete private secrets.
- Child install args still receive the correct private values.
- Indexer REST, SQLite, and realtime serialization cannot contain sentinel
  credential values.
- Existing direct and proxy transport spawn tests remain green.

## Done criteria

- [ ] `cargo test -p factory` exits 0.
- [ ] `npm test` exits 0.
- [ ] `npm run lint` exits 0.
- [ ] No public Candid response type contains either credential field.
- [ ] No indexer persistence or response model contains either credential field.
- [ ] New tests prove retry retention and terminal cleanup.
- [ ] `git diff` contains no credential values.
- [ ] `plans/README.md` row 001 is `DONE`.

## STOP conditions

- The current code no longer stores `ProviderConfig` inside `SpawnSession`.
- A private secret map would require reusing an occupied stable-memory ID.
- A retry path cannot distinguish retryable from terminal sessions.
- The child init wire shape must change to complete the fix.
- Any verification command fails twice after one focused correction.
- The worktree contains unrelated edits in an in-scope file that cannot be
  preserved cleanly.

## Maintenance notes

Reviewers should trace the full secret lifetime: browser request -> factory
private stable map -> child init args -> deletion. Future response-model changes
must keep request secrets separate from public session data. Treat any logging,
indexing, realtime broadcasting, or debug serialization of the private map as a
security regression.
