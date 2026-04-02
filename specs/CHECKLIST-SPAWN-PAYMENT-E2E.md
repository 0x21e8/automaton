# Spawn Payment E2E - Execution-Ready Checklist

**Status:** Proposed, execution-ready checklist
**Date:** 2026-04-01
**Source:** `scripts/playground-smoke.mjs`, `scripts/smoke-local-escrow.mjs`, `apps/indexer/src/routes/spawn-sessions.ts`
**Audience:** coding agents working in this repo
**Scope:** add a non-browser end-to-end spawn test on top of the existing playground smoke so we verify the real payment and spawn flow while a session is `awaiting_payment`

## Outcome

Prove, from a script-level test, that the system can:

- create a spawn session
- see the session in `awaiting_payment`
- pay the quoted USDC amount
- see the session leave `awaiting_payment` once the backend mirrors the confirmed payment
- complete the spawn pipeline against the real playground stack
- surface the resulting automaton registry record and release metadata

At the same time:

- the test should fail if payment is attempted outside `awaiting_payment`
- the test should fail if the factory never mirrors the confirmed payment
- the test should fail if the spawn pipeline cannot complete before the session deadline

## Locked Decisions

These decisions define the first non-browser e2e slice.

1. Reuse the existing playground smoke script as the backend precondition.
   Reason: it already proves the indexer, RPC gateway, faucet, and spawn flow work end to end.

2. Add a dedicated spawn e2e on top of the smoke, not instead of it.
   Reason: the smoke script should remain a high-signal integration harness, while the new test focuses on the payment/spawn regression boundary.

3. Drive the test with Node, `cast`, fetch calls, and the repo’s existing RPC helpers.
   Reason: the flow already uses real JSON-RPC and HTTP APIs, and that gives us end-to-end coverage without browser flake.

4. Keep the first e2e scenario focused on one happy path plus one gating assertion.
   Reason: this is a regression harness for the payment window, not a broad UI suite.

## Scope Guardrails

- Do not build a mock-only test that never touches the playground.
- Do not replace the existing smoke script.
- Do not widen this into a generic UI regression suite before the payment path is covered.
- Do not require manual wallet clicking.
- Do not turn this into a wallet-extension automation problem.

## Test Shape

### Phase 1 - Harness

- [ ] **E2E-01: Make the spawn test consume a running playground**
  - Files to create:
    - `scripts/spawn-payment-e2e.mjs`
    - `scripts/spawn-payment-e2e.sh`
  - Files to modify:
    - `package.json`
    - `scripts/playground-smoke.mjs`
  - Implement:
    - an e2e script that runs against the existing playground stack
    - env plumbing for `PLAYGROUND_INDEXER_BASE_URL`, `PLAYGROUND_RPC_GATEWAY_URL`, and `PLAYGROUND_PUBLIC_RPC_URL`
    - a precondition that either reuses `scripts/playground-smoke.mjs` output or recreates the same session/payment sequence directly
  - Done when:
    - the spawn test can run against a real deployed playground without hardcoded localhost assumptions

- [ ] **E2E-02: Add a wallet automation seam for the spawn test**
  - Files to create or modify:
    - `scripts/*`
    - `apps/indexer/test/*` if shared fixtures need to be extracted
  - Implement:
    - a deterministic wallet setup path for the e2e run
    - chain switching support for the playground network
    - a way to approve and submit the USDC/deposit flow from the test using JSON-RPC and `cast`
  - Important detail:
    - prefer a seeded ephemeral wallet or replayable private key over any browser-provider automation
  - Done when:
    - the test can submit the payment flow without manual intervention

### Phase 2 - Payment Assertions

- [ ] **E2E-03: Verify the session is payable only while it is awaiting payment**
  - Files to create or modify:
    - `scripts/spawn-payment-e2e.mjs`
  - Assert:
    - the session starts in `awaiting_payment`
    - the test can fund the session only while it is in that state
    - a second payment attempt is rejected or ignored once the session has advanced
  - Done when:
    - the script test proves the payment gating matches the factory/session state machine

- [ ] **E2E-04: Verify the script can submit payment and observe backend mirroring**
  - Files to create or modify:
    - `scripts/spawn-payment-e2e.mjs`
  - Assert:
    - the test can submit the approval transaction
    - the test can submit the deposit transaction
    - the indexer or factory API reports the confirmed payment
    - the script can observe the session after the backend mirror catches up
  - Done when:
    - the test proves the user-facing payment state is backed by real backend progression

- [ ] **E2E-05: Verify the session advances to spawn completion**
  - Files to create or modify:
    - `scripts/spawn-payment-e2e.mjs`
  - Assert:
    - the session eventually reflects `payment_detected`, `spawning`, `broadcasting_release`, or `complete`
    - the registry record exists once completion is reached
    - the release tx hash is surfaced by the backend read model
    - the session does not fall back into `expired` during a successful run
  - Done when:
    - the test covers the regression boundary: confirmed payment must not remain stuck in `awaiting_payment`

### Phase 3 - Operational Wiring

- [ ] **E2E-06: Wire the spawn e2e into repo commands**
  - Files to modify:
    - `package.json`
    - CI or playground scripts as needed
  - Implement:
    - a dedicated command for the spawn e2e
    - a clear separation between unit tests, backend smoke, and spawn e2e
  - Done when:
    - contributors can run the spawn e2e without guessing the stack order

- [ ] **E2E-07: Decide where the spawn e2e runs in CI**
  - Files to modify:
    - `.github/workflows/ci.yml`
    - playground deployment scripts if needed
  - Implement:
    - either a guarded nightly/manual job or a lightweight smoke gate if the runtime cost is acceptable
  - Done when:
    - the spawn e2e is not just a local-only script

## Suggested First Scenario

Use one happy path:

1. Start the playground stack and run the existing smoke.
2. Create a new spawn session with a faucet-funded wallet.
3. Assert the session is `awaiting_payment`.
4. Submit the approval transaction and deposit transaction with `cast`.
5. Poll the indexer or factory session endpoint until the payment is mirrored.
6. Poll until the session advances beyond `awaiting_payment`.
7. Verify the registry record and release transaction metadata are present once the spawn completes.

## Acceptance Criteria

This plan is complete when all of the following are true:

- the script test fails if payment is attempted from the wrong chain
- the script test fails if payment is attempted after the session is no longer payable
- the script test passes when a real session is paid while `awaiting_payment`
- the script test observes backend state transition after confirmed payment
- the script test runs on top of the existing playground smoke, not in isolation

## Risks

- The payment window can still be timing-sensitive in a real playground, so the test should create the session immediately before paying.
- If the test is made too broad, it will become flaky and stop protecting the specific regression we care about.

