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
- Kept: shortened `SECTION_SEPARATOR` from blank-line padded form to `\n---\n` and made the benchmark read the separator from source. Result: `prompt_bytes_total=12223` (`full=8579`, `compact=3644`, `lines_total=199`).
- Kept: removed the explicit `- none active` line from the no-active-skills case in layer 5 rendering; the heading alone is enough. Result: `prompt_bytes_total=12195` (`full=8565`, `compact=3630`, `lines_total=197`).
- Kept: condensed high-volume wording in layers 5-7 (especially layer 6 section labels and repeated phrasing) without changing tested directives or prompt structure. Result: `prompt_bytes_total=11944` (`full=8375`, `compact=3569`, `lines_total=197`).
- Kept: trimmed layer 0/1 formatting plus small wording cuts in layers 4, 8, and 9. Result: `prompt_bytes_total=11905` (`full=8345`, `compact=3560`, `lines_total=195`).
- Kept: collapsed layers 6 and 7 from multi-line substeps into compact labeled bullets. Semantics stayed intact and checks passed. Result: `prompt_bytes_total=11834` (`full=8274`, `compact=3560`, `lines_total=166`).
- Kept: collapsed layer 5 into four labeled bullets (`Capability`, `Constraints`, `Dialogue`, `Memory`) while preserving all tested directives. Result: `prompt_bytes_total=11762` (`full=8238`, `compact=3524`, `lines_total=128`).
- Kept: further collapsed layers 0, 2, 3, 4, and 8 by merging related bullets and removing extra line overhead. Result: `prompt_bytes_total=11718` (`full=8213`, `compact=3505`, `lines_total=120`).
- Kept: merged adjacent safety/self-mod bullets in layers 1 and 9 for small additional savings without changing behavior. Result: `prompt_bytes_total=11712` (`full=8209`, `compact=3503`, `lines_total=117`).
- Kept: removed a blank line in layer 1, merged layer 6 risk/value into one bullet, and compressed the static Active Skills guidance into one line. Result: `prompt_bytes_total=11703` (`full=8203`, `compact=3500`, `lines_total=112`).
