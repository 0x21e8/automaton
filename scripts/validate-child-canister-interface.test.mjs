import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const scriptPath = path.join(path.dirname(fileURLToPath(import.meta.url)), "validate-child-canister-interface.mjs");
const rootDir = path.resolve(path.dirname(scriptPath), "..");
const checkedDidPath = path.join(rootDir, "components/ic-automaton/ic-automaton.did");

test("an altered checked DID fails the built child contract gate", () => {
  const fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), "child-contract-") );
  const fixturePath = path.join(fixtureDir, "altered.did");
  const alteredDid = fs
    .readFileSync(checkedDidPath, "utf8")
    .replace("get_steward_status :", "get_steward_status_drifted :");
  fs.writeFileSync(fixturePath, alteredDid);

  assert.throws(
    () =>
      execFileSync("node", [scriptPath], {
        cwd: rootDir,
        env: { ...process.env, CHILD_DID_PATH: fixturePath },
        encoding: "utf8",
        stdio: "pipe"
      }),
    (error) =>
      error.status !== 0 &&
      `${error.stderr ?? ""}${error.stdout ?? ""}`.includes("generated child Candid differs")
  );

  fs.rmSync(fixtureDir, { recursive: true, force: true });
});
