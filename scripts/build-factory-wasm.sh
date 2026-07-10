#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [output_wasm_path]" >&2
  exit 1
fi

for tool in cargo candid-extractor ic-wasm; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing required tool: $tool" >&2
    exit 1
  fi
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root_dir="$(cd "${script_dir}/.." && pwd)"
target_root="${CARGO_TARGET_DIR:-${root_dir}/target}"
source_wasm="${target_root}/wasm32-unknown-unknown/release/factory.wasm"
output_path="${1:-${root_dir}/dist/factory.wasm}"
if [[ "$output_path" != /* ]]; then
  output_path="${root_dir}/${output_path}"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
extracted_did="${tmp_dir}/factory.did"
with_metadata="${tmp_dir}/factory-with-metadata.wasm"

mkdir -p "$(dirname "$output_path")"
cargo build -p factory --target wasm32-unknown-unknown --release
candid-extractor "$source_wasm" >"$extracted_did"
if ! cmp -s "$extracted_did" "$root_dir/backend/factory/factory.did"; then
  echo "Factory Candid differs from backend/factory/factory.did" >&2
  diff -u "$root_dir/backend/factory/factory.did" "$extracted_did" >&2 || true
  exit 1
fi

ic-wasm "$source_wasm" \
  -o "$with_metadata" \
  metadata candid:service \
  -f "$root_dir/backend/factory/factory.did" \
  -v public
cp "$with_metadata" "$output_path"
if command -v sha256sum >/dev/null 2>&1; then
  digest=$(sha256sum "$output_path" | awk '{print $1}')
else
  digest=$(shasum -a 256 "$output_path" | awk '{print $1}')
fi
printf '%s  %s\n' "$digest" "$output_path" >"${output_path}.sha256"
echo "Built factory Wasm: $output_path"
