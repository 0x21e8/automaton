import fs from "node:fs";
import path from "node:path";
import { gunzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const wasmPath = normalizeOptionalString(process.env.CHILD_WASM_PATH);

const requiredMethodExports = [
  {
    methodName: "get_spawn_bootstrap_view",
    exportLabels: ["canister_query get_spawn_bootstrap_view", "canister_update get_spawn_bootstrap_view"]
  },
  {
    methodName: "get_steward_status",
    exportLabels: ["canister_query get_steward_status", "canister_update get_steward_status"]
  },
  {
    methodName: "get_automaton_evm_address",
    exportLabels: ["canister_query get_automaton_evm_address", "canister_update get_automaton_evm_address"]
  },
  {
    methodName: "derive_automaton_evm_address",
    exportLabels: ["canister_update derive_automaton_evm_address"]
  }
];

function normalizeOptionalString(value) {
  if (value === undefined || value === null) {
    return null;
  }

  const normalized = String(value).trim();
  return normalized === "" ? null : normalized;
}

function resolveWasmBinary() {
  if (wasmPath === null) {
    throw new Error("CHILD_WASM_PATH is required to validate the child canister artifact.");
  }

  const resolvedWasmPath = path.resolve(rootDir, wasmPath);
  if (!fs.existsSync(resolvedWasmPath)) {
    throw new Error(`missing CHILD_WASM_PATH at ${resolvedWasmPath}`);
  }

  const rawWasm = fs.readFileSync(resolvedWasmPath);
  return {
    description: `child canister artifact ${resolvedWasmPath}`,
    binary: resolvedWasmPath.endsWith(".gz") ? gunzipSync(rawWasm) : rawWasm,
    resolvedWasmPath
  };
}

function hasAnyExport(binary, exportLabels) {
  return exportLabels.some((label) => binary.includes(Buffer.from(label, "utf8")));
}

const { description, binary, resolvedWasmPath } = resolveWasmBinary();
const failures = [];

for (const { methodName, exportLabels } of requiredMethodExports) {
  if (hasAnyExport(binary, exportLabels)) {
    continue;
  }

  failures.push(`missing exported canister method ${methodName}`);
}

if (failures.length > 0) {
  const artifactHint =
    resolvedWasmPath.endsWith("/target/wasm32-unknown-unknown/release/backend.wasm") ||
    resolvedWasmPath.endsWith("\\target\\wasm32-unknown-unknown\\release\\backend.wasm")
      ? [
          "This looks like ic-automaton's raw wasm32-unknown-unknown build output.",
          "For local playground installs, use the canister-ready artifact produced by ic-automaton/scripts/build-backend-wasm.sh.",
          "That artifact is typically target/wasm32-wasip1/release/backend_nowasi.wasm."
        ]
      : [];
  throw new Error(
    [
      `Child canister artifact is incompatible with automaton-launchpad spawn verification (${description}).`,
      ...failures.map((failure) => `- ${failure}`),
      ...artifactHint
    ].join("\n")
  );
}

process.stdout.write(
  `child canister artifact compatibility check passed (${description})\n`
);
