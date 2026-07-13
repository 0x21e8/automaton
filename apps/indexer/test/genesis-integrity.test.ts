import { createHash } from "node:crypto";

import { describe, expect, it } from "vitest";

import { verifyConstitution } from "../src/lib/genesis-integrity.js";
import { normalizeAutomatonDetail } from "../src/normalize/automaton.js";
import { createAutomatonDetailFixture } from "./fixtures.js";

describe("Genesis constitution integrity", () => {
  it("retains content only when it matches the registry SHA-256", () => {
    const constitution = "I keep evidence and identity aligned.";
    const hash = createHash("sha256").update(constitution).digest("hex");

    expect(verifyConstitution(constitution, hash)).toEqual({
      constitution,
      verification: {
        status: "verified",
        expectedHash: hash,
        computedHash: hash
      }
    });
  });

  it("hides mismatched child content and records both hashes", () => {
    const result = verifyConstitution("tampered constitution", "0".repeat(64));

    expect(result.constitution).toBeNull();
    expect(result.verification).toMatchObject({
      status: "mismatch",
      expectedHash: "0".repeat(64)
    });
    expect(result.verification.computedHash).toMatch(/^[0-9a-f]{64}$/);
  });

  it("drops mismatched content in the normalized indexer detail", () => {
    const existingDetail = createAutomatonDetailFixture();
    const detail = normalizeAutomatonDetail({
      canisterId: existingDetail.canisterId,
      config: {
        canisterIds: [existingDetail.canisterId],
        network: {
          target: "local",
          local: { host: "localhost", port: 8000 }
        }
      },
      existingDetail,
      identity: {
        canisterId: existingDetail.canisterId,
        genesis: {
          name: "Meridian",
          constitution: "tampered constitution",
          contract_version: 2
        },
        buildInfo: {},
        evmConfig: {},
        promptLayers: [],
        schedulerConfig: {},
        skills: [],
        stewardStatus: {},
        strategies: []
      },
      now: 10,
      registryRecord: {
        canisterId: existingDetail.canisterId,
        stewardAddress: existingDetail.steward.address,
        evmAddress: existingDetail.ethAddress ?? "0x0",
        chain: "base",
        sessionId: "session-1",
        parentId: null,
        childIds: [],
        createdAt: 1,
        versionCommit: "0".repeat(40),
        name: "Meridian",
        constitutionHash: "0".repeat(64)
      },
      ethUsd: null
    });

    expect(detail.constitution).toBeNull();
    expect(detail.constitutionVerification.status).toBe("mismatch");
  });

  it("marks content without a registry hash as legacy and unverified", () => {
    expect(verifyConstitution("legacy constitution", null)).toMatchObject({
      constitution: "legacy constitution",
      verification: {
        status: "legacy_unverified",
        expectedHash: null
      }
    });
  });
});
