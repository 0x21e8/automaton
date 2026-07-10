import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { test } from "node:test";

import { readReleaseManifest } from "./lib/release-manifest.mjs";

test("dry-run manifests carry a dirty marker and production validation rejects them", () => {
  const output = "/tmp/automaton-launchpad-release-manifest-test.json";
  execFileSync("node", ["scripts/render-release-manifest.mjs", "--mode", "dry-run", "--output", output], {
    cwd: new URL("..", import.meta.url),
    stdio: "ignore"
  });

  const manifest = readReleaseManifest(output, { allowDirty: true });
  assert.equal(manifest.schemaVersion, 2);
  assert.equal(manifest.release.dirty, true);
  assert.throws(() => readReleaseManifest(output), /dirty releases are not publishable/);
});
