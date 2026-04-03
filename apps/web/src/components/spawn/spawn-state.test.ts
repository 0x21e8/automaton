import { describe, expect, it } from "vitest";

import {
  createInitialSpawnWizardState,
  describeFundingValidation,
  getFundingPreview,
  listSelectableRepositoryStrategies,
  normalizeSelectedRepositoryStrategyIds
} from "./spawn-state";

describe("spawn-state", () => {
  it("computes the locked fee disclosure from gross USDC funding", () => {
    const preview = getFundingPreview(createInitialSpawnWizardState());

    expect(preview.grossDisplay).toBe("100.00 USDC");
    expect(preview.platformFeeDisplay).toBe("4.50 USDC");
    expect(preview.creationCostDisplay).toBe("8.00 USDC");
    expect(preview.netForwardDisplay).toBe("87.50 USDC");
    expect(preview.minimumMet).toBe(true);
  });

  it("rejects gross USDC funding below the $50 minimum", () => {
    const preview = getFundingPreview({
      ...createInitialSpawnWizardState(),
      grossAmountInput: "32"
    });

    expect(preview.grossUsd).toBe(32);
    expect(preview.minimumMet).toBe(false);
    expect(describeFundingValidation(preview)).toContain(
      "Gross payment must be at least"
    );
  });

  it("starts with no mock strategies or skills selected", () => {
    expect(createInitialSpawnWizardState()).toMatchObject({
      strategies: [],
      skills: []
    });
  });

  it("filters repository strategies to active entries compatible with the selected chain", () => {
    expect(
      listSelectableRepositoryStrategies(
        [
          {
            strategyId: "base-aave-usdc-reserve-01",
            name: "Base Aave USDC Reserve",
            description: "Park surplus Base USDC on Aave V3.",
            canonicalChain: "base",
            canonicalChainId: 8453,
            compatibleSpawnChains: ["base"],
            protocol: "aave-v3",
            primitive: "lend_supply",
            recipeJson: "{}",
            status: "active",
            source: {
              sourcePath: "docs/strategies/base-aave-usdc-reserve-01/recipe.json",
              sourceCommit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa"
            },
            createdAt: 1,
            updatedAt: 1,
            deprecatedAt: null,
            revokedAt: null
          },
          {
            strategyId: "base-moonwell-usdc-reserve-01",
            name: "Base Moonwell USDC Reserve",
            description: "Park surplus Base USDC on Moonwell.",
            canonicalChain: "base",
            canonicalChainId: 8453,
            compatibleSpawnChains: ["base"],
            protocol: "moonwell",
            primitive: "lend_supply",
            recipeJson: "{}",
            status: "deprecated",
            source: {
              sourcePath: "docs/strategies/base-moonwell-usdc-reserve-01/recipe.json",
              sourceCommit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa"
            },
            createdAt: 1,
            updatedAt: 1,
            deprecatedAt: 2,
            revokedAt: null
          }
        ],
        "base"
      ).map((strategy) => strategy.strategyId)
    ).toEqual(["base-aave-usdc-reserve-01"]);
  });

  it("drops selected strategy ids that are no longer present in the compatible repository view", () => {
    expect(
      normalizeSelectedRepositoryStrategyIds(
        ["base-aave-usdc-reserve-01", "missing"],
        [
          {
            strategyId: "base-aave-usdc-reserve-01",
            name: "Base Aave USDC Reserve",
            description: "Park surplus Base USDC on Aave V3.",
            canonicalChain: "base",
            canonicalChainId: 8453,
            compatibleSpawnChains: ["base"],
            protocol: "aave-v3",
            primitive: "lend_supply",
            recipeJson: "{}",
            status: "active",
            source: {
              sourcePath: "docs/strategies/base-aave-usdc-reserve-01/recipe.json",
              sourceCommit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa"
            },
            createdAt: 1,
            updatedAt: 1,
            deprecatedAt: null,
            revokedAt: null
          }
        ]
      )
    ).toEqual(["base-aave-usdc-reserve-01"]);
  });
});
