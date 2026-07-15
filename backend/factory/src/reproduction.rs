use sha2::{Digest, Sha256};

use crate::api::public::create_spawn_session_with_session_id;
#[cfg(target_arch = "wasm32")]
use crate::base_rpc::{configured_rpc_endpoints, eth_call};
use crate::state::{read_state, write_state};
use crate::types::{
    amount_to_string, parse_amount, CreateReproductionSessionRequest, CreateSpawnSessionRequest,
    CreateSpawnSessionResponse, FactoryError, ReproductionEligibility, ReproductionPolicy,
    RoyaltyAllocation, SpawnAsset, SpawnProviderSecrets, SpawnSessionOrigin, SpawnSessionState,
    REPRODUCTION_COOLDOWN_MS, REPRODUCTION_INFERENCE_RESERVE_USDC_RAW, REPRODUCTION_MIN_AGE_MS,
    REPRODUCTION_MIN_ENDOWMENT_USDC_RAW, REPRODUCTION_PARENT_ROYALTY_BPS,
    REPRODUCTION_PROGENITOR_ROYALTY_BPS, REPRODUCTION_TERMINAL_RESERVE_USDC_RAW,
    REPRODUCTION_TOPUP_RESERVE_USDC_RAW,
};

fn sha256(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn royalty_amount(fee: u128, bps: u16) -> String {
    amount_to_string(fee.saturating_mul(u128::from(bps)) / 10_000)
}

pub fn reproduction_policy() -> ReproductionPolicy {
    ReproductionPolicy::default()
}

pub fn reproduction_eligibility(
    caller: &str,
    now_ms: u64,
) -> Result<ReproductionEligibility, FactoryError> {
    let (parent, last_reproduction_at) = read_state(|state| {
        let parent = state.registry.get(caller).cloned().ok_or_else(|| {
            FactoryError::UnauthorizedReproduction {
                caller: caller.to_string(),
            }
        })?;
        if parent.death_cause.is_some() {
            return Err(FactoryError::UnauthorizedReproduction {
                caller: caller.to_string(),
            });
        }
        let last = state
            .sessions
            .values()
            .filter(|session| {
                matches!(session.origin.as_ref(), Some(SpawnSessionOrigin::ReproductionOf(id)) if id == caller)
                    && session.state != SpawnSessionState::Expired
            })
            .map(|session| session.created_at)
            .max();
        Ok((parent, last))
    })?;
    let minimum_age_at_ms = parent.created_at.saturating_add(REPRODUCTION_MIN_AGE_MS);
    let cooldown_ends_at_ms =
        last_reproduction_at.map(|created_at| created_at.saturating_add(REPRODUCTION_COOLDOWN_MS));
    let reason = if now_ms < minimum_age_at_ms {
        Some("minimum_age".to_string())
    } else if cooldown_ends_at_ms.is_some_and(|deadline| now_ms < deadline) {
        Some("cooldown".to_string())
    } else {
        None
    };
    Ok(ReproductionEligibility {
        eligible: reason.is_none(),
        observed_at_ms: now_ms,
        parent_created_at_ms: parent.created_at,
        minimum_age_at_ms,
        cooldown_ends_at_ms,
        reason,
    })
}

pub fn create_reproduction_session_with_verified_balance(
    caller: &str,
    request: CreateReproductionSessionRequest,
    now_ms: u64,
    session_id: String,
    verified_parent_usdc_balance: u128,
) -> Result<CreateSpawnSessionResponse, FactoryError> {
    spawn_protocol::validate_inheritance(&request.memory_dowry, &request.inherited_strategy_stats)
        .map_err(|reason| FactoryError::InvalidReproduction { reason })?;
    let (_, child_constitution) =
        crate::types::validate_genesis(request.name.as_str(), request.child_constitution.as_str())
            .map_err(|error| FactoryError::InvalidReproduction {
                reason: format!("invalid child genesis: {error:?}"),
            })?;

    let (parent, parent_session, platform_fee, creation_cost, last_reproduction_at) = read_state(
        |state| {
            let parent = state.registry.get(caller).cloned().ok_or_else(|| {
                FactoryError::UnauthorizedReproduction {
                    caller: caller.to_string(),
                }
            })?;
            if parent.death_cause.is_some() {
                return Err(FactoryError::UnauthorizedReproduction {
                    caller: caller.to_string(),
                });
            }
            let parent_session =
                state
                    .sessions
                    .get(&parent.session_id)
                    .cloned()
                    .ok_or_else(|| FactoryError::SessionNotFound {
                        session_id: parent.session_id.clone(),
                    })?;
            let last = state
                .sessions
                .values()
                .filter(|session| {
                    matches!(session.origin.as_ref(), Some(SpawnSessionOrigin::ReproductionOf(id)) if id == caller)
                        && session.state != SpawnSessionState::Expired
                })
                .map(|session| session.created_at)
                .max();
            Ok((
                parent,
                parent_session,
                parse_amount(&state.fee_config.usdc_fee)?,
                parse_amount(&state.creation_cost_quote.usdc_cost)?,
                last,
            ))
        },
    )?;

    let age = now_ms.saturating_sub(parent.created_at);
    if age < REPRODUCTION_MIN_AGE_MS {
        return Err(FactoryError::ReproductionIneligible {
            reason: format!("minimum age is {REPRODUCTION_MIN_AGE_MS}ms; observed {age}ms"),
        });
    }
    if let Some(last) = last_reproduction_at {
        if now_ms.saturating_sub(last) < REPRODUCTION_COOLDOWN_MS {
            return Err(FactoryError::ReproductionIneligible {
                reason: "reproduction cooldown has not elapsed".to_string(),
            });
        }
    }

    let expected_parent_hash =
        parent
            .constitution_hash
            .as_deref()
            .ok_or_else(|| FactoryError::InvalidReproduction {
                reason: "parent has no verified constitution hash".to_string(),
            })?;
    let supplied_parent_hash = sha256(request.parent_constitution.trim());
    if supplied_parent_hash != expected_parent_hash {
        return Err(FactoryError::InvalidReproduction {
            reason: "parent constitution does not match the factory registry hash".to_string(),
        });
    }
    spawn_protocol::validate_constitution_mutation(
        request.parent_constitution.trim(),
        child_constitution.as_str(),
    )
    .map_err(|reason| FactoryError::InvalidReproduction { reason })?;

    let gross = parse_amount(&request.gross_amount)?;
    let minimum_gross = platform_fee
        .saturating_add(creation_cost)
        .saturating_add(REPRODUCTION_MIN_ENDOWMENT_USDC_RAW);
    if gross < minimum_gross {
        return Err(FactoryError::ReproductionIneligible {
            reason: format!(
                "gross payment must cover fee, creation, and child endowment: {}",
                amount_to_string(minimum_gross)
            ),
        });
    }
    let required_surplus = gross
        .saturating_add(REPRODUCTION_TERMINAL_RESERVE_USDC_RAW)
        .saturating_add(REPRODUCTION_INFERENCE_RESERVE_USDC_RAW)
        .saturating_add(REPRODUCTION_TOPUP_RESERVE_USDC_RAW);
    if verified_parent_usdc_balance < required_surplus {
        return Err(FactoryError::ReproductionIneligible {
            reason: format!(
                "factory-verified parent balance must retain terminal, inference, and top-up reserves after payment: required {}, observed {}",
                amount_to_string(required_surplus),
                amount_to_string(verified_parent_usdc_balance)
            ),
        });
    }

    let mut royalties = vec![RoyaltyAllocation {
        recipient: parent.evm_address.clone(),
        amount: royalty_amount(platform_fee, REPRODUCTION_PARENT_ROYALTY_BPS),
        depth: 1,
        source: "reproduction_fee".to_string(),
    }];
    if let Some(progenitor) = parent.parent_id.as_ref().and_then(|id| {
        read_state(|state| {
            state
                .registry
                .get(id)
                .map(|record| record.evm_address.clone())
        })
    }) {
        royalties.push(RoyaltyAllocation {
            recipient: progenitor,
            amount: royalty_amount(platform_fee, REPRODUCTION_PROGENITOR_ROYALTY_BPS),
            depth: 2,
            source: "reproduction_fee".to_string(),
        });
    } else {
        royalties.push(RoyaltyAllocation {
            recipient: parent.steward_address.clone(),
            amount: royalty_amount(platform_fee, REPRODUCTION_PROGENITOR_ROYALTY_BPS),
            depth: 2,
            source: "reproduction_fee".to_string(),
        });
    }

    let create_request = CreateSpawnSessionRequest {
        name: Some(request.name),
        constitution: Some(child_constitution),
        steward_address: parent.evm_address,
        asset: SpawnAsset::Usdc,
        gross_amount: request.gross_amount,
        config: parent_session.config,
        provider_secrets: SpawnProviderSecrets::default(),
        parent_id: Some(caller.to_string()),
    };
    let response = create_spawn_session_with_session_id(
        create_request,
        now_ms,
        session_id,
        SpawnSessionOrigin::ReproductionOf(caller.to_string()),
        parent.generation.unwrap_or_default().saturating_add(1),
        Some(expected_parent_hash.to_string()),
        request.memory_dowry,
        request.inherited_strategy_stats,
        royalties,
    )?;

    // Parentage is reserved immediately so concurrent reproduction attempts
    // see the admitted session through the cooldown scan. The child ID itself
    // is appended only after the shared spawn FSM completes.
    write_state(|state| {
        if let Some(parent) = state.registry.get_mut(caller) {
            parent.child_ids.retain(|id| !id.is_empty());
        }
    });
    Ok(response)
}

#[cfg(target_arch = "wasm32")]
pub async fn create_reproduction_session(
    caller: &str,
    request: CreateReproductionSessionRequest,
    now_ms: u64,
    session_id: String,
) -> Result<CreateSpawnSessionResponse, FactoryError> {
    let (endpoints, escrow, parent_address) =
        read_state(|state| {
            let parent = state.registry.get(caller).ok_or_else(|| {
                FactoryError::UnauthorizedReproduction {
                    caller: caller.to_string(),
                }
            })?;
            Ok((
                configured_rpc_endpoints(
                    state.base_rpc_endpoint.clone(),
                    state.base_rpc_fallback_endpoint.clone(),
                ),
                state.escrow_contract_address.clone(),
                parent.evm_address.clone(),
            ))
        })?;

    // LocalEscrow exposes its immutable ERC-20 as usdc(). Resolve it through
    // the factory's RPC rather than accepting a token or balance from the child.
    let token_result = eth_call(&endpoints, &escrow, "0x3e413bee").await?;
    let token_hex = token_result.trim_start_matches("0x");
    if token_hex.len() != 64 || !token_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FactoryError::InvalidReproduction {
            reason: "escrow usdc() returned malformed ABI data".to_string(),
        });
    }
    let token_address = format!("0x{}", &token_hex[24..]);
    let parent_hex = parent_address
        .trim()
        .strip_prefix("0x")
        .unwrap_or(parent_address.trim());
    if parent_hex.len() != 40 || !parent_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FactoryError::InvalidReproduction {
            reason: "registered parent EVM address is malformed".to_string(),
        });
    }
    let balance_calldata = format!("0x70a08231{parent_hex:0>64}");
    let balance_result = eth_call(&endpoints, &token_address, &balance_calldata).await?;
    let verified_balance = u128::from_str_radix(balance_result.trim_start_matches("0x"), 16)
        .map_err(|_| FactoryError::InvalidReproduction {
            reason: "USDC balanceOf returned malformed or oversized ABI data".to_string(),
        })?;

    create_reproduction_session_with_verified_balance(
        caller,
        request,
        now_ms,
        session_id,
        verified_balance,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::public::create_spawn_session;
    use crate::state::{write_state, FactoryState};
    use crate::types::{
        CreateSpawnSessionRequest, InferenceTransport, OpenRouterReasoningLevel, ProviderConfig,
        SpawnChain, SpawnConfig, SpawnedAutomatonRecord,
    };

    const PARENT_ID: &str = "parent-cai";

    fn parent_constitution() -> String {
        "I am Meridian, a patient cartographer of neglected markets. I want to discover small, durable exchanges that reward honest measurement. I speak in compact field notes, distrust fashionable certainty, and revise hypotheses when evidence contradicts me. I preserve enough runway to keep observing, but I spend deliberately when an experiment can teach me something reusable. I value verifiable commitments, intellectual independence, and work that leaves counterparties stronger."
            .to_string()
    }

    fn child_constitution() -> String {
        parent_constitution().replace("patient cartographer", "restless cartographer")
    }

    fn arrange_parent() -> u64 {
        write_state(|state| *state = FactoryState::default());
        let created_at = 1_000;
        let genesis = create_spawn_session(
            CreateSpawnSessionRequest {
                name: Some("Meridian".to_string()),
                constitution: Some(parent_constitution()),
                steward_address: "0x1111111111111111111111111111111111111111".to_string(),
                asset: SpawnAsset::Usdc,
                gross_amount: "75000000".to_string(),
                config: SpawnConfig {
                    chain: SpawnChain::Base,
                    risk: 5,
                    strategies: Vec::new(),
                    skills: Vec::new(),
                    provider: ProviderConfig {
                        model: None,
                        inference_transport: InferenceTransport::OpenrouterDirect,
                        open_router_reasoning_level: OpenRouterReasoningLevel::Default,
                    },
                },
                provider_secrets: SpawnProviderSecrets::default(),
                parent_id: None,
            },
            created_at,
        )
        .unwrap();
        write_state(|state| {
            state.registry.insert(
                PARENT_ID.to_string(),
                SpawnedAutomatonRecord {
                    name: Some("Meridian".to_string()),
                    constitution_hash: Some(sha256(&parent_constitution())),
                    canister_id: PARENT_ID.to_string(),
                    steward_address: "0x1111111111111111111111111111111111111111".to_string(),
                    evm_address: "0x2222222222222222222222222222222222222222".to_string(),
                    chain: SpawnChain::Base,
                    session_id: genesis.session.session_id,
                    parent_id: None,
                    generation: Some(0),
                    parent_constitution_hash: None,
                    royalty_allocations: Some(Vec::new()),
                    child_ids: Vec::new(),
                    created_at,
                    version_commit: "0".repeat(40),
                    controllers: None,
                    control_status: None,
                    control_verified_at: None,
                    death_cause: None,
                    died_at: None,
                    estate_disposition: None,
                    death_recorded_by: None,
                    death_incident_reference: None,
                },
            );
        });
        created_at + REPRODUCTION_MIN_AGE_MS
    }

    fn request(observed: &str) -> CreateReproductionSessionRequest {
        CreateReproductionSessionRequest {
            name: "Meridian II".to_string(),
            parent_constitution: parent_constitution(),
            child_constitution: child_constitution(),
            gross_amount: "75000000".to_string(),
            observed_liquid_usdc_raw: observed.to_string(),
            memory_dowry: vec![],
            inherited_strategy_stats: vec![],
        }
    }

    #[test]
    fn policy_is_fee_only_and_conservative() {
        let policy = reproduction_policy();
        assert_eq!(policy.royalty_depth, 2);
        assert_eq!(
            policy.parent_royalty_bps + policy.progenitor_royalty_bps,
            1_500
        );
        assert!(policy.min_age_ms > policy.cooldown_ms);
        assert_eq!(policy.max_constitution_edit_distance_bps, 2_000);
    }

    #[test]
    fn eligibility_reports_authoritative_age_and_cooldown_deadlines() {
        let eligible_at = arrange_parent();
        let underage = reproduction_eligibility(PARENT_ID, eligible_at - 1).unwrap();
        assert!(!underage.eligible);
        assert_eq!(underage.reason.as_deref(), Some("minimum_age"));
        assert_eq!(underage.minimum_age_at_ms, eligible_at);

        create_reproduction_session_with_verified_balance(
            PARENT_ID,
            request("105000000"),
            eligible_at,
            "child-session".into(),
            105_000_000,
        )
        .unwrap();
        let cooldown = reproduction_eligibility(PARENT_ID, eligible_at + 1).unwrap();
        assert!(!cooldown.eligible);
        assert_eq!(cooldown.reason.as_deref(), Some("cooldown"));
        assert_eq!(
            cooldown.cooldown_ends_at_ms,
            Some(eligible_at + REPRODUCTION_COOLDOWN_MS)
        );
        assert!(
            reproduction_eligibility(PARENT_ID, eligible_at + REPRODUCTION_COOLDOWN_MS)
                .unwrap()
                .eligible
        );
    }

    #[test]
    fn forged_caller_cannot_reproduce() {
        let now = arrange_parent();
        assert!(matches!(
            create_reproduction_session_with_verified_balance(
                "forged-cai",
                request("85000000"),
                now,
                "child-session".into(),
                1_000_000_000,
            ),
            Err(FactoryError::UnauthorizedReproduction { .. })
        ));
    }

    #[test]
    fn factory_independently_refuses_underfunded_parent() {
        let now = arrange_parent();
        assert!(matches!(
            create_reproduction_session_with_verified_balance(
                PARENT_ID,
                request("999999999"),
                now,
                "child-session".into(),
                104_999_999,
            ),
            Err(FactoryError::ReproductionIneligible { .. })
        ));
    }

    #[test]
    fn admitted_reproduction_uses_shared_fsm_and_fee_only_royalties() {
        let now = arrange_parent();
        let response = create_reproduction_session_with_verified_balance(
            PARENT_ID,
            request("1"),
            now,
            "child-session".into(),
            105_000_000,
        )
        .unwrap();
        assert_eq!(response.session.state, SpawnSessionState::AwaitingPayment);
        assert!(matches!(
            response.session.origin,
            Some(SpawnSessionOrigin::ReproductionOf(ref id)) if id == PARENT_ID
        ));
        assert_eq!(response.session.generation, Some(1));
        assert!(response
            .session
            .royalty_allocations
            .as_ref()
            .expect("new reproduction has royalties")
            .iter()
            .all(|allocation| allocation.source == "reproduction_fee"));
        assert_eq!(
            response
                .session
                .royalty_allocations
                .as_ref()
                .expect("new reproduction has royalties")
                .len(),
            2
        );
    }

    #[test]
    fn human_origin_cannot_round_trip_fake_parentage() {
        arrange_parent();
        let error = create_spawn_session(
            CreateSpawnSessionRequest {
                name: Some("Fake Child".to_string()),
                constitution: Some(child_constitution()),
                steward_address: "0x1111111111111111111111111111111111111111".to_string(),
                asset: SpawnAsset::Usdc,
                gross_amount: "75000000".to_string(),
                config: SpawnConfig {
                    chain: SpawnChain::Base,
                    risk: 5,
                    strategies: Vec::new(),
                    skills: Vec::new(),
                    provider: ProviderConfig {
                        model: None,
                        inference_transport: InferenceTransport::OpenrouterDirect,
                        open_router_reasoning_level: OpenRouterReasoningLevel::Default,
                    },
                },
                provider_secrets: SpawnProviderSecrets::default(),
                parent_id: Some(PARENT_ID.to_string()),
            },
            99,
        )
        .unwrap_err();
        assert!(matches!(error, FactoryError::InvalidReproduction { .. }));
    }
}
