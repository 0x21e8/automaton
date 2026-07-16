use std::collections::{BTreeMap, BTreeSet};

use crate::base_rpc::{
    configured_rpc_endpoints, BaseDepositLog, PaymentScanPlan, BASE_LOG_WINDOW_LIMIT,
};
use crate::expiry::expire_session_in_state;
use crate::scheduler::{
    enqueue_spawn_execution_in_state, session_needs_payment_poll, sync_payment_poll_job_in_state,
};
use crate::session_transitions::{
    apply_session_event_in_state, sync_session_derived_flags_in_state, SpawnSessionEvent,
};
use crate::state::{
    clear_provider_secrets, delete_spawn_provider_secrets, read_state, record_session_audit,
    write_state, FactoryState,
};
use crate::types::{
    amount_to_string, parse_amount, EscrowClaim, FactoryError, PaymentStatus, RefundSpawnResponse,
    SessionAuditActor, SpawnSession, SpawnSessionState,
};

pub fn register_escrow_claim(session: &SpawnSession, now_ms: u64) -> EscrowClaim {
    write_state(|state| {
        let claim = EscrowClaim {
            session_id: session.session_id.clone(),
            claim_id: session.claim_id.clone(),
            quote_terms_hash: session.quote_terms_hash.clone(),
            payment_address: state.payment_address.clone(),
            chain: session.chain.clone(),
            asset: session.asset.clone(),
            required_gross_amount: session.gross_amount.clone(),
            paid_amount: "0".to_string(),
            payment_status: PaymentStatus::Unpaid,
            last_scanned_block: session.last_scanned_block,
            refundable: false,
            refunded_at: None,
            refund_broadcast: None,
            created_at: now_ms,
            updated_at: now_ms,
        };
        state
            .escrow_claims
            .insert(session.session_id.clone(), claim.clone());
        claim
    })
}

pub fn get_escrow_claim(session_id: &str) -> Result<EscrowClaim, FactoryError> {
    read_state(|state| {
        state.escrow_claims.get(session_id).cloned().ok_or_else(|| {
            FactoryError::EscrowClaimNotFound {
                session_id: session_id.to_string(),
            }
        })
    })
}

