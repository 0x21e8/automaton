#!/bin/sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PLAYGROUND_ICP_HOME=${PLAYGROUND_ICP_HOME:-"$ROOT_DIR/tmp/icp-home"}
PLAYGROUND_ICP_NETWORK_NAME=${PLAYGROUND_ICP_NETWORK_NAME:-local}

export ICP_HOME="$PLAYGROUND_ICP_HOME"

icp --project-root-override "$ROOT_DIR" network ping "$PLAYGROUND_ICP_NETWORK_NAME" >/dev/null

if [ "${PLAYGROUND_SPAWN_PAYMENT_E2E_SKIP_SMOKE:-0}" != "1" ]; then
  PLAYGROUND_SPAWN_PAYMENT_E2E_REQUIRE_SMOKE_OUTPUT=1 \
    sh "$ROOT_DIR/scripts/playground-smoke.sh"
fi

exec sh "$ROOT_DIR/scripts/with-repo-node.sh" node "$ROOT_DIR/scripts/spawn-payment-e2e.mjs"
