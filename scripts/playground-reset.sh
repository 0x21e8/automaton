#!/bin/sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TMP_DIR=${PLAYGROUND_TMP_DIR:-"$ROOT_DIR/tmp"}
STATUS_FILE=${PLAYGROUND_STATUS_FILE:-"$TMP_DIR/playground-status.json"}
SERVICE_DIR=${PLAYGROUND_SERVICE_DIR:-"$TMP_DIR/playground-services"}
PLAYGROUND_ICP_HOME=${PLAYGROUND_ICP_HOME:-"$TMP_DIR/icp-home"}
PLAYGROUND_ICP_NETWORK_NAME=${PLAYGROUND_ICP_NETWORK_NAME:-local}
PLAYGROUND_ANVIL_MANAGED=${PLAYGROUND_ANVIL_MANAGED:-1}
PLAYGROUND_ANVIL_RESET_COMMAND=${PLAYGROUND_ANVIL_RESET_COMMAND:-}
PLAYGROUND_MANAGE_SERVICES=${PLAYGROUND_MANAGE_SERVICES:-1}
PLAYGROUND_ICP_STATE_DIR=${PLAYGROUND_ICP_STATE_DIR:-"$ROOT_DIR/.icp/cache/networks/local"}
LOCAL_EVM_RPC_URL=${LOCAL_EVM_RPC_URL:-http://127.0.0.1:8545}
LOCAL_EVM_PID_PATH=${LOCAL_EVM_PID_PATH:-"$SERVICE_DIR/anvil.pid"}
AUTOMATON_INBOX_DEPLOYMENT_FILE=${AUTOMATON_INBOX_DEPLOYMENT_FILE:-"$TMP_DIR/automaton-inbox-deployment.json"}
PLAYGROUND_FACTORY_CANISTER_ID_FILE=${PLAYGROUND_FACTORY_CANISTER_ID_FILE:-"$TMP_DIR/factory-canister-id.txt"}
INDEXER_DB_PATH=${INDEXER_DB_PATH:-"$TMP_DIR/playground-indexer.sqlite"}
PLAYGROUND_INDEXER_PID_FILE=${PLAYGROUND_INDEXER_PID_FILE:-"$SERVICE_DIR/indexer.pid"}
PLAYGROUND_RPC_GATEWAY_PID_FILE=${PLAYGROUND_RPC_GATEWAY_PID_FILE:-"$SERVICE_DIR/rpc-gateway.pid"}
LOCAL_EVM_DEPLOYMENT_FILE=${LOCAL_EVM_DEPLOYMENT_FILE:-"$TMP_DIR/local-escrow-deployment.json"}

export ICP_HOME="$PLAYGROUND_ICP_HOME"
export PLAYGROUND_STATUS_FILE="$STATUS_FILE"

run_with_repo_node() {
  sh "$ROOT_DIR/scripts/with-repo-node.sh" "$@"
}

write_status() {
  run_with_repo_node node "$ROOT_DIR/scripts/write-playground-status.mjs"
}

stop_pid_file_process() {
  pid_file=$1
  if [ ! -f "$pid_file" ]; then
    return 0
  fi

  pid=$(cat "$pid_file" 2>/dev/null || true)
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
  fi

  rm -f "$pid_file"
}

anvil_available() {
  run_with_repo_node node -e '
    const url = process.argv[1];
    fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "eth_chainId", params: [] })
    })
      .then(async (response) => {
        const body = await response.json();
        if (!response.ok || body.error || typeof body.result !== "string") {
          process.exit(1);
        }
      })
      .catch(() => {
        process.exit(1);
      });
  ' "$LOCAL_EVM_RPC_URL" >/dev/null 2>&1
}

PLAYGROUND_STATUS_MAINTENANCE="true" \
PLAYGROUND_STATUS_UPDATED_AT="now" \
  write_status >/dev/null

if [ "$PLAYGROUND_MANAGE_SERVICES" = "1" ]; then
  stop_pid_file_process "$PLAYGROUND_INDEXER_PID_FILE"
  stop_pid_file_process "$PLAYGROUND_RPC_GATEWAY_PID_FILE"
fi

if [ "$PLAYGROUND_ANVIL_MANAGED" = "1" ]; then
  stop_pid_file_process "$LOCAL_EVM_PID_PATH"
elif [ -n "$PLAYGROUND_ANVIL_RESET_COMMAND" ]; then
  sh -c "$PLAYGROUND_ANVIL_RESET_COMMAND"
elif anvil_available; then
  echo "cannot hard reset externally managed Anvil at $LOCAL_EVM_RPC_URL without PLAYGROUND_ANVIL_RESET_COMMAND" >&2
  exit 1
fi

icp --project-root-override "$ROOT_DIR" network stop "$PLAYGROUND_ICP_NETWORK_NAME" >/dev/null 2>&1 || true

rm -rf "$PLAYGROUND_ICP_STATE_DIR"
rm -rf "$PLAYGROUND_ICP_HOME/port-descriptors"
rm -f "$INDEXER_DB_PATH" "$LOCAL_EVM_DEPLOYMENT_FILE" "$AUTOMATON_INBOX_DEPLOYMENT_FILE" "$PLAYGROUND_FACTORY_CANISTER_ID_FILE"

exec sh "$ROOT_DIR/scripts/playground-bootstrap.sh"
