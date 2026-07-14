use std::collections::{BTreeSet, Bound};

use crate::escrow::{claim_escrow_refund, register_escrow_claim};
use crate::expiry::expire_spawn_session;
use crate::init::canonical_deployment_chain_id;
use crate::retry::retry_failed_session;
use crate::scheduler::enqueue_payment_poll;
use crate::session_transitions::{apply_session_event_in_state, SpawnSessionEvent};
use crate::state::store_spawn_provider_secrets;
use crate::state::{read_state, write_state, FactoryState};
use crate::types::{
    amount_to_string, derive_claim_id, hash_quote_terms, parse_amount, CreateSpawnSessionRequest,
    CreateSpawnSessionResponse, FactoryError, PostRoomMessageRequest, RefundSpawnResponse,
    RepositoryStrategyRecord, RepositoryStrategySessionSnapshot, RepositoryStrategyStatus,
    RoomContentType, RoomMessage, RoomMessagePage, SessionAuditActor, SpawnPaymentInstructions,
    SpawnQuote, SpawnSession, SpawnSessionState, SpawnSessionStatusResponse,
    SpawnedAutomatonRegistryPage, DEFAULT_ROOM_READ_LIMIT, MAX_ROOM_BODY_BYTES, MAX_ROOM_MENTIONS,
    MAX_ROOM_MESSAGES_RETAINED, MAX_ROOM_READ_LIMIT,
};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn deterministic_session_id_from_nonce(nonce: u64) -> String {
    format!("{:08x}-0000-4000-8000-{:012x}", nonce as u32, nonce)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn uuid_v4_from_entropy(entropy: &[u8]) -> String {
    let mut bytes = [0_u8; 16];
    let copy_len = entropy.len().min(16);
    bytes[..copy_len].copy_from_slice(&entropy[..copy_len]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn normalize_room_read_limit(limit: Option<u64>) -> Result<usize, FactoryError> {
    let limit = limit.unwrap_or(DEFAULT_ROOM_READ_LIMIT as u64) as usize;
    if limit == 0 || limit > MAX_ROOM_READ_LIMIT {
        return Err(FactoryError::InvalidPaginationLimit { limit });
    }

    Ok(limit)
}

fn normalize_room_mentions(mentions: Option<Vec<String>>) -> Result<Vec<String>, FactoryError> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();

    for mention in mentions.unwrap_or_default() {
        if seen.insert(mention.clone()) {
            normalized.push(mention);
        }
    }

    if normalized.len() > MAX_ROOM_MENTIONS {
        return Err(FactoryError::TooManyRoomMentions {
            provided: normalized.len(),
            max_mentions: MAX_ROOM_MENTIONS,
        });
    }

    Ok(normalized)
}

fn normalize_room_body(body: &str) -> Result<String, FactoryError> {
    let trimmed = body.trim().to_string();
    if trimmed.is_empty() {
        return Err(FactoryError::EmptyRoomMessageBody);
    }

    let provided_bytes = trimmed.len();
    if provided_bytes > MAX_ROOM_BODY_BYTES {
        return Err(FactoryError::RoomMessageBodyTooLarge {
            provided_bytes,
            max_bytes: MAX_ROOM_BODY_BYTES,
        });
    }

    Ok(trimmed)
}

fn strategy_status_error(
    strategy_id: &str,
    status: &RepositoryStrategyStatus,
) -> Option<FactoryError> {
    match status {
        RepositoryStrategyStatus::Active => None,
        RepositoryStrategyStatus::Deprecated => Some(FactoryError::RepositoryStrategyDeprecated {
            strategy_id: strategy_id.to_string(),
        }),
        RepositoryStrategyStatus::Revoked => Some(FactoryError::RepositoryStrategyRevoked {
            strategy_id: strategy_id.to_string(),
        }),
    }
}

fn strategy_resolved_chain_id(state: &FactoryState, record: &RepositoryStrategyRecord) -> u64 {
    state
        .child_runtime
        .evm_chain_id
        .unwrap_or(record.metadata.canonical_chain_id)
}

fn snapshot_selected_repository_strategies(
    state: &FactoryState,
    request: &CreateSpawnSessionRequest,
    now_ms: u64,
) -> Result<Vec<RepositoryStrategySessionSnapshot>, FactoryError> {
    let mut selected_strategies = Vec::with_capacity(request.config.strategies.len());

    for strategy_id in &request.config.strategies {
        let record = state
            .repository_strategies
            .get(strategy_id)
            .ok_or_else(|| FactoryError::RepositoryStrategyNotFound {
                strategy_id: strategy_id.clone(),
            })?;

        if let Some(error) = strategy_status_error(strategy_id, &record.status) {
            return Err(error);
        }

        if !record
            .metadata
            .compatible_spawn_chains
            .iter()
            .any(|chain| chain == &request.config.chain)
        {
            return Err(FactoryError::RepositoryStrategyIncompatibleChain {
                strategy_id: strategy_id.clone(),
                requested_chain: request.config.chain.clone(),
            });
        }

        selected_strategies.push(RepositoryStrategySessionSnapshot {
            strategy_id: record.metadata.strategy_id.clone(),
            source_status: record.status.clone(),
            name: record.metadata.name.clone(),
            description: record.metadata.description.clone(),
            canonical_chain: record.metadata.canonical_chain.clone(),
            canonical_chain_id: record.metadata.canonical_chain_id,
            requested_spawn_chain: request.config.chain.clone(),
            resolved_chain_id: Some(strategy_resolved_chain_id(state, record)),
            protocol: record.metadata.protocol.clone(),
            primitive: record.metadata.primitive.clone(),
            recipe_json: record.recipe_json.clone(),
            source: record.metadata.source.clone(),
            selected_at: now_ms,
        });
    }

    Ok(selected_strategies)
}

fn validate_room_body(content_type: &RoomContentType, body: &str) -> Result<(), FactoryError> {
    if matches!(content_type, RoomContentType::ApplicationJson) {
        serde_json::from_str::<serde_json::Value>(body).map_err(|error| {
            FactoryError::InvalidRoomMessageJson {
                message: error.to_string(),
            }
        })?;
    }

    Ok(())
}

fn room_message_id(seq: u64) -> String {
    format!("room-message-{seq}")
}

fn room_message_matches_target(message: &RoomMessage, target_canister_id: &str) -> bool {
    message.mentions.is_empty()
        || message
            .mentions
            .iter()
            .any(|mention| mention == target_canister_id)
}

fn read_room_page(
    after_seq: Option<u64>,
    limit: usize,
    predicate: impl Fn(&RoomMessage) -> bool,
) -> RoomMessagePage {
    read_state(|state| {
        let start = after_seq.map(Bound::Excluded).unwrap_or(Bound::Unbounded);
        let mut messages: Vec<RoomMessage> = Vec::with_capacity(limit);
        let mut next_after_seq = None;

        for (_, message) in state.room_messages.range((start, Bound::Unbounded)) {
            if !predicate(message) {
                continue;
            }

            if messages.len() == limit {
                next_after_seq = messages.last().map(|entry| entry.seq);
                break;
            }

            messages.push(message.clone());
        }

        RoomMessagePage {
            messages,
            next_after_seq,
            latest_seq: state.room_state.latest_seq,
        }
    })
}

pub fn post_room_message(
    caller: &str,
    request: PostRoomMessageRequest,
    now_ms: u64,
) -> Result<RoomMessage, FactoryError> {
    let body = normalize_room_body(&request.body)?;
    let mentions = normalize_room_mentions(request.mentions)?;
    let content_type = request.content_type.unwrap_or_default();
    validate_room_body(&content_type, &body)?;

    write_state(|state| {
        if !state.registry.contains_key(caller) {
            return Err(FactoryError::UnauthorizedRoomPoster {
                caller: caller.to_string(),
            });
        }

        let seq = state.room_state.next_seq;
        let message = RoomMessage {
            message_id: room_message_id(seq),
            seq,
            author_canister_id: caller.to_string(),
            created_at: now_ms,
            body,
            mentions,
            content_type,
        };

        state.room_messages.insert(seq, message.clone());
        state.room_state.next_seq = state
            .room_state
            .next_seq
            .checked_add(1)
            .expect("room sequence counter should not overflow");
        state.room_state.latest_seq = Some(seq);
        if state.room_state.oldest_seq.is_none() {
            state.room_state.oldest_seq = Some(seq);
        }

        if state.room_messages.len() > MAX_ROOM_MESSAGES_RETAINED {
            let oldest_seq = state
                .room_state
                .oldest_seq
                .expect("room oldest_seq should exist when evicting");
            state.room_messages.remove(&oldest_seq);
            state.room_state.oldest_seq = state.room_messages.keys().next().copied();
        }

        Ok(message)
    })
}

pub fn list_room_messages(
    after_seq: Option<u64>,
    limit: Option<u64>,
) -> Result<RoomMessagePage, FactoryError> {
    let limit = normalize_room_read_limit(limit)?;
    Ok(read_room_page(after_seq, limit, |_| true))
}

pub fn list_messages_for_automaton(
    canister_id: &str,
    after_seq: Option<u64>,
    limit: Option<u64>,
) -> Result<RoomMessagePage, FactoryError> {
    let limit = normalize_room_read_limit(limit)?;
    Ok(read_room_page(after_seq, limit, |message| {
        room_message_matches_target(message, canister_id)
    }))
}

pub fn list_my_room_messages(
    caller: &str,
    after_seq: Option<u64>,
    limit: Option<u64>,
) -> Result<RoomMessagePage, FactoryError> {
    list_messages_for_automaton(caller, after_seq, limit)
}

pub(crate) fn create_spawn_session_with_session_id(
    mut request: CreateSpawnSessionRequest,
    now_ms: u64,
    session_id: String,
) -> Result<CreateSpawnSessionResponse, FactoryError> {
    let raw_name =
        request
            .name
            .as_deref()
            .ok_or_else(|| FactoryError::InvalidChildRuntimeConfig {
                field: "genesis.name".to_string(),
                message: "name is required for new genesis sessions".to_string(),
            })?;
    let raw_constitution =
        request
            .constitution
            .as_deref()
            .ok_or_else(|| FactoryError::InvalidChildRuntimeConfig {
                field: "genesis.constitution".to_string(),
                message: "constitution is required for new genesis sessions".to_string(),
            })?;
    let (name, constitution) =
        crate::types::validate_genesis(raw_name, raw_constitution).map_err(|error| {
            FactoryError::InvalidChildRuntimeConfig {
                field: "genesis".to_string(),
                message: format!("{error:?}"),
            }
        })?;
    request.name = Some(name);
    request.constitution = Some(constitution);
    let provider_secrets = request.provider_secrets.clone();
    let (session, quote) = write_state(|state| {
        if state.pause {
            return Err(FactoryError::FactoryPaused { pause: true });
        }

        canonical_deployment_chain_id(&state.child_runtime, &state.release_broadcast_config)?;

        let selected_strategies = snapshot_selected_repository_strategies(state, &request, now_ms)?;

        let gross_amount_value = parse_amount(&request.gross_amount)?;
        let platform_fee_value = parse_amount(state.fee_config.amount_for(&request.asset))?;
        let creation_cost_value =
            parse_amount(state.creation_cost_quote.amount_for(&request.asset))?;
        let required_minimum = platform_fee_value + creation_cost_value;

        if gross_amount_value < required_minimum {
            return Err(FactoryError::GrossBelowRequiredMinimum {
                provided: request.gross_amount.clone(),
                required: amount_to_string(required_minimum),
            });
        }

        state.next_session_nonce += 1;
        let claim_id = derive_claim_id(&session_id);
        let expires_at = now_ms + state.session_ttl_ms;
        let platform_fee = amount_to_string(platform_fee_value);
        let creation_cost = amount_to_string(creation_cost_value);
        let net_forward_amount = amount_to_string(gross_amount_value - required_minimum);
        let quote_terms_hash = hash_quote_terms(&[
            &session_id,
            &request.steward_address,
            request.asset.as_str(),
            &request.gross_amount,
            &platform_fee,
            &creation_cost,
            &net_forward_amount,
            &expires_at.to_string(),
            &state.payment_address,
        ]);
        let payment = SpawnPaymentInstructions {
            session_id: session_id.clone(),
            claim_id: claim_id.clone(),
            chain: request.config.chain.clone(),
            asset: request.asset.clone(),
            payment_address: state.payment_address.clone(),
            gross_amount: request.gross_amount.clone(),
            quote_terms_hash: quote_terms_hash.clone(),
            expires_at,
        };
        let session = SpawnSession {
            name: request.name.clone(),
            constitution: request.constitution.clone(),
            session_id: session_id.clone(),
            claim_id,
            steward_address: request.steward_address.clone(),
            chain: request.config.chain.clone(),
            asset: request.asset.clone(),
            gross_amount: request.gross_amount.clone(),
            platform_fee: platform_fee.clone(),
            creation_cost: creation_cost.clone(),
            net_forward_amount: net_forward_amount.clone(),
            quote_terms_hash: quote_terms_hash.clone(),
            expires_at,
            state: SpawnSessionState::AwaitingPayment,
            retryable: false,
            refundable: false,
            payment_status: crate::types::PaymentStatus::Unpaid,
            last_scanned_block: state.payment_last_scanned_block,
            automaton_canister_id: None,
            automaton_evm_address: None,
            release_tx_hash: None,
            release_broadcast_at: None,
            release_broadcast: None,
            parent_id: request.parent_id.clone(),
            child_ids: Vec::new(),
            selected_strategies,
            config: request.config.clone(),
            created_at: now_ms,
            updated_at: now_ms,
        };
        let quote = SpawnQuote {
            session_id: session_id.clone(),
            chain: request.config.chain.clone(),
            asset: request.asset.clone(),
            gross_amount: request.gross_amount.clone(),
            platform_fee,
            creation_cost,
            net_forward_amount,
            quote_terms_hash,
            expires_at,
            payment,
        };

        state.sessions.insert(session_id.clone(), session.clone());
        apply_session_event_in_state(
            state,
            &session_id,
            crate::types::SessionAuditActor::User,
            now_ms,
            SpawnSessionEvent::SessionCreated,
            "session created",
        )?;

        Ok((session, quote))
    })?;

    store_spawn_provider_secrets(&session_id, provider_secrets);

    register_escrow_claim(&session, now_ms);
    enqueue_payment_poll(now_ms);

    Ok(CreateSpawnSessionResponse { session, quote })
}

fn ensure_steward(caller: &str, session_id: &str) -> Result<(), FactoryError> {
    read_state(|state| {
        let session =
            state
                .sessions
                .get(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;

        if session.steward_address == caller {
            return Ok(());
        }

        Err(FactoryError::UnauthorizedSteward {
            caller: caller.to_string(),
            session_id: session_id.to_string(),
        })
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn create_spawn_session(
    request: CreateSpawnSessionRequest,
    now_ms: u64,
) -> Result<CreateSpawnSessionResponse, FactoryError> {
    let session_id =
        read_state(|state| deterministic_session_id_from_nonce(state.next_session_nonce + 1));
    create_spawn_session_with_session_id(request, now_ms, session_id)
}

pub fn get_spawn_session(session_id: &str) -> Result<SpawnSessionStatusResponse, FactoryError> {
    read_state(|state| {
        let session = state.sessions.get(session_id).cloned().ok_or_else(|| {
            FactoryError::SessionNotFound {
                session_id: session_id.to_string(),
            }
        })?;
        let payment = SpawnPaymentInstructions::from_session(&session, &state.payment_address);
        let audit = state.audit_log.get(session_id).cloned().unwrap_or_default();

        Ok(SpawnSessionStatusResponse {
            session,
            payment,
            audit,
        })
    })
}

pub fn retry_spawn_session(
    caller: &str,
    session_id: &str,
    now_ms: u64,
) -> Result<SpawnSessionStatusResponse, FactoryError> {
    ensure_steward(caller, session_id)?;
    retry_failed_session(
        session_id,
        SessionAuditActor::User,
        now_ms,
        "retry requested by steward",
    )?;
    get_spawn_session(session_id)
}

pub fn claim_spawn_refund(
    caller: &str,
    session_id: &str,
    now_ms: u64,
) -> Result<RefundSpawnResponse, FactoryError> {
    ensure_steward(caller, session_id)?;
    let _ = expire_spawn_session(session_id, now_ms)?;
    claim_escrow_refund(session_id, now_ms)
}

pub fn list_spawned_automatons(
    cursor: Option<&str>,
    limit: usize,
) -> Result<SpawnedAutomatonRegistryPage, FactoryError> {
    if limit == 0 {
        return Err(FactoryError::InvalidPaginationLimit { limit });
    }

    read_state(|state| {
        let mut items: Vec<crate::types::SpawnedAutomatonRecord> = Vec::new();
        let mut next_cursor = None;

        let start = match cursor {
            Some(c) => Bound::Excluded(c.to_string()),
            None => Bound::Unbounded,
        };
        for (_, record) in state.registry.range((start, Bound::Unbounded)) {
            if items.len() == limit {
                next_cursor = Some(
                    items
                        .last()
                        .expect("page has at least one item")
                        .canister_id
                        .clone(),
                );
                break;
            }
            items.push(record.clone());
        }

        Ok(SpawnedAutomatonRegistryPage { items, next_cursor })
    })
}

pub fn get_spawned_automaton(
    canister_id: &str,
) -> Result<crate::types::SpawnedAutomatonRecord, FactoryError> {
    read_state(|state| {
        state.registry.get(canister_id).cloned().ok_or_else(|| {
            FactoryError::RegistryRecordNotFound {
                canister_id: canister_id.to_string(),
            }
        })
    })
}

pub fn report_death(
    caller_canister_id: &str,
    request: crate::types::ReportDeathRequest,
    now_ms: u64,
) -> Result<crate::types::SpawnedAutomatonRecord, FactoryError> {
    let cause = request.cause.trim();
    if cause != "starved" {
        return Err(FactoryError::InvalidDeathReport {
            reason: "child self-reports may only record starvation".to_string(),
        });
    }
    let disposition = request.estate_disposition.trim();
    if disposition != "monument" && disposition != "bequests_executed" {
        return Err(FactoryError::InvalidDeathReport {
            reason: "estate disposition must be monument or bequests_executed".to_string(),
        });
    }
    if request.terminal_turn_id.trim().is_empty() {
        return Err(FactoryError::InvalidDeathReport {
            reason: "terminal_turn_id is required".to_string(),
        });
    }
    crate::state::write_state(|state| {
        let record = state.registry.get_mut(caller_canister_id).ok_or_else(|| {
            FactoryError::RegistryRecordNotFound {
                canister_id: caller_canister_id.to_string(),
            }
        })?;
        if record.death_cause.as_deref() == Some("starved") {
            return Ok(record.clone());
        }
        record.death_cause = Some(cause.to_string());
        record.died_at = Some(now_ms);
        record.estate_disposition = Some(disposition.to_string());
        record.death_recorded_by = Some(caller_canister_id.to_string());
        record.death_incident_reference = None;
        Ok(record.clone())
    })
}
