import type { ChainSlug } from "./automaton.js";
import type { SpawnChain } from "./spawn.js";

export const CATALOG_ENTRY_STATUSES = [
  "available",
  "coming_soon"
] as const;

export const REPOSITORY_STRATEGY_STATUSES = [
  "active",
  "deprecated",
  "revoked"
] as const;

export type CatalogEntryStatus = (typeof CATALOG_ENTRY_STATUSES)[number];
export type RepositoryStrategyStatus = (typeof REPOSITORY_STRATEGY_STATUSES)[number];
export type StrategyRiskLevel = 1 | 2 | 3 | 4 | 5;

export interface RepositoryStrategySourceProvenance {
  sourcePath: string;
  sourceCommit: string;
}

export interface RepositoryStrategyRecord {
  strategyId: string;
  name: string;
  description: string;
  canonicalChain: ChainSlug;
  canonicalChainId: number;
  compatibleSpawnChains: SpawnChain[];
  protocol: string;
  primitive: string;
  recipeJson: string;
  status: RepositoryStrategyStatus;
  source: RepositoryStrategySourceProvenance;
  createdAt: number;
  updatedAt: number;
  deprecatedAt: number | null;
  revokedAt: number | null;
}

export interface StrategyCatalogStats {
  apy: number | null;
  tvl: number | null;
}

export interface StrategyCatalogEntry {
  id: string;
  name: string;
  description: string;
  category: string;
  chains: ChainSlug[];
  riskLevel: StrategyRiskLevel;
  stats: StrategyCatalogStats;
  status: CatalogEntryStatus;
}

export interface SkillCatalogEntry {
  id: string;
  name: string;
  description: string;
  dependencies: string[];
  category: string;
  status: CatalogEntryStatus;
}

export interface CatalogResponse<TEntry> {
  items: TEntry[];
  updatedAt: number;
}

export interface RepositoryStrategyGetResponse {
  item: RepositoryStrategyRecord | null;
  updatedAt: number;
}

export type StrategyCatalogResponse = CatalogResponse<StrategyCatalogEntry>;
export type SkillCatalogResponse = CatalogResponse<SkillCatalogEntry>;
export type RepositoryStrategyListResponse = CatalogResponse<RepositoryStrategyRecord>;
