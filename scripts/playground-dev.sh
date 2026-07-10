#!/bin/sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
AUTOMATON_COMPONENT_ROOT=$(AUTOMATON_LAUNCHPAD_ROOT="$ROOT_DIR" sh "$ROOT_DIR/scripts/resolve-automaton-component.sh")
export AUTOMATON_COMPONENT_ROOT
PLAYGROUND_LOCAL_ENV_FILE=${PLAYGROUND_LOCAL_ENV_FILE:-"$ROOT_DIR/playground.local.env"}

if [ -f "$PLAYGROUND_LOCAL_ENV_FILE" ]; then
  set -a
  . "$PLAYGROUND_LOCAL_ENV_FILE"
  set +a
fi

TMP_DIR=${PLAYGROUND_TMP_DIR:-"$ROOT_DIR/tmp"}
PLAYGROUND_REQUIRE_FORK=${PLAYGROUND_REQUIRE_FORK:-1}
PLAYGROUND_CHAIN_ID=${PLAYGROUND_CHAIN_ID:-20260326}
PLAYGROUND_CHAIN_NAME=${PLAYGROUND_CHAIN_NAME:-Automaton Playground}
PLAYGROUND_INDEXER_HOST=${PLAYGROUND_INDEXER_HOST:-127.0.0.1}
PLAYGROUND_INDEXER_PORT=${PLAYGROUND_INDEXER_PORT:-3001}
PLAYGROUND_INDEXER_BASE_URL=${PLAYGROUND_INDEXER_BASE_URL:-http://$PLAYGROUND_INDEXER_HOST:$PLAYGROUND_INDEXER_PORT}
PLAYGROUND_RPC_GATEWAY_HOST=${PLAYGROUND_RPC_GATEWAY_HOST:-127.0.0.1}
PLAYGROUND_RPC_GATEWAY_PORT=${PLAYGROUND_RPC_GATEWAY_PORT:-3002}
PLAYGROUND_RPC_GATEWAY_URL=${PLAYGROUND_RPC_GATEWAY_URL:-http://$PLAYGROUND_RPC_GATEWAY_HOST:$PLAYGROUND_RPC_GATEWAY_PORT}
PLAYGROUND_PUBLIC_RPC_URL=${PLAYGROUND_PUBLIC_RPC_URL:-$PLAYGROUND_RPC_GATEWAY_URL}
LOCAL_EVM_DEPLOYMENT_FILE=${LOCAL_EVM_DEPLOYMENT_FILE:-"$TMP_DIR/local-escrow-deployment.json"}
AUTOMATON_INBOX_DEPLOYMENT_FILE=${AUTOMATON_INBOX_DEPLOYMENT_FILE:-"$TMP_DIR/automaton-inbox-deployment.json"}
WEB_HOST=${WEB_HOST:-127.0.0.1}
WEB_PORT=${WEB_PORT:-5173}

run_with_repo_node() {
  sh "$ROOT_DIR/scripts/with-repo-node.sh" "$@"
}

require_value() {
  variable_name=$1
  message=$2
  eval "value=\${$variable_name:-}"

  if [ -n "$value" ]; then
    return 0
  fi

  echo "$message" >&2
  exit 1
}

if [ "$PLAYGROUND_REQUIRE_FORK" = "1" ]; then
  require_value "LOCAL_EVM_FORK_URL" \
    "LOCAL_EVM_FORK_URL is required. Copy playground.local.env.example to playground.local.env and set your Base RPC fork URL."
fi

sh "$ROOT_DIR/scripts/playground-bootstrap.sh"

if [ ! -f "$LOCAL_EVM_DEPLOYMENT_FILE" ]; then
  echo "missing local EVM deployment file after bootstrap: $LOCAL_EVM_DEPLOYMENT_FILE" >&2
  exit 1
fi

deployment_values=$(
  run_with_repo_node node -e '
    const fs = require("node:fs");
    const deployment = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    process.stdout.write(`${deployment.rpcUrl ?? ""}\n${deployment.usdcTokenAddress ?? deployment.mockUsdcAddress ?? ""}\n`);
  ' "$LOCAL_EVM_DEPLOYMENT_FILE"
)

deployment_rpc_url=$(printf '%s\n' "$deployment_values" | sed -n '1p')
deployment_usdc_address=$(printf '%s\n' "$deployment_values" | sed -n '2p')

if [ -z "${VITE_SPAWN_USDC_CONTRACT_ADDRESS:-}" ] && [ -z "$deployment_usdc_address" ]; then
  echo "failed to resolve the local USDC token address from $LOCAL_EVM_DEPLOYMENT_FILE" >&2
  exit 1
fi

cd "$ROOT_DIR"

VITE_INDEXER_BASE_URL=${VITE_INDEXER_BASE_URL:-$PLAYGROUND_INDEXER_BASE_URL} \
VITE_SPAWN_CHAIN_ID=${VITE_SPAWN_CHAIN_ID:-$PLAYGROUND_CHAIN_ID} \
VITE_SPAWN_CHAIN_NAME=${VITE_SPAWN_CHAIN_NAME:-$PLAYGROUND_CHAIN_NAME} \
VITE_SPAWN_CHAIN_RPC_URL=${VITE_SPAWN_CHAIN_RPC_URL:-${PLAYGROUND_PUBLIC_RPC_URL:-$deployment_rpc_url}} \
VITE_SPAWN_USDC_CONTRACT_ADDRESS=${VITE_SPAWN_USDC_CONTRACT_ADDRESS:-$deployment_usdc_address} \
  npm exec --workspace @ic-automaton/web vite -- --host "$WEB_HOST" --port "$WEB_PORT"
