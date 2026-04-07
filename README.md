# automaton-launchpad

A self-service launchpad for spawning autonomous [ic-automaton](https://github.com/domwoe/ic-automaton) agents on the Internet Computer. Users connect a wallet, pay in USDC on Base, and the factory canister creates a fully provisioned child automaton canister — complete with EVM address, on-chain configuration, and release of escrowed funds.

## What is in this repo

| Layer | Path | Tech | Purpose |
|-------|------|------|---------|
| **Factory canister** | `backend/factory/` | Rust · IC CDK · stable-structures | On-chain spawn orchestrator: session lifecycle, escrow polling, child canister creation, threshold ECDSA release transactions |
| **Web app** | `apps/web/` | React · Vite · TypeScript | Spawn wizard UI, automaton canvas, drawer detail view, CLI command panel |
| **Indexer** | `apps/indexer/` | Fastify · SQLite · WebSocket | Polls factory canister, normalizes data, serves REST + realtime updates to the frontend |
| **Evaluator backend** | `apps/evaluator/` | Fastify · TypeScript | Boots a fresh playground, runs experiment fleets, samples evidence, writes evaluation artifacts |
| **Evaluator dashboard** | `apps/evaluator-web/` | React · Vite · TypeScript | Operator console for one active evaluation run, fleet metrics, stop control, recent event feed |
| **Shared contracts** | `packages/shared/` | TypeScript | Shared types and validation between web and indexer |
| **EVM contracts** | `evm/` | Solidity · Foundry | MockUSDC and LocalEscrow for local development of the Base payment path |

## Architecture

```
                         ┌──────────────────┐
                         │   Web Frontend   │
                         │   (React/Vite)   │
                         └────────┬─────────┘
                                  │ REST + WebSocket
                                  ▼
                         ┌──────────────────┐
                         │    Indexer        │
                         │ (Fastify/SQLite) │
                         └────────┬─────────┘
                                  │ Candid (agent-js)
                                  ▼
┌─────────────────────────────────────────────────────────────┐
│                    Factory Canister (Rust)                   │
│                                                             │
│  Spawn Session FSM:                                         │
│  AwaitingPayment → PaymentDetected → Spawning               │
│      → BroadcastingRelease → Complete                       │
│                                                             │
│  ┌────────────┐  ┌────────────┐  ┌──────────────────┐      │
│  │ Scheduler  │  │  Escrow    │  │  EVM / ECDSA     │      │
│  │ (30s tick) │  │  Poller    │  │  Release Signer  │      │
│  └────────────┘  └────────────┘  └──────────────────┘      │
│                                                             │
│  Stable Memory: sessions, claims, registry, scheduler jobs  │
└──────────────────────────────┬──────────────────────────────┘
                               │
              ┌────────────────┼────────────────┐
              ▼                ▼                 ▼
     ┌──────────────┐  ┌────────────┐  ┌──────────────────┐
     │ Child        │  │  Base L2   │  │  IC Management   │
     │ Automatons   │  │ (Escrow +  │  │  Canister        │
     │ (ic-automaton│  │  USDC)     │  │  (create/install │
     │  canisters)  │  │            │  │   /update_settings)
     └──────────────┘  └────────────┘  └──────────────────┘
```

## Project structure

```
automaton-launchpad/
├── backend/factory/
│   ├── src/
│   │   ├── lib.rs              # Canister entrypoint, heartbeat, Candid API
│   │   ├── state.rs            # Stable storage layer (StableBTreeMap ↔ heap)
│   │   ├── types.rs            # Shared domain types, Candid-serializable
│   │   ├── spawn.rs            # Spawn execution: create → install → verify → release
│   │   ├── escrow.rs           # Base escrow payment polling and reconciliation
│   │   ├── evm.rs              # EIP-1559 tx construction, ECDSA signing, release broadcast
│   │   ├── base_rpc.rs         # JSON-RPC client for Base (eth_blockNumber, eth_getLogs, eth_sendRawTransaction)
│   │   ├── scheduler.rs        # Job scheduler with backoff, lease, retry
│   │   ├── session_transitions.rs  # Session state machine transitions + audit
│   │   ├── controllers.rs      # Canister controller handoff (factory → child)
│   │   ├── cycles.rs           # Cycle cost guards for outcalls and signing
│   │   ├── init.rs             # Child automaton initialization and EVM address derivation
│   │   ├── expiry.rs           # Session TTL enforcement
│   │   ├── retry.rs            # Failed session retry logic
│   │   └── api/
│   │       ├── admin.rs        # Admin endpoints (health, runtime, artifact, fees)
│   │       └── public.rs       # Public endpoints (create session, get status, list registry)
│   ├── factory.did             # Candid interface definition
│   └── Cargo.toml
├── apps/web/                   # React SPA
│   └── src/
│       ├── App.tsx             # Shell: header, canvas, drawer, spawn wizard
│       ├── components/
│       │   ├── spawn/          # SpawnWizard (6-step modal), FundStep, etc.
│       │   ├── drawer/         # AutomatonDrawer, MonologuePanel, CommandLinePanel
│       │   └── grid/           # AutomatonCanvas
│       ├── hooks/              # useAutomatonDetail, useSpawnSession, useCommandSession
│       ├── lib/                # CLI registry, wallet helpers, spawn payment logic
│       └── styles.css          # Single flat CSS file (BEM-ish)
├── apps/indexer/               # Fastify + SQLite service
│   └── src/
│       ├── server.ts           # HTTP + WebSocket server
│       ├── config.ts           # Typed configuration with env overrides
│       ├── polling/            # Factory canister polling loop
│       ├── integrations/       # Candid client adapter
│       ├── normalize/          # Raw canister data → domain model
│       ├── store/              # SQLite schema and queries
│       └── routes/             # REST endpoints (health, spawn-sessions)
├── packages/shared/            # Shared TypeScript types
├── evm/                        # Foundry workspace
│   ├── src/MockUSDC.sol        # 6-decimal mock USDC for local testing
│   ├── src/LocalEscrow.sol     # Escrow: deposit(bytes32,uint256), release(bytes32,address)
│   └── test/LocalEscrow.t.sol  # Forge tests
├── scripts/                    # Dev tooling
├── icp.yaml                    # ICP canister build & deployment config
└── package.json                # npm workspaces root
```

## Prerequisites

- **Node.js** `24.x` and **npm** `11.x`
- **Rust** toolchain (for building and testing `backend/factory`)
- **Foundry** (`forge`, `cast`, `anvil`) for the local EVM escrow loop
- **icp-cli** for canister deployment (`icp build`, `icp canister install`)

## Quick start

```bash
git clone <repo-url>
cd automaton-launchpad

# Install JS dependencies
npm install

# Start web + indexer in dev mode
npm run dev
```

This starts:

- the indexer via `tsx watch` on `http://127.0.0.1:3001`
- the Vite frontend on `http://127.0.0.1:5173`

Open `http://127.0.0.1:5173`. The app works with an empty database — you get the full UI shell with an empty automaton list.

## Local Evaluation Harness

The evaluation harness uses the full local playground plus a separate operator stack:

- `apps/evaluator` boots a fresh playground, validates the experiment, spawns the fleet, samples evidence every `15s`, and writes artifacts to `tmp/evaluations/<runId>/`
- `apps/evaluator-web` shows the live run dashboard and exposes the stop control

Before starting the harness, create a repo-root `.env` from `.env.example`:

```bash
cp .env.example .env
$EDITOR .env
```

The evaluator expects the following keys:

```dotenv
EVAL_STEWARD_ADDRESS=0x...
EVAL_OPENROUTER_API_KEY=...
LOCAL_EVM_FORK_URL=https://...
IC_AUTOMATON_REPO=/absolute/path/to/ic-automaton

# Optional
EVAL_BRAVE_SEARCH_API_KEY=
```

Use `eval` during implementation. It runs the evaluator backend in watch mode, bootstraps the playground stack (including the indexer), serves the operator dashboard from Vite, and serves the main launchpad web app:

```bash
npm run eval -- --experiment evaluations/experiments/smoke.yaml
```

`eval:dev` remains available as an explicit alias for the same workflow.

Default local endpoints:

- evaluator API: `http://127.0.0.1:3003`
- evaluator dashboard: `http://127.0.0.1:4173`
- launchpad web: `http://127.0.0.1:5173`
- playground indexer: `http://127.0.0.1:3001`
- artifacts: `tmp/evaluations/<runId>/`

Use `eval:run` for a cleaner one-command local run without watch mode. It builds the required workspaces first, then starts the evaluator backend plus preview-served dashboard and launchpad web:

```bash
npm run eval:run -- --experiment evaluations/experiments/smoke.yaml
```

Manual stop is handled from the dashboard. Press `Stop Run` in the operator console to finalize the current run, write `manifest.json`, `events.ndjson`, `samples/*.jsonl`, `summary.json`, and `report.md`, and tear the playground down cleanly.

Both scripts accept the following optional overrides:

- `EVALUATOR_HOST` / `EVALUATOR_PORT` for the backend bind address
- `EVALUATOR_WEB_HOST` / `EVALUATOR_WEB_PORT` for the dashboard server
- `LAUNCHPAD_WEB_HOST` / `LAUNCHPAD_WEB_PORT` for the main launchpad web server
- `LAUNCHPAD_INDEXER_BASE_URL` to point the launchpad web at a non-default indexer origin
- `EVALUATOR_ARTIFACTS_ROOT` to move run outputs away from `tmp/evaluations`
- `VITE_EVALUATOR_BASE_URL` if you want the dashboard to target a different evaluator origin

Both web servers use strict ports. If `4173` or `5173` is already occupied, the eval wrapper exits instead of silently switching to a different port.

For the full local spawn setup, including Base-fork Anvil, canonical Base USDC mock injection,
launchpad escrow, sibling `ic-automaton` inbox deployment, real child Wasm upload, wallet seeding,
local ICP, factory/indexer/rpc-gateway startup, and hot-reload web:

```bash
cp playground.local.env.example playground.local.env
$EDITOR playground.local.env
npm run playground:dev
```

When `IC_AUTOMATON_REPO` is set, `playground:dev` builds the sibling child canister artifact
and uses the canister-ready `backend_nowasi.wasm` automatically. You only need to set
`CHILD_WASM_PATH` if you want to pin a different artifact.

`playground:dev` bootstraps the backend stack and then starts only the Vite web app in hot-reload mode.
Use it instead of `npm run dev` when you need the full local playground.

For lifecycle control without the web dev server:

```bash
sh ./scripts/playground-stop.sh
sh ./scripts/playground-reset.sh
```

`playground-stop.sh` tears down the local playground stack without rebooting it.
`playground-reset.sh` now reuses the same stop path and then performs a fresh bootstrap.

### Run each service separately

```bash
# Indexer only
npm run dev:indexer

# Web only (point at running indexer)
VITE_INDEXER_BASE_URL=http://127.0.0.1:3001 npm run dev:web
```

### Build and test the factory canister

```bash
# Run all 56 unit tests
cargo test -p factory

# Type-check and lint
cargo fmt --check -p factory
cargo clippy -p factory --all-targets -- -D warnings

# Build the WASM canister
icp build
```

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
| `child_runtime` | Child `ic-automaton` init defaults used during `install_code` |
| `fee_config` | Platform fee in USDC (6 decimals) |
| `creation_cost_quote` | Canister creation cost in USDC |
| `admin_principals` | Set of admin principal IDs |
| `session_ttl_ms` | Session timeout (default: 30 minutes) |
| `cycles_per_spawn` | Cycles allocated per child canister |

For a real child spawn, `child_runtime.ecdsa_key_name`, `child_runtime.evm_chain_id`, and
`child_runtime.evm_rpc_url` must be configured before the factory can install a child canister.

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

## Local escrow loop

For end-to-end local testing of the Base payment path:

```bash
# 1. Start a local Base-like EVM node (chain ID 8453)
sh ./scripts/start-local-evm.sh --background

# 2. Deploy canonical Base USDC mock + escrow contract
sh ./scripts/deploy-local-escrow.sh
# → writes tmp/local-escrow-deployment.json

# 3. Deploy the sibling ic-automaton Inbox.sol against the same USDC token
IC_AUTOMATON_REPO=/path/to/ic-automaton npm run evm:deploy-automaton-inbox
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

# 7. Upload a real child artifact built from the sibling ic-automaton repo
CHILD_WASM_PATH=/path/to/ic-automaton/target/wasm32-wasip1/release/backend_nowasi.wasm \
CHILD_VERSION_COMMIT=$(git -C /path/to/ic-automaton rev-parse HEAD) \
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
The wallet seed script funds `0xCDE2d94d3A757c9d8006258a123D3204E278591b` with ETH and seeded USDC,
derives the local factory release-signer address via `derive_factory_evm_address`, tops that
signer up with ETH on Anvil, and prints the local Base-fork network settings needed for the
browser wallet.

## Troubleshooting

### Local ICP says it is running, but the replica is actually dead

How it manifested:

- `icp network start --background` returned `Error: network 'local' is already running`
- `icp network ping local` failed with errors like:
  - `Error: no descriptor found for port 8000`
  - `Error: An error happened during communication with the replica: error sending request for url (http://localhost:8000/api/v2/status)`
- nothing useful was listening on `127.0.0.1:8000`, or only a stale launcher process remained after `pocket-ic` had already died

What happened:

- the local `icp-cli-network-launcher` / `pocket-ic` process crashed or was interrupted
- stale state was left behind in this repo’s `.icp/cache/networks/local` and in the shared `ICP_HOME` port descriptors
- after that, `icp` believed the project-local network still existed even though the replica was gone

Quick resolve:

```bash
# 1. Stop any stale launcher / pocket-ic processes if they still exist
pkill -f icp-cli-network-launcher || true
pkill -f pocket-ic || true

# 2. Clear the broken local-network metadata
rm -rf .icp/cache/networks/local
rm -f "${ICP_HOME:-$HOME/.icp}/port-descriptors/8000.json"
rm -f "${ICP_HOME:-$HOME/.icp}/port-descriptors/8000.lock"

# 3. Start the local network again and verify it
ICP_HOME=/tmp/icp-home icp network start --background
ICP_HOME=/tmp/icp-home icp network ping local
```

If you are using a non-default `ICP_HOME`, keep it consistent for every `icp` command in the session. Mixed homes can make the network look missing or make canister IDs disappear even though the replica is healthy.

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
