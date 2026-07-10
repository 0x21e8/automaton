# Plan 002: Execute Lab steward commands instead of simulating success

> **Executor instructions**: Follow every step and verification gate. Stop and
> report on any STOP condition; do not substitute a different signing scheme.
> Update this plan's status row in `plans/README.md` when complete.
>
> **Drift check (run first)**:
> `git diff --stat 09fdfe2..HEAD -- apps/web/src components/ic-automaton/src`
> Before plan 003, the embedded reference implementation is at
> `../ic-automaton/src/ui_app.js`; after plan 003 it is at
> `components/ic-automaton/src/ui_app.js`. If neither exists, stop.

## Status

- **Status**: DONE
- **Priority**: P0
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: destination commit `09fdfe2`, source commit `eda66fd`, 2026-07-10

## Why this matters

The Lab advertises three `steward_signed` commands, but its dispatch hook only
executes commands whose transport is `wallet`. Signed steward commands fall
through to a synchronous model that prints “accepted” without preparing,
signing, or submitting anything. The embedded console already contains the
correct prepare -> `personal_sign` -> execute -> nonce reconciliation flow.

## Current state

- `apps/web/src/lib/cli-command-registry.ts:131-154` declares
  `steward-send`, `steward-model`, and `steward-reasoning` with transport
  `steward_signed`.
- `apps/web/src/hooks/useCommandSession.ts:96-165` dispatches only
  `definition.transport === "wallet"` to an asynchronous executor.
- `apps/web/src/hooks/command-session-model.ts:557-605,833-845` returns
  acceptance text locally.
- `../ic-automaton/src/ui_app.js:1956-2069` is the reference for signing,
  bounded retry, and nonce reconciliation.
- `../ic-automaton/src/ui_app.js:2072-2215` defines the exact endpoint and body
  mappings for all three commands.
- `apps/web/src/wallet/useWalletSession.ts:43-58` exposes the EIP-1193
  `request` method needed for `personal_sign`.

Required signing behavior from the reference implementation:

```text
prepare endpoint -> receive proof_template + signing_payload
personal_sign([UTF-8 payload as 0x hex, connected steward address])
execute endpoint -> submit normalized command fields + proof + signature
on ambiguous transient failure -> compare latest next_nonce with prepared nonce
```

Do not sign JSON produced by the frontend. Sign only the exact
`signing_payload` returned by the canister.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Focused tests | `npm run test --workspace @ic-automaton/web -- src/lib/steward-command-executor.test.ts src/hooks/useCommandSession.test.ts` | all focused tests pass |
| Web tests | `npm run test --workspace @ic-automaton/web` | all web tests pass |
| Typecheck | `npm run lint --workspace @ic-automaton/web` | exit 0 |
| Full JS gate | `npm test` | all workspace tests pass |

## Scope

**In scope**:

- `apps/web/src/lib/steward-command-executor.ts` (create)
- `apps/web/src/lib/steward-command-executor.test.ts` (create)
- `apps/web/src/hooks/useCommandSession.ts`
- `apps/web/src/hooks/useCommandSession.test.ts` (create if absent)
- `apps/web/src/hooks/command-session-model.ts`
- `apps/web/src/hooks/command-session-model.test.ts`
- `apps/web/src/lib/cli-command-registry.ts`
- `apps/web/src/api/automaton.ts` only for typed steward endpoint helpers

**Out of scope**:

- Changing canister signing domains, proof hashes, nonce semantics, TTL, or
  endpoint paths.
- Changing EVM wallet connection or transaction behavior.
- Refactoring the embedded console.
- Adding new steward commands.
- Treating a rejected signature or failed HTTP call as success.

## Git workflow

- Branch: `advisor/002-execute-steward-signed-commands`
- Suggested commits:
  `fix(web): execute signed steward commands` and
  `test(web): cover steward nonce reconciliation`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Add typed prepare and execute contracts

In `steward-command-executor.ts`, define narrow types for:

- proof template fields: canister ID, chain ID, address, command hash, nonce,
  expiry string, and signature after signing;
- prepare responses for direct message, model, and reasoning;
- execute responses containing the applied result text;
- an executor context containing canister URL, connected address and chain,
  `WalletSession.request`, and a function to refresh steward status.

