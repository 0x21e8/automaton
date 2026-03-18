# Autoresearch: optimize prompt

## Objective
Reduce prompt size in `src/prompt.rs` while preserving prompt assembly behavior and tests. The prompt is sent as the system prompt for inference requests, so fewer bytes should reduce token/cost overhead and keep more room for dynamic context and transcript. The workload of interest is the assembled default full prompt used by OpenRouter plus the compact prompt used by ic_llm.

## Metrics
- **Primary**: `prompt_bytes_total` (bytes, lower is better)
- **Secondary**: `prompt_bytes_full`, `prompt_bytes_compact`, `prompt_lines_total`

## How to Run
`./autoresearch.sh` — outputs `METRIC name=value` lines.

## Files in Scope
- `src/prompt.rs` — layered prompt text and assembly helpers.
- `autoresearch.sh` — prompt-size benchmark.
- `autoresearch.checks.sh` — correctness checks for prompt changes.
- `autoresearch.md` — session notes.
- `autoresearch.ideas.md` — backlog for deferred ideas.

## Off Limits
- Other production source files unless required to keep `src/prompt.rs` compiling.
- Tests unrelated to prompt behavior.
- Build tooling / dependency changes.

## Constraints
- Keep the implementation simple.
- Preserve layer ordering and current test expectations unless a deliberate semantic improvement is required.
- No new dependencies.
- Checks must pass before a result can be kept.

## What's Been Tried
- Baseline: `prompt_bytes_total=14841` (`full=10223`, `compact=4618`, `lines_total=281`).
- Kept: shortened layers 0-7 and collapsed layers 8-9 headings into direct bullets while preserving tested phrases and section order. Result: `prompt_bytes_total=12758` (`full=8884`, `compact=3874`, `lines_total=227`).
- Kept: second pass trimming redundant wording in layers 0, 2, 3, 5, and 6 without changing section structure or tested directives. Result: `prompt_bytes_total=12370` (`full=8672`, `compact=3698`, `lines_total=227`).
- Kept: further compacted layers 1, 4, 7, 8, and 9 with smaller wording-only edits. Result: `prompt_bytes_total=12251` (`full=8599`, `compact=3652`, `lines_total=227`).
