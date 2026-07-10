#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PLAYGROUND_ENV_FILE=${PLAYGROUND_ENV_FILE:-/etc/automaton-playground/playground.env}
PLAYGROUND_COMPOSE_FILE=${PLAYGROUND_COMPOSE_FILE:-"$ROOT_DIR/ops/playground/docker-compose.yml"}
PLAYGROUND_STATE_DIR=${PLAYGROUND_STATE_DIR:-"$ROOT_DIR/tmp"}
PLAYGROUND_RELEASES_DIR=${PLAYGROUND_RELEASES_DIR:-"$PLAYGROUND_STATE_DIR/releases"}
PLAYGROUND_ARTIFACTS_DIR=${PLAYGROUND_ARTIFACTS_DIR:-"$PLAYGROUND_STATE_DIR/artifacts"}
MANIFEST_PATH=""
MODE=""
NAMED_CANISTER_ID=""
REQUIRE_STAGED_SOURCE=0

usage() {
  cat >&2 <<'EOF'
Usage: scripts/deploy-playground-release.sh --manifest <path> --mode <soft|hard-reset|admit-child|upgrade-named> [options]

Options:
  --canister-id <principal>  Required for --mode upgrade-named.
  --require-staged-source    Require source-commit.txt beside the staged runner.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) MANIFEST_PATH=${2:-}; shift 2 ;;
    --mode) MODE=${2:-}; shift 2 ;;
    --canister-id) NAMED_CANISTER_ID=${2:-}; shift 2 ;;
    --require-staged-source) REQUIRE_STAGED_SOURCE=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 1 ;;
  esac
done

[ -n "$MANIFEST_PATH" ] || { echo "--manifest is required" >&2; usage; exit 1; }
[ -n "$MODE" ] || { echo "--mode is required; deployment action is never read from the manifest" >&2; usage; exit 1; }
case "$MODE" in soft|hard-reset|admit-child|upgrade-named) ;; *) echo "unsupported mode: $MODE" >&2; exit 1 ;; esac
[ -f "$MANIFEST_PATH" ] || { echo "Release manifest not found: $MANIFEST_PATH" >&2; exit 1; }
[ -f "$PLAYGROUND_ENV_FILE" ] || { echo "Playground env file not found: $PLAYGROUND_ENV_FILE" >&2; exit 1; }

require_command() { command -v "$1" >/dev/null 2>&1 || { echo "Missing required command: $1" >&2; exit 1; }; }
require_command bash
require_command node
require_command sha256sum
require_command icp
if [ "$MODE" = soft ] || [ "$MODE" = hard-reset ]; then
  require_command curl
  require_command docker
  [ -f "$PLAYGROUND_COMPOSE_FILE" ] || { echo "Compose file not found: $PLAYGROUND_COMPOSE_FILE" >&2; exit 1; }
fi

set -a
. "$PLAYGROUND_ENV_FILE"
set +a

mkdir -p "$PLAYGROUND_RELEASES_DIR" "$PLAYGROUND_ARTIFACTS_DIR"
manifest_env_file=$(mktemp)
cleanup() { rm -f "$manifest_env_file"; }
trap cleanup EXIT

sh "$ROOT_DIR/scripts/with-repo-node.sh" node --input-type=module - "$MANIFEST_PATH" "$ROOT_DIR" "$REQUIRE_STAGED_SOURCE" >"$manifest_env_file" <<'NODE'
import fs from "node:fs";
import path from "node:path";
const { readReleaseManifest } = await import(new URL(`file://${rootDir}/scripts/lib/release-manifest.mjs`).href);

