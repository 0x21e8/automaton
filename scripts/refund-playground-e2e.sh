#!/bin/sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

exec sh "$ROOT_DIR/scripts/with-repo-node.sh" node "$ROOT_DIR/scripts/refund-playground-e2e.mjs"
