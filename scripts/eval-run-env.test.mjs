import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const helper = join(root, "scripts/lib/eval-run-env.sh");

test("isolated eval runs discover every bootstrap deployment artifact from PLAYGROUND_TMP_DIR", () => {
  const output = execFileSync("sh", ["-c", `. "$1"; printf '%s\\n' "$LOCAL_EVM_DEPLOYMENT_FILE" "$AUTOMATON_INBOX_DEPLOYMENT_FILE" "$PLAYGROUND_FACTORY_CANISTER_ID_FILE"`, "sh", helper], {
    encoding: "utf8",
    env: { ...process.env, ROOT_DIR: root, PLAYGROUND_TMP_DIR: "/tmp/eval-isolated", LOCAL_EVM_DEPLOYMENT_FILE: "", AUTOMATON_INBOX_DEPLOYMENT_FILE: "", PLAYGROUND_FACTORY_CANISTER_ID_FILE: "" }
  });
  assert.deepEqual(output.trim().split("\n"), [
    "/tmp/eval-isolated/local-escrow-deployment.json",
    "/tmp/eval-isolated/automaton-inbox-deployment.json",
    "/tmp/eval-isolated/factory-canister-id.txt"
  ]);
});

test("isolated eval runs propagate non-default service ports into nested playground bootstrap", () => {
  const output = execFileSync("sh", ["-c", `. "$1"; printf '%s\n' "$LOCAL_EVM_PORT" "$PLAYGROUND_INDEXER_PORT" "$PLAYGROUND_RPC_GATEWAY_PORT" "$PLAYGROUND_SERVICE_DIR" "$PLAYGROUND_STATUS_FILE"`, "sh", helper], {
    encoding: "utf8",
    env: {
      ...process.env,
      ROOT_DIR: root,
      PLAYGROUND_TMP_DIR: "/tmp/eval-non-default",
      LOCAL_EVM_PORT: "",
      PLAYGROUND_INDEXER_PORT: "",
      PLAYGROUND_RPC_GATEWAY_PORT: "",
      PLAYGROUND_SERVICE_DIR: "",
      PLAYGROUND_STATUS_FILE: "",
      LOCAL_EVM_RPC_URL: "http://127.0.0.1:6605",
      PLAYGROUND_INDEXER_BASE_URL: "http://127.0.0.1:6601",
      LAUNCHPAD_INDEXER_BASE_URL: "http://127.0.0.1:6601",
      PLAYGROUND_RPC_GATEWAY_URL: "http://127.0.0.1:6602"
    }
  });
  assert.deepEqual(output.trim().split("\n"), [
    "6605",
    "6601",
    "6602",
    "/tmp/eval-non-default/playground-services",
    "/tmp/eval-non-default/playground-status.json"
  ]);
});
