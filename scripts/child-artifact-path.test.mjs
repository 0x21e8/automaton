import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = path.resolve(dirname(fileURLToPath(import.meta.url)), "..");
const canonical = path.join(root, "target/wasm32-wasip1/release/backend_nowasi.wasm");
const legacy = path.join(root, "components/ic-automaton/target/wasm32-wasip1/release/backend_nowasi.wasm");
const sha = (file) => createHash("sha256").update(fs.readFileSync(file)).digest("hex");

test("workspace consumers select canonical child Wasm even when a legacy candidate diverges", () => {
  assert.ok(fs.existsSync(canonical), `missing canonical child Wasm: ${canonical}`);
  const canonicalSha = sha(canonical);
  const fixtureDirectory = fs.mkdtempSync(path.join(os.tmpdir(), "child-artifact-"));
  try {
    const divergentLegacy = path.join(fixtureDirectory, "components/ic-automaton/target/wasm32-wasip1/release/backend_nowasi.wasm");
    fs.mkdirSync(path.dirname(divergentLegacy), { recursive: true });
    fs.writeFileSync(divergentLegacy, "divergent legacy fixture");
    assert.notEqual(sha(divergentLegacy), canonicalSha);
    for (const relative of [
      "scripts/playground-bootstrap.sh",
      "scripts/upload-factory-artifact.mjs",
      "scripts/validate-child-canister-interface.mjs",
      "package.json"
    ]) {
      const source = fs.readFileSync(path.join(root, relative), "utf8");
      assert.match(source, /target.{0,80}wasm32-wasip1.{0,80}backend_nowasi\.wasm/s);
      assert.doesNotMatch(source, /components\/ic-automaton\/target\/wasm32-wasip1\/release\/backend_nowasi\.wasm/);
    }
  } finally {
    fs.rmSync(fixtureDirectory, { recursive: true, force: true });
  }
  if (fs.existsSync(legacy)) console.log(`ignored legacy child Wasm: ${legacy} sha256=${sha(legacy)}`);
  assert.equal(sha(canonical), canonicalSha, "canonical child Wasm changed while checking consumer selection");
});