fn payment_status_for_amount(total_paid: u128, required: u128) -> PaymentStatus {
    if total_paid >= required {
        PaymentStatus::Paid
    } else if total_paid > 0 {
        PaymentStatus::Partial
    } else {
        PaymentStatus::Unpaid
    }
}
pub fn reconcile_escrow_payments(
    logs: &[BaseDepositLog],
    scan_to_block: u64,
    now_ms: u64,
) -> Result<Vec<EscrowClaim>, FactoryError> {
    let mut amounts_by_claim: BTreeMap<String, (u128, u64)> = BTreeMap::new();
    for log in logs {
        let amount = parse_amount(&log.amount)?;
        let entry = amounts_by_claim
            .entry(log.claim_id.clone())
            .or_insert((0, log.block_number));
        entry.0 = entry
            .0
            .checked_add(amount)
            .ok_or_else(|| FactoryError::InvalidAmount {
                value: log.amount.clone(),
            })?;
        entry.1 = entry.1.max(log.block_number);
    }

    let result = write_state(|state| {
        state.payment_last_scanned_block = Some(scan_to_block);

        let active_session_ids: Vec<String> = state
            .sessions
            .iter()
            .filter(|(_, session)| session_needs_payment_poll(session))
            .map(|(session_id, _)| session_id.clone())
            .collect();

        let mut updated_claims = Vec::new();

        for session_id in active_session_ids {
            let session_snapshot = state.sessions.get(&session_id).cloned().ok_or_else(|| {
                FactoryError::SessionNotFound {
                    session_id: session_id.clone(),
                }
            })?;

            let prior_claim = state.escrow_claims.get(&session_id).ok_or_else(|| {
                FactoryError::EscrowClaimNotFound {
                    session_id: session_id.clone(),
                }
            })?;
            let mut total_paid = parse_amount(&prior_claim.paid_amount)?;
            let mut claim_cursor = prior_claim.last_scanned_block;

            if let Some((incremental_amount, block_number)) =
                amounts_by_claim.get(&session_snapshot.claim_id)
            {
                total_paid = total_paid.checked_add(*incremental_amount).ok_or_else(|| {
                    FactoryError::InvalidAmount {
                        value: incremental_amount.to_string(),
                    }
                })?;
                claim_cursor = Some(claim_cursor.unwrap_or(0).max(*block_number));
            }

            let payment_status = payment_status_for_amount(
                total_paid,
                parse_amount(&session_snapshot.gross_amount)?,
            );
            let payment_detected = session_snapshot.state == SpawnSessionState::AwaitingPayment
                && payment_status == PaymentStatus::Paid
                && now_ms <= session_snapshot.expires_at;

            {
                let claim = state.escrow_claims.get_mut(&session_id).ok_or_else(|| {
                    FactoryError::EscrowClaimNotFound {
                        session_id: session_id.clone(),
                    }
                })?;
                claim.paid_amount = amount_to_string(total_paid);
                claim.payment_status = payment_status.clone();
                claim.last_scanned_block = Some(scan_to_block.max(claim_cursor.unwrap_or(0)));
                claim.updated_at = now_ms;
            }

            {
                let session = state.sessions.get_mut(&session_id).ok_or_else(|| {
                    FactoryError::SessionNotFound {
                        session_id: session_id.clone(),
                    }
                })?;
                session.payment_status = payment_status.clone();
                session.last_scanned_block = Some(scan_to_block.max(claim_cursor.unwrap_or(0)));
                session.updated_at = now_ms;
            }
            sync_session_derived_flags_in_state(state, &session_id, now_ms)?;

            if payment_detected {
                apply_session_event_in_state(
                    state,
                    &session_id,
                    SessionAuditActor::System,
                    now_ms,
                    SpawnSessionEvent::PaymentObserved,
                    "payment detected from Base logs",
                )?;
                enqueue_spawn_execution_in_state(state, &session_id, now_ms);
            }

            if now_ms > session_snapshot.expires_at {
                let _ = expire_session_in_state(
                    state,
                    &session_id,
                    SessionAuditActor::System,
                    now_ms,
                    "payment scan observed expired session",
                )?;
            }

            updated_claims.push(state.escrow_claims.get(&session_id).cloned().ok_or_else(
                || FactoryError::EscrowClaimNotFound {
                    session_id: session_id.clone(),
                },
            )?);
        }

        sync_payment_poll_job_in_state(state, now_ms);

        Ok(updated_claims)
    });

    if result.is_ok() {
        let expired_ids = read_state(|state| {
            state
                .sessions
                .iter()
                .filter(|(_, session)| session.state == SpawnSessionState::Expired)
                .map(|(session_id, _)| session_id.clone())
                .collect::<Vec<_>>()
        });
        for session_id in expired_ids {
            delete_spawn_provider_secrets(&session_id);
        }
    }

    result
}

