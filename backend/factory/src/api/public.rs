use std::collections::{BTreeSet, Bound};

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};

#[cfg(not(target_arch = "wasm32"))]
use crate::escrow::claim_escrow_refund;
use crate::escrow::register_escrow_claim;
use crate::expiry::expire_session_in_state;
#[cfg(not(target_arch = "wasm32"))]
use crate::expiry::expire_spawn_session;
use crate::init::canonical_deployment_chain_id;
#[cfg(not(target_arch = "wasm32"))]
use crate::retry::retry_failed_session;
use crate::retry::retry_spawn_session_in_state;
use crate::scheduler::enqueue_payment_poll;
use crate::session_transitions::{apply_session_event_in_state, SpawnSessionEvent};
use crate::state::store_spawn_provider_secrets;
use crate::state::{read_state, write_state, FactoryState};
#[cfg(not(target_arch = "wasm32"))]
use crate::types::RefundSpawnResponse;
use crate::types::{
    amount_to_string, derive_claim_id, hash_quote_terms, parse_amount, CreateSpawnSessionRequest,
    CreateSpawnSessionResponse, FactoryError, FactoryStewardCommand, FactoryStewardCommandResult,
    FactoryStewardProof, FactoryStewardProofTemplate, InheritedStrategyStat, MemoryDowryFact,
    PostRoomMessageRequest, RepositoryStrategyRecord, RepositoryStrategySessionSnapshot,
    RepositoryStrategyStatus, RoomContentType, RoomMessage, RoomMessagePage, RoyaltyAllocation,
    SessionAuditActor, SpawnPaymentInstructions, SpawnQuote, SpawnSession, SpawnSessionOrigin,
    SpawnSessionState, SpawnSessionStatusResponse, SpawnedAutomatonRegistryPage,
    DEFAULT_ROOM_READ_LIMIT, MAX_ROOM_BODY_BYTES, MAX_ROOM_MENTIONS, MAX_ROOM_MESSAGES_RETAINED,
    MAX_ROOM_READ_LIMIT,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_spawn_session_with_session_id(
    mut request: CreateSpawnSessionRequest,
    now_ms: u64,
    session_id: String,
    origin: SpawnSessionOrigin,
    generation: u32,
    parent_constitution_hash: Option<String>,
    memory_dowry: Vec<MemoryDowryFact>,
    inherited_strategy_stats: Vec<InheritedStrategyStat>,
    royalty_allocations: Vec<RoyaltyAllocation>,
) -> Result<CreateSpawnSessionResponse, FactoryError> {
    match &origin {
        SpawnSessionOrigin::Human if request.parent_id.is_some() => {
            return Err(FactoryError::InvalidReproduction {
                reason: "human spawn sessions cannot claim parentage".to_string(),
            });
        }
        SpawnSessionOrigin::ReproductionOf(parent_id)
            if request.parent_id.as_deref() != Some(parent_id.as_str()) =>
        {
            return Err(FactoryError::InvalidReproduction {
                reason: "reproduction origin and parent_id disagree".to_string(),
            });
        }
        _ => {}
    }
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
            steward_address: request.steward_address.trim().to_ascii_lowercase(),
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
            origin: Some(origin),
            generation: Some(generation),
            parent_constitution_hash,
            memory_dowry: Some(memory_dowry),
            inherited_strategy_stats: Some(inherited_strategy_stats),
            royalty_allocations: Some(royalty_allocations),
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

const FACTORY_STEWARD_DOMAIN: &str = "ic-automaton:factory-steward:v1";
const FACTORY_STEWARD_PROOF_TTL_NS: u64 = 5 * 60 * 1_000_000_000;
const REFUND_COMMAND_LEASE_MS: u64 = 60_000;

fn invalid_proof(reason: impl Into<String>) -> FactoryError {
    FactoryError::InvalidStewardProof {
        reason: reason.into(),
    }
}

fn normalize_address(raw: &str) -> Result<String, FactoryError> {
    let value = raw.trim().to_ascii_lowercase();
    let body = value
        .strip_prefix("0x")
        .ok_or_else(|| invalid_proof("address must be 0x-prefixed"))?;
    if body.len() != 40 || !body.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_proof("address must be 20 bytes of hex"));
    }
    Ok(format!("0x{body}"))
}

