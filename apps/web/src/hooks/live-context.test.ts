import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AutomatonContext } from "../api/automaton";
import { fetchAutomatonContext } from "../api/automaton";
import {
  clearLiveAutomatonContextCache,
  loadLiveAutomatonContext
} from "./live-context";

vi.mock("../api/automaton", async () => {
  const actual = await vi.importActual<typeof import("../api/automaton")>("../api/automaton");
  return { ...actual, fetchAutomatonContext: vi.fn() };
});

const mockedFetch = vi.mocked(fetchAutomatonContext);

function context(fetchedAt: number): AutomatonContext {
  return {
    buildInfo: {} as AutomatonContext["buildInfo"],
    evmConfig: {} as AutomatonContext["evmConfig"],
    schedulerConfig: {} as AutomatonContext["schedulerConfig"],
    stewardStatus: {} as AutomatonContext["stewardStatus"],
    snapshot: {} as AutomatonContext["snapshot"],
    walletBalance: {} as AutomatonContext["walletBalance"],
    fetchedAt
  };
}

describe("live automaton context", () => {
  beforeEach(() => {
    clearLiveAutomatonContextCache();
    mockedFetch.mockReset();
  });

  it("loads one aggregate context and reuses it inside the freshness window", async () => {
    const first = context(1);
    mockedFetch.mockResolvedValue(first);

    await expect(loadLiveAutomatonContext("https://automaton.test")).resolves.toBe(first);
    await expect(loadLiveAutomatonContext("https://automaton.test")).resolves.toBe(first);

    expect(mockedFetch).toHaveBeenCalledTimes(1);
  });

  it("does not cache an aborted load", async () => {
    const controller = new AbortController();
    const first = context(1);
    mockedFetch.mockResolvedValue(first);
    controller.abort();

    await loadLiveAutomatonContext("https://automaton.test", controller.signal);
    mockedFetch.mockResolvedValue(context(2));
    await loadLiveAutomatonContext("https://automaton.test");

    expect(mockedFetch).toHaveBeenCalledTimes(2);
  });
});
