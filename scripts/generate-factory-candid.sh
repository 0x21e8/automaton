#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wasm_path="${1:-${repo_root}/dist/factory.wasm}"
output_path="${2:-${repo_root}/backend/factory/factory.did}"

if [[ ! -f "${wasm_path}" ]]; then
  echo "Factory Wasm not found: ${wasm_path}" >&2
  exit 1
fi

candid-extractor "${wasm_path}" | sed 's/[[:space:]]*$//' > "${output_path}"
echo "Generated factory Candid: ${output_path}"
