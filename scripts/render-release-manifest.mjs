import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { renderReleaseManifest } from "./lib/render-release-manifest.mjs";

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
  automatonWasm: artifact("dist/automaton.wasm"),
  factoryWasm: artifact("dist/factory.wasm")
};
const imageRefs = {
  web: imageRef("RELEASE_WEB_IMAGE_REF", "ghcr.io/example/automaton-playground-web"),
  indexer: imageRef("RELEASE_INDEXER_IMAGE_REF", "ghcr.io/example/automaton-playground-indexer"),
  rpcGateway: imageRef("RELEASE_RPC_GATEWAY_IMAGE_REF", "ghcr.io/example/automaton-playground-rpc-gateway")
};

const manifest = renderReleaseManifest({
  sourceCommit,
  dirty,
  mode,
  environmentVersion: process.env.RELEASE_ENVIRONMENT_VERSION?.trim() || `playground-${sourceCommit.slice(0, 12)}`,
  imageRefs,
  artifacts,
  createdAt: new Date().toISOString()
});
const outputPath = path.resolve(rootDir, output);
fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
process.stdout.write(`${outputPath}\n`);

function option(name) {
  const index = args.indexOf(name);
  return index === -1 ? null : args[index + 1] ?? null;
}

function artifact(relativePath) {
  const filePath = path.join(rootDir, relativePath);
  if (!fs.existsSync(filePath)) throw new Error(`missing release artifact ${filePath}`);
  return {
    fileName: path.basename(filePath),
    bytes: fs.readFileSync(filePath)
  };
}

function imageRef(envName, fallbackRepository) {
  const supplied = process.env[envName]?.trim();
  const digest = "sha256:" + "0".repeat(64);
  return supplied || `${fallbackRepository}@${digest}`;
}
