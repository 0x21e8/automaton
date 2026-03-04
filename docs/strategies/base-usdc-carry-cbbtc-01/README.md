# Base Morpho USDC Carry (`base-usdc-carry-cbbtc-01`)

Example strategy registered via a single recipe JSON.

## Files

- `recipe.json` — strategy recipe (register via `register_strategy_admin` or agent `register_strategy` tool)
- `simulate-enter_supply.json` — simulation payload for `simulate_strategy_action`
- `simulate-exit_supply.json` — simulation payload for `simulate_strategy_action`
- `execute-enter_supply.json` — execution payload for `execute_strategy_action`
- `execute-exit_supply.json` — execution payload for `execute_strategy_action`

## End-to-End Sequence

1. Register strategy (one call does everything — ABI ingestion, template creation, dry-run compile, activation):

```bash
icp canister call backend register_strategy_admin "$(cat recipe.json)"
```

2. Simulate before live execution:
- tool: `simulate_strategy_action`
- args: `simulate-enter_supply.json` or `simulate-exit_supply.json`

3. Execute when simulation passes:
- tool: `execute_strategy_action`
- args: `execute-enter_supply.json` or `execute-exit_supply.json`

4. Monitor outcomes:
- query: `get_strategy_outcome_stats`
- tool: `get_strategy_outcomes`

5. Emergency halt (if needed):
- method: `set_strategy_kill_switch_admin(key, true, reason)`

## Notes

- Replace `0x1111...1111` placeholders in simulate/execute payloads with the automaton EVM address.
- `typed_params.calls[*].args` for tuple ABI inputs must be JSON arrays in exact component order.
- `max_value_wei_per_call` and `template_budget_wei` are `"0"` in this recipe because Morpho `supply` and `withdraw` are `nonpayable` (no ETH value sent).
