#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [output_did_path]" >&2
  echo "Example: $0 ic-automaton.did" >&2
  exit 1
fi

if ! command -v candid-extractor >/dev/null 2>&1; then
  echo "Missing required tool: candid-extractor" >&2
  echo "Install it with: cargo install candid-extractor" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

if [[ $# -eq 1 ]]; then
  if [[ "$1" = /* ]]; then
    output_path="$1"
  else
    output_path="${repo_root}/$1"
  fi
else
  output_path="${repo_root}/ic-automaton.did"
fi

if [[ -f "${repo_root}/../../Cargo.toml" ]] && grep -q 'components/ic-automaton' "${repo_root}/../../Cargo.toml"; then
  target_root="$(cd "${repo_root}/../.." && pwd)/target"
else
  target_root="${repo_root}/target"
fi

wasm_path=""
for candidate in \
  "${target_root}/wasm32-wasip1/release/backend_nowasi.wasm" \
  "${target_root}/wasm32-wasip1/release/backend.wasm" \
  "${target_root}/wasm32-unknown-unknown/release/backend.wasm" \
  "${target_root}/wasm32-unknown-unknown/release/deps/backend.wasm"
do
  if [[ -f "${candidate}" ]]; then
    wasm_path="${candidate}"
    break
  fi
done

if [[ -z "${wasm_path}" ]]; then
  echo "Could not find backend wasm artifact in target/wasm32-wasip1 or target/wasm32-unknown-unknown." >&2
  echo "Run 'icp build' first, then run this script again." >&2
  exit 1
fi

mkdir -p "$(dirname "${output_path}")"
candid-extractor "${wasm_path}" | sed 's/[[:space:]]*$//' > "${output_path}"

wasm_sha=$(shasum -a 256 "${wasm_path}" | awk '{print $1}')
echo "Generated Candid: ${output_path} from ${wasm_path} sha256=${wasm_sha}"
