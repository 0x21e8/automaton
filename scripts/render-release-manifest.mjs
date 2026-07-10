import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { validateReleaseManifest } from "./lib/release-manifest.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const mode = option("--mode") ?? "dry-run";
const output = option("--output") ?? path.join(rootDir, "tmp", "release-manifest.json");

if (!new Set(["dry-run", "publish"]).has(mode)) {
  throw new Error(`--mode must be dry-run or publish, got ${mode}`);
}

const sourceCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: rootDir, encoding: "utf8" }).trim();
if (!/^[0-9a-f]{40}$/.test(sourceCommit)) throw new Error("git HEAD is not a lowercase 40-character SHA");
const dirty = execFileSync("git", ["status", "--porcelain", "--untracked-files=all"], {
  cwd: rootDir,
  encoding: "utf8"
}).trim() !== "";
if (mode === "publish" && dirty) throw new Error("refusing to publish from a dirty checkout");

const artifacts = {
  automatonWasm: artifact("dist/automaton.wasm", sourceCommit),
  factoryWasm: artifact("dist/factory.wasm", sourceCommit)
};
const images = {
  web: image("RELEASE_WEB_IMAGE_REF", "ghcr.io/example/automaton-playground-web"),
  indexer: image("RELEASE_INDEXER_IMAGE_REF", "ghcr.io/example/automaton-playground-indexer"),
  rpcGateway: image("RELEASE_RPC_GATEWAY_IMAGE_REF", "ghcr.io/example/automaton-playground-rpc-gateway")
};

const manifest = {
  schemaVersion: 2,
  release: {
    sourceCommit,
    environmentVersion: process.env.RELEASE_ENVIRONMENT_VERSION?.trim() || `playground-${sourceCommit.slice(0, 12)}`,
    createdAt: new Date().toISOString(),
    ...(dirty ? { dirty: true } : {})
  },
  images,
  artifacts,
  ops: { sourceCommit }
};
validateReleaseManifest(manifest, { allowDirty: mode === "dry-run" });
const outputPath = path.resolve(rootDir, output);
fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
process.stdout.write(`${outputPath}\n`);

function option(name) {
  const index = args.indexOf(name);
  return index === -1 ? null : args[index + 1] ?? null;
}

function artifact(relativePath, commit) {
  const filePath = path.join(rootDir, relativePath);
  if (!fs.existsSync(filePath)) throw new Error(`missing release artifact ${filePath}`);
  return {
    fileName: path.basename(filePath),
    sha256: createHash("sha256").update(fs.readFileSync(filePath)).digest("hex"),
    sourceCommit: commit
  };
}

function image(envName, fallbackRepository) {
  const supplied = process.env[envName]?.trim();
  const digest = "sha256:" + "0".repeat(64);
  const ref = supplied || `${fallbackRepository}@${digest}`;
  const at = ref.lastIndexOf("@");
  if (at <= 0) throw new Error(`${envName} must be an immutable digest ref`);
  return {
    ref,
    digest: ref.slice(at + 1),
    repository: ref.slice(0, at),
    tag: sourceCommit
  };
}
