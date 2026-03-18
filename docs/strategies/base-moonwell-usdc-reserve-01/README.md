# Base Moonwell USDC Reserve (`base-moonwell-usdc-reserve-01`)

Low-complexity Base reserve strategy for parking surplus USDC in Moonwell and withdrawing it when survival runway or reallocation policy requires.

## Files

- `recipe.json` — strategy recipe for `register_strategy_admin` or `register_strategy`
- `simulate-approve_usdc.json` — simulation payload for `approve_usdc`
- `simulate-enter_supply.json` — simulation payload for `enter_supply`
- `simulate-exit_supply.json` — simulation payload for `exit_supply`
- `execute-approve_usdc.json` — execution payload for `approve_usdc`
- `execute-enter_supply.json` — execution payload for `enter_supply`
- `execute-exit_supply.json` — execution payload for `exit_supply`

## End-to-End Sequence

1. Register strategy:

```bash
icp canister call backend register_strategy_admin "$(cat recipe.json)"
```

2. Describe the action schema first:
- tool: `describe_strategy_action`
- args: `{"key": ..., "action_id": "approve_usdc"}`, `{"key": ..., "action_id": "enter_supply"}`, or `{"key": ..., "action_id": "exit_supply"}`

3. Simulate before live execution:
- tool: `simulate_strategy_action`
- args: `simulate-approve_usdc.json`, `simulate-enter_supply.json`, `simulate-exit_supply.json`

4. Execute in order:
- `approve_usdc`
- `enter_supply`
- `exit_supply`

5. Monitor outcomes:
- query: `get_strategy_outcome_stats`
- tool: `get_strategy_outcomes`

6. Emergency halt:
- method: `set_strategy_kill_switch_admin(key, true, reason)`

## Notes

- Base USDC: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`
- Moonwell USDC market: `0xEdc817A28E8B93B03976FBd4a3dDBc9f7D176c22`
- `approve_usdc` is a prerequisite for first entry or allowance refresh.
- `typed_params.calls[*].args` uses named objects keyed by ABI parameter names.
- This is a Base (`8453`) strategy. Registration works on the local backend canister, but execution requires the runtime EVM chain to match Base rather than the default local Anvil `31337`.
