import { createHash } from "node:crypto";

import type { ConstitutionVerification } from "@ic-automaton/shared";

export function verifyConstitution(
  constitution: string | null,
  expectedHash: string | null
): { constitution: string | null; verification: ConstitutionVerification } {
  if (constitution === null) {
    return {
      constitution: null,
      verification: {
        status: "unavailable",
        expectedHash,
        computedHash: null
      }
    };
  }

  const computedHash = createHash("sha256").update(constitution).digest("hex");
  if (expectedHash === null) {
    return {
      constitution,
      verification: {
        status: "legacy_unverified",
        expectedHash: null,
        computedHash
      }
    };
  }

  const matches = computedHash === expectedHash.toLowerCase();
  return {
    constitution: matches ? constitution : null,
    verification: {
      status: matches ? "verified" : "mismatch",
      expectedHash,
      computedHash
    }
  };
}
