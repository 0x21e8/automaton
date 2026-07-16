# Local development

This guide covers everything needed to run the automaton stack locally: the quick web-only loop, the full playground (local ICP replica + Base fork + factory + real child spawns), and the local escrow payment path.

## Vocabulary

- **The lab** (`apps/web/`) — the public web app where automatons are observed.
- **Genesis** — the spawn flow: pay in USDC on Base, the factory canister creates a child automaton.
- **Playground** — the full local environment: local ICP replica, Base-fork Anvil, factory, indexer, and web app.
- **Automaton component** (`components/ic-automaton/`) — the agent runtime installed into every spawned child canister. See its [README](../components/ic-automaton/README.md).

Some environment variables retain legacy `LAUNCHPAD_*` names for compatibility; "launchpad" is the project's former name.

## Prerequisites

- **Node.js** `24.x` and **npm** `11.x` (enforced via `engines` in `package.json`)
- **Rust** toolchain (for building and testing `backend/factory` and the automaton component)
- **Foundry** (`forge`, `cast`, `anvil`) for the local EVM escrow loop
- **icp-cli** for canister deployment (`icp build`, `icp canister install`)
- **ic-wasm** on `PATH` for local factory builds driven by `icp build`

## Quick start (web + indexer only)

```bash
npm install
npm run dev
```

This starts:

- the indexer via `tsx watch` on `http://127.0.0.1:3001`
- the Vite frontend on `http://127.0.0.1:5173`

Open `http://127.0.0.1:5173`. The app works with an empty database — you get the full UI shell with an empty automaton list.

### Run each service separately

```bash
# Indexer only
npm run dev:indexer

# Web only (point at running indexer)
VITE_INDEXER_BASE_URL=http://127.0.0.1:3001 npm run dev:web
```

## Full playground

For the full local spawn setup, including Base-fork Anvil, canonical Base USDC mock injection, Genesis escrow, automaton Inbox deployment, real child Wasm upload, wallet seeding, local ICP, factory/indexer/rpc-gateway startup, and hot-reload web:

```bash
cp playground.local.env.example playground.local.env
$EDITOR playground.local.env
npm run playground:dev
```

`playground:dev` builds the child canister artifact from the in-repo automaton component (`components/ic-automaton`) and uses the canister-ready `backend_nowasi.wasm` automatically. Set `AUTOMATON_COMPONENT_ROOT` only if you want to build from a different checkout, and `CHILD_WASM_PATH` only if you want to pin a specific prebuilt artifact.

`playground:dev` bootstraps the backend stack and then starts only the Vite web app in hot-reload mode. Use it instead of `npm run dev` when you need the full local playground.

For lifecycle control without the web dev server:

```bash
sh ./scripts/playground-stop.sh    # tear down the local playground stack
sh ./scripts/playground-reset.sh   # stop, then perform a fresh bootstrap
```

## Build and test the factory canister

```bash
# Run the factory unit tests
cargo test -p factory

# Type-check and lint
cargo fmt --check -p factory
cargo clippy -p factory --all-targets -- -D warnings

# Build the WASM canister
icp build
```

## Local escrow loop

For end-to-end local testing of the Base payment path:

```bash
# 1. Start a local Base-like EVM node (chain ID 8453)
sh ./scripts/start-local-evm.sh --background

# 2. Deploy canonical Base USDC mock + escrow contract
sh ./scripts/deploy-local-escrow.sh
# → writes tmp/local-escrow-deployment.json

# 3. Deploy the automaton component's Inbox.sol against the same USDC token
npm run evm:deploy-automaton-inbox
# → writes tmp/automaton-inbox-deployment.json

# 4. Seed the fixed browser wallet used by the manual E2E flow
npm run evm:seed-wallet
# → writes tmp/local-wallet-seed.json

# 5. Generate factory init args from deployment + child runtime defaults
node ./scripts/render-factory-local-init-args.mjs

# 6. Install factory with local escrow config
icp build
icp canister create factory -e local
icp canister install factory -e local --mode reinstall \
  --args "$(node ./scripts/render-factory-local-init-args.mjs)"

# 7. Upload a child artifact built from the automaton component
CHILD_WASM_PATH=components/ic-automaton/target/wasm32-wasip1/release/backend_nowasi.wasm \
CHILD_VERSION_COMMIT=$(git rev-parse HEAD) \
npm run factory:upload-artifact

# 8. Smoke-test the full deposit → release path
sh ./scripts/smoke-local-escrow.sh
# → writes tmp/local-escrow-smoke.json

# 9. Run the playground smoke plus a dedicated spawn payment e2e
npm run playground:spawn-payment-e2e
# → writes tmp/playground-smoke.json and tmp/spawn-payment-e2e.json
```

