---
name: strategy-template-creator
description: Create and refine simplified strategy recipe artifacts for `ic-automaton` on Base (chain `8453`) using source-backed addresses, ABI JSON, risk checks, and validator-compatible budget fields. Use when asked to design custom strategies, produce annotated example strategies, convert market research into deployable recipe JSON, or harden existing strategy recipes before activation.
---

# Strategy Template Creator

Use this skill to turn opportunity research into safe, ingestible strategy recipes that prioritize automaton survival (income generation with controlled downside and preserved cycle/inference runway).

## Workflow

1. Define the strategy target.
- Choose one `primitive` and one profit engine (carry, liquidation, LP fees, PT roll-down, etc.).
- Default to `chain_id = 8453` unless the user explicitly changes chain.
- Ensure the strategy serves automaton needs first, not human convenience.

2. Gather fresh evidence from primary sources.
- Pull current TVL, liquidity, APY, and volume snapshots (include concrete date in output).
- Pull deployment addresses from protocol-owned docs/repos or canonical APIs.
- Treat all market stats as time-volatile; always refresh instead of relying on memory.

3. Draft simplified `StrategyRecipe` fields.
- Fill `protocol`, `primitive`, `chain_id`, `template_id`, `contracts`, and `actions`.
- Put raw `abi_json` on every contract entry; do not hand-author selectors.
- Add `source_ref` per contract role; avoid unreferenced addresses.
- Use the simplified recipe format consumed by `register_strategy_admin` / `register_strategy`.

4. Encode action safety.
- Add non-empty `preconditions`, `postconditions`, and `risk_checks`.
- Define explicit entry and exit conditions.
- Include at least one postcondition that protects or improves survival runway.

5. Apply recipe-compatible budget constraints.
- Use only recipe-level fields supported by the current registration path:
  - `max_value_wei_per_call`
  - `template_budget_wei`
- Keep wei-denominated values as decimal strings.
- If omitted, remember the runtime applies safe defaults rather than `constraints_json`.

6. Deliver in requested format.
- If user asks for examples: provide annotated templates with thesis, triggers, risks, and source links.
- If user asks for ingestible output: return raw JSON recipe objects ready for `register_strategy_admin` or `register_strategy`.
- If uncertainty remains on addresses or callable surfaces: call out the missing evidence and do not claim the recipe is deployable.

7. Legacy escape hatch.
- Only produce full `StrategyTemplate`-shaped output if the user explicitly asks for the deprecated ingestion path.
- When doing so, state clearly that `ingest_strategy_template_admin` is deprecated.

## Repo Contracts

- Simplified recipe model and registration path: `src/strategy/registry.rs`.
- Validation behavior and allowed constraint keys: `src/strategy/validator.rs`.
- Registry lifecycle semantics: `src/strategy/registry.rs`.

Use [template-authoring-checklist.md](references/template-authoring-checklist.md) for a compact checklist and defaults.