fn decode_hex(raw: &str, bytes: usize, field: &str) -> Result<Vec<u8>, FactoryError> {
    let body = raw
        .trim()
        .strip_prefix("0x")
        .ok_or_else(|| invalid_proof(format!("{field} must be 0x-prefixed")))?;
    if body.len() != bytes * 2 || !body.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_proof(format!(
            "{field} must be {bytes} bytes of hex"
        )));
    }
    (0..body.len())
        .step_by(2)
        .map(|offset| {
            u8::from_str_radix(&body[offset..offset + 2], 16)
                .map_err(|_| invalid_proof(format!("invalid {field}")))
        })
        .collect()
}

pub(crate) fn factory_steward_command_hash(command: &FactoryStewardCommand) -> String {
    let encoded = candid::encode_one(command).expect("factory steward command candid encoding");
    format!("0x{:x}", Keccak256::digest(encoded))
}

fn signing_payload(
    canister_id: &str,
    chain_id: u64,
    address: &str,
    command_hash: &str,
    nonce: u64,
    expires_at_ns: u64,
) -> String {
    format!("{FACTORY_STEWARD_DOMAIN}\ncanister_id:{canister_id}\nchain_id:{chain_id}\naddress:{address}\ncommand_hash:{command_hash}\nnonce:{nonce}\nexpires_at_ns:{expires_at_ns}")
}

