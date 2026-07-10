#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/validate.sh [fast|strict|full] [--working-tree]

Modes:
  fast    Run commit-time validation checks.
  strict  Run fast checks plus built-wasm and PocketIC validation.
  full    Alias for strict.

Flags:
  --working-tree  Allow running with uncommitted changes.
  -h, --help      Show this help text.
EOF
}

mode="fast"
allow_dirty=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    fast|strict|full)
      mode="$1"
      ;;
    --working-tree)
      allow_dirty=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
cd "${repo_root}"

if [[ "${mode}" == "full" ]]; then
  mode="strict"
fi

if [[ -z "${ICP_HOME:-}" ]]; then
  export ICP_HOME="/tmp/icp-home-validate"
fi
mkdir -p "${ICP_HOME}"

if [[ "${allow_dirty}" -ne 1 ]] && [[ -n "$(git status --short)" ]]; then
  echo "Working tree is not clean. Re-run with --working-tree to validate local edits." >&2
  exit 1
fi

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required tool: $1" >&2
    exit 1
  fi
}

run_step() {
  local label="$1"
  shift
  echo
  echo "==> ${label}"
  "$@"
}

check_candid_in_sync() {
  require_cmd candid-extractor

  local tmp_did
  tmp_did="$(mktemp)"

  "${repo_root}/scripts/generate-candid.sh" "${tmp_did}" >/dev/null
  if ! diff -u "${repo_root}/ic-automaton.did" "${tmp_did}"; then
    rm -f "${tmp_did}"
    echo "Candid interface is out of sync. Run 'icp build' and './scripts/generate-candid.sh ic-automaton.did'." >&2
    exit 1
  fi

  rm -f "${tmp_did}"
}

sync_pocketic_wasm_artifact() {
  local source_wasm="${repo_root}/target/wasm32-wasip1/release/backend_nowasi.wasm"
  local target_wasm="${repo_root}/target/wasm32-unknown-unknown/release/backend.wasm"

  if [[ ! -f "${source_wasm}" ]]; then
    echo "Missing PocketIC source artifact: ${source_wasm}" >&2
    exit 1
  fi

  mkdir -p "$(dirname "${target_wasm}")"
  cp "${source_wasm}" "${target_wasm}"
}

run_pre_commit_if_configured() {
  if [[ -f "${repo_root}/.pre-commit-config.yaml" || -f "${repo_root}/.pre-commit-config.yml" ]]; then
    require_cmd pre-commit
    run_step "pre-commit" pre-commit run --all-files
  fi
}

require_cmd cargo
require_cmd git

run_pre_commit_if_configured
run_step "cargo fmt" cargo fmt --all -- --check
run_step "cargo check" cargo check
run_step "check backend wasi" cargo check --target wasm32-wasip1 -p backend
run_step "cargo clippy" cargo clippy --all-targets --all-features -- -D warnings
run_step "cargo test" cargo test

if command -v forge >/dev/null 2>&1; then
  run_step "forge test" forge test --root evm --offline
fi

if [[ "${mode}" == "strict" ]]; then
  require_cmd icp
  run_step "icp build backend" icp build backend
  run_step "sync pocketic wasm" sync_pocketic_wasm_artifact
  run_step "candid sync" check_candid_in_sync
  run_step "pocketic tests" cargo test --features pocketic_tests
fi

echo
echo "Validation passed (${mode})"
