// @vitest-environment happy-dom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import type { AutomatonDetail } from "@ic-automaton/shared";
import { AncestryPanel } from "./AncestryPanel";

const child = {
  canisterId: "child-cai",
  parentId: "parent-cai",
  parentConstitutionHash: "verified-parent-hash",
  generation: 2,
  childIds: ["grandchild-cai"],
  constitution: "I observe patient markets.",
  constitutionVerification: { status: "verified" },
  constitutionHash: "child-hash"
} as AutomatonDetail;

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("shows a bounded diff only when the parent public constitution matches the recorded hash", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({
    ...child,
    canisterId: "parent-cai",
    parentId: null,
    constitution: "I observe durable markets.",
    constitutionHash: "verified-parent-hash",
    constitutionVerification: { status: "verified" }
  }), { status: 200 }));

  render(<AncestryPanel automaton={child} />);

  expect(screen.getByText("Generation 2")).toBeTruthy();
  expect(await screen.findByText("Verified constitutional drift")).toBeTruthy();
  expect(screen.queryByText(/diff withheld/)).toBeNull();
});

it("withholds the diff when the fetched parent hash is not the factory-recorded hash", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({
    ...child,
    canisterId: "parent-cai",
    parentId: null,
    constitution: "Untrusted parent text.",
    constitutionHash: "different-hash",
    constitutionVerification: { status: "verified" }
  }), { status: 200 }));

  render(<AncestryPanel automaton={child} />);

  expect(await screen.findByText(/Parent diff withheld/)).toBeTruthy();
  expect(screen.queryByText("Verified constitutional drift")).toBeNull();
});

it("withholds the diff when the child public constitution is not verified", async () => {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({
    ...child,
    canisterId: "parent-cai",
    parentId: null,
    constitution: "I observe durable markets.",
    constitutionHash: "verified-parent-hash",
    constitutionVerification: { status: "verified" }
  }), { status: 200 }));

  render(<AncestryPanel automaton={{
    ...child,
    constitutionVerification: { status: "mismatch", expectedHash: "expected", computedHash: "actual" }
  } as AutomatonDetail} />);

  expect(await screen.findByText(/Parent diff withheld/)).toBeTruthy();
  expect(screen.queryByText("Verified constitutional drift")).toBeNull();
});