fn recover_signer(payload: &str, signature: &str) -> Result<String, FactoryError> {
    let bytes = decode_hex(signature, 65, "signature")?;
    let recovery = match bytes[64] {
        0 | 1 => bytes[64],
        27 | 28 => bytes[64] - 27,
        value => return Err(invalid_proof(format!("unsupported recovery id {value}"))),
    };
    let signature = Signature::from_slice(&bytes[..64])
        .map_err(|error| invalid_proof(format!("malformed signature: {error}")))?;
    let prefix = format!("\x19Ethereum Signed Message:\n{}", payload.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(payload.as_bytes());
    let digest = hasher.finalize();
    let key = VerifyingKey::recover_from_prehash(
        &digest,
        &signature,
        RecoveryId::try_from(recovery)
            .map_err(|error| invalid_proof(format!("invalid recovery id: {error}")))?,
    )
    .map_err(|error| invalid_proof(format!("signature recovery failed: {error}")))?;
    let point = key.to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    Ok(format!(
        "0x{}",
        hash[12..]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

pub fn prepare_spawn_steward_command(
    command: FactoryStewardCommand,
    canister_id: &str,
    now_ns: u64,
) -> Result<FactoryStewardProofTemplate, FactoryError> {
    let session_id = command.session_id();
    read_state(|state| {
        let session =
            state
                .sessions
                .get(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session.chain != crate::types::SpawnChain::Base {
            return Err(invalid_proof("unsupported session chain"));
        }
        let chain_id =
            canonical_deployment_chain_id(&state.child_runtime, &state.release_broadcast_config)?;
        let address = normalize_address(&session.steward_address)?;
        let nonce = *state.steward_command_nonces.get(session_id).unwrap_or(&0);
        let expires_at_ns = now_ns
            .checked_add(FACTORY_STEWARD_PROOF_TTL_NS)
            .ok_or_else(|| invalid_proof("expiry overflow"))?;
        let command_hash = factory_steward_command_hash(&command);
        Ok(FactoryStewardProofTemplate {
            signing_payload: signing_payload(
                canister_id,
                chain_id,
                &address,
                &command_hash,
                nonce,
                expires_at_ns,
            ),
            chain_id,
            address,
            command_hash,
            nonce,
            expires_at_ns,
        })
    })
}

pub(crate) fn authorize_and_consume(
    command: &FactoryStewardCommand,
    proof: &FactoryStewardProof,
    canister_id: &str,
    now_ns: u64,
    now_ms: u64,
) -> Result<(Option<SpawnSessionStatusResponse>, Option<u64>), FactoryError> {
    let session_id = command.session_id();
    let expected_hash = factory_steward_command_hash(command);
    let address = normalize_address(&proof.address)?;
    let (expected_chain_id, expected_address, expected_nonce) = read_state(|state| {
        let session =
            state
                .sessions
                .get(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        if session.chain != crate::types::SpawnChain::Base {
            return Err(invalid_proof("unsupported session chain"));
        }
        Ok::<_, FactoryError>((
            canonical_deployment_chain_id(&state.child_runtime, &state.release_broadcast_config)?,
            normalize_address(&session.steward_address)?,
            *state.steward_command_nonces.get(session_id).unwrap_or(&0),
        ))
    })?;
    if proof.chain_id != expected_chain_id {
        return Err(invalid_proof("chain id mismatch"));
    }
    if address != expected_address {
        return Err(invalid_proof("signer is not the session steward"));
    }
    if proof.nonce != expected_nonce {
        return Err(invalid_proof(format!(
            "nonce mismatch: expected {expected_nonce}, got {}",
            proof.nonce
        )));
    }
    if proof.command_hash.to_ascii_lowercase() != expected_hash {
        return Err(invalid_proof("command hash mismatch"));
    }
    if now_ns > proof.expires_at_ns {
        return Err(invalid_proof("proof expired"));
    }
    if proof.expires_at_ns.saturating_sub(now_ns) > FACTORY_STEWARD_PROOF_TTL_NS {
        return Err(invalid_proof("proof expiry exceeds maximum"));
    }
    let payload = signing_payload(
        canister_id,
        proof.chain_id,
        &address,
        &expected_hash,
        proof.nonce,
        proof.expires_at_ns,
    );
    if recover_signer(&payload, &proof.signature)? != address {
        return Err(invalid_proof(
            "recovered signer does not match proof address",
        ));
    }
    write_state(|state| {
        let session =
            state
                .sessions
                .get(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let chain_id =
            canonical_deployment_chain_id(&state.child_runtime, &state.release_broadcast_config)?;
        if session.chain != crate::types::SpawnChain::Base {
            return Err(invalid_proof("unsupported session chain"));
        }
        if proof.chain_id != chain_id {
            return Err(invalid_proof("chain id mismatch"));
        }
        if normalize_address(&session.steward_address)? != address {
            return Err(invalid_proof("signer is not the session steward"));
        }
        let expected_nonce = *state.steward_command_nonces.get(session_id).unwrap_or(&0);
        if proof.nonce != expected_nonce {
            return Err(invalid_proof(format!(
                "nonce mismatch: expected {expected_nonce}, got {}",
                proof.nonce
            )));
        }
        let refund_resume = matches!(command, FactoryStewardCommand::ClaimSpawnRefund { .. })
            && state.steward_refunds_in_flight.contains(session_id);
        let refund_generation = if matches!(command, FactoryStewardCommand::ClaimSpawnRefund { .. })
        {
            match state.steward_refund_leases.get(session_id) {
                Some(lease) if refund_resume && now_ms <= lease.expires_at_ms => {
                    return Err(invalid_proof(
                        "refund command is already accepted and awaiting reconciliation",
                    ));
                }
                Some(lease) => Some(
                    lease
                        .generation
                        .checked_add(1)
                        .ok_or_else(|| invalid_proof("refund generation overflow"))?,
                ),
                None => Some(1),
            }
        } else {
            None
        };
        match command {
            FactoryStewardCommand::RetrySpawnSession { .. } => {
                retry_spawn_session_in_state(
                    state,
                    session_id,
                    SessionAuditActor::User,
                    now_ms,
                    "retry requested by verified EVM steward",
                )?;
            }
            FactoryStewardCommand::ClaimSpawnRefund { .. } => {
                if !refund_resume {
                    let _ = expire_session_in_state(
                        state,
                        session_id,
                        SessionAuditActor::User,
                        now_ms,
                        "refund requested by verified EVM steward",
                    )?;
                }
                let session = state
                    .sessions
                    .get(session_id)
                    .expect("session remains after expiry");
                let claim = state.escrow_claims.get(session_id).ok_or_else(|| {
                    FactoryError::EscrowClaimNotFound {
                        session_id: session_id.to_string(),
                    }
                })?;
                if session.payment_status != crate::types::PaymentStatus::Refunded
                    && (!matches!(
                        session.state,
                        SpawnSessionState::Expired | SpawnSessionState::Failed
                    ) || !session.refundable
                        || !claim.refundable)
                {
                    return Err(FactoryError::SessionNotRefundable {
                        session_id: session_id.to_string(),
                        state: session.state.clone(),
                        payment_status: session.payment_status.clone(),
                    });
                }
            }
        }
        if !refund_resume {
            state.steward_command_nonces.insert(
                session_id.to_string(),
                expected_nonce
                    .checked_add(1)
                    .ok_or_else(|| invalid_proof("nonce overflow"))?,
            );
        }
        if matches!(command, FactoryStewardCommand::ClaimSpawnRefund { .. }) && !refund_resume {
            state
                .steward_refunds_in_flight
                .insert(session_id.to_string());
        }
        if let Some(generation) = refund_generation {
            state.steward_refund_leases.insert(
                session_id.to_string(),
                crate::state::RefundCommandLease {
                    generation,
                    expires_at_ms: now_ms.saturating_add(REFUND_COMMAND_LEASE_MS),
                },
            );
        }
        if matches!(command, FactoryStewardCommand::RetrySpawnSession { .. }) {
            let session = state
                .sessions
                .get(session_id)
                .cloned()
                .expect("session remains after retry");
            let payment = SpawnPaymentInstructions::from_session(&session, &state.payment_address);
            let audit = state.audit_log.get(session_id).cloned().unwrap_or_default();
            Ok((
                Some(SpawnSessionStatusResponse {
                    session,
                    payment,
                    audit,
                }),
                None,
            ))
        } else {
            Ok((None, refund_generation))
        }
    })
}

fn clear_refund_in_flight(session_id: &str, generation: u64) {
    write_state(|state| {
        if state
            .steward_refund_leases
            .get(session_id)
            .is_some_and(|lease| lease.generation == generation)
        {
            state.steward_refunds_in_flight.remove(session_id);
            state.steward_refund_leases.remove(session_id);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn create_spawn_session(
    request: CreateSpawnSessionRequest,
    now_ms: u64,
) -> Result<CreateSpawnSessionResponse, FactoryError> {
    let session_id =
        read_state(|state| deterministic_session_id_from_nonce(state.next_session_nonce + 1));
    create_spawn_session_with_session_id(
        request,
        now_ms,
        session_id,
        SpawnSessionOrigin::Human,
        0,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
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

#[cfg(not(target_arch = "wasm32"))]
pub fn execute_spawn_steward_command(
    command: FactoryStewardCommand,
    proof: FactoryStewardProof,
    canister_id: &str,
    now_ns: u64,
    now_ms: u64,
) -> Result<FactoryStewardCommandResult, FactoryError> {
    let (result, refund_generation) =
        authorize_and_consume(&command, &proof, canister_id, now_ns, now_ms)?;
    match command {
        FactoryStewardCommand::RetrySpawnSession { .. } => Ok(FactoryStewardCommandResult::Retry(
            Box::new(result.expect("retry response")),
        )),
        FactoryStewardCommand::ClaimSpawnRefund { session_id } => {
            let generation = refund_generation.expect("refund generation");
            let result =
                crate::escrow::claim_escrow_refund_authorized(&session_id, now_ms, generation)
                    .map(FactoryStewardCommandResult::Refund);
            clear_refund_in_flight(&session_id, generation);
            result
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn claim_spawn_refund_for_test(
    caller: &str,
    session_id: &str,
    now_ms: u64,
) -> Result<RefundSpawnResponse, FactoryError> {
    let _ = caller;
    let _ = expire_spawn_session(session_id, now_ms)?;
    claim_escrow_refund(session_id, now_ms)
}

#[cfg(target_arch = "wasm32")]
pub async fn execute_spawn_steward_command(
    command: FactoryStewardCommand,
    proof: FactoryStewardProof,
    canister_id: &str,
    now_ns: u64,
    now_ms: u64,
) -> Result<FactoryStewardCommandResult, FactoryError> {
    let (result, refund_generation) =
        authorize_and_consume(&command, &proof, canister_id, now_ns, now_ms)?;
    match command {
        FactoryStewardCommand::RetrySpawnSession { .. } => Ok(FactoryStewardCommandResult::Retry(
            Box::new(result.expect("retry response")),
        )),
        FactoryStewardCommand::ClaimSpawnRefund { session_id } => {
            let generation = refund_generation.expect("refund generation");
            let result =
                crate::escrow::claim_escrow_refund_authorized(&session_id, now_ms, generation)
                    .await
                    .map(FactoryStewardCommandResult::Refund);
            clear_refund_in_flight(&session_id, generation);
            result
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn retry_spawn_session_for_test(
    caller: &str,
    session_id: &str,
    now_ms: u64,
) -> Result<SpawnSessionStatusResponse, FactoryError> {
    let _ = caller;
    retry_failed_session(
        session_id,
        SessionAuditActor::User,
        now_ms,
        "retry requested in test",
    )?;
    get_spawn_session(session_id)
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
