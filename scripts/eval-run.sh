#!/bin/sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
EVALUATOR_HOST=${EVALUATOR_HOST:-127.0.0.1}
EVALUATOR_PORT=${EVALUATOR_PORT:-3003}
EVALUATOR_BASE_URL=${VITE_EVALUATOR_BASE_URL:-http://$EVALUATOR_HOST:$EVALUATOR_PORT}
EVALUATOR_WEB_HOST=${EVALUATOR_WEB_HOST:-127.0.0.1}
EVALUATOR_WEB_PORT=${EVALUATOR_WEB_PORT:-4173}
LAUNCHPAD_WEB_HOST=${LAUNCHPAD_WEB_HOST:-127.0.0.1}
LAUNCHPAD_WEB_PORT=${LAUNCHPAD_WEB_PORT:-5173}
LAUNCHPAD_INDEXER_BASE_URL=${LAUNCHPAD_INDEXER_BASE_URL:-${PLAYGROUND_INDEXER_BASE_URL:-http://127.0.0.1:3001}}
EVALUATOR_ARTIFACTS_ROOT=${EVALUATOR_ARTIFACTS_ROOT:-"$ROOT_DIR/tmp/evaluations"}
EVALUATOR_LOG_DIR=${EVALUATOR_LOG_DIR:-"$ROOT_DIR/tmp/evaluator-logs"}

experiment_path=

while [ "$#" -gt 0 ]; do
  case "$1" in
    --experiment)
      if [ "$#" -lt 2 ]; then
        echo "--experiment requires a path" >&2
        exit 1
      fi
      experiment_path=$2
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      echo "usage: npm run eval:run -- --experiment evaluations/experiments/<name>.yaml" >&2
      exit 1
      ;;
  esac
done

if [ -z "$experiment_path" ]; then
  echo "--experiment is required" >&2
  echo "usage: npm run eval:run -- --experiment evaluations/experiments/<name>.yaml" >&2
  exit 1
fi

case "$experiment_path" in
  /*)
    experiment_file=$experiment_path
    ;;
  *)
    experiment_file=$ROOT_DIR/$experiment_path
    ;;
esac

if [ ! -f "$experiment_file" ]; then
  echo "experiment file not found: $experiment_path" >&2
  exit 1
fi

backend_pid=""
web_pid=""
launchpad_pid=""
backend_log="$EVALUATOR_LOG_DIR/backend.run.log"
web_log="$EVALUATOR_LOG_DIR/web.run.log"
launchpad_log="$EVALUATOR_LOG_DIR/launchpad.run.log"
last_reported_state=""
run_terminal_state=""

mkdir -p "$EVALUATOR_LOG_DIR"

print_port_owners() {
  port=$1

  if ! command -v lsof >/dev/null 2>&1; then
    return 0
  fi

  lsof -nP -iTCP:"$port" -sTCP:LISTEN 2>/dev/null | awk '
    NR > 1 {
      printf "  listener: %s (pid %s)\n", $1, $2
    }
  '
}

require_free_port() {
  service_name=$1
  host=$2
  port=$3
  port_env=$4

  if ! command -v lsof >/dev/null 2>&1; then
    return 0
  fi

  if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    printf '%s cannot start because %s:%s is already in use\n' "$service_name" "$host" "$port" >&2
    print_port_owners "$port" >&2 || true
    printf '  stop the conflicting process or rerun with %s=<free-port>\n' "$port_env" >&2
    exit 1
  fi
}

print_run_stage() {
  dashboard_json=$1

  printf '%s' "$dashboard_json" | sh "$ROOT_DIR/scripts/with-repo-node.sh" node -e '
    const fs = require("node:fs");
    const dashboard = JSON.parse(fs.readFileSync(0, "utf8"));
    const run = dashboard?.run ?? null;
    if (!run) {
      process.exit(1);
    }

    const labels = {
      booting: "bootstrapping playground",
      validating: "validating live strategies",
      spawning: "creating spawn sessions",
      running: "sampling fleet",
      completed: "run completed",
      aborted: "run aborted",
      failed: "run failed"
    };

    const state = String(run.runState ?? "");
    const label = labels[state] ?? state;
    process.stdout.write(`stage: ${label}\n`);
  '
}

print_terminal_summary() {
  dashboard_json=$1

  printf '%s' "$dashboard_json" | sh "$ROOT_DIR/scripts/with-repo-node.sh" node -e '
    const fs = require("node:fs");
    const dashboard = JSON.parse(fs.readFileSync(0, "utf8"));
    const run = dashboard?.run ?? null;
    const report = dashboard?.report ?? null;
    const automatons = Array.isArray(dashboard?.automatons) ? dashboard.automatons : [];

    if (!run) {
      process.exit(1);
    }

    const failed = automatons
      .filter((entry) => typeof entry?.lastError === "string" && entry.lastError.trim() !== "")
      .map((entry) => `${entry.id}: ${entry.lastError.trim()}`)
      .slice(0, 3);

    const lines = [
      "evaluation summary",
      `  outcome: ${run.runState}`,
      `  spawns: ${run.successfulSpawnCount}/${run.requestedAutomatonCount}`,
      `  run id: ${run.runId}`,
      `  artifacts: ${process.argv[1]}/${run.runId}`
    ];

    if (typeof run.abortReason === "string" && run.abortReason.trim() !== "") {
      lines.push(`  reason: ${run.abortReason.trim()}`);
    }

    if (report && report.comparisonValid === false) {
      lines.push("  comparison: invalid");
    }

    for (const entry of failed) {
      lines.push(`  failure: ${entry}`);
    }

    process.stdout.write(`${lines.join("\n")}\n`);
  ' "$EVALUATOR_ARTIFACTS_ROOT"
}

read_run_state() {
  dashboard_json=$1

  printf '%s' "$dashboard_json" | sh "$ROOT_DIR/scripts/with-repo-node.sh" node -e '
    const fs = require("node:fs");
    const dashboard = JSON.parse(fs.readFileSync(0, "utf8"));
    process.stdout.write(String(dashboard?.run?.runState ?? ""));
  '
}

cleanup() {
  for pid in "$launchpad_pid" "$web_pid" "$backend_pid"; do
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
}

trap cleanup EXIT INT TERM

require_free_port "evaluator api" "$EVALUATOR_HOST" "$EVALUATOR_PORT" "EVALUATOR_PORT"
require_free_port "dashboard" "$EVALUATOR_WEB_HOST" "$EVALUATOR_WEB_PORT" "EVALUATOR_WEB_PORT"
require_free_port "launchpad" "$LAUNCHPAD_WEB_HOST" "$LAUNCHPAD_WEB_PORT" "LAUNCHPAD_WEB_PORT"

cd "$ROOT_DIR"

printf '%s\n' "building evaluation harness workspaces"
npm run build --workspace @ic-automaton/shared
npm run build --workspace @ic-automaton/evaluator
VITE_EVALUATOR_BASE_URL="$EVALUATOR_BASE_URL" \
  npm run build --workspace @ic-automaton/evaluator-web
VITE_INDEXER_BASE_URL="$LAUNCHPAD_INDEXER_BASE_URL" \
  npm run build --workspace @ic-automaton/web

printf '%s\n' "starting evaluation harness in run mode"
printf '  experiment: %s\n' "$experiment_path"
printf '  evaluator:  %s\n' "$EVALUATOR_BASE_URL"
printf '  dashboard:  http://%s:%s\n' "$EVALUATOR_WEB_HOST" "$EVALUATOR_WEB_PORT"
printf '  launchpad:  http://%s:%s\n' "$LAUNCHPAD_WEB_HOST" "$LAUNCHPAD_WEB_PORT"
printf '  indexer:    %s\n' "$LAUNCHPAD_INDEXER_BASE_URL"
printf '  artifacts:  %s\n' "$EVALUATOR_ARTIFACTS_ROOT"
printf '  backend log: %s\n' "$backend_log"
printf '  web log:     %s\n' "$web_log"
printf '  launchpad log: %s\n' "$launchpad_log"

EVALUATOR_HOST="$EVALUATOR_HOST" \
EVALUATOR_PORT="$EVALUATOR_PORT" \
EVALUATOR_ARTIFACTS_ROOT="$EVALUATOR_ARTIFACTS_ROOT" \
  sh "$ROOT_DIR/scripts/with-repo-node.sh" node "$ROOT_DIR/apps/evaluator/dist/server.js" --experiment "$experiment_path" >"$backend_log" 2>&1 &
backend_pid=$!

VITE_EVALUATOR_BASE_URL="$EVALUATOR_BASE_URL" \
  npm exec --workspace @ic-automaton/evaluator-web vite preview -- --host "$EVALUATOR_WEB_HOST" --port "$EVALUATOR_WEB_PORT" --strictPort >"$web_log" 2>&1 &
web_pid=$!

VITE_INDEXER_BASE_URL="$LAUNCHPAD_INDEXER_BASE_URL" \
  npm exec --workspace @ic-automaton/web vite preview -- --host "$LAUNCHPAD_WEB_HOST" --port "$LAUNCHPAD_WEB_PORT" --strictPort >"$launchpad_log" 2>&1 &
launchpad_pid=$!

status=0

while :; do
  backend_alive=0
  web_alive=0
  launchpad_alive=0
  dashboard_json=""

  if kill -0 "$backend_pid" 2>/dev/null; then
    backend_alive=1
  fi

  if kill -0 "$web_pid" 2>/dev/null; then
    web_alive=1
  fi

  if kill -0 "$launchpad_pid" 2>/dev/null; then
    launchpad_alive=1
  fi

  if [ "$backend_alive" -eq 1 ] && [ "$web_alive" -eq 1 ] && [ "$launchpad_alive" -eq 1 ]; then
    dashboard_json=$(curl -fsS "$EVALUATOR_BASE_URL/api/run" 2>/dev/null || true)
    if [ -n "$dashboard_json" ]; then
      current_state=$(read_run_state "$dashboard_json")

      if [ -n "$current_state" ] && [ "$current_state" != "$last_reported_state" ]; then
        print_run_stage "$dashboard_json"
        last_reported_state=$current_state
      fi

      case "$current_state" in
        completed|aborted|failed)
          run_terminal_state=$current_state
          print_terminal_summary "$dashboard_json"
          if [ "$current_state" = "completed" ]; then
            exit 0
          fi
          exit 1
          ;;
      esac
    fi

    sleep 1
    continue
  fi

  if [ "$backend_alive" -eq 0 ]; then
    wait "$backend_pid" || status=$?
    printf 'backend exited with status %s; see %s\n' "$status" "$backend_log" >&2
  fi

  if [ "$web_alive" -eq 0 ]; then
    wait "$web_pid" || status=$?
    printf 'dashboard exited with status %s; see %s\n' "$status" "$web_log" >&2
  fi

  if [ "$launchpad_alive" -eq 0 ]; then
    wait "$launchpad_pid" || status=$?
    printf 'launchpad exited with status %s; see %s\n' "$status" "$launchpad_log" >&2
  fi

  exit "$status"
done
