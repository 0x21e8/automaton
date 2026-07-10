import assert from "node:assert/strict";
import { test } from "node:test";

import { verifyTerminalParity } from "./verify-terminal-parity.mjs";

const canonicalSource = '["help", "help", "Help", "public", "local", "local"]';
const generatedSource = '"name": "help"';
const registrySource = 'import { terminalCommandRegistry } from "@ic-automaton/shared";';

test("terminal parity accepts matching surfaces", () => {
  assert.deepEqual(
    verifyTerminalParity({
      canonicalSource,
      generatedSource,
      embeddedSource: 'switch (cmd) { case "help": default:',
      registrySource
    }),
    { commandCount: 1 }
  );
});

test("terminal parity rejects a missing embedded command", () => {
  assert.throws(
    () => verifyTerminalParity({ canonicalSource, generatedSource, embeddedSource: "switch (cmd) { default:", registrySource }),
    /embedded dispatcher missing help/
  );
});
