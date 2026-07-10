import assert from "node:assert/strict";
import { test } from "node:test";

import { assertAllowedOperations } from "./lib/release-actions.mjs";

test("soft release cannot admit, install, upgrade, or reset", () => {
  assert.throws(() =>
    assertAllowedOperations("soft", ["pull-images", "artifact-upload", "install", "upgrade", "reset"])
  );
  assert.doesNotThrow(() => assertAllowedOperations("soft", ["pull-images", "compose-up", "health", "smoke", "record"]));
});

test("each explicit action has a distinct operation contract", () => {
  assert.doesNotThrow(() => assertAllowedOperations("hard-reset", ["pull-images", "compose-up", "reset", "bootstrap", "record"]));
  assert.doesNotThrow(() => assertAllowedOperations("admit-child", ["upload-child", "verify-factory-health", "record"]));
  assert.doesNotThrow(() => assertAllowedOperations("upgrade-named", ["snapshot", "upgrade", "verify", "record"]));
});
