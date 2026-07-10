import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { gunzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const childWasmPath = resolvePath(
  process.env.CHILD_WASM_PATH,
  "components/ic-automaton/target/wasm32-wasip1/release/backend_nowasi.wasm"
);
const checkedDidPath = resolvePath(
  process.env.CHILD_DID_PATH,
  "components/ic-automaton/ic-automaton.did"
);
const requiredMethods = JSON.parse(
  fs.readFileSync(path.join(rootDir, "packages/canister-clients/automaton-methods.json"), "utf8")
);
const httpSourcePath = path.join(rootDir, "components/ic-automaton/src/http.rs");
const clientSourcePath = path.join(rootDir, "packages/canister-clients/src/index.ts");

function resolvePath(value, fallback) {
  return path.resolve(rootDir, value?.trim() || fallback);
}

function readArtifact(filePath) {
  if (!fs.existsSync(filePath)) {
    throw new Error(`missing child artifact at ${filePath}`);
  }

  const raw = fs.readFileSync(filePath);
  return filePath.endsWith(".gz") ? gunzipSync(raw) : raw;
}

function extractDid(wasmPath) {
  try {
    return execFileSync("candid-extractor", [wasmPath], { encoding: "utf8" });
  } catch (error) {
    throw new Error(
      `failed to extract Candid from ${wasmPath}: ${error instanceof Error ? error.message : String(error)}`
    );
  }
}

function assertExactDid(generatedDid, fixturePath) {
  const checkedDid = fs.readFileSync(fixturePath, "utf8");
  const normalizeDid = (did) =>
    (did
      .replace(/\/\/.*$/gm, "")
      .replace(/;\s*}/g, "}")
      .match(/[A-Za-z0-9_]+|[{}():;,<>?=]/g) || []
    ).join(" ");
  if (normalizeDid(generatedDid) !== normalizeDid(checkedDid)) {
    throw new Error(
      [
        "generated child Candid differs from the checked contract:",
        `- generated from ${childWasmPath}`,
        `- checked file ${fixturePath}`,
        "Run candid-extractor against the intended child artifact and review the DID change."
      ].join("\n")
    );
  }
}

function assertRequiredMethods(did) {
  const service = did.slice(did.indexOf("service :"));
  const missing = requiredMethods.filter(
    (methodName) => !new RegExp(`\\b${methodName}\\s*:`).test(service)
  );
  if (missing.length > 0) {
    throw new Error(`child Candid is missing required methods: ${missing.join(", ")}`);
  }
}

function assertHttpSchemas() {
  const httpSource = fs.readFileSync(httpSourcePath, "utf8");
  const clientSource = fs.readFileSync(clientSourcePath, "utf8");
  const requiredRoutes = [
    "/api/build-info",
    "/api/evm/config",
    "/api/scheduler/config",
    "/api/steward/status",
    "/api/snapshot",
    "/api/wallet/balance"
  ];
  const requiredClientFields = [
    "commit",
    "automaton_address",
    "chain_id",
    "base_tick_secs",
    "active_steward",
    "recent_turns",
    "bootstrap_pending"
  ];
  const missingRoutes = requiredRoutes.filter((route) => !httpSource.includes(`\"${route}\"`));
  const missingFields = requiredClientFields.filter((field) => !clientSource.includes(field));

  if (missingRoutes.length > 0 || missingFields.length > 0) {
    throw new Error(
      [
        "centralized automaton HTTP contract is incomplete:",
        missingRoutes.length > 0 ? `missing runtime routes: ${missingRoutes.join(", ")}` : null,
        missingFields.length > 0 ? `missing client fields: ${missingFields.join(", ")}` : null
      ]
        .filter(Boolean)
        .join("\n")
    );
  }
}

const childBinary = readArtifact(childWasmPath);
const generatedDid = extractDid(childWasmPath);
assertExactDid(generatedDid, checkedDidPath);
assertRequiredMethods(generatedDid);
assertHttpSchemas();

process.stdout.write(
  `child canister contract check passed (Wasm ${childBinary.length} bytes, ${requiredMethods.length} required methods)\n`
);