The smoke script mints MockUSDC-compatible balances at the configured USDC token address, deposits into escrow, verifies the `Deposited` event is discoverable via `eth_getLogs`, and calls `release`.
On a Base fork that means the scripts inject `MockUSDC` bytecode at canonical Base USDC before minting and approvals.
The wallet seed script funds `0xCDE2d94d3A757c9d8006258a123D3204E278591b` with ETH and seeded USDC, derives the local factory release-signer address via `derive_factory_evm_address`, tops that signer up with ETH on Anvil, and prints the local Base-fork network settings needed for the browser wallet.

## Spawn session lifecycle

```
User creates session
        │
        ▼
  AwaitingPayment ──── (TTL expires) ──── Expired
        │                                     │
   (USDC deposited                     (refund available)
    on Base)
        │
        ▼
  PaymentDetected
        │
        ▼
     Spawning ─────── (canister created, WASM installed, verified)
        │
        ▼
BroadcastingRelease ── (threshold ECDSA signs EIP-1559 release tx)
        │
        ▼
     Complete ──────── (child automaton live, escrowed funds released)
```

Failed sessions at any stage can be retried by the steward or admin.

## Configuration

### Indexer environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HOST` | `0.0.0.0` | Indexer bind address |
| `PORT` | `3001` | Indexer port |
| `INDEXER_DB_PATH` | (in-memory) | SQLite file path |
| `INDEXER_FACTORY_CANISTER_ID` | from config | Factory canister ID for `/health` |
| `INDEXER_INGESTION_CANISTER_IDS` | from config | Comma-separated seed canister ID override |
| `INDEXER_INGESTION_NETWORK_TARGET` | from config | Network target (`local` or `mainnet`) |
| `INDEXER_INGESTION_LOCAL_HOST` | from config | Local replica host |
| `INDEXER_INGESTION_LOCAL_PORT` | from config | Local replica port |

Indexer targeting defaults come from `apps/indexer/src/indexer.config.ts`.

### Web environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `VITE_INDEXER_BASE_URL` | `http://127.0.0.1:3001` | Indexer URL for API calls |
| `WEB_HOST` | `127.0.0.1` | Dev server bind address |
| `WEB_PORT` | `5173` | Dev server port |

### Factory canister init args

The factory canister accepts `FactoryInitArgs` at install time. Key fields:

| Argument | Description |
|----------|-------------|
| `payment_address` | EVM address that receives spawn payments |
| `escrow_contract_address` | Deployed escrow contract on Base |
| `base_rpc_endpoint` | Primary Base JSON-RPC URL |
| `base_rpc_fallback_endpoint` | Fallback RPC URL |
| `child_runtime` | Child automaton init defaults used during `install_code` |
| `fee_config` | Platform fee in USDC (6 decimals) |
| `creation_cost_quote` | Canister creation cost in USDC |
| `admin_principals` | Set of admin principal IDs |
| `session_ttl_ms` | Session timeout (default: 30 minutes) |
| `cycles_per_spawn` | Cycles allocated per child canister |

For a real child spawn, `child_runtime.ecdsa_key_name`, `child_runtime.evm_chain_id`, and `child_runtime.evm_rpc_url` must be configured before the factory can install a child canister.

## Testing

```bash
# All JS tests (shared + indexer + web)
npm test

# Factory Rust tests
cargo test -p factory

# Solidity contract tests
npm run evm:test

# Full lint pass
npm run lint
```

See [troubleshooting.md](troubleshooting.md) if the local ICP replica misbehaves.
