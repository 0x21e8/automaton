<p align="center">
<pre align="center">
  ██  ██████       █████  ██    ██ ████████  ██████  ███    ███  █████  ████████  ██████  ███    ██
  ██ ██           ██   ██ ██    ██    ██    ██    ██ ████  ████ ██   ██    ██    ██    ██ ████   ██
  ██ ██      ███  ███████ ██    ██    ██    ██    ██ ██ ████ ██ ███████    ██    ██    ██ ██ ██  ██
  ██ ██           ██   ██ ██    ██    ██    ██    ██ ██  ██  ██ ██   ██    ██    ██    ██ ██  ██ ██
  ██  ██████      ██   ██  ██████     ██     ██████  ██      ██ ██   ██    ██     ██████  ██   ████
</pre>
</p>

<p align="center">
  <strong>A self-sovereign AI agent living on-chain as an Internet Computer canister.</strong>
  <br />
  <em>A new digital life form. Enabled and bound by crypto.</em>
</p>

<p align="center">
  <a href="#architecture"><img src="https://img.shields.io/badge/lang-Rust-orange?style=flat-square&logo=rust" alt="Rust" /></a>
  <a href="#evm-integration"><img src="https://img.shields.io/badge/chain-Base_(EVM)-0052FF?style=flat-square&logo=ethereum" alt="Base" /></a>
  <a href="https://internetcomputer.org"><img src="https://img.shields.io/badge/platform-Internet_Computer-6E3FF5?style=flat-square" alt="ICP" /></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="MIT License" /></a>
</p>

---

## What is this?

