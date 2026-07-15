import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

import { IDL } from "@dfinity/candid";
const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tmpDir = path.join(rootDir, "tmp");
const canister = process.env.FACTORY_CANISTER ?? "factory";
const environment = process.env.FACTORY_ENVIRONMENT ?? "local";
const wasmPath = process.env.CHILD_WASM_PATH ??
  path.join(rootDir, "target", "wasm32-wasip1", "release", "backend_nowasi.wasm");
const chunkSizeBytes = Number.parseInt(
  process.env.FACTORY_ARTIFACT_CHUNK_SIZE_BYTES ?? `${512 * 1024}`,
  10
);

if (!wasmPath) {
  throw new Error("CHILD_WASM_PATH is required");
}

if (!Number.isFinite(chunkSizeBytes) || chunkSizeBytes <= 0) {
  throw new Error(
    `FACTORY_ARTIFACT_CHUNK_SIZE_BYTES must be a positive integer, got ${JSON.stringify(
      process.env.FACTORY_ARTIFACT_CHUNK_SIZE_BYTES
    )}`
  );
}

function normalizeCommit(value) {
  const trimmed = value.trim();
  if (!/^[0-9a-f]{40}$/.test(trimmed)) {
    throw new Error(
      `invalid version commit ${JSON.stringify(value)}; expected a 40-character lowercase git SHA`
    );
  }
  return trimmed;
}

function resolveVersionCommit() {
  if (process.env.CHILD_VERSION_COMMIT) {
    return normalizeCommit(process.env.CHILD_VERSION_COMMIT);
  }

  if (process.env.FACTORY_VERSION_COMMIT) {
    return normalizeCommit(process.env.FACTORY_VERSION_COMMIT);
  }

  const head = execFileSync("git", ["-C", rootDir, "rev-parse", "HEAD"], {
    cwd: rootDir,
    encoding: "utf8"
  }).trim();
  return normalizeCommit(head);
}

const versionCommit = resolveVersionCommit();
const wasmBytes = fs.readFileSync(path.resolve(rootDir, wasmPath));
const expectedSha256 = createHash("sha256").update(wasmBytes).digest("hex");

fs.mkdirSync(tmpDir, { recursive: true });
const argsFilePath = path.join(tmpDir, "factory-artifact-upload.args.hex");

function writeArgsFile(types, values) {
  const encodedArgs = IDL.encode(types, values);
  fs.writeFileSync(argsFilePath, Buffer.from(encodedArgs).toString("hex"));
  return argsFilePath;
}

function callCanister(method, types = [], values = []) {
  const args = [
    "canister",
    "call",
    "-e",
    environment,
    canister,
    method,
    "--args-file",
    writeArgsFile(types, values),
    "--args-format",
    "hex"
  ];
  return execFileSync("icp", args, {
    cwd: rootDir,
    encoding: "utf8"
  }).trim();
}

callCanister(
  "begin_artifact_upload",
  [IDL.Text, IDL.Text, IDL.Nat64],
  [expectedSha256, versionCommit, BigInt(wasmBytes.length)]
);

const totalChunks = Math.ceil(wasmBytes.length / chunkSizeBytes);
for (let chunkIndex = 0; chunkIndex < totalChunks; chunkIndex += 1) {
  const start = chunkIndex * chunkSizeBytes;
  const end = Math.min(start + chunkSizeBytes, wasmBytes.length);
  const chunk = wasmBytes.subarray(start, end);
  callCanister("append_artifact_chunk", [IDL.Vec(IDL.Nat8)], [[...chunk]]);
  process.stdout.write(
    `uploaded chunk ${chunkIndex + 1}/${totalChunks} (${end}/${wasmBytes.length} bytes)\n`
  );
}

callCanister("commit_artifact_upload");

const healthOutput = callCanister("get_factory_health");
const expectedHealthSha256 = process.env.FACTORY_EXPECTED_SHA256 ?? expectedSha256;
const expectedHealthVersion = process.env.FACTORY_EXPECTED_VERSION_COMMIT ?? versionCommit;
if (!healthOutput.includes(expectedHealthSha256) || !healthOutput.includes(expectedHealthVersion)) {
  throw new Error(
    `factory health did not report the admitted artifact ${expectedHealthSha256} at ${expectedHealthVersion}`
  );
}

const summary = {
  canister,
  environment,
  wasmPath: path.resolve(rootDir, wasmPath),
  versionCommit,
  expectedSha256,
  chunkSizeBytes,
  totalChunks,
  argsFilePath,
  healthOutput
};

const summaryPath = path.join(tmpDir, "factory-artifact-upload.json");
fs.writeFileSync(summaryPath, `${JSON.stringify(summary, null, 2)}\n`);
process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
