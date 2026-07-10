import { describe, expect, it } from "vitest";

import { validateReleaseManifest } from "../src/release.ts";

const sha = "0123456789abcdef0123456789abcdef01234567";
const digest = "a".repeat(64);

function validManifest() {
  return {
    schemaVersion: 2,
    release: {
      sourceCommit: sha,
      environmentVersion: "playground-test",
      createdAt: "2026-07-10T00:00:00Z"
    },
    images: {
      web: { ref: `registry.example/web@sha256:${digest}`, digest: `sha256:${digest}` },
      indexer: { ref: `registry.example/indexer@sha256:${digest}`, digest: `sha256:${digest}` },
      rpcGateway: { ref: `registry.example/gateway@sha256:${digest}`, digest: `sha256:${digest}` }
    },
    artifacts: {
      automatonWasm: { fileName: "automaton.wasm", sha256: digest, sourceCommit: sha },
      factoryWasm: { fileName: "factory.wasm", sha256: digest, sourceCommit: sha }
    },
    ops: { sourceCommit: sha }
  };
}

describe("release manifest schema v2", () => {
  it("accepts a complete immutable manifest", () => {
    expect(validateReleaseManifest(validManifest()).schemaVersion).toBe(2);
  });

  it.each([
    ["schemaVersion", { schemaVersion: 1 }],
    ["source commit", { release: { sourceCommit: "bad" } }],
    ["image digest", { images: { web: { ref: "registry/web:latest", digest: "bad" } } }],
    ["image ref", { images: { web: { ref: `registry/web@sha256:${"b".repeat(64)}`, digest: `sha256:${digest}` } } }],
    ["artifact hash", { artifacts: { factoryWasm: { fileName: "factory.wasm", sha256: "bad", sourceCommit: sha } } }],
    ["artifact filename", { artifacts: { factoryWasm: { fileName: "../factory.wasm", sha256: digest, sourceCommit: sha } } }],
    ["createdAt", { release: { createdAt: "not-a-date" } }],
    ["ops revision", { ops: { sourceCommit: "fedcba9876543210fedcba9876543210fedcba98" } }],
    ["deployment mode", { release: { mode: "soft" } }]
  ])("rejects invalid %s", (_, change) => {
    const manifest = validManifest();
    for (const [section, values] of Object.entries(change)) {
      if (section === "schemaVersion") {
        (manifest as any).schemaVersion = values;
      } else {
        Object.assign((manifest as any)[section], values);
      }
    }
    expect(() => validateReleaseManifest(manifest)).toThrow();
  });

  it("allows an explicitly reviewed artifact source override", () => {
    const manifest = validManifest() as any;
    manifest.release.reviewedSourceOverrides = ["artifacts.factoryWasm"];
    manifest.artifacts.factoryWasm.sourceCommit = "fedcba9876543210fedcba9876543210fedcba98";
    expect(validateReleaseManifest(manifest).artifacts.factoryWasm.sourceCommit).toBe(
      manifest.artifacts.factoryWasm.sourceCommit
    );
  });
});
