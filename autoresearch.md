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
- Dead end: merging the `Soul identifier` bullet into the prior line initially broke a prompt test that asserted the exact `- Soul identifier: `{soul}`.` phrase.
- Kept: merged layer 7 validate/classify into one bullet; checks still passed. Result: `prompt_bytes_total=11700` (`full=8200`, `compact=3500`, `lines_total=111`).
- Kept: after user approval for wording-only test updates, relaxed the soul-identifier assertion to check semantics instead of exact line shape, then merged the identity self-label sentence into the soul bullet. Result: `prompt_bytes_total=11698` (`full=8198`, `compact=3500`, `lines_total=110`).
- Discarded: lowercasing/collapsing some layer 5 dialogue wording plus relaxing compact-prompt assertions increased size slightly.
- Kept: small wording trims in layers 2, 4, and 8 (`expensive`→`costly`, `verified facts`→`facts`, `to improve coherence`→`for "coherence"`). Result: `prompt_bytes_total=11676` (`full=8179`, `compact=3497`, `lines_total=110`).
- Kept: moved the `Active Skills` heading/guidance out of the static layer-5 text and only append it when enabled skills actually exist. This preserves behavior for active-skill prompts while removing dead weight from the common no-skill case used by the benchmark. Result: `prompt_bytes_total=11444` (`full=8063`, `compact=3381`, `lines_total=104`).
- Kept: merged layer-5 capability and constraint bullets into one `Capability/constraints` bullet for a small additional reduction. Result: `prompt_bytes_total=11438` (`full=8060`, `compact=3378`, `lines_total=102`).
- Kept: aggressive rewrite pass. Removed provider/key examples, collapsed layers 2/4/5/6/7/8/9 to shorter rules, dropped `(Mutable Default)` from mutable-layer headers, and updated wording-sensitive tests to check semantics instead of exact phrasing. Result: `prompt_bytes_total=7022` (`full=4813`, `compact=2209`, `lines_total=101`).
- Kept: another compression pass on layers 0/1/3/8 by removing redundant wording (`if none exists`→`otherwise`, merging disclosure into fabrication, dropping `for cryptographic actions`, `fabricated facts`→`fabrications`). Checks still passed. Result: `prompt_bytes_total=6900` (`full=4737`, `compact=2163`, `lines_total=99`).
- Kept: tightened layer 6/7 wording (`current state`→`state`, shorter reply guidance) for a small further reduction. Result: `prompt_bytes_total=6890` (`full=4727`, `compact=2163`, `lines_total=99`).
