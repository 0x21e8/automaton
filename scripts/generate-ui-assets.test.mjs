import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("UI token generation is deterministic and check detects a stale fixture", () => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "automaton-ui-"));
  fs.cpSync(path.join(repoRoot, "packages/ui"), path.join(fixture, "packages/ui"), { recursive: true });
  fs.cpSync(path.join(repoRoot, "packages/shared/src"), path.join(fixture, "packages/shared/src"), { recursive: true });
  fs.mkdirSync(path.join(fixture, "components/ic-automaton/src"), { recursive: true });
  execFileSync("node", ["scripts/generate-ui-assets.mjs", "--write", "--root", fixture], { cwd: repoRoot });
  execFileSync("node", ["scripts/generate-ui-assets.mjs", "--check", "--root", fixture], { cwd: repoRoot });
  const generated = path.join(fixture, "packages/ui/src/generated/themes.css");
  fs.appendFileSync(generated, "/* stale fixture edit */\n");
  assert.throws(
    () => execFileSync("node", ["scripts/generate-ui-assets.mjs", "--check", "--root", fixture], { cwd: repoRoot }),
    /generated UI assets are stale/
  );
});
