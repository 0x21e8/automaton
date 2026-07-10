use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::types::{
    validate_version_commit, AddRepositoryStrategyRequest, FactoryError,
    RepositoryStrategyMetadata, RepositoryStrategyRecord, RepositoryStrategySource,
    RepositoryStrategyStatus, SpawnChain,
};

const STRATEGY_MANIFEST_JSON: &str = include_str!("../../../strategies/manifest.json");
const BASE_AAVE_USDC_RESERVE_RECIPE_JSON: &str =
    include_str!("../../../strategies/base-aave-usdc-reserve-01/recipe.json");
const BASE_MOONWELL_USDC_RESERVE_RECIPE_JSON: &str =
    include_str!("../../../strategies/base-moonwell-usdc-reserve-01/recipe.json");
const BASE_USDC_CARRY_CBBTC_RECIPE_JSON: &str =
    include_str!("../../../strategies/base-usdc-carry-cbbtc-01/recipe.json");

#[derive(Clone, Debug, Deserialize)]
struct StrategySeedManifest {
    strategies: Vec<StrategySeedEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct StrategySeedEntry {
    strategy_id: String,
    display_name: String,
    description: String,
    canonical_chain: String,
    canonical_chain_id: u64,
    compatible_spawn_chains: Vec<String>,
    protocol: String,
    primitive: String,
    status: String,
    recipe_file: String,
    source_path: String,
    source_commit: String,
}

#[derive(Clone, Debug, Deserialize)]
struct StrategyRecipeSummary {
    template_id: String,
    chain_id: u64,
    protocol: String,
    primitive: String,
}

fn invalid_repository_strategy(field: &str, message: impl Into<String>) -> FactoryError {
    FactoryError::InvalidRepositoryStrategy {
        field: field.to_string(),
        message: message.into(),
    }
}

fn require_trimmed(value: &str, field: &str) -> Result<String, FactoryError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(invalid_repository_strategy(field, "must not be empty"));
    }

    Ok(trimmed.to_string())
}

fn parse_spawn_chain(value: &str, field: &str) -> Result<SpawnChain, FactoryError> {
    match value.trim() {
        "base" => Ok(SpawnChain::Base),
        unsupported => Err(invalid_repository_strategy(
            field,
            format!("unsupported spawn chain: {unsupported}"),
        )),
    }
}

fn parse_strategy_status(
    value: &str,
    field: &str,
) -> Result<RepositoryStrategyStatus, FactoryError> {
    match value.trim() {
        "active" => Ok(RepositoryStrategyStatus::Active),
        "deprecated" => Ok(RepositoryStrategyStatus::Deprecated),
        "revoked" => Ok(RepositoryStrategyStatus::Revoked),
        unsupported => Err(invalid_repository_strategy(
            field,
            format!("unsupported repository strategy status: {unsupported}"),
        )),
    }
}

fn embedded_recipe_json(recipe_file: &str) -> Result<&'static str, FactoryError> {
    match recipe_file {
        "base-aave-usdc-reserve-01/recipe.json" => Ok(BASE_AAVE_USDC_RESERVE_RECIPE_JSON),
        "base-moonwell-usdc-reserve-01/recipe.json" => Ok(BASE_MOONWELL_USDC_RESERVE_RECIPE_JSON),
        "base-usdc-carry-cbbtc-01/recipe.json" => Ok(BASE_USDC_CARRY_CBBTC_RECIPE_JSON),
        _ => Err(invalid_repository_strategy(
            "strategy_seeds.recipe_file",
            format!("missing embedded recipe asset: {recipe_file}"),
        )),
    }
}

fn normalize_compatible_spawn_chains(
    chains: &[SpawnChain],
    canonical_chain: &SpawnChain,
) -> Result<Vec<SpawnChain>, FactoryError> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();

    for chain in chains {
        if seen.insert(chain.as_str()) {
            normalized.push(chain.clone());
        }
    }

    if normalized.is_empty() {
        return Err(invalid_repository_strategy(
            "metadata.compatible_spawn_chains",
            "must contain at least one chain",
        ));
    }

    if !normalized.iter().any(|chain| chain == canonical_chain) {
        return Err(invalid_repository_strategy(
            "metadata.compatible_spawn_chains",
            "must include the canonical chain",
        ));
    }

    Ok(normalized)
}

