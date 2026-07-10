#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [output_wasm_path]" >&2
  exit 1
fi

for tool in cargo wasi2ic candid-extractor ic-wasm; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing required tool: $tool" >&2
    exit 1
  fi
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

if [[ -f "${repo_root}/../../Cargo.toml" ]] && grep -q 'components/ic-automaton' "${repo_root}/../../Cargo.toml"; then
  workspace_root="$(cd "${repo_root}/../.." && pwd)"
else
  workspace_root="${repo_root}"
fi
target_root="${CARGO_TARGET_DIR:-${workspace_root}/target}"

if [[ -f "${workspace_root}/package.json" ]] && grep -q 'verify:ui-tokens' "${workspace_root}/package.json"; then
  npm --prefix "${workspace_root}" run verify:ui-tokens
fi

wasm_build_path="${target_root}/wasm32-wasip1/release/backend.wasm"
wasm_nowasi_path="${target_root}/wasm32-wasip1/release/backend_nowasi.wasm"

if [[ $# -eq 1 ]]; then
  if [[ "$1" = /* ]]; then
    output_path="$1"
  else
    output_path="$(pwd)/$1"
  fi
elif [[ -n "${ICP_WASM_OUTPUT_PATH:-}" ]]; then
  output_path="${ICP_WASM_OUTPUT_PATH}"
else
  output_path="${repo_root}/target/wasm32-wasip1/release/backend_nowasi.wasm"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

tmp_did_path="${tmp_dir}/backend.did"
tmp_wasm_path="${tmp_dir}/backend_with_metadata.wasm"

mkdir -p "$(dirname "${output_path}")"

cargo build -p backend --target wasm32-wasip1 --release
wasi2ic "${wasm_build_path}" "${wasm_nowasi_path}"
candid-extractor "${wasm_nowasi_path}" > "${tmp_did_path}"
ic-wasm "${wasm_nowasi_path}" -o "${tmp_wasm_path}" metadata candid:service -f "${tmp_did_path}" -v public
cp "${tmp_wasm_path}" "${wasm_nowasi_path}"
if [[ "${output_path}" != "${wasm_nowasi_path}" ]]; then
  cp "${tmp_wasm_path}" "${output_path}"
fi
