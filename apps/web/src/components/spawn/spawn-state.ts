import type { RepositoryStrategyRecord } from "@ic-automaton/shared";
import {
  MINIMUM_GROSS_PAYMENT_USD,
  type SpawnAsset,
  type SpawnChain
} from "../../../../../packages/shared/src/spawn.js";

export const TOTAL_SPAWN_STEPS = 4;
export const MOCK_ETH_USD_RATE = 3200;
export const MOCK_PLATFORM_FEE_USD = 4.5;
export const MOCK_CREATION_COST_USD = 8;

export interface ChainOption {
  id: SpawnChain | "ethereum" | "arbitrum" | "optimism" | "polygon" | "hyperliquid";
  label: string;
  description: string;
  active: boolean;
}

export interface RiskProfile {
  value: 1 | 2 | 3 | 4 | 5;
  label: string;
  description: string;
}

export interface SpawnWizardState {
  chain: SpawnChain;
  risk: RiskProfile["value"];
  strategies: string[];
  skills: string[];
  openRouterApiKey: string;
  selectedModelId: string;
  braveSearchApiKey: string;
  asset: SpawnAsset;
  grossAmountInput: string;
}

export interface FundingPreview {
  grossAmount: number;
  grossUsd: number;
  minimumUsd: number;
  minimumMet: boolean;
  platformFeeUsd: number;
  creationCostUsd: number;
  netForwardUsd: number;
  grossDisplay: string;
  platformFeeDisplay: string;
  creationCostDisplay: string;
  netForwardDisplay: string;
}

export const chainOptions: ChainOption[] = [
  {
    id: "base",
    label: "Base",
    description: "Ethereum L2. Lowest-friction launch surface for the initial spawn flow.",
    active: true
  },
  {
    id: "ethereum",
    label: "Ethereum",
    description: "Mainnet L1. Highest security envelope, higher gas costs.",
    active: false
  },
  {
    id: "arbitrum",
    label: "Arbitrum",
    description: "Optimistic rollup with a broad DeFi footprint.",
    active: false
  },
  {
    id: "optimism",
    label: "Optimism",
    description: "OP Stack L2 for low-fee execution and future expansion.",
    active: false
  },
  {
    id: "polygon",
    label: "Polygon",
    description: "Low-fee sidechain kept visible for roadmap continuity.",
    active: false
  },
  {
    id: "hyperliquid",
    label: "Hyperliquid",
    description: "Future high-throughput market venue support.",
    active: false
  }
];

export const riskProfiles: RiskProfile[] = [
  {
    value: 1,
    label: "Conservative",
    description: "Preserve principal first and prefer slower, lower-volatility loops."
  },
  {
    value: 2,
    label: "Cautious",
    description: "Allow moderate repositioning while keeping liquidity buffers intact."
  },
  {
    value: 3,
    label: "Balanced",
    description: "Blend capital preservation with steady yield-seeking behavior."
  },
  {
    value: 4,
    label: "Aggressive",
    description: "Accept deeper swings to pursue faster growth and wider market coverage."
  },
  {
    value: 5,
    label: "Degen",
    description: "Maximize opportunity-seeking and tolerate the highest execution volatility."
  }
];

export function createInitialSpawnWizardState(): SpawnWizardState {
  return {
    chain: "base",
    risk: 3,
    strategies: [],
    skills: [],
    openRouterApiKey: "",
    selectedModelId: "",
    braveSearchApiKey: "",
    asset: "usdc",
    grossAmountInput: "100"
  };
}

export function getRiskProfile(value: RiskProfile["value"]): RiskProfile {
  return riskProfiles.find((profile) => profile.value === value) ?? riskProfiles[2];
}

export function getActiveChainLabel(chain: SpawnChain): string {
  return chainOptions.find((entry) => entry.id === chain)?.label ?? chain;
}

export function toggleSelection(
  values: string[],
  candidate: string
): string[] {
  return values.includes(candidate)
    ? values.filter((value) => value !== candidate)
    : [...values, candidate];
}

export function listSelectableRepositoryStrategies(
  strategies: RepositoryStrategyRecord[],
  chain: SpawnChain
): RepositoryStrategyRecord[] {
  return strategies
    .filter(
      (strategy) =>
        strategy.status === "active" && strategy.compatibleSpawnChains.includes(chain)
    )
    .sort((left, right) => left.name.localeCompare(right.name));
}

export function normalizeSelectedRepositoryStrategyIds(
  selectedIds: string[],
  strategies: RepositoryStrategyRecord[]
): string[] {
  const availableIds = new Set(strategies.map((strategy) => strategy.strategyId));
  return selectedIds.filter((strategyId) => availableIds.has(strategyId));
}

function parseGrossAmount(rawValue: string): number {
  const parsed = Number(rawValue);

  return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
}

function assetToUsd(asset: SpawnAsset, amount: number): number {
  return asset === "usdc" ? amount : amount * MOCK_ETH_USD_RATE;
}

function usdToAsset(asset: SpawnAsset, amountUsd: number): number {
  return asset === "usdc" ? amountUsd : amountUsd / MOCK_ETH_USD_RATE;
}

function formatUsd(value: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2
  }).format(value);
}

function formatAsset(asset: SpawnAsset, amount: number): string {
  return `${new Intl.NumberFormat("en-US", {
    minimumFractionDigits: asset === "usdc" ? 2 : 4,
    maximumFractionDigits: asset === "usdc" ? 2 : 4
  }).format(amount)} ${asset.toUpperCase()}`;
}

export function getFundingPreview(state: SpawnWizardState): FundingPreview {
  const grossAmount = parseGrossAmount(state.grossAmountInput);
  const grossUsd = assetToUsd(state.asset, grossAmount);
  const platformFeeAmount = usdToAsset(state.asset, MOCK_PLATFORM_FEE_USD);
  const creationCostAmount = usdToAsset(state.asset, MOCK_CREATION_COST_USD);
  const netForwardAmount = Math.max(
    0,
    grossAmount - platformFeeAmount - creationCostAmount
  );
  const netForwardUsd = Math.max(
    0,
    grossUsd - MOCK_PLATFORM_FEE_USD - MOCK_CREATION_COST_USD
  );

  return {
    grossAmount,
    grossUsd,
    minimumUsd: MINIMUM_GROSS_PAYMENT_USD,
    minimumMet: grossUsd >= MINIMUM_GROSS_PAYMENT_USD,
    platformFeeUsd: MOCK_PLATFORM_FEE_USD,
    creationCostUsd: MOCK_CREATION_COST_USD,
    netForwardUsd,
    grossDisplay: formatAsset(state.asset, grossAmount),
    platformFeeDisplay: formatAsset(state.asset, platformFeeAmount),
    creationCostDisplay: formatAsset(state.asset, creationCostAmount),
    netForwardDisplay: formatAsset(state.asset, netForwardAmount)
  };
}

export function getSelectedModel(state: SpawnWizardState): string | null {
  return state.selectedModelId.trim() === "" ? null : state.selectedModelId.trim();
}

export function buildProviderSummary(state: SpawnWizardState): string {
  const model = getSelectedModel(state);

  if (model === null) {
    return "Inference disabled until steward config";
  }

  return model;
}

export function describeFundingValidation(preview: FundingPreview): string {
  if (preview.minimumMet) {
    return `Gross payment clears the ${formatUsd(preview.minimumUsd)} minimum.`;
  }

  return `Gross payment must be at least ${formatUsd(preview.minimumUsd)} before fees.`;
}

export function formatUsdValue(value: number): string {
  return formatUsd(value);
}