fn normalize_metadata(
    metadata: &RepositoryStrategyMetadata,
) -> Result<RepositoryStrategyMetadata, FactoryError> {
    validate_version_commit(&metadata.source.source_commit)?;

    Ok(RepositoryStrategyMetadata {
        strategy_id: require_trimmed(&metadata.strategy_id, "metadata.strategy_id")?,
        name: require_trimmed(&metadata.name, "metadata.name")?,
        description: require_trimmed(&metadata.description, "metadata.description")?,
        canonical_chain: metadata.canonical_chain.clone(),
        canonical_chain_id: metadata.canonical_chain_id,
        compatible_spawn_chains: normalize_compatible_spawn_chains(
            &metadata.compatible_spawn_chains,
            &metadata.canonical_chain,
        )?,
        protocol: require_trimmed(&metadata.protocol, "metadata.protocol")?,
        primitive: require_trimmed(&metadata.primitive, "metadata.primitive")?,
        source: RepositoryStrategySource {
            source_path: require_trimmed(
                &metadata.source.source_path,
                "metadata.source.source_path",
            )?,
            source_commit: metadata.source.source_commit.trim().to_string(),
        },
    })
}

fn parse_recipe_summary(recipe_json: &str) -> Result<StrategyRecipeSummary, FactoryError> {
    let trimmed = recipe_json.trim();
    if trimmed.is_empty() {
        return Err(invalid_repository_strategy(
            "recipe_json",
            "must not be empty",
        ));
    }

    serde_json::from_str::<StrategyRecipeSummary>(trimmed).map_err(|error| {
        invalid_repository_strategy(
            "recipe_json",
            format!("must decode as an ic-automaton strategy recipe: {error}"),
        )
    })
}

fn validate_recipe_matches_metadata(
    metadata: &RepositoryStrategyMetadata,
    recipe_json: &str,
) -> Result<(), FactoryError> {
    let recipe = parse_recipe_summary(recipe_json)?;

    if recipe.template_id != metadata.strategy_id {
        return Err(invalid_repository_strategy(
            "recipe_json.template_id",
            format!(
                "must match metadata.strategy_id: expected {}, got {}",
                metadata.strategy_id, recipe.template_id
            ),
        ));
    }
    if recipe.chain_id != metadata.canonical_chain_id {
        return Err(invalid_repository_strategy(
            "recipe_json.chain_id",
            format!(
                "must match metadata.canonical_chain_id: expected {}, got {}",
                metadata.canonical_chain_id, recipe.chain_id
            ),
        ));
    }
    if recipe.protocol != metadata.protocol {
        return Err(invalid_repository_strategy(
            "recipe_json.protocol",
            format!(
                "must match metadata.protocol: expected {}, got {}",
                metadata.protocol, recipe.protocol
            ),
        ));
    }
    if recipe.primitive != metadata.primitive {
        return Err(invalid_repository_strategy(
            "recipe_json.primitive",
            format!(
                "must match metadata.primitive: expected {}, got {}",
                metadata.primitive, recipe.primitive
            ),
        ));
    }

    Ok(())
}

pub fn build_repository_strategy_record(
    request: AddRepositoryStrategyRequest,
    now_ms: u64,
) -> Result<RepositoryStrategyRecord, FactoryError> {
    let metadata = normalize_metadata(&request.metadata)?;
    let recipe_json = request.recipe_json.trim().to_string();
    validate_recipe_matches_metadata(&metadata, &recipe_json)?;

    Ok(RepositoryStrategyRecord {
        metadata,
        recipe_json,
        status: RepositoryStrategyStatus::Active,
        created_at: now_ms,
        updated_at: now_ms,
        deprecated_at: None,
        revoked_at: None,
    })
}

pub fn repository_updated_at(
    repository_strategies: &BTreeMap<String, RepositoryStrategyRecord>,
) -> u64 {
    repository_strategies
        .values()
        .map(|record| record.updated_at)
        .max()
        .unwrap_or(0)
}

