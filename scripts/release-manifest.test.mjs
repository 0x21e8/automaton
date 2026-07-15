import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { readReleaseManifest, validateReleaseManifest } from "./lib/release-manifest.mjs";
import { renderReleaseManifest } from "./lib/render-release-manifest.mjs";

const sourceCommit = "1".repeat(40);
const createdAt = "2026-07-15T10:20:30.000Z";
const digest = `sha256:${"2".repeat(64)}`;
const inputs = {
  sourceCommit,
  dirty: false,
  mode: "dry-run",
  environmentVersion: "playground-test",
  imageRefs: {
    web: `ghcr.io/example/web@${digest}`,
    indexer: `ghcr.io/example/indexer@${digest}`,
    rpcGateway: `ghcr.io/example/rpc-gateway@${digest}`
  },
  artifacts: {
    automatonWasm: { fileName: "automaton.wasm", bytes: Buffer.from("automaton fixture") },
    factoryWasm: { fileName: "factory.wasm", bytes: Buffer.from("factory fixture") }
  },
  createdAt
};

test("dry-run + clean omits the dirty marker and validates normally", () => {
  const manifest = renderReleaseManifest(inputs);
  assert.equal(manifest.release.dirty, undefined);
  assert.deepEqual(validateReleaseManifest(manifest), manifest);
});

test("dry-run + dirty sets the marker and requires allowDirty", (t) => {
  const manifest = renderReleaseManifest({ ...inputs, dirty: true });
  assert.equal(manifest.release.dirty, true);
  assert.deepEqual(validateReleaseManifest(manifest, { allowDirty: true }), manifest);
  assert.throws(() => validateReleaseManifest(manifest), /dirty releases are not publishable/);
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "release-manifest-test-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const file = path.join(directory, "manifest.json");
  fs.writeFileSync(file, JSON.stringify(manifest));
  assert.deepEqual(readReleaseManifest(file, { allowDirty: true }), manifest);
  assert.throws(() => readReleaseManifest(file), /dirty releases are not publishable/);
});

test("publish + dirty rejects before producing a manifest", () => {
  assert.throws(
    () => renderReleaseManifest({ ...inputs, mode: "publish", dirty: true }),
    /dirty releases are not publishable/
  );
});

test("publish + clean produces a publishable manifest", () => {
  const manifest = renderReleaseManifest({ ...inputs, mode: "publish" });
  assert.deepEqual(validateReleaseManifest(manifest), manifest);
});

test("artifact hashes are deterministic and change with fixture bytes", () => {
  const first = renderReleaseManifest(inputs);
  const second = renderReleaseManifest(inputs);
  const changed = renderReleaseManifest({
    ...inputs,
    artifacts: {
      ...inputs.artifacts,
      automatonWasm: { fileName: "automaton.wasm", bytes: Buffer.from("changed fixture") }
    }
  });
  assert.equal(first.artifacts.automatonWasm.sha256, second.artifacts.automatonWasm.sha256);
  assert.equal(
    first.artifacts.automatonWasm.sha256,
    createHash("sha256").update(inputs.artifacts.automatonWasm.bytes).digest("hex")
  );
  assert.notEqual(first.artifacts.automatonWasm.sha256, changed.artifacts.automatonWasm.sha256);
});

test("timestamp and source commit come from injected values", () => {
  const manifest = renderReleaseManifest(inputs);
  assert.equal(manifest.release.createdAt, createdAt);
  assert.equal(manifest.release.sourceCommit, sourceCommit);
  assert.equal(manifest.ops.sourceCommit, sourceCommit);
  assert.equal(manifest.artifacts.factoryWasm.sourceCommit, sourceCommit);
});

test("malformed source commits are rejected", () => {
  assert.throws(() => renderReleaseManifest({ ...inputs, sourceCommit: "not-a-sha" }), /sourceCommit/);
});

test("image refs without an immutable digest are rejected", () => {
  assert.throws(
    () => renderReleaseManifest({ ...inputs, imageRefs: { ...inputs.imageRefs, web: "ghcr.io/example/web:latest" } }),
    /immutable digest ref/
  );
});

test("production CLI rejects publish mode from a dirty checkout", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "release-manifest-cli-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, "scripts", "lib"), { recursive: true });
  fs.mkdirSync(path.join(root, "dist"));
  fs.copyFileSync(new URL("render-release-manifest.mjs", import.meta.url), path.join(root, "scripts", "render-release-manifest.mjs"));
  fs.copyFileSync(
    new URL("lib/render-release-manifest.mjs", import.meta.url),
    path.join(root, "scripts", "lib", "render-release-manifest.mjs")
  );
  fs.copyFileSync(
    new URL("lib/release-manifest.mjs", import.meta.url),
    path.join(root, "scripts", "lib", "release-manifest.mjs")
  );
  fs.writeFileSync(path.join(root, "dist", "automaton.wasm"), "automaton fixture");
  fs.writeFileSync(path.join(root, "dist", "factory.wasm"), "factory fixture");
  fs.writeFileSync(path.join(root, "README.md"), "clean baseline\n");
  git(root, "init");
  git(root, "config", "user.email", "release-manifest-test@example.invalid");
  git(root, "config", "user.name", "Release Manifest Test");
  git(root, "add", ".");
  git(root, "commit", "-m", "fixture baseline");
  fs.appendFileSync(path.join(root, "README.md"), "dirty marker\n");

  const result = spawnSync(process.execPath, ["scripts/render-release-manifest.mjs", "--mode", "publish"], {
    cwd: root,
    encoding: "utf8"
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /refusing to publish from a dirty checkout/);
  assert.equal(fs.existsSync(path.join(root, "tmp", "release-manifest.json")), false);
});

function git(cwd, ...args) {
  execFileSync("git", args, { cwd, stdio: "ignore" });
}