pub fn next_payment_scan_plan(latest_block: u64) -> Option<PaymentScanPlan> {
    read_state(|state| {
        let active_sessions: Vec<&SpawnSession> = state
            .sessions
            .values()
            .filter(|session| session_needs_payment_poll(session))
            .collect();

        if active_sessions.is_empty() {
            return None;
        }

        let fallback_from_block = latest_block.saturating_sub(BASE_LOG_WINDOW_LIMIT - 1);
        let from_block = active_sessions
            .iter()
            .filter_map(|session| {
                session
                    .last_scanned_block
                    .map(|block| block.saturating_add(1))
            })
            .min()
            .or_else(|| {
                state
                    .payment_last_scanned_block
                    .map(|block| block.saturating_add(1))
            })
            .unwrap_or(fallback_from_block);
        let from_block = from_block.min(latest_block);
        let to_block = from_block
            .saturating_add(BASE_LOG_WINDOW_LIMIT - 1)
            .min(latest_block);
        let claim_ids = active_sessions
            .iter()
            .map(|session| session.claim_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        Some(PaymentScanPlan {
            claim_ids,
            from_block,
            to_block,
        })
    })
}

#[cfg(target_arch = "wasm32")]
pub async fn poll_escrow_payments(now_ms: u64) -> Result<Vec<EscrowClaim>, FactoryError> {
    let (base_rpc_endpoint, base_rpc_fallback_endpoint, escrow_contract_address) =
        read_state(|state| {
            (
                state.base_rpc_endpoint.clone(),
                state.base_rpc_fallback_endpoint.clone(),
                state.escrow_contract_address.clone(),
            )
        });

    let endpoints = configured_rpc_endpoints(base_rpc_endpoint, base_rpc_fallback_endpoint);
    if endpoints.is_empty() {
        return Ok(Vec::new());
    }
    if escrow_contract_address.is_empty() {
        return Ok(Vec::new());
    }

    let latest_block = crate::base_rpc::eth_block_number(&endpoints).await?;
    let Some(plan) = next_payment_scan_plan(latest_block) else {
        return Ok(Vec::new());
    };
    let logs = crate::base_rpc::eth_get_deposited_logs(&endpoints, &escrow_contract_address, &plan)
        .await?;

    reconcile_escrow_payments(&logs, plan.to_block, now_ms)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn poll_escrow_payments(now_ms: u64) -> Result<Vec<EscrowClaim>, FactoryError> {
    let (base_rpc_endpoint, base_rpc_fallback_endpoint, escrow_contract_address) =
        read_state(|state| {
            (
                state.base_rpc_endpoint.clone(),
                state.base_rpc_fallback_endpoint.clone(),
                state.escrow_contract_address.clone(),
            )
        });

    let endpoints = configured_rpc_endpoints(base_rpc_endpoint, base_rpc_fallback_endpoint);
    if endpoints.is_empty() {
        return Ok(Vec::new());
    }
    if escrow_contract_address.is_empty() {
        return Ok(Vec::new());
    }

    let latest_block = crate::base_rpc::eth_block_number(&endpoints)?;
    let Some(plan) = next_payment_scan_plan(latest_block) else {
        return Ok(Vec::new());
    };
    let logs =
        crate::base_rpc::eth_get_deposited_logs(&endpoints, &escrow_contract_address, &plan)?;

    reconcile_escrow_payments(&logs, plan.to_block, now_ms)
}

fn validate_escrow_refund_in_state(
    state: &FactoryState,
    session_id: &str,
    now_ms: u64,
) -> Result<Option<RefundSpawnResponse>, FactoryError> {
    let session = state
        .sessions
        .get(session_id)
        .ok_or_else(|| FactoryError::SessionNotFound {
            session_id: session_id.to_string(),
        })?;
    let claim =
        state
            .escrow_claims
            .get(session_id)
            .ok_or_else(|| FactoryError::EscrowClaimNotFound {
                session_id: session_id.to_string(),
            })?;

    if session.payment_status == PaymentStatus::Refunded {
        return Ok(Some(RefundSpawnResponse {
            session_id: session_id.to_string(),
            state: session.state.clone(),
            payment_status: PaymentStatus::Refunded,
            refunded_at: claim.refunded_at.unwrap_or(now_ms),
            refund_tx_hash: claim
                .refund_broadcast
                .as_ref()
                .and_then(|record| record.rpc_tx_hash.clone()),
        }));
    }

    if !matches!(
        session.state,
        SpawnSessionState::Expired | SpawnSessionState::Failed
    ) || !session.refundable
        || !claim.refundable
    {
        return Err(FactoryError::SessionNotRefundable {
            session_id: session_id.to_string(),
            state: session.state.clone(),
            payment_status: session.payment_status.clone(),
        });
    }
    Ok(None)
}

fn persist_refund_broadcast_record(
    state: &mut FactoryState,
    session_id: &str,
    record: crate::types::ReleaseBroadcastRecord,
) {
    if let Some(claim) = state.escrow_claims.get_mut(session_id) {
        claim.refund_broadcast = Some(record);
    }
}

pub(crate) fn write_refund_guarded<T>(
    session_id: &str,
    generation: Option<u64>,
    mutation: impl FnOnce(&mut FactoryState) -> Result<T, FactoryError>,
) -> Result<T, FactoryError> {
    write_state(|state| {
        if let Some(generation) = generation {
            if state
                .steward_refund_leases
                .get(session_id)
                .is_none_or(|lease| lease.generation != generation)
            {
                return Err(FactoryError::InvalidStewardProof {
                    reason: "stale refund command continuation".to_string(),
                });
            }
        }
        mutation(state)
    })
}

pub(crate) fn finalize_escrow_refund_in_state(
    state: &mut FactoryState,
    session_id: &str,
    now_ms: u64,
    refund_tx_hash: &str,
) -> Result<RefundSpawnResponse, FactoryError> {
    if let Some(response) = validate_escrow_refund_in_state(state, session_id, now_ms)? {
        return Ok(response);
    }
    let session = state
        .sessions
        .get(session_id)
        .cloned()
        .expect("session validated");

    if let Some(canister_id) = session.automaton_canister_id.as_ref() {
        let runtime = state.runtimes.get_mut(canister_id).ok_or_else(|| {
            FactoryError::AutomatonRuntimeNotFound {
                canister_id: canister_id.clone(),
            }
        })?;
        if runtime.controller_handoff_completed_at.is_none() {
            runtime.controller_handoff_completed_at = Some(now_ms);
        }
        runtime.provider_keys_cleared = true;
    }

    {
        let session = state
            .sessions
            .get_mut(session_id)
            .expect("session existence checked");
        clear_provider_secrets(session, None);
        session.payment_status = PaymentStatus::Refunded;
        session.updated_at = now_ms;
    }

    {
        let claim = state
            .escrow_claims
            .get_mut(session_id)
            .expect("claim existence checked");
        claim.payment_status = PaymentStatus::Refunded;
        claim.refunded_at = Some(now_ms);
        claim.updated_at = now_ms;
    }
    let _ = sync_session_derived_flags_in_state(state, session_id, now_ms)?;

    record_session_audit(
        state,
        session_id,
        Some(session.state.clone()),
        session.state.clone(),
        SessionAuditActor::User,
        now_ms,
        "refund transaction confirmed on-chain",
    );

    Ok(RefundSpawnResponse {
        session_id: session_id.to_string(),
        state: session.state,
        payment_status: PaymentStatus::Refunded,
        refunded_at: now_ms,
        refund_tx_hash: Some(refund_tx_hash.to_string()),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn claim_escrow_refund_inner(
    session_id: &str,
    now_ms: u64,
    generation: Option<u64>,
) -> Result<RefundSpawnResponse, FactoryError> {
    if let Some(response) =
        read_state(|state| validate_escrow_refund_in_state(state, session_id, now_ms))?
    {
        return Ok(response);
    }
    let (existing_record, existing_endpoints) = read_state(|state| {
        let claim = state
            .escrow_claims
            .get(session_id)
            .expect("refund validated");
        (
            claim.refund_broadcast.clone(),
            configured_rpc_endpoints(
                state.base_rpc_endpoint.clone(),
                state.base_rpc_fallback_endpoint.clone(),
            ),
        )
    });
    if let Some(record) = existing_record.as_ref() {
        if record.raw_transaction_hash.is_some() {
            if let Some(generation) = generation {
                ensure_refund_generation(session_id, generation)?;
            }
            let receipt = crate::evm::resume_persisted_refund_transaction(
                &existing_endpoints,
                record.clone(),
                now_ms,
                generation.map(|generation| (session_id, generation)),
            )
            .map_err(|error| *error.source)?;
            let result = write_refund_guarded(session_id, generation, |state| {
                persist_refund_broadcast_record(state, session_id, receipt.record);
                finalize_escrow_refund_in_state(state, session_id, now_ms, &receipt.release_tx_hash)
            });
            if result.is_ok() {
                delete_spawn_provider_secrets(session_id);
            }
            return result;
        }
    }
    let reserved_draft_nonce = existing_record.as_ref().map(|record| record.nonce);
    let (claim_id, recipient, endpoints, escrow, config, nonce) =
        write_refund_guarded(session_id, generation, |state| {
            let session = state
                .sessions
                .get(session_id)
                .expect("refund validated")
                .clone();
            if let Some(record) = existing_record.as_ref() {
                let config = crate::evm::validate_refund_transaction_draft(
                    record,
                    &session.claim_id,
                    &session.steward_address,
                    &state.release_broadcast_config.ecdsa_key_name,
                )?;
                return Ok((
                    record.claim_id.clone(),
                    record.recipient.clone(),
                    configured_rpc_endpoints(
                        state.base_rpc_endpoint.clone(),
                        state.base_rpc_fallback_endpoint.clone(),
                    ),
                    record.escrow_contract_address.clone(),
                    config,
                    record.nonce,
                ));
            }
            let nonce =
                reserved_draft_nonce.unwrap_or_else(|| state.next_release_nonce.unwrap_or(0));
            let draft = crate::evm::build_refund_transaction_draft(
                &session.claim_id,
                &session.steward_address,
                &state.escrow_contract_address,
                nonce,
                &state.release_broadcast_config,
            )
            .expect("validated refund transaction draft");
            if reserved_draft_nonce.is_none() {
                state.next_release_nonce = Some(nonce.saturating_add(1));
            }
            persist_refund_broadcast_record(state, session_id, draft);
            Ok((
                session.claim_id.clone(),
                session.steward_address.clone(),
                configured_rpc_endpoints(
                    state.base_rpc_endpoint.clone(),
                    state.base_rpc_fallback_endpoint.clone(),
                ),
                state.escrow_contract_address.clone(),
                state.release_broadcast_config.clone(),
                nonce,
            ))
        })?;
    let receipt = match crate::evm::broadcast_release_transaction(
        &claim_id,
        &recipient,
        &endpoints,
        &escrow,
        nonce,
        now_ms,
        &config,
        None,
        true,
        generation.map(|generation| (session_id, generation)),
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            write_refund_guarded(session_id, generation, |state| {
                persist_refund_broadcast_record(state, session_id, error.record);
                Ok(())
            })?;
            return Err(*error.source);
        }
    };
    debug_assert!(receipt.confirmed);
    let result = write_refund_guarded(session_id, generation, |state| {
        persist_refund_broadcast_record(state, session_id, receipt.record);
        finalize_escrow_refund_in_state(state, session_id, now_ms, &receipt.release_tx_hash)
    });
    if result.is_ok() {
        delete_spawn_provider_secrets(session_id);
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
pub fn claim_escrow_refund(
    session_id: &str,
    now_ms: u64,
) -> Result<RefundSpawnResponse, FactoryError> {
    claim_escrow_refund_inner(session_id, now_ms, None)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn claim_escrow_refund_authorized(
    session_id: &str,
    now_ms: u64,
    generation: u64,
) -> Result<RefundSpawnResponse, FactoryError> {
    ensure_refund_generation(session_id, generation)?;
    let result = claim_escrow_refund_inner(session_id, now_ms, Some(generation));
    ensure_refund_generation(session_id, generation)?;
    result
}

#[cfg(target_arch = "wasm32")]
async fn claim_escrow_refund_inner(
    session_id: &str,
    now_ms: u64,
    generation: Option<u64>,
) -> Result<RefundSpawnResponse, FactoryError> {
    if let Some(response) =
        read_state(|state| validate_escrow_refund_in_state(state, session_id, now_ms))?
    {
        return Ok(response);
    }
    let (session, claim, endpoints, escrow, config) = read_state(|state| {
        (
            state
                .sessions
                .get(session_id)
                .cloned()
                .expect("refund validated"),
            state
                .escrow_claims
                .get(session_id)
                .cloned()
                .expect("refund claim validated"),
            configured_rpc_endpoints(
                state.base_rpc_endpoint.clone(),
                state.base_rpc_fallback_endpoint.clone(),
            ),
            state.escrow_contract_address.clone(),
            state.release_broadcast_config.clone(),
        )
    });
    if let Some(record) = claim.refund_broadcast.as_ref() {
        if let Some(tx_hash) = record.rpc_tx_hash.as_ref() {
            if let Some(generation) = generation {
                ensure_refund_generation(session_id, generation)?;
            }
            let receipt_result =
                crate::base_rpc::eth_get_transaction_receipt_status(&endpoints, tx_hash).await;
            if let Some(generation) = generation {
                ensure_refund_generation(session_id, generation)?;
            }
            match receipt_result? {
                Some(true) => {
                    let result = write_refund_guarded(session_id, generation, |state| {
                        finalize_escrow_refund_in_state(state, session_id, now_ms, tx_hash)
                    });
                    if result.is_ok() {
                        delete_spawn_provider_secrets(session_id);
                    }
                    return result;
                }
                None => {
                    return Err(FactoryError::ManagementCallFailed {
                        method: "eth_getTransactionReceipt".to_string(),
                        message: "refund transaction is pending or dropped".to_string(),
                    })
                }
                Some(false) => {}
            }
        } else if record.raw_transaction_hash.is_some() {
            if let Some(generation) = generation {
                ensure_refund_generation(session_id, generation)?;
            }
            let receipt = crate::evm::resume_persisted_refund_transaction(
                &endpoints,
                record.clone(),
                now_ms,
                generation.map(|generation| (session_id, generation)),
            )
            .await
            .map_err(|error| *error.source)?;
            write_refund_guarded(session_id, generation, |state| {
                persist_refund_broadcast_record(state, session_id, receipt.record.clone());
                Ok(())
            })?;
            if !receipt.confirmed {
                return Err(FactoryError::ManagementCallFailed {
                    method: "eth_getTransactionReceipt".to_string(),
                    message: "refund transaction remains pending".to_string(),
                });
            }
            let result = write_refund_guarded(session_id, generation, |state| {
                finalize_escrow_refund_in_state(state, session_id, now_ms, &receipt.release_tx_hash)
            });
            if result.is_ok() {
                delete_spawn_provider_secrets(session_id);
            }
            return result;
        }
    }
    let pending_nonce = crate::spawn::pending_release_nonce(&endpoints).await?;
    if let Some(generation) = generation {
        ensure_refund_generation(session_id, generation)?;
    }
    let (claim_id, recipient, escrow, config, nonce) =
        write_refund_guarded(session_id, generation, |state| {
            if let Some(record) = state
                .escrow_claims
                .get(session_id)
                .and_then(|claim| claim.refund_broadcast.as_ref())
            {
                let draft_config = crate::evm::validate_refund_transaction_draft(
                    record,
                    &session.claim_id,
                    &session.steward_address,
                    &config.ecdsa_key_name,
                )?;
                return Ok((
                    record.claim_id.clone(),
                    record.recipient.clone(),
                    record.escrow_contract_address.clone(),
                    draft_config,
                    record.nonce,
                ));
            }
            let nonce = state
                .next_release_nonce
                .unwrap_or(pending_nonce)
                .max(pending_nonce);
            let draft = crate::evm::build_refund_transaction_draft(
                &session.claim_id,
                &session.steward_address,
                &escrow,
                nonce,
                &config,
            )?;
            state.next_release_nonce = Some(nonce.saturating_add(1));
            persist_refund_broadcast_record(state, session_id, draft);
            Ok::<_, FactoryError>((
                session.claim_id.clone(),
                session.steward_address.clone(),
                escrow.clone(),
                config.clone(),
                nonce,
            ))
        })?;
    let receipt = match crate::evm::broadcast_release_transaction(
        &claim_id,
        &recipient,
        &endpoints,
        &escrow,
        nonce,
        now_ms,
        &config,
        None,
        true,
        generation.map(|generation| (session_id, generation)),
    )
    .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            write_refund_guarded(session_id, generation, |state| {
                persist_refund_broadcast_record(state, session_id, error.record);
                Ok(())
            })?;
            return Err(*error.source);
        }
    };
    write_refund_guarded(session_id, generation, |state| {
        persist_refund_broadcast_record(state, session_id, receipt.record.clone());
        Ok(())
    })?;
    if !receipt.confirmed {
        return Err(FactoryError::ManagementCallFailed {
            method: "eth_getTransactionReceipt".to_string(),
            message: "refund transaction remains pending".to_string(),
        });
    }
    let result = write_refund_guarded(session_id, generation, |state| {
        finalize_escrow_refund_in_state(state, session_id, now_ms, &receipt.release_tx_hash)
    });
    if result.is_ok() {
        delete_spawn_provider_secrets(session_id);
    }
    result
}

#[cfg(target_arch = "wasm32")]
pub async fn claim_escrow_refund(
    session_id: &str,
    now_ms: u64,
) -> Result<RefundSpawnResponse, FactoryError> {
    claim_escrow_refund_inner(session_id, now_ms, None).await
}

fn ensure_refund_generation(session_id: &str, generation: u64) -> Result<(), FactoryError> {
    read_state(|state| {
        if state
            .steward_refund_leases
            .get(session_id)
            .is_some_and(|lease| lease.generation == generation)
        {
            Ok(())
        } else {
            Err(FactoryError::InvalidStewardProof {
                reason: "stale refund command continuation".to_string(),
            })
        }
    })
}

#[cfg(target_arch = "wasm32")]
pub async fn claim_escrow_refund_authorized(
    session_id: &str,
    now_ms: u64,
    generation: u64,
) -> Result<RefundSpawnResponse, FactoryError> {
    ensure_refund_generation(session_id, generation)?;
    let result = claim_escrow_refund_inner(session_id, now_ms, Some(generation)).await;
    ensure_refund_generation(session_id, generation)?;
    result
}