pub fn seed_repository_records(now_ms: u64) -> BTreeMap<String, RepositoryStrategyRecord> {
    let manifest: StrategySeedManifest =
        serde_json::from_str(STRATEGY_MANIFEST_JSON).expect("embedded strategy manifest should decode");
    let mut records = BTreeMap::new();

    for entry in manifest.strategies {
        let canonical_chain =
            parse_spawn_chain(&entry.canonical_chain, "strategies.canonical_chain")
                .expect("embedded strategy chain should be valid");
        let compatible_spawn_chains = entry
            .compatible_spawn_chains
            .iter()
            .map(|chain| parse_spawn_chain(chain, "strategies.compatible_spawn_chains"))
            .collect::<Result<Vec<_>, _>>()
            .expect("embedded strategy compatible chains should be valid");
        let metadata = RepositoryStrategyMetadata {
            strategy_id: entry.strategy_id,
            name: entry.display_name,
            description: entry.description,
            canonical_chain: canonical_chain.clone(),
            canonical_chain_id: entry.canonical_chain_id,
            compatible_spawn_chains,
            protocol: entry.protocol,
            primitive: entry.primitive,
            source: RepositoryStrategySource {
                source_path: entry.source_path,
                source_commit: entry.source_commit,
            },
        };
        let request = AddRepositoryStrategyRequest {
            metadata,
            recipe_json: embedded_recipe_json(&entry.recipe_file)
                .expect("embedded strategy recipe should exist")
                .to_string(),
        };
        let mut record = build_repository_strategy_record(request, now_ms)
                .expect("embedded strategy should validate");
        record.status = parse_strategy_status(&entry.status, "strategies.status")
            .expect("embedded strategy status should be valid");
        if matches!(record.status, RepositoryStrategyStatus::Deprecated) {
            record.deprecated_at = Some(now_ms);
        }
        if matches!(record.status, RepositoryStrategyStatus::Revoked) {
            record.revoked_at = Some(now_ms);
        }

        let strategy_id = record.metadata.strategy_id.clone();
        assert!(
            records.insert(strategy_id.clone(), record).is_none(),
            "duplicate embedded strategy seed: {strategy_id}"
        );
    }

    records
}

#[cfg(test)]
mod tests {
    use super::{build_repository_strategy_record, repository_updated_at, seed_repository_records};
    use crate::types::{
        AddRepositoryStrategyRequest, RepositoryStrategyMetadata, RepositoryStrategySource,
        SpawnChain,
    };

    fn sample_request(strategy_id: &str, recipe_template_id: &str) -> AddRepositoryStrategyRequest {
        AddRepositoryStrategyRequest {
            metadata: RepositoryStrategyMetadata {
                strategy_id: strategy_id.to_string(),
                name: "Sample Strategy".to_string(),
                description: "Sample description".to_string(),
                canonical_chain: SpawnChain::Base,
                canonical_chain_id: 8_453,
                compatible_spawn_chains: vec![SpawnChain::Base],
                protocol: "aave-v3".to_string(),
                primitive: "lend_supply".to_string(),
                source: RepositoryStrategySource {
                    source_path: "docs/strategies/sample/recipe.json".to_string(),
                    source_commit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa".to_string(),
                },
            },
            recipe_json: format!(
                r#"{{"template_id":"{recipe_template_id}","chain_id":8453,"protocol":"aave-v3","primitive":"lend_supply"}}"#
            ),
        }
    }

    #[test]
    fn loads_embedded_seed_records() {
        let records = seed_repository_records(0);

        assert_eq!(records.len(), 3);
        assert!(records.contains_key("base-aave-usdc-reserve-01"));
        assert!(records.contains_key("base-moonwell-usdc-reserve-01"));
        assert!(records.contains_key("base-usdc-carry-cbbtc-01"));
        assert_eq!(repository_updated_at(&records), 0);
    }

    #[test]
    fn rejects_recipe_metadata_mismatches() {
        let error = build_repository_strategy_record(
            sample_request("custom-strategy-01", "different-template-id"),
            123,
        )
        .expect_err("mismatched template id should fail");

        assert!(matches!(
            error,
            crate::types::FactoryError::InvalidRepositoryStrategy { ref field, .. }
                if field == "recipe_json.template_id"
        ));
    }
}
