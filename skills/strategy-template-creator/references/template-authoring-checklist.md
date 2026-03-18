# Strategy Recipe Authoring Checklist

## Required Fields

- `protocol`
- `primitive`
- `chain_id`
- `template_id`
- `contracts[]` with `role`, `address`, `abi_json`, `source_ref`
- `actions[]` with `action_id`, `calls`, `postconditions`

Optional but normally expected:

- `actions[].preconditions`
- `actions[].risk_checks`
- `contracts[].codehash`
- `max_value_wei_per_call`
- `template_budget_wei`

## Defaults

- `chain_id`: `8453` (Base)
- If `max_value_wei_per_call` is omitted, the runtime default is `100000000000000000`
- If `template_budget_wei` is omitted, the runtime default is `1000000000000000000`

Example:

```json
{
  "max_value_wei_per_call": "0",
  "template_budget_wei": "0"
}
```

## Minimal Action Shape

```json
{
  "action_id": "enter",
  "calls": [
    {
      "role": "router",
      "function": "swapExactTokensForTokens"
    }
  ],
  "preconditions": ["liquidity_ok"],
  "postconditions": ["wallet_usdc_delta_positive_expected"],
  "risk_checks": ["max_slippage_bps_obeyed"]
}
```

## Minimal Contract Shape

```json
{
  "role": "router",
  "address": "0x0000000000000000000000000000000000000000",
  "abi_json": "[{\"type\":\"function\",\"name\":\"swapExactTokensForTokens\",...}]",
  "source_ref": "https://example.com/deployments"
}
```

## Quality Gates Before Registration

- Every `contracts[]` address has a trusted `source_ref`.
- Every `contracts[]` entry includes raw `abi_json`; do not hand-author selectors.
- Every `actions[].calls[*]` references a valid contract `role` and ABI function `name`.
- `preconditions`/`postconditions` are concrete and testable.
- Every action has at least one postcondition.
- Recipe remains non-deployable until the callable surface and addresses are source-backed.

## Deprecated Path

- Full `StrategyTemplate` output with `key`, `version`, `status`, `contract_roles`, `call_sequence`, and `constraints_json` is legacy-only.
- Use it only if the user explicitly asks for the deprecated direct-ingestion path.
