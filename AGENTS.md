# Agent Instructions

## Advisor-plan orchestration

- Invoke `.codex/agents/advisor-plan-executor.toml` and `.codex/agents/best-practice-reviewer.toml` as registered profiles. A generic subagent with a matching task name is not equivalent.
- Require runtime provenance before advisor-plan worktree access: executor `gpt-5.3-codex-spark`/high; reviewer `gpt-5.6-sol`/medium. Stop if runtime provenance is unavailable or mismatched.
- When several advisor plans share a worktree, only one executor may modify it at a time. The parent owns review-cycle exceptions, `IMPLEMENTATION_LEARNINGS.md`, explicit staging, and the one-plan/one-commit boundary.
- Run `npm run verify:advisor-process` after changing either registered profile or this routing section.