Use a single JSON request helper that rejects non-2xx responses and preserves
status information so transient failures can be classified. Do not add a new
HTTP library.

**Verify**: `npm run lint --workspace @ic-automaton/web` -> exit 0.

### Step 2: Port the generic signed-command transaction

Port the behavior from `ui_app.js:1956-2069` without copying UI rendering code:

1. POST the command-specific prepare body.
2. Validate that `proof_template` and non-empty `signing_payload` exist.
3. Verify the prepared address and chain match the connected steward context.
4. UTF-8 encode the exact payload and call `personal_sign` with parameters in
   the same order as the embedded console.
5. POST the command-specific execute payload with the returned signature.
6. Retry once only for a classified transient boundary/network failure.
7. If the response remains ambiguous, refresh steward status and report success
   only when `next_nonce >= prepared_nonce + 1`.
8. Return structured terminal entries; do not directly mutate React state.

**Verify**: focused unit tests cover happy path, user rejection, malformed
prepare response, execute 4xx, transient retry, nonce-reconciled success, and
unchanged-nonce failure.

### Step 3: Map the three commands exactly

Implement these mappings:

- `steward-send`: prepare `/api/steward/direct-message/prepare` with connected
  sender and message; execute `/api/steward/direct-message/execute`.
- `steward-model`: prepare `/api/steward/model/prepare` with model; execute
  `/api/steward/model/execute`.
- `steward-reasoning`: accept only `default|low|medium|high`; use the reasoning
  prepare and execute endpoints.

Use the endpoint request/response shapes from the embedded reference. Preserve
normalized values returned by prepare when constructing execute bodies.

**Verify**: `npm run test --workspace @ic-automaton/web -- src/lib/steward-command-executor.test.ts` -> all pass.

### Step 4: Dispatch `steward_signed` asynchronously

Update `useCommandSession.ts` so the dispatch order is explicit:

1. `wallet` -> existing `executeWalletCommand`.
2. `steward_signed` -> new `executeStewardCommand`.
3. `local` and `query` -> existing session model.

Remove the local “accepted” fallback for steward commands from
`command-session-model.ts`. If a steward command reaches the synchronous model,
return an internal routing error rather than success-like output.

**Verify**: hook tests assert that each steward command calls prepare,
`personal_sign`, and execute exactly once, and that no “accepted” output appears
without an execute response or nonce advancement.

### Step 5: Run the complete web and workspace gates

```bash
npm run lint --workspace @ic-automaton/web
npm run test --workspace @ic-automaton/web
npm test
```

**Verify**: all three commands exit 0.

## Test plan

- Exact endpoint/body mapping for all three commands.
- Wrong connected address or chain rejects before signing.
- Wallet rejection produces an error entry and no execute call.
- Prepare payload is signed byte-for-byte as UTF-8 hex.
- Execute 4xx is not retried.
- One transient execute failure is retried once.
- Lost execute response succeeds only after nonce advancement.
- Unchanged nonce remains a failure.
- Synchronous model cannot emit false steward success.

Use `apps/web/src/lib/wallet-command-executor.test.ts` as the pattern for a
mock EIP-1193 transport and structured terminal entries.

## Done criteria

- [ ] All three steward commands execute the real signed HTTP flow.
- [ ] No code path prints success merely because local authorization passed.
- [ ] `personal_sign` receives the canister-provided payload, not frontend JSON.
- [ ] Focused and full JS tests pass.
- [ ] Web typecheck passes.
- [ ] `plans/README.md` row 002 is `DONE`.

## STOP conditions

- Canister endpoint paths or body shapes differ from the embedded reference.
- The wallet provider cannot perform `personal_sign` with the existing request
  interface.
- Implementing this requires changing the signing domain or canister verifier.
- The connected chain/address cannot be checked from existing context.
- A verification command fails twice after one focused correction.

## Maintenance notes

The canister is the signing-payload authority. Any future signed command should
be added to the command registry, executor mapping, embedded console, and parity
tests together. Review retry logic carefully: replay protection comes from the
nonce, and a lost response must never trigger an unbounded resubmission loop.
