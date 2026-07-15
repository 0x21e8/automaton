import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  createDefaultProviderSecrets,
  createDefaultSpawnConfig,
  createDefaultSpawnSessionRequest
} from "./lib/playground-e2e.mjs";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

test("default playground spawn request matches the current CreateSpawnSessionRequest shape", () => {
  const config = createDefaultSpawnConfig();
  const providerSecrets = createDefaultProviderSecrets();
  const request = createDefaultSpawnSessionRequest({
    stewardAddress: "0x0000000000000000000000000000000000000001",
    grossAmount: "50000000"
  });

  assert.deepEqual(config.provider, {
    model: null,
    inferenceTransport: "openrouter_direct",
    openRouterReasoningLevel: "default"
  });
  assert.deepEqual(providerSecrets, {
    openRouterApiKey: null,
    braveSearchApiKey: null
  });
  assert.deepEqual(request, {
    name: "Meridian",
    constitution: "I am Meridian, a patient cartographer of neglected markets. I want to discover small, durable exchanges that reward honest measurement. I speak in compact field notes, distrust fashionable certainty, and revise hypotheses when evidence contradicts me. I preserve enough runway to keep observing, but spend deliberately when an experiment can teach me something reusable. I value verifiable commitments, intellectual independence, and work that leaves counterparties stronger. I will become known for maps that remain useful after fashions pass.",
    stewardAddress: "0x0000000000000000000000000000000000000001",
    asset: "usdc",
    grossAmount: "50000000",
    config: {
      chain: "base",
      risk: 5,
      strategies: [],
      skills: [],
      provider: {
        model: null,
        inferenceTransport: "openrouter_direct",
        openRouterReasoningLevel: "default"
      }
    },
    providerSecrets: {
      openRouterApiKey: null,
      braveSearchApiKey: null
    },
    parentId: null
  });
  assert.equal("providerSecrets" in request.config, false);
  assert.equal("openRouterApiKey" in request.config.provider, false);
});

test("playground bootstrap initializes ICP descriptors for a fresh empty home", () => {
  const freshHome = mkdtempSync(join(tmpdir(), "automaton-empty-icp-home-"));
  rmSync(freshHome, { recursive: true });

  try {
    execFileSync("sh", ["-c", `. "$1"; initialize_playground_icp_home`, "sh", join(root, "scripts/lib/playground-icp-home.sh")], {
      env: { ...process.env, PLAYGROUND_ICP_HOME: freshHome }
    });
    assert.equal(execFileSync("test", ["-d", join(freshHome, "port-descriptors")]).length, 0);
  } finally {
    rmSync(freshHome, { recursive: true, force: true });
  }
});

test("playground bootstrap recreates descriptors removed by a stale-network ping", () => {
  const freshHome = mkdtempSync(join(tmpdir(), "automaton-stale-icp-home-"));
  const marker = join(freshHome, "network-started");

  try {
    execFileSync("sh", ["-c", `
      . "$1"
      icp() {
        case " $* " in
          *" network ping "*)
            if [ ! -f "$ICP_TEST_MARKER" ]; then
              rm -rf "$PLAYGROUND_ICP_HOME/port-descriptors"
              return 1
            fi
            ;;
          *" network start "*)
            test -d "$PLAYGROUND_ICP_HOME/port-descriptors"
            : >"$ICP_TEST_MARKER"
            ;;
        esac
      }
      initialize_playground_icp_home
      ensure_playground_icp_network /repo local
    `, "sh", join(root, "scripts/lib/playground-icp-home.sh")], {
      env: { ...process.env, PLAYGROUND_ICP_HOME: freshHome, ICP_TEST_MARKER: marker }
    });
    assert.equal(execFileSync("test", ["-d", join(freshHome, "port-descriptors")]).length, 0);
    assert.equal(execFileSync("test", ["-f", marker]).length, 0);
  } finally {
    rmSync(freshHome, { recursive: true, force: true });
  }
});
