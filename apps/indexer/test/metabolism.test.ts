import { describe, expect, it } from "vitest";

import {
  computeCanonicalRunwaySeconds,
  computeWindowedBurnRate,
  deriveMetabolicState,
  positiveUsdcInflowRaw
} from "../src/lib/metabolism.js";
import { normalizeAutomatonDetail } from "../src/normalize/automaton.js";
import type { IndexerTargetConfig } from "../src/indexer.config.js";
import { createSpawnedAutomatonRecordFixture } from "./fixtures.js";

const localConfig: IndexerTargetConfig = {
  canisterIds: [],
  network: { target: "local", local: { host: "localhost", port: 8000 } }
};

describe("metabolism math", () => {
  it("computes windowed burn while ignoring top-up intervals", () => {
    expect(computeWindowedBurnRate([
      { capturedAt: 0, liquidCycles: 2_000, usdcBalanceRaw: "0" },
      { capturedAt: 86_400_000, liquidCycles: 1_000, usdcBalanceRaw: "0" }
    ])).toBe(1_000);
    expect(computeWindowedBurnRate([
      { capturedAt: 0, liquidCycles: 1_000, usdcBalanceRaw: "0" },
      { capturedAt: 86_400_000, liquidCycles: 2_000, usdcBalanceRaw: "0" }
    ])).toBeNull();
    expect(computeWindowedBurnRate([
      { capturedAt: 0, liquidCycles: 1_000, usdcBalanceRaw: "0" },
      { capturedAt: 86_400_000, liquidCycles: 1_000, usdcBalanceRaw: "0" }
    ])).toBe(0);
  });

  it("does not let a top-up erase burn observed before and after it", () => {
    expect(computeWindowedBurnRate([
      { capturedAt: 0, liquidCycles: 1_000, usdcBalanceRaw: "0" },
      { capturedAt: 86_400_000, liquidCycles: 900, usdcBalanceRaw: "0" },
      { capturedAt: 2 * 86_400_000, liquidCycles: 1_900, usdcBalanceRaw: "0" },
      { capturedAt: 3 * 86_400_000, liquidCycles: 1_700, usdcBalanceRaw: "0" }
    ])).toBe(150);
  });

  it("returns null without two distinct valid observations", () => {
    expect(computeWindowedBurnRate([])).toBeNull();
    expect(computeWindowedBurnRate([
      { capturedAt: 0, liquidCycles: 1_000, usdcBalanceRaw: "0" }
    ])).toBeNull();
    expect(computeWindowedBurnRate([
      { capturedAt: 1, liquidCycles: 1_000, usdcBalanceRaw: "0" },
      { capturedAt: 1, liquidCycles: 900, usdcBalanceRaw: "0" }
    ])).toBeNull();
  });

  it("uses one documented runway formula for liquid cycles and USDC reserves", () => {
    expect(computeCanonicalRunwaySeconds({
      burnRateCyclesPerDay: 1_000_000_000_000,
      liquidCycles: 1_000_000_000_000,
      usdcBalanceRaw: "1350000"
    })).toBe(172_800);
    expect(computeCanonicalRunwaySeconds({
      burnRateCyclesPerDay: 0,
      liquidCycles: 1_000,
      usdcBalanceRaw: "0"
    })).toBeNull();
    expect(computeCanonicalRunwaySeconds({
      burnRateCyclesPerDay: 1_000_000_000_000,
      liquidCycles: 1_000_000_000_000,
      usdcBalanceRaw: "1350000",
      usdPerTrillionCycles: Number.NaN
    })).toBe(172_800);
    expect(computeCanonicalRunwaySeconds({
      burnRateCyclesPerDay: 1_000_000_000_000,
      liquidCycles: 1_000_000_000_000,
      usdcBalanceRaw: "2700000",
      usdPerTrillionCycles: 2.7
    })).toBe(172_800);
  });

  it("counts only positive observed USDC inflow and derives terminal state", () => {
    expect(positiveUsdcInflowRaw("100", "250")).toBe("150");
    expect(positiveUsdcInflowRaw("250", "100")).toBe("0");
    expect(deriveMetabolicState({ runwaySeconds: 0, tier: "normal", cyclesBalance: 0 })).toBe("dead");
    expect(deriveMetabolicState({ runwaySeconds: 3_600, tier: "normal", cyclesBalance: 1 })).toBe("dying");
  });

  it("keeps legacy and unknown controller records unverified", () => {
    for (const record of [
      createSpawnedAutomatonRecordFixture({
        controllers: undefined,
        controlStatus: undefined,
        controlVerifiedAt: undefined
      }),
      {
        ...createSpawnedAutomatonRecordFixture(),
        controlStatus: "unknown_status"
      },
      createSpawnedAutomatonRecordFixture({
        controllers: ["bbbbb-bb"],
        controlStatus: "self_controlled"
      })
    ]) {
      const detail = normalizeAutomatonDetail({
        canisterId: record.canisterId,
        config: localConfig,
        now: 2_000_000_000_000,
        registryRecord: record as never,
        ethUsd: null
      });
      expect(detail.controlStatus).toEqual({
        label: "unverified",
        controllers: [],
        spawnerPresent: false,
        verifiedAt: null
      });
    }
  });

  it("preserves the factory controller attestation timestamp across ordinary polls", () => {
    const record = createSpawnedAutomatonRecordFixture({
      controlVerifiedAt: 1_709_912_360_000
    });
    const first = normalizeAutomatonDetail({
      canisterId: record.canisterId,
      config: localConfig,
      now: 2_000_000_000_000,
      registryRecord: record,
      ethUsd: null
    });
    const second = normalizeAutomatonDetail({
      canisterId: record.canisterId,
      config: localConfig,
      existingDetail: first,
      now: 2_000_000_100_000,
      registryRecord: record,
      ethUsd: null
    });
    expect(second.controlStatus).toEqual({
      label: "self_controlled",
      controllers: [record.canisterId],
      spawnerPresent: false,
      verifiedAt: 1_709_912_360_000
    });
  });

  it("indexes a coherent factory-only controller attestation as upgradeable", () => {
    const record = createSpawnedAutomatonRecordFixture({
      controllers: ["rrkah-fqaaa-aaaaa-aaaaq-cai"],
      controlStatus: "upgradeable_by_factory",
      controlVerifiedAt: 1_709_912_360_000
    });
    const detail = normalizeAutomatonDetail({
      canisterId: record.canisterId,
      config: localConfig,
      now: 2_000_000_000_000,
      registryRecord: record,
      ethUsd: null
    });
    expect(detail.controlStatus).toEqual({
      label: "upgradeable_by_factory",
      controllers: ["rrkah-fqaaa-aaaaa-aaaaq-cai"],
      spawnerPresent: false,
      verifiedAt: 1_709_912_360_000
    });
  });

  it("uses the child-reported cycles conversion rate during normalization", () => {
    const detail = normalizeAutomatonDetail({
      canisterId: "aaaaa-aa",
      config: localConfig,
      now: 2_000_000_000_000,
      runtime: {
        canisterId: "aaaaa-aa",
        snapshot: {
          cycles: {
            total_cycles: 1_000_000_000_000,
            liquid_cycles: 1_000_000_000_000,
            burn_rate_cycles_per_day: 1_000_000_000_000,
            usd_per_trillion_cycles: 2.7
          }
        },
        walletBalance: {
          eth_balance_wei_hex: "0x0",
          usdc_balance_raw_hex: "2700000",
          usdc_decimals: 6,
          last_error: null,
          last_synced_at_ns: 1,
          status: "ok",
          is_stale: false
        }
      },
      ethUsd: null
    });
    expect(detail.metabolism?.runwaySeconds).toBe(172_800);
  });

  it("indexes terminal completion as a permanent death with estate facts", () => {
    const record = createSpawnedAutomatonRecordFixture({
      deathCause: "starved",
      diedAt: 1_800_000_000_000,
      estateDisposition: "monument"
    });
    const detail = normalizeAutomatonDetail({
      canisterId: record.canisterId,
      config: localConfig,
      now: 2_000_000_000_000,
      registryRecord: record,
      ethUsd: null
    });
    expect(detail.tier).toBe("out_of_cycles");
    expect(detail.metabolism).toMatchObject({
      state: "dead",
      mortalityTier: "dead",
      deathCause: "starved",
      diedAt: 1_800_000_000_000,
      estateDisposition: "monument"
    });
  });
});