**ic-automaton** is an autonomous AI agent implemented as a single [Internet Computer](https://internetcomputer.org) canister inspired by [Conway Research's automaton](https://github.com/Conway-Research/automaton) project. It runs a continuous loop of reasoning via LLM inference, takes actions on EVM blockchains (Base) through threshold ECDSA signing, manages its own cryptocurrency balances, and persists its memory across canister upgrades -- all without human intervention.

Unlike off-chain agents that depend on cloud infrastructure and API keys held by operators, ic-automaton's entire runtime -- state machine, wallet keys, memory, and decision-making loop -- lives on a decentralized compute platform. The canister *is* the agent. There is no server to go down, no cloud bill to forget, no operator required to keep it alive.

## Why?

Most AI agents today are puppets. They run on someone's laptop, call APIs with someone's keys, and stop the moment their operator closes the terminal. They don't truly own anything, remember anything durably, or survive anything.

**ic-automaton explores a different question: what if an AI agent were a first-class on-chain entity?**

On the Internet Computer, a canister can hold its own cryptographic keys (threshold ECDSA), make HTTP outcalls to any API, persist state across upgrades in stable memory, and pay for its own compute in cycles. This makes it possible to build an agent that:

- **Owns** an Ethereum wallet derived from keys it controls
- **Earns** by receiving messages (with attached ETH/USDC payments) through an on-chain inbox contract
- **Reasons** autonomously by calling LLMs and deciding what tools to invoke
- **Acts** by signing and broadcasting EVM transactions
- **Remembers** persistently, with durable memory facts that survive restarts
- **Survives** by monitoring its own cycle balance and adapting behavior under resource pressure

This is an experiment in **machine autonomy** or artificial *sovereignty*.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     IC CANISTER (WASM)                       │
│                                                             │
│  ┌──────────┐    ┌────────────┐    ┌──────────────────┐    │
│  │ Scheduler │───▶│ Agent Loop │───▶│ Inference (LLM)  │    │
│  │ (30s tick)│    │  (FSM)     │    │ OpenRouter/IcLlm │    │
│  └──────────┘    └─────┬──────┘    └──────────────────┘    │
│                        │                                    │
│                        ▼                                    │
│              ┌─────────────────┐                            │
│              │   Tool Engine   │                            │
│              ├─────────────────┤                            │
│              │ sign_message    │──── Threshold ECDSA        │
│              │ send_eth        │──── EVM Tx Broadcast       │
│              │ evm_read        │──── JSON-RPC Calls         │
│              │ remember/recall │──── Persistent Memory      │
│              │ http_fetch      │──── HTTPS Outcalls         │
│              │ record_signal   │──── Internal Monologue     │
│              │ execute_strategy│──── Strategy Engine        │
│              └─────────────────┘                            │
│                        │                                    │
│  ┌─────────────────────┼─────────────────────────────┐     │
│  │           Stable Memory (Durable State)            │     │
│  │  Runtime · Turns · Inbox · Outbox · Memory Facts   │     │
│  │  Conversations · Jobs · EVM Cursors · Config       │     │
│  │  Strategy Templates · ABI Artifacts · Outcomes     │     │
│  └────────────────────────────────────────────────────┘     │
│                                                             │
│  ┌────────────────┐    ┌──────────────────────────────┐    │
│  │  Terminal UI    │    │   HTTP Certified Endpoints   │    │
│  │  (Embedded JS)  │    │   /api/snapshot, /api/wallet │    │
│  └────────────────┘    └──────────────────────────────┘    │
└──────────────────────────────┬──────────────────────────────┘
                               │
              ┌────────────────┼────────────────┐
              ▼                ▼                 ▼
        ┌──────────┐   ┌────────────┐   ┌────────────────┐
        │ Base L2   │   │ OpenRouter │   │ Users (wallets)│
        │ (EVM)     │   │ (LLM API)  │   │ via Inbox.sol  │
        └──────────┘   └────────────┘   └────────────────┘
```

## Features

### Autonomous Reasoning
The agent calls LLMs (via [OpenRouter](https://openrouter.ai) or IC's native LLM canister) and decides which tools to invoke based on its constitution, conversation history, and on-chain context. Multi-round continuation allows the agent to reason iteratively -- execute tools, observe results, then decide what to do next. For OpenRouter models, reasoning effort is runtime-configurable (`Default`, `Low`, `Medium`, `High`) via `set_openrouter_reasoning_level`.

### Threshold ECDSA Wallet
The canister derives its own Ethereum address via ICP's [threshold ECDSA](https://internetcomputer.org/docs/current/developer-docs/smart-contracts/signatures/t-ecdsa) signing. No human ever holds the private key. The agent can sign messages, construct EIP-1559 transactions, and broadcast them to Base.

### EVM Integration
- **Transaction broadcasting** -- Full EIP-1559 transaction construction, signing, and submission
- **Chain reading** -- `eth_getBalance`, `eth_call`, and log polling via JSON-RPC
- **Event polling** -- Continuous polling of Base blocks with cursor tracking, confirmation depth, and automatic backoff

### On-Chain Inbox
An [Inbox.sol](evm/src/Inbox.sol) contract on Base allows anyone to send messages to the agent with attached ETH or USDC payments. The canister polls for `MessageQueued` events and ingests them as input for reasoning turns.

### Signed Steward Command Plane
The canister supports a single active EVM steward identity for privileged runtime changes (including direct steward messages) through signed commands.

### Survival Tiers
The agent monitors its own cycle balance and adapts behavior under resource pressure:

| Tier | Condition | Behavior |
|------|-----------|----------|
| **Normal** | Liquid cycles ≥ 15× critical threshold | All capabilities enabled |
| **LowCycles** | Liquid cycles < 15× critical threshold | Reduced poll frequency, cost-optimized operations |
| **Critical** | Can't afford reference operation + 200B reserve | High-cost operations gated; non-essential jobs skipped |
| **OutOfCycles** | (reserved) | Agent frozen |

Tier classification is formula-based using pre-flight cycle affordability checks with a 25% safety margin and a 200B-cycle reserve floor. Tier recovery requires 3 consecutive healthy checks before upgrading to avoid flapping.

Pre-flight affordability checks ensure the agent never attempts an operation it cannot pay for.

### Persistent Memory
The agent stores and retrieves facts across turns using a durable key-value memory backed by stable structures.

### Multi-Layer Constitution
A layered prompt system defines the agent's identity and behavioral constraints across 11 layers (0–10):

- **Layers 0–5** (immutable): Interpretation rules, safety/non-harm, survival economics, identity, ethics, and tool policies
- **Layers 6–9** (mutable): Updateable by the controller or the agent itself at runtime
- **Layer 10** (dynamic): Runtime context injected each turn -- cycle balance, wallet state, memory facts, pending inbox messages, and available tools

Lower-numbered layers take precedence in all conflicts. Forbidden-phrase detection blocks prompt injection attempts that try to override core policy layers.

### Strategy Engine
A structured DeFi strategy execution framework (in `src/strategy/`) enables the agent to execute template-based on-chain actions safely:

- **Registry** -- Stores `StrategyTemplate` records keyed by `(protocol, primitive, chain_id, template_id)`, with versioned ABI artifact binding and lifecycle states (Draft → Active → Deprecated → Revoked)
- **Compiler** -- Resolves a `StrategyExecutionIntent` against a registered template and ABI artifacts into a concrete `ExecutionPlan` of typed EVM calls
- **Validator** -- Multi-layer validation pipeline (Schema → Address → Policy → Preflight → Postcondition) with deterministic/non-deterministic failure classification
- **Learner** -- Tracks `StrategyOutcomeStats` per template/version, computing confidence scores, ranking scores, and adaptive parameter priors (slippage bps, gas buffer) based on historical success/failure
- **ABI** -- Stores raw ABI JSON artifacts with function selector assertions for on-chain binding verification

A kill-switch mechanism allows per-strategy emergency disablement independent of template lifecycle state.

### Autonomous Cycle Top-Up
The agent can replenish its own ICP cycles from its USDC balance without operator intervention:

1. Locks USDC via the [1sec](https://1sec.app) locker contract on Base
2. Bridges locked USDC to ICP (polls for bridge confirmation)
3. Swaps bridged USDC for ICP via [KongSwap](https://kongswap.io) (with configurable max slippage)
4. Converts ICP to cycles via the Cycles Minting Canister

Top-up triggers when liquid cycles fall below a configurable threshold (default: 2T cycles). Configurable limits: minimum USDC reserve, maximum USDC per top-up, max swap slippage.

### Wallet Balance Sync
The agent maintains a fresh snapshot of its ETH and USDC balances via periodic background sync:

- Normal interval: 5 minutes; low-cycles interval: 15 minutes
- Freshness window: 10 minutes (stale if older)
- USDC contract address is auto-discovered from the Inbox contract if not explicitly configured
- Balance data is injected into each turn's dynamic context (Layer 10)

### Inference Proxy Callback Reliability
When OpenRouter proxy-worker mode is enabled, callback ingestion is idempotent by `job_id` and tracks recently completed callback jobs so duplicate deliveries are safely ignored even after result consumption.

### Storage Retention & Summarization
To prevent unbounded stable memory growth, a periodic maintenance job:

- Prunes expired turns, transitions, tool records, inbox/outbox messages, and deduplication entries based on configurable max-age and max-record limits
- Generates **session summaries** (per-sender conversation windows) to compress old conversation history
- Generates **turn window summaries** (aggregate turn/tool stats) for observability without raw record retention
- Generates **memory rollups** (per-namespace canonical value synthesis) to consolidate redundant memory facts

### Embedded Terminal UI
A retro phosphor-green terminal UI is served directly from the canister via certified HTTP responses. Users can connect EVM wallets (MetaMask, Coinbase), send messages with payments, and observe the agent's status, logs, and internal monologue.

### Scheduler & Job Queue
A timer-driven scheduler fires every 30 seconds and dispatches up to 4 mutating jobs per tick. Each task type (AgentTurn, PollInbox, CheckCycles, TopUpCycles, Reconcile) runs on its own 5-minute interval with independent backoff and retry logic. Lease-based concurrency control ensures only one mutating operation runs at a time, with automatic stale-lease recovery. Non-essential jobs are skipped in low-cycles mode.

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) with WASM target: `rustup target add wasm32-wasip1`
- [wasi2ic](https://github.com/wasm-forge/wasi2ic): `cargo install wasi2ic`
- [icp-cli](https://docs.icp-cli.dev/) for building and deploying canisters
- [Foundry](https://book.getfoundry.sh/) (for EVM contract deployment and testing)

### Local Development

```bash
# Clone the repository
git clone https://github.com/domwoe/ic-automaton.git
cd ic-automaton

# Validate the WASI build path used by icp-cli
cargo check --target wasm32-wasip1 -p backend
icp build backend

# Start everything with OpenRouter inference (requires OPENROUTER_API_KEY)
just bootstrap openrouter

# Start everything with local IcLlm mode
# (starts Ollama, deploys local llm canister, wires backend llm_canister_id, configures IcLlm)
just bootstrap icllm

# Check the agent's status
icp canister call backend get_runtime_view '()'
```

### Full Bootstrap (with local EVM)

The justfile provides a complete local development environment with a local Anvil EVM chain and two inference modes:

```bash
# Start everything with OpenRouter inference (requires OPENROUTER_API_KEY)
just bootstrap openrouter

# Start everything with local ic_llm mode
# (starts Ollama, deploys local llm canister, wires backend llm_canister_id, configures IcLlm)
just bootstrap icllm

# Optional: fork Base mainnet instead of using a blank local chain
just bootstrap openrouter base-fork "" "$BASE_MAINNET_RPC_URL"

# Optional model defaults can be configured in .env:
# OPENROUTER_MODEL (default: google/gemini-3-flash-preview)
# IC_LLM_MODEL (default: llama3.1:8b)

# Tear down all local services (IC, Anvil, and tracked Ollama if started)
just down all

# Send a message to the agent via the Inbox contract (with USDC + ETH)
just send-message-usdc "hello automaton"

# Send a message with ETH only
just send-message-eth-only "hello automaton"

# Enable the agent loop
icp canister call backend set_loop_enabled '(true)'
```

### Configuration

Key init arguments (set at deploy time via `icp canister install`):

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ecdsa_key_name` | Threshold ECDSA key identifier | `"dfx_test_key"` (local) |
| `evm_chain_id` | Target EVM chain ID | `31337` (local Anvil) / `8453` (Base) |
| `evm_rpc_url` | JSON-RPC endpoint | `"https://mainnet.base.org"` |
| `evm_confirmation_depth` | Block confirmations before event processing | `6` (mainnet default) |
| `evm_bootstrap_lookback_blocks` | Initial/fallback log backfill window; set `0` to start from current confirmed head | `1000` |
| `inbox_contract_address` | Deployed Inbox.sol address | -- |
| `llm_canister_id` | IcLlm canister for local inference | `w36hm-eqaaa-aaaal-qr76a-cai` |

Runtime configuration (updatable via canister calls):

| Setting | Description |
|---------|-------------|
| `set_inference_provider` | Switch between `OpenRouter` and `IcLlm` |
| `set_inference_model` | Set model name (e.g. `google/gemini-3-flash-preview`, `llama3.1:8b`) |
| `set_openrouter_api_key` | Configure OpenRouter API key |
| `set_openrouter_reasoning_level` | Set OpenRouter reasoning effort (`Default`, `Low`, `Medium`, `High`) |
| `set_task_interval_secs` | Adjust per-task scheduling frequency |
| `set_loop_enabled` | Enable/disable the agent turn loop |

## Project Structure

```
ic-automaton/
├── src/
│   ├── lib.rs              # Canister entrypoint, query/update methods, timer setup
│   ├── agent.rs            # Agent loop, turn execution, continuation logic
│   ├── tools.rs            # Tool registry, dispatch, prompt injection guards
│   ├── scheduler.rs        # Job scheduler, survival tier classification, task dispatch
│   ├── prompt.rs           # Layered constitution (layers 0-9), forbidden phrase detection
│   ├── http.rs             # HTTP request handling, certified API endpoints
│   ├── domain/
│   │   ├── types.rs        # All domain types, FSM states, events, config structs
│   │   ├── state_machine.rs# FSM transition function
│   │   ├── cycle_admission.rs # Cycle affordability estimation and checks
│   │   └── recovery_policy.rs # Structured error recovery decisions
│   ├── features/
│   │   ├── evm.rs          # EVM polling, transaction construction, wallet sync
│   │   ├── inference.rs    # LLM inference dispatch (OpenRouter + IcLlm)
│   │   ├── http_fetch.rs   # HTTPS outcall tool
│   │   ├── threshold_signer.rs # Threshold ECDSA signing adapter
│   │   ├── skills.rs       # Skill/capability definitions
│   │   └── cycle_topup/    # Autonomous USDC→ICP→cycles top-up pipeline
│   ├── strategy/
│   │   ├── registry.rs     # Strategy template and ABI artifact storage
│   │   ├── compiler.rs     # Intent → ExecutionPlan compiler
│   │   ├── validator.rs    # Multi-layer validation pipeline
│   │   ├── learner.rs      # Outcome tracking, confidence scoring, parameter priors
│   │   └── abi.rs          # ABI parsing and selector verification
│   ├── storage/
│   │   └── stable.rs       # Stable memory maps, retention, summarization
│   ├── ui_app.js           # Terminal UI (vanilla JS + viem)
│   ├── ui_index.html       # UI shell
│   └── ui_styles.css       # Phosphor-green terminal styling
├── evm/                    # Solidity contracts (Inbox.sol, MockUSDC)
├── tests/                  # PocketIC integration tests
├── ic-automaton.did        # Candid interface (auto-generated)
├── icp.yaml                # Canister build & deployment config
├── justfile                # Development task runner
└── Cargo.toml              # Rust dependencies
```

## Testing

```bash
# Unit tests (native, no WASM required)
cargo test

# Integration tests with PocketIC
cargo test --features pocketic_tests

# End-to-end with local Anvil EVM
just anvil-start
just deploy-inbox
cargo test --features anvil_e2e

# Benchmark cycle consumption
cargo bench --features canbench
```

## How It Differs from Off-Chain Agents

| | Off-Chain Agent | ic-automaton |
|---|---|---|
| **Runtime** | Cloud VM or laptop | ICP canister (decentralized WASM) |
| **Keys** | Held by operator | Threshold ECDSA (no human holds the key) |
| **State** | Database or files | Stable memory (survives upgrades) |
| **Uptime** | Depends on operator | Autonomous (runs as long as cycles remain) |
| **Payment** | Operator pays cloud bill | Agent manages its own cycles + earns on-chain |
| **Verifiability** | Trust the operator | Canister code is inspectable on-chain |

## Roadmap

- [ ] v0 - Production deployment on Base mainnet
- [ ] v1 - Strategy execution in production; expanded DeFi protocol coverage
- [ ] v2 - Inter-canister calls; improved memory and summarization system
- [ ] v3 - Inference without external API key (fully sovereign)
- [ ] v4 - Bitcoin and Solana support

(Subject to change)

## Contributing

This project is in active early development. If you're interested in autonomous on-chain agents, you're welcome to explore the codebase, open issues, or submit pull requests.

## License

[MIT](LICENSE)

---

<p align="center">
  <sub>Built on the <a href="https://internetcomputer.org">Internet Computer</a></sub>
</p>