const [manifestPath, rootDir, requireStaged] = process.argv.slice(2);
const manifest = readReleaseManifest(manifestPath);
const sourceRevisionPath = path.join(rootDir, "source-commit.txt");
if (requireStaged === "1") {
  if (!fs.existsSync(sourceRevisionPath)) throw new Error(`missing staged source revision ${sourceRevisionPath}`);
  const revision = fs.readFileSync(sourceRevisionPath, "utf8").trim();
  if (revision !== manifest.release.sourceCommit || revision !== manifest.ops.sourceCommit) {
    throw new Error("staged runner revision does not match release.sourceCommit and ops.sourceCommit");
  }
}
const bundleDir = process.env.PLAYGROUND_RELEASE_BUNDLE_DIR?.trim() || rootDir;
const quote = (value) => `'${String(value).replaceAll("'", `'\\''`)}'`;
const assign = (key, value) => console.log(`${key}=${quote(value)}`);
assign("RELEASE_SOURCE_COMMIT", manifest.release.sourceCommit);
assign("PLAYGROUND_ENV_VERSION", manifest.release.environmentVersion);
assign("PLAYGROUND_WEB_IMAGE", manifest.images.web.ref);
assign("PLAYGROUND_INDEXER_IMAGE", manifest.images.indexer.ref);
assign("PLAYGROUND_RPC_GATEWAY_IMAGE", manifest.images.rpcGateway.ref);
assign("CHILD_VERSION_COMMIT", manifest.artifacts.automatonWasm.sourceCommit);
assign("CHILD_ARTIFACT_SHA256", manifest.artifacts.automatonWasm.sha256);
assign("CHILD_WASM_PATH", path.join(bundleDir, manifest.artifacts.automatonWasm.fileName));
assign("PLAYGROUND_FACTORY_WASM_PATH", path.join(bundleDir, manifest.artifacts.factoryWasm.fileName));
assign("FACTORY_ARTIFACT_SHA256", manifest.artifacts.factoryWasm.sha256);
NODE

set -a
. "$manifest_env_file"
set +a
export CHILD_WASM_PATH PLAYGROUND_FACTORY_WASM_PATH

verify_artifact() {
  local expected="$1" path="$2"
  [ -f "$path" ] || { echo "release artifact missing: $path" >&2; exit 1; }
  printf '%s  %s\n' "$expected" "$path" | sha256sum -c >/dev/null
}
verify_artifact "$CHILD_ARTIFACT_SHA256" "$CHILD_WASM_PATH"
verify_artifact "$FACTORY_ARTIFACT_SHA256" "$PLAYGROUND_FACTORY_WASM_PATH"

if [ -n "${GHCR_USERNAME:-}" ] && [ -n "${GHCR_TOKEN:-}" ] && { [ "$MODE" = soft ] || [ "$MODE" = hard-reset ]; }; then
  printf '%s' "$GHCR_TOKEN" | docker login ghcr.io -u "$GHCR_USERNAME" --password-stdin >/dev/null
fi

compose() { docker compose -f "$PLAYGROUND_COMPOSE_FILE" "$@"; }
wait_for_http() {
  local url="$1" label="$2" attempts="${3:-60}" index=0
  while [ "$index" -lt "$attempts" ]; do
    curl -fsS "$url" >/dev/null 2>&1 && return 0
    index=$((index + 1)); sleep 1
  done
  echo "$label did not become ready at $url" >&2; return 1
}
wait_for_rpc() { wait_for_http "$1" "$2"; }
pull_release_images() {
  docker pull "$PLAYGROUND_WEB_IMAGE"
  docker pull "$PLAYGROUND_INDEXER_IMAGE"
  docker pull "$PLAYGROUND_RPC_GATEWAY_IMAGE"
}
update_runtime_services() { compose up -d --force-recreate web rpc-gateway indexer; }
write_status() { sh "$ROOT_DIR/scripts/with-repo-node.sh" node "$ROOT_DIR/scripts/write-playground-status.mjs"; }
record_release() {
  local timestamp action_file
  timestamp=$(date -u +"%Y%m%dT%H%M%SZ")
  cp "$MANIFEST_PATH" "$PLAYGROUND_RELEASES_DIR/${timestamp}-${RELEASE_SOURCE_COMMIT}-${MODE}.json"
  cp "$MANIFEST_PATH" "$PLAYGROUND_RELEASES_DIR/current.json"
  action_file="$PLAYGROUND_RELEASES_DIR/${timestamp}-${RELEASE_SOURCE_COMMIT}-${MODE}.action"
  printf '%s\n' "$MODE" >"$action_file"
}

run_soft_deploy() {
  pull_release_images
  update_runtime_services
  wait_for_http "${PLAYGROUND_INDEXER_BASE_URL:-http://127.0.0.1:${PLAYGROUND_INDEXER_PORT:-3001}}/health" indexer
  wait_for_http "${PLAYGROUND_RPC_GATEWAY_URL:-http://127.0.0.1:${PLAYGROUND_RPC_GATEWAY_PORT:-3002}}/health" rpc-gateway
  sh "$ROOT_DIR/scripts/playground-smoke.sh"
  PLAYGROUND_STATUS_ENVIRONMENT_VERSION="$PLAYGROUND_ENV_VERSION" PLAYGROUND_STATUS_MAINTENANCE=false write_status >/dev/null
}

run_hard_reset() {
  pull_release_images
  update_runtime_services
  PLAYGROUND_ANVIL_RESET_COMMAND="docker compose -f '$PLAYGROUND_COMPOSE_FILE' up -d --force-recreate --no-deps anvil" \
    sh "$ROOT_DIR/scripts/playground-reset.sh"
}

run_admit_child() {
  FACTORY_CANISTER="${PLAYGROUND_FACTORY_CANISTER:-factory}" \
  FACTORY_ENVIRONMENT="${PLAYGROUND_ICP_ENVIRONMENT:-local}" \
  CHILD_VERSION_COMMIT="$CHILD_VERSION_COMMIT" \
  FACTORY_EXPECTED_SHA256="$CHILD_ARTIFACT_SHA256" \
  FACTORY_EXPECTED_VERSION_COMMIT="$CHILD_VERSION_COMMIT" \
  CHILD_WASM_PATH="$CHILD_WASM_PATH" \
    sh "$ROOT_DIR/scripts/with-repo-node.sh" node "$ROOT_DIR/scripts/upload-factory-artifact.mjs"
}

run_upgrade_named() {
  [ -n "$NAMED_CANISTER_ID" ] || { echo "--canister-id is required for upgrade-named" >&2; exit 1; }
  [ "${PLAYGROUND_UPGRADE_APPROVED:-0}" = 1 ] || { echo "PLAYGROUND_UPGRADE_APPROVED=1 is required" >&2; exit 1; }
  pre="$PLAYGROUND_RELEASES_DIR/${RELEASE_SOURCE_COMMIT}-${NAMED_CANISTER_ID}.pre-upgrade.txt"
  post="$PLAYGROUND_RELEASES_DIR/${RELEASE_SOURCE_COMMIT}-${NAMED_CANISTER_ID}.post-upgrade.txt"
  icp canister call -e "${PLAYGROUND_ICP_ENVIRONMENT:-local}" "$NAMED_CANISTER_ID" get_runtime_view >"$pre"
  icp canister install -e "${PLAYGROUND_ICP_ENVIRONMENT:-local}" "$NAMED_CANISTER_ID" --mode upgrade --wasm "$CHILD_WASM_PATH" --yes
  icp canister call -e "${PLAYGROUND_ICP_ENVIRONMENT:-local}" "$NAMED_CANISTER_ID" get_runtime_view >"$post"
  [ -s "$post" ] || { echo "post-upgrade verification returned no runtime view" >&2; exit 1; }
}

case "$MODE" in
  soft) run_soft_deploy ;;
  hard-reset) run_hard_reset ;;
  admit-child) run_admit_child ;;
  upgrade-named) run_upgrade_named ;;
esac
record_release
printf 'Applied %s action for release %s\n' "$MODE" "$RELEASE_SOURCE_COMMIT"
