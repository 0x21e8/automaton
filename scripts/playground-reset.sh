#!/bin/sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

sh "$ROOT_DIR/scripts/playground-stop.sh"

exec sh "$ROOT_DIR/scripts/playground-bootstrap.sh"
