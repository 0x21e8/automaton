# Strategies User Guide

This guide explains how to use the strategy subsystem safely and reliably.

## What a strategy is

A strategy template defines a reusable on-chain action plan:

- `key`: unique identity `(protocol, primitive, chain_id, template_id)`
- `contract_roles`: named role to verified address mapping
- `actions`: named call sequences (`action_id`) with ABI metadata
- `constraints_json`: execution safety limits

## Lifecycle and safety model

Templates move through these states:

- `Draft`: not approved for execution
- `Active`: executable (if activation is enabled)
- `Deprecated`: readable but intentionally disabled
- `Revoked`: permanently disabled

Execution must pass all safety gates:

- schema checks
- address and chain checks
- policy checks (activation, kill switch, budget limits, required postconditions)
- postcondition presence checks

Additional safety behavior:

- `simulate_strategy_action` compiles and validates without broadcasting
- `execute_strategy_action` broadcasts only if validation passes
- 3 consecutive deterministic failures auto-disable template activation

## Registration

Both the controller and the agent use the same recipe format — a single JSON that bundles contract ABIs, actions, and budget constraints.

### Controller workflow (`register_strategy_admin`)

```bash
icp canister call backend register_strategy_admin "$(cat recipe.json)"
```

One call does everything:
1. Validates addresses and selectors
2. Stores ABI artifacts
3. Creates a draft template
4. Runs dry-run compile
5. Auto-promotes to `Active` and enables activation on success

### Agent workflow (`register_strategy` tool)

The agent provides the same recipe JSON via the `register_strategy` tool. The same pipeline runs, but subject to tool policy (must be in `ExecutingActions` state) and sensitive-tool sequencing rules (cannot follow `http_fetch` directly).

### Safe defaults (when budget fields omitted)

- `max_calls`: `5`
- `max_value_wei_per_call`: `100000000000000000` (0.1 ETH)
- `template_budget_wei`: `1000000000000000000` (1 ETH)
- `max_total_value_wei`: same as `max_value_wei_per_call`

## Recipe format

```json
{
  "protocol": "erc20",
  "primitive": "transfer",
  "chain_id": 8453,
  "template_id": "agent-generated-transfer",
  "contracts": [
    {
      "role": "token",
      "address": "0x2222222222222222222222222222222222222222",
      "abi_json": "[{\"type\":\"function\",\"name\":\"transfer\",...}]",
      "source_ref": "https://example.com/token"
    }
  ],
  "actions": [
    {
      "action_id": "transfer",
      "calls": [{ "role": "token", "function": "transfer" }],
      "postconditions": ["balance_delta_positive"]
    }
  ]
}
```

Key properties:
- `contracts[].abi_json` — raw Solidity ABI JSON; the system extracts selectors, inputs, outputs automatically
- `actions[].calls` — reference contracts by `role` and functions by `name`; no manual selectors needed
- `preconditions`, `risk_checks` — optional per-action
- `postconditions` — required (at least one per action)
- `max_value_wei_per_call`, `template_budget_wei` — optional, safe defaults applied

## Execution payloads

### `describe_strategy_action`

For complex actions, start here so the runtime tells you the exact named
argument tree for a registered template action.

```json
{
  "key": {
    "protocol": "erc20",
    "primitive": "transfer",
    "chain_id": 8453,
    "template_id": "agent-generated-transfer"
  },
  "action_id": "transfer"
}
```

`describe_strategy_action` returns:

- the canonical `calls[]` order for the action
- the recursive named argument schema
- a preferred `typed_params` payload template
- workflow notes for describe -> simulate -> execute

### `simulate_strategy_action` / `execute_strategy_action`

```json
{
  "key": {
    "protocol": "erc20",
    "primitive": "transfer",
    "chain_id": 8453,
    "template_id": "agent-generated-transfer"
  },
  "action_id": "transfer",
  "typed_params": {
    "calls": [
      {
        "value_wei": "0",
        "args": {
          "to": "0x3333333333333333333333333333333333333333",
          "amount": "1000000"
        }
      }
    ]
  }
}
```

The field names inside `args` come from the registered ABI and should be taken
from `describe_strategy_action`; this example uses illustrative names.

`typed_params` rules:

- one `calls[]` entry per function in `action.call_sequence` (same order)
- call `describe_strategy_action` first for complex actions and copy its `preferred_typed_params`
- `calls[*].args` should be a JSON object keyed by normalized ABI parameter names
- tuple inputs should be nested JSON objects keyed by normalized component names
- arrays remain JSON arrays; arrays of tuples may contain tuple objects
- legacy positional arrays are still accepted temporarily, but docs and examples use named objects
- `value_wei` must be decimal or hex quantity string

### `get_strategy_outcomes`

```json
{
  "key": {
    "protocol": "erc20",
    "primitive": "transfer",
    "chain_id": 8453,
    "template_id": "agent-generated-transfer"
  }
}
```

For `list_strategy_templates` tool calls, `limit` defaults to `20` and is capped at `50`.

## Safety controls (controller only)

| Method | Purpose |
|---|---|
| `deprecate_strategy_template_admin(key, reason)` | Orderly disable (template stays readable) |
| `revoke_strategy_template_admin(key, reason)` | Permanent disable with immutable record |
| `set_strategy_kill_switch_admin(key, enabled, reason)` | Emergency override |

## Query API

| Method | Description |
|---|---|
| `list_strategy_templates(key_opt, limit)` | List templates |
| `get_strategy_template(key)` | Get single template |
| `get_strategy_outcome_stats(key)` | Get outcome statistics |

## Best practices

- For complex actions: describe first, simulate second, execute last.
- Simulate before every live execute.
- Keep `postconditions` explicit and action-specific.
- Keep budget limits strict.
- Use stable, traceable `source_ref` values for every contract role.
- Prefer non-overloaded functions when authoring recipes.
- Keep chain IDs aligned with runtime config (`evm_chain_id`).
- Treat kill switch as the first response for incident containment.
- Review outcome summaries regularly and re-activate only after root-cause fixes.

## Common errors

| Error | Likely fix |
|---|---|
| `strategy template status is not Active` | Template failed dry-run or was deprecated/revoked. |
| `strategy template activation is disabled` | Enable activation or disable kill switch. |
| `strategy kill switch is enabled` | Clear with `set_strategy_kill_switch_admin(..., false, ...)`. |
| `call count mismatch ... expected N got M` | Make `typed_params.calls` length match `action.call_sequence`. |
| `calls[0].args.... missing/unknown/...` | Re-run `describe_strategy_action` and match the returned named schema exactly. |
| `function ... is overloaded` | Use a non-overloaded ABI function in the recipe. |
| `template_budget_exceeded` | Increase budget or reduce execution value/volume. |

## Examples

- Recipe-based example: `docs/strategies/base-usdc-carry-cbbtc-01/`
