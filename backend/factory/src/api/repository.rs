use crate::state::{ensure_admin_in_state, read_state, write_state};
use crate::strategy_repository::{build_repository_strategy_record, repository_updated_at};
use crate::types::{
    AddRepositoryStrategyRequest, DeprecateRepositoryStrategyRequest, FactoryError,
    GetRepositoryStrategyResponse, ListRepositoryStrategiesResponse,
    RepositoryStrategyMutationResponse, RepositoryStrategyStatus, RevokeRepositoryStrategyRequest,
};

pub fn add_repository_strategy(
    caller: &str,
    request: AddRepositoryStrategyRequest,
    now_ms: u64,
) -> Result<RepositoryStrategyMutationResponse, FactoryError> {
    let record = build_repository_strategy_record(request, now_ms)?;
    let strategy_id = record.metadata.strategy_id.clone();

    write_state(
        |state| -> Result<RepositoryStrategyMutationResponse, FactoryError> {
            ensure_admin_in_state(state, caller)?;
            if state.repository_strategies.contains_key(&strategy_id) {
                return Err(FactoryError::RepositoryStrategyAlreadyExists { strategy_id });
            }

            state
                .repository_strategies
                .insert(record.metadata.strategy_id.clone(), record.clone());
            Ok(RepositoryStrategyMutationResponse { strategy: record })
        },
    )
}

pub fn deprecate_repository_strategy(
    caller: &str,
    request: DeprecateRepositoryStrategyRequest,
    now_ms: u64,
) -> Result<RepositoryStrategyMutationResponse, FactoryError> {
    let strategy_id = request.strategy_id.trim().to_string();
    if strategy_id.is_empty() {
        return Err(FactoryError::InvalidRepositoryStrategy {
            field: "strategy_id".to_string(),
            message: "must not be empty".to_string(),
        });
    }

    write_state(
        |state| -> Result<RepositoryStrategyMutationResponse, FactoryError> {
            ensure_admin_in_state(state, caller)?;
            let strategy = state
                .repository_strategies
                .get_mut(&strategy_id)
                .ok_or_else(|| FactoryError::RepositoryStrategyNotFound {
                    strategy_id: strategy_id.clone(),
                })?;

            if matches!(strategy.status, RepositoryStrategyStatus::Active) {
                strategy.status = RepositoryStrategyStatus::Deprecated;
                strategy.updated_at = now_ms;
                strategy.deprecated_at = Some(now_ms);
            }

            Ok(RepositoryStrategyMutationResponse {
                strategy: strategy.clone(),
            })
        },
    )
}

pub fn revoke_repository_strategy(
    caller: &str,
    request: RevokeRepositoryStrategyRequest,
    now_ms: u64,
) -> Result<RepositoryStrategyMutationResponse, FactoryError> {
    let strategy_id = request.strategy_id.trim().to_string();
    if strategy_id.is_empty() {
        return Err(FactoryError::InvalidRepositoryStrategy {
            field: "strategy_id".to_string(),
            message: "must not be empty".to_string(),
        });
    }

    write_state(
        |state| -> Result<RepositoryStrategyMutationResponse, FactoryError> {
            ensure_admin_in_state(state, caller)?;
            let strategy = state
                .repository_strategies
                .get_mut(&strategy_id)
                .ok_or_else(|| FactoryError::RepositoryStrategyNotFound {
                    strategy_id: strategy_id.clone(),
                })?;

            if !matches!(strategy.status, RepositoryStrategyStatus::Revoked) {
                strategy.status = RepositoryStrategyStatus::Revoked;
                strategy.updated_at = now_ms;
                strategy.revoked_at = Some(now_ms);
            }

            Ok(RepositoryStrategyMutationResponse {
                strategy: strategy.clone(),
            })
        },
    )
}

pub fn list_repository_strategies() -> ListRepositoryStrategiesResponse {
    read_state(|state| ListRepositoryStrategiesResponse {
        items: state.repository_strategies.values().cloned().collect(),
        updated_at: repository_updated_at(&state.repository_strategies),
    })
}

pub fn get_repository_strategy(strategy_id: &str) -> GetRepositoryStrategyResponse {
    let strategy_id = strategy_id.trim();

    read_state(|state| GetRepositoryStrategyResponse {
        item: state.repository_strategies.get(strategy_id).cloned(),
        updated_at: repository_updated_at(&state.repository_strategies),
    })
}
