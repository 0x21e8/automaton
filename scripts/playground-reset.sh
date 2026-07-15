#!/bin/sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$ROOT_DIR/scripts/load-repo-env.sh"

sh "$ROOT_DIR/scripts/playground-stop.sh"

exec sh "$ROOT_DIR/scripts/playground-bootstrap.sh"
