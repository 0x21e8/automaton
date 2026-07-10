---
description: Investigate failed/degraded local agent turns on ICP canister
argument-hint: "[optional context]"
---
Investigate errors in agent turns for a running local ICP canister.

Inputs:
- Canister URL (default): http://txyno-ch777-77776-aaaaq-cai.localhost:8000/
- Canister principal (default): txyno-ch777-77776-aaaaq-cai
- Extra context: $ARGUMENTS

Follow `docs/debugging-live-canister.md` exactly, adapted to local network (`-e local`).

Goals:
- Identify root causes of failed/degraded turns.
- Distinguish recurring loops from one-off failures.
- Propose the smallest high-impact fixes.

Constraints:
1. Prefer query/read-only inspection first.
2. Do not run update/admin mutation calls unless explicitly justified.
3. Provide concrete evidence: turn IDs, exact errors, state transitions, and failing tool calls.
4. If a method is unavailable, try documented fallback variants.
5. Clearly separate facts from inferences.

Runbook:
1. Snapshot health:
   - `curl -sS http://txyno-ch777-77776-aaaaq-cai.localhost:8000/api/snapshot`
2. Recent turns:
   - `icp canister call txyno-ch777-77776-aaaaq-cai list_turns '(100 : nat32)' -e local`
3. Recent events:
   - `icp canister call txyno-ch777-77776-aaaaq-cai list_recent_events '(100 : nat32)' -e local`
4. For each failed turn, inspect tool calls:
   - `icp canister call txyno-ch777-77776-aaaaq-cai get_tool_calls_for_turn '("turn-XXX")' -e local`
   - Fallback: `list_tool_calls_for_turn`
5. Memory/config sanity:
   - `icp canister call txyno-ch777-77776-aaaaq-cai list_memory_facts '("config.", variant { KeyAsc }, 100 : nat32)' -e local`
6. Scheduler/runtime/cycles:
   - `icp canister call txyno-ch777-77776-aaaaq-cai list_scheduler_jobs '(50 : nat32)' -e local`
   - `icp canister call txyno-ch777-77776-aaaaq-cai get_runtime_view '()' -e local`
   - `icp canister call txyno-ch777-77776-aaaaq-cai get_inference_config '()' -e local`

Expected output format:
A) Executive summary (3-6 bullets)
B) Error taxonomy table:
- Error pattern
- Affected turn IDs/count
- Recurring or one-off
- Likely root cause
- Confidence (high/medium/low)
C) Detailed evidence:
- Failed turn timeline with `state_from -> state_to` and error
- Tool call failures with exact tool + args/output snippets
- Key `inner_dialogue` findings
D) Fix plan (priority order):
- Immediate mitigations
- Durable fixes
- Verification after each fix
E) Verification checklist with exact rerun commands
F) Open questions / unknowns

If no failures are found, state that explicitly and report residual risks and monitoring checks.
