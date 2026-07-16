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
    amount_to_string, parse_amount, EscrowClaim, FactoryError, PaymentEvidenceBlock, PaymentStatus,
    RefundSpawnResponse, SessionAuditActor, SpawnSession, SpawnSessionState,
};

type DepositAmountByBlock = BTreeMap<u64, (u128, String)>;
type DepositAmountsByClaim = BTreeMap<String, DepositAmountByBlock>;

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
            payment_evidence_block_number: None,
            payment_evidence_block_hash: None,
            payment_evidence_increment: None,
            payment_evidence: Vec::new(),
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

fn finalize_payment_status_for_escrow(
    payment_status: PaymentStatus,
    block_hashes_canonical: bool,
) -> PaymentStatus {
    if matches!(payment_status, PaymentStatus::Paid) && !block_hashes_canonical {
        PaymentStatus::Partial
    } else {
        payment_status
    }
}

fn amounts_by_claim(logs: &[BaseDepositLog]) -> Result<DepositAmountsByClaim, FactoryError> {
    let mut amounts_by_claim: DepositAmountsByClaim = BTreeMap::new();
    for log in logs {
        let amount = parse_amount(&log.amount)?;
        let per_block = amounts_by_claim
            .entry(log.claim_id.clone())
            .or_default()
            .entry(log.block_number)
            .or_insert((0, log.block_hash.clone()));
        if !per_block.1.eq_ignore_ascii_case(&log.block_hash) {
            return Err(FactoryError::ManagementCallFailed {
                method: "reconcile_escrow_payments".to_string(),
                message: "deposit log block hash is inconsistent for the same block".to_string(),
            });
        }
        per_block.0 =
            per_block
                .0
                .checked_add(amount)
                .ok_or_else(|| FactoryError::InvalidAmount {
                    value: log.amount.clone(),
                })?;
    }
    Ok(amounts_by_claim)
}

fn merged_claim_evidence_for_session(
    prior_claim: &EscrowClaim,
    blocks: Option<&DepositAmountByBlock>,
) -> Result<(Vec<PaymentEvidenceBlock>, u128, u128, Option<u64>), FactoryError> {
    let mut total_paid = parse_amount(&prior_claim.paid_amount)?;
    let mut claim_cursor = prior_claim.last_scanned_block;
    let mut incremental_payment: u128 = 0;

    let mut evidence = prior_claim.payment_evidence.clone();
    if let Some(blocks) = blocks {
        let mut evidence_by_block = evidence
            .iter()
            .map(|entry| (entry.block_number, entry))
            .collect::<BTreeMap<_, _>>();
        for (block_number, (amount, block_hash)) in blocks {
            if let Some(existing) = evidence_by_block.get(block_number) {
                if !existing.block_hash.eq_ignore_ascii_case(block_hash) {
                    return Err(FactoryError::ManagementCallFailed {
                        method: "reconcile_escrow_payments".to_string(),
                        message: "persisted payment evidence block hash changed for the same block"
                            .to_string(),
                    });
                }
                continue;
            }
            evidence.push(PaymentEvidenceBlock {
                block_number: *block_number,
                block_hash: block_hash.clone(),
                amount: amount_to_string(*amount),
            });
            evidence_by_block = evidence
                .iter()
                .map(|entry| (entry.block_number, entry))
                .collect::<BTreeMap<_, _>>();
            total_paid =
                total_paid
                    .checked_add(*amount)
                    .ok_or_else(|| FactoryError::InvalidAmount {
                        value: amount.to_string(),
                    })?;
            incremental_payment = incremental_payment.checked_add(*amount).ok_or_else(|| {
                FactoryError::InvalidAmount {
                    value: amount.to_string(),
                }
            })?;
            claim_cursor = Some(claim_cursor.unwrap_or(0).max(*block_number));
        }
        evidence.sort_by_key(|entry| entry.block_number);
    }

    Ok((evidence, total_paid, incremental_payment, claim_cursor))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn reconcile_escrow_payments(
    logs: &[BaseDepositLog],
    scan_to_block: u64,
    now_ms: u64,
) -> Result<Vec<EscrowClaim>, FactoryError> {
    let amounts_by_claim = amounts_by_claim(logs)?;

    let result = write_state(|state| {
        state.payment_last_scanned_block = Some(scan_to_block);

        let payment_endpoints = configured_rpc_endpoints(
            state.base_rpc_endpoint.clone(),
            state.base_rpc_fallback_endpoint.clone(),
        );
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

            let (evidence, total_paid, incremental_payment, claim_cursor) =
                merged_claim_evidence_for_session(
                    prior_claim,
                    amounts_by_claim.get(&session_snapshot.claim_id),
                )?;

            let mut payment_status = payment_status_for_amount(
                total_paid,
                parse_amount(&session_snapshot.gross_amount)?,
            );
            let mut block_hashes_canonical = true;
            if matches!(payment_status, PaymentStatus::Paid) && !payment_endpoints.is_empty() {
                for proof in &evidence {
                    let canonical_block_hash = crate::base_rpc::eth_get_block_hash_by_number(
                        &payment_endpoints,
                        proof.block_number,
                    )?;
                    if !canonical_block_hash.eq_ignore_ascii_case(&proof.block_hash) {
                        block_hashes_canonical = false;
                        break;
                    }
                }
            }
            payment_status =
                finalize_payment_status_for_escrow(payment_status, block_hashes_canonical);

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
                claim.payment_evidence = evidence;
                claim.payment_evidence_block_number = claim
                    .payment_evidence
                    .last()
                    .map(|entry| entry.block_number);
                claim.payment_evidence_block_hash = claim
                    .payment_evidence
                    .last()
                    .map(|entry| entry.block_hash.clone());
                claim.payment_evidence_increment = if incremental_payment > 0 {
                    Some(incremental_payment.to_string())
                } else {
                    None
                };
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

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
struct EscrowPaymentScanGuard {
    session_id: String,
    session_state: SpawnSessionState,
    session_payment_status: PaymentStatus,
    session_updated_at: u64,
    session_last_scanned_block: Option<u64>,
    session_expires_at: u64,
    claim_payment_status: PaymentStatus,
    claim_updated_at: u64,
    claim_last_scanned_block: Option<u64>,
    claim_paid_amount: String,
    claim_payment_evidence_block_number: Option<u64>,
    claim_payment_evidence_block_hash: Option<String>,
}

#[cfg(target_arch = "wasm32")]
impl EscrowPaymentScanGuard {
    fn new(session: &SpawnSession, claim: &EscrowClaim) -> Self {
        Self {
            session_id: session.session_id.clone(),
            session_state: session.state.clone(),
            session_payment_status: session.payment_status.clone(),
            session_updated_at: session.updated_at,
            session_last_scanned_block: session.last_scanned_block,
            session_expires_at: session.expires_at,
            claim_payment_status: claim.payment_status.clone(),
            claim_updated_at: claim.updated_at,
            claim_last_scanned_block: claim.last_scanned_block,
            claim_paid_amount: claim.paid_amount.clone(),
            claim_payment_evidence_block_number: claim.payment_evidence_block_number,
            claim_payment_evidence_block_hash: claim.payment_evidence_block_hash.clone(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn stale_payment_scan_guard() -> &'static str {
    "stale payment scan continuation"
}

#[cfg(target_arch = "wasm32")]
fn ensure_payment_scan_guard(
    state: &FactoryState,
    guard: &EscrowPaymentScanGuard,
) -> Result<(), FactoryError> {
    let session =
        state
            .sessions
            .get(&guard.session_id)
            .ok_or_else(|| FactoryError::SessionNotFound {
                session_id: guard.session_id.clone(),
            })?;
    if session.state != guard.session_state
        || session.payment_status != guard.session_payment_status
        || session.updated_at != guard.session_updated_at
        || session.last_scanned_block != guard.session_last_scanned_block
        || session.expires_at != guard.session_expires_at
    {
        return Err(FactoryError::InvalidStewardProof {
            reason: stale_payment_scan_guard().to_string(),
        });
    }

    let claim = state.escrow_claims.get(&guard.session_id).ok_or_else(|| {
        FactoryError::EscrowClaimNotFound {
            session_id: guard.session_id.clone(),
        }
    })?;
    if claim.payment_status != guard.claim_payment_status
        || claim.updated_at != guard.claim_updated_at
        || claim.last_scanned_block != guard.claim_last_scanned_block
        || claim.paid_amount != guard.claim_paid_amount
        || claim.payment_evidence_block_number != guard.claim_payment_evidence_block_number
        || claim.payment_evidence_block_hash != guard.claim_payment_evidence_block_hash
    {
        return Err(FactoryError::InvalidStewardProof {
            reason: stale_payment_scan_guard().to_string(),
        });
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
struct EscrowPaymentScanUpdate {
    session_id: String,
    guard: EscrowPaymentScanGuard,
    payment_status: PaymentStatus,
    payment_evidence: Vec<PaymentEvidenceBlock>,
    payment_evidence_increment: Option<String>,
    claim_cursor: Option<u64>,
    payment_detected: bool,
    total_paid: String,
}

#[cfg(target_arch = "wasm32")]
pub async fn reconcile_escrow_payments(
    logs: &[BaseDepositLog],
    scan_to_block: u64,
    now_ms: u64,
) -> Result<Vec<EscrowClaim>, FactoryError> {
    let amounts_by_claim = amounts_by_claim(logs)?;

    let (payment_endpoints, session_state): (Vec<String>, Vec<(SpawnSession, EscrowClaim)>) =
        read_state(|state| {
            (
                configured_rpc_endpoints(
                    state.base_rpc_endpoint.clone(),
                    state.base_rpc_fallback_endpoint.clone(),
                ),
                state
                    .sessions
                    .iter()
                    .filter(|(_, session)| session_needs_payment_poll(session))
                    .filter_map(|(session_id, session)| {
                        state
                            .escrow_claims
                            .get(session_id)
                            .map(|claim| (session.clone(), claim.clone()))
                    })
                    .collect(),
            )
        });

    let mut planned_updates = Vec::new();
    for (session_snapshot, prior_claim) in session_state {
        let (evidence, total_paid, incremental_payment, claim_cursor) =
            merged_claim_evidence_for_session(
                &prior_claim,
                amounts_by_claim.get(&session_snapshot.claim_id),
            )?;

        let mut payment_status =
            payment_status_for_amount(total_paid, parse_amount(&session_snapshot.gross_amount)?);
        let mut block_hashes_canonical = true;
        let guard = EscrowPaymentScanGuard::new(&session_snapshot, &prior_claim);
        ensure_payment_scan_guard(&read_state(|state| state.clone()), &guard)?;
        if matches!(payment_status, PaymentStatus::Paid) && !payment_endpoints.is_empty() {
            for proof in &evidence {
                let canonical_block_hash = crate::base_rpc::eth_get_block_hash_by_number(
                    &payment_endpoints,
                    proof.block_number,
                )
                .await?;
                ensure_payment_scan_guard(&read_state(|state| state.clone()), &guard)?;
                if !canonical_block_hash.eq_ignore_ascii_case(&proof.block_hash) {
                    block_hashes_canonical = false;
                    break;
                }
            }
        }
        payment_status = finalize_payment_status_for_escrow(payment_status, block_hashes_canonical);

        let payment_detected = session_snapshot.state == SpawnSessionState::AwaitingPayment
            && payment_status == PaymentStatus::Paid
            && now_ms <= session_snapshot.expires_at;

        planned_updates.push(EscrowPaymentScanUpdate {
            session_id: session_snapshot.session_id.clone(),
            guard,
            payment_status,
            payment_evidence: evidence,
            payment_evidence_increment: if incremental_payment > 0 {
                Some(incremental_payment.to_string())
            } else {
                None
            },
            claim_cursor,
            payment_detected,
            total_paid: amount_to_string(total_paid),
        });
    }

    let result = write_state(|state| {
        state.payment_last_scanned_block = Some(scan_to_block);
        let mut updated_claims = Vec::new();

        for update in planned_updates {
            ensure_payment_scan_guard(state, &update.guard)?;

            {
                let claim = state
                    .escrow_claims
                    .get_mut(&update.session_id)
                    .ok_or_else(|| FactoryError::EscrowClaimNotFound {
                        session_id: update.session_id.clone(),
                    })?;
                claim.paid_amount = update.total_paid.clone();
                claim.payment_status = update.payment_status.clone();
                claim.payment_evidence = update.payment_evidence;
                claim.payment_evidence_block_number = claim
                    .payment_evidence
                    .last()
                    .map(|entry| entry.block_number);
                claim.payment_evidence_block_hash = claim
                    .payment_evidence
                    .last()
                    .map(|entry| entry.block_hash.clone());
                claim.payment_evidence_increment = update.payment_evidence_increment;
                claim.last_scanned_block =
                    Some(scan_to_block.max(update.claim_cursor.unwrap_or(0)));
                claim.updated_at = now_ms;
            }

            {
                let session = state.sessions.get_mut(&update.session_id).ok_or_else(|| {
                    FactoryError::SessionNotFound {
                        session_id: update.session_id.clone(),
                    }
                })?;
                session.payment_status = update.payment_status.clone();
                session.last_scanned_block =
                    Some(scan_to_block.max(update.claim_cursor.unwrap_or(0)));
                session.updated_at = now_ms;
            }
            sync_session_derived_flags_in_state(state, &update.session_id, now_ms)?;

            let session = state
                .sessions
                .get(&update.session_id)
                .ok_or_else(|| FactoryError::SessionNotFound {
                    session_id: update.session_id.clone(),
                })?
                .clone();
            if update.payment_detected {
                apply_session_event_in_state(
                    state,
                    &update.session_id,
                    SessionAuditActor::System,
                    now_ms,
                    SpawnSessionEvent::PaymentObserved,
                    "payment detected from Base logs",
                )?;
                enqueue_spawn_execution_in_state(state, &update.session_id, now_ms);
            }

            if now_ms > session.expires_at {
                let _ = expire_session_in_state(
                    state,
                    &update.session_id,
                    SessionAuditActor::System,
                    now_ms,
                    "payment scan observed expired session",
                )?;
            }

            updated_claims.push(
                state
                    .escrow_claims
                    .get(&update.session_id)
                    .cloned()
                    .ok_or_else(|| FactoryError::EscrowClaimNotFound {
                        session_id: update.session_id.clone(),
                    })?,
            );
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

        if state.evm_confirmation_depth == 0 {
            return None;
        }

        let confirmations_to_wait = state.evm_confirmation_depth.saturating_sub(1);
        if latest_block < confirmations_to_wait {
            return None;
        }

        let safe_head_block = latest_block - confirmations_to_wait;

        let fallback_from_block = safe_head_block.saturating_sub(BASE_LOG_WINDOW_LIMIT - 1);
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
        let from_block = from_block.min(safe_head_block);
        let to_block = from_block
            .saturating_add(BASE_LOG_WINDOW_LIMIT - 1)
            .min(safe_head_block);
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

    reconcile_escrow_payments(&logs, plan.to_block, now_ms).await
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
    let (existing_record, existing_endpoints, confirmation_depth) = read_state(|state| {
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
            state.evm_confirmation_depth,
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
                confirmation_depth,
                generation.map(|generation| (session_id, generation)),
            )
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
        confirmation_depth,
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
    write_refund_guarded(session_id, generation, |state| {
        persist_refund_broadcast_record(state, session_id, receipt.record);
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
    let (session, claim, endpoints, escrow, config, confirmation_depth) = read_state(|state| {
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
            state.evm_confirmation_depth,
        )
    });
    let continuation_guard = generation
        .as_ref()
        .map(|generation| crate::evm::ReleaseTransactionGuard::refund(session_id, *generation));
    if let Some(record) = claim.refund_broadcast.as_ref() {
        if let Some(tx_hash) = record.rpc_tx_hash.as_ref() {
            if let Some(generation) = generation {
                ensure_refund_generation(session_id, generation)?;
            }
            let mut receipt_record = record.clone();
            let confirmed = crate::evm::confirm_release_receipt_depth(
                &endpoints,
                confirmation_depth,
                &mut receipt_record,
                tx_hash,
                continuation_guard.as_ref(),
            )
            .await?;
            if let Some(generation) = generation {
                ensure_refund_generation(session_id, generation)?;
            }
            write_refund_guarded(session_id, generation, |state| {
                persist_refund_broadcast_record(state, session_id, receipt_record.clone());
                Ok(())
            })?;
            if !confirmed {
                return Err(FactoryError::ManagementCallFailed {
                    method: "eth_getTransactionReceipt".to_string(),
                    message: "refund transaction remains pending".to_string(),
                });
            }
            let result = write_refund_guarded(session_id, generation, |state| {
                finalize_escrow_refund_in_state(state, session_id, now_ms, tx_hash)
            });
            if result.is_ok() {
                delete_spawn_provider_secrets(session_id);
            }
            return result;
        } else if record.raw_transaction_hash.is_some() {
            if let Some(generation) = generation {
                ensure_refund_generation(session_id, generation)?;
            }
            let receipt = crate::evm::resume_persisted_refund_transaction(
                &endpoints,
                record.clone(),
                now_ms,
                confirmation_depth,
                continuation_guard.clone(),
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
        confirmation_depth,
        continuation_guard.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::public::create_spawn_session,
        state::restore_state,
        types::{
            AutomatonChildRuntimeConfig, CreateSpawnSessionRequest, InferenceTransport,
            OpenRouterReasoningLevel, PaymentEvidenceBlock, ProviderConfig, SpawnAsset,
            SpawnConfig, SpawnProviderSecrets, SpawnSessionState,
        },
    };
    use candid::Principal;
    use std::collections::BTreeMap;

    const CANONICAL_BLOCK_HASH: &str =
        "0x0000000000000000000000000000000000000000000000000000000000000059";
    const REORGANIZED_BLOCK_HASH: &str =
        "0x000000000000000000000000000000000000000000000000000000000000002a";

    fn configure_child_runtime() {
        write_state(|state| {
            state.child_runtime = AutomatonChildRuntimeConfig {
                ecdsa_key_name: Some("key_1".to_string()),
                inbox_contract_address: Some("0xInbox".to_string()),
                evm_chain_id: Some(8_453),
                evm_rpc_url: Some("http://127.0.0.1:18545".to_string()),
                evm_confirmation_depth: Some(12),
                evm_bootstrap_lookback_blocks: Some(256),
                http_allowed_domains: Some(vec![
                    "https://openrouter.ai".to_string(),
                    "https://api.search.brave.com".to_string(),
                ]),
                llm_canister_id: Some(Principal::from_text("aaaaa-aa").expect("valid principal")),
                search_api_key: Some("brave-key".to_string()),
                inference_proxy_worker_base_url: Some("https://proxy.example.com".to_string()),
                inference_proxy_trusted_callback_principal: Some("aaaaa-aa".to_string()),
                cycle_topup_enabled: Some(true),
                auto_topup_cycle_threshold: Some(123_456),
            };
        });
    }

    fn sample_request(gross_amount: &str) -> CreateSpawnSessionRequest {
        CreateSpawnSessionRequest {
            name: Some("Meridian".to_string()),
            constitution: Some("I am Meridian. ".repeat(30)),
            steward_address: "0xsteward".to_string(),
            asset: SpawnAsset::Usdc,
            gross_amount: gross_amount.to_string(),
            config: SpawnConfig {
                chain: crate::types::SpawnChain::Base,
                risk: 7,
                strategies: vec!["base-aave-usdc-reserve-01".to_string()],
                skills: vec!["search".to_string()],
                provider: ProviderConfig {
                    model: Some("openrouter/auto".to_string()),
                    inference_transport: InferenceTransport::OpenrouterDirect,
                    open_router_reasoning_level: OpenRouterReasoningLevel::Default,
                },
            },
            provider_secrets: SpawnProviderSecrets {
                open_router_api_key: Some("or-key".to_string()),
                brave_search_api_key: Some("brave-key".to_string()),
            },
            parent_id: None,
        }
    }

    fn deposit_log(
        claim_id: &str,
        amount: &str,
        block_number: u64,
        block_hash: &str,
    ) -> BaseDepositLog {
        BaseDepositLog {
            claim_id: claim_id.to_string(),
            amount: amount.to_string(),
            block_number,
            block_hash: block_hash.to_string(),
        }
    }

    fn reset_escrow_factory_state() {
        restore_state(Default::default());
        configure_child_runtime();
        write_state(|state| {
            state.base_rpc_endpoint = Some("mock://success".to_string());
            state.escrow_contract_address =
                "0x3333333333333333333333333333333333333333".to_string();
        });
    }

    fn create_refundable_failed_session(now_ms: u64, payment_status: PaymentStatus) -> String {
        let response = create_spawn_session(sample_request("75000000"), now_ms)
            .expect("session should be created");
        write_state(|state| {
            if let Some(session) = state.sessions.get_mut(&response.session.session_id) {
                session.state = SpawnSessionState::Failed;
                session.payment_status = payment_status.clone();
                session.refundable = true;
                session.updated_at = now_ms + 10;
            }
            if let Some(claim) = state.escrow_claims.get_mut(&response.session.session_id) {
                claim.payment_status = payment_status.clone();
                claim.paid_amount = "75000000".to_string();
                claim.refundable = true;
                claim.refunded_at = None;
                claim.updated_at = now_ms + 10;
            }
        });
        response.session.session_id
    }

    #[test]
    fn escrow_evidence_merge_rejects_conflicting_block_hash() {
        reset_escrow_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 1_000)
            .expect("session should be created");
        write_state(|state| {
            let claim = state
                .escrow_claims
                .get_mut(&response.session.session_id)
                .expect("claim should exist");
            claim.paid_amount = "10000000".to_string();
            claim.payment_evidence = vec![PaymentEvidenceBlock {
                block_number: 1_000,
                block_hash: CANONICAL_BLOCK_HASH.to_string(),
                amount: "10000000".to_string(),
            }];
            claim.payment_evidence_block_number = Some(1_000);
            claim.payment_evidence_block_hash = Some(CANONICAL_BLOCK_HASH.to_string());
            claim.last_scanned_block = Some(1_000);
            claim.updated_at = 1;
        });

        let existing_claim = get_escrow_claim(&response.session.session_id)
            .expect("claim should be retrievable before reconciliation");

        let mut blocks = BTreeMap::new();
        blocks.insert(1_050, (30_000_000u128, CANONICAL_BLOCK_HASH.to_string()));
        let (merged, paid, incremental, cursor) =
            merged_claim_evidence_for_session(&existing_claim, Some(&blocks))
                .expect("merge should succeed for matching historical hashes");

        assert_eq!(merged.len(), 2);
        assert_eq!(paid, 40_000_000);
        assert_eq!(incremental, 30_000_000);
        assert_eq!(cursor, Some(1_050));

        let mut conflicting_blocks = BTreeMap::new();
        conflicting_blocks.insert(1_000, (10_000u128, REORGANIZED_BLOCK_HASH.to_string()));
        assert!(
            merged_claim_evidence_for_session(&existing_claim, Some(&conflicting_blocks)).is_err()
        );
    }

    #[test]
    fn escrow_finalize_payment_status_preserves_paid_only_with_canonical_evidence() {
        assert_eq!(
            finalize_payment_status_for_escrow(PaymentStatus::Paid, true),
            PaymentStatus::Paid
        );
        assert_eq!(
            finalize_payment_status_for_escrow(PaymentStatus::Paid, false),
            PaymentStatus::Partial
        );
        assert_eq!(
            finalize_payment_status_for_escrow(PaymentStatus::Partial, false),
            PaymentStatus::Partial
        );
    }

    #[test]
    fn escrow_reconciliation_multiblock_evidence_canonicalized_before_paid() {
        reset_escrow_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 1_000)
            .expect("session should be created");

        let _ = reconcile_escrow_payments(
            &[
                deposit_log(
                    &response.session.claim_id,
                    "30000000",
                    1_000,
                    CANONICAL_BLOCK_HASH,
                ),
                deposit_log(
                    &response.session.claim_id,
                    "45000000",
                    1_005,
                    CANONICAL_BLOCK_HASH,
                ),
            ],
            1_005,
            1_100,
        )
        .expect("escrow scan should persist all block evidence and transition paid");

        let session = read_state(|state| {
            state
                .sessions
                .get(&response.session.session_id)
                .expect("session should exist")
                .clone()
        });
        let claim = get_escrow_claim(&response.session.session_id)
            .expect("claim should exist after reconciliation");

        assert_eq!(session.state, SpawnSessionState::PaymentDetected);
        assert_eq!(session.payment_status, PaymentStatus::Paid);
        assert_eq!(claim.payment_evidence.len(), 2);
        assert_eq!(claim.payment_evidence[0].block_number, 1_000);
        assert_eq!(claim.payment_evidence[1].block_number, 1_005);
        assert_eq!(claim.payment_evidence[0].amount, "30000000");
        assert_eq!(claim.payment_evidence[1].amount, "45000000");
    }

    #[test]
    fn escrow_reconciliation_multiblock_reorg_keeps_payment_pending() {
        reset_escrow_factory_state();

        let response = create_spawn_session(sample_request("75000000"), 2_000)
            .expect("session should be created");

        let _ = reconcile_escrow_payments(
            &[
                deposit_log(
                    &response.session.claim_id,
                    "40000000",
                    2_010,
                    CANONICAL_BLOCK_HASH,
                ),
                deposit_log(
                    &response.session.claim_id,
                    "35000000",
                    2_011,
                    REORGANIZED_BLOCK_HASH,
                ),
            ],
            2_011,
            2_100,
        )
        .expect("non-canonical evidence should remain pending");

        let mut session = read_state(|state| {
            state
                .sessions
                .get(&response.session.session_id)
                .expect("session should exist")
                .clone()
        });
        let mut claim = get_escrow_claim(&response.session.session_id)
            .expect("claim should exist after reconciliation");

        assert_eq!(session.state, SpawnSessionState::AwaitingPayment);
        assert_eq!(session.payment_status, PaymentStatus::Partial);
        assert_eq!(claim.payment_status, PaymentStatus::Partial);
        assert_eq!(claim.payment_evidence.len(), 2);

        let _ = reconcile_escrow_payments(&[], 2_050, 2_200)
            .expect("empty scan should preserve pending when reorg remains unresolved");
        session = read_state(|state| {
            state
                .sessions
                .get(&response.session.session_id)
                .expect("session should exist")
                .clone()
        });
        claim = get_escrow_claim(&response.session.session_id)
            .expect("claim should continue to exist after empty scan");

        assert_eq!(session.payment_status, PaymentStatus::Partial);
        assert_eq!(claim.payment_status, PaymentStatus::Partial);
    }

    #[test]
    fn confirmation_next_payment_scan_plan_latest_block_behind_depth_returns_none() {
        reset_escrow_factory_state();
        create_spawn_session(sample_request("60000000"), 7_000).expect("session should be created");

        write_state(|state| {
            state.evm_confirmation_depth = 12;
        });

        assert!(next_payment_scan_plan(10).is_none());
        assert!(next_payment_scan_plan(11).is_some());
    }

    #[test]
    fn confirmation_next_payment_scan_plan_max_depth_returns_none() {
        reset_escrow_factory_state();
        create_spawn_session(sample_request("60000000"), 7_000).expect("session should be created");

        write_state(|state| {
            state.evm_confirmation_depth = u64::MAX;
        });

        assert!(next_payment_scan_plan(10_000).is_none());
    }

    #[test]
    fn claim_escrow_refund_new_broadcast_pending_below_depth_stays_retryable() {
        reset_escrow_factory_state();
        write_state(|state| {
            state.evm_confirmation_depth = 99;
        });
        let session_id = create_refundable_failed_session(1_000, PaymentStatus::Paid);

        let error = claim_escrow_refund(&session_id, 1_001)
            .expect_err("refund with insufficient depth should remain pending");
        assert!(matches!(
            error,
            FactoryError::ManagementCallFailed {
                ref method,
                ref message,
                ..
            } if method == "eth_getTransactionReceipt" && message == "refund transaction remains pending"
        ));

        let session = read_state(|state| {
            state
                .sessions
                .get(&session_id)
                .cloned()
                .expect("session should exist")
        });
        let claim = get_escrow_claim(&session_id).expect("claim should exist");
        assert_eq!(session.payment_status, PaymentStatus::Paid);
        assert_eq!(session.state, SpawnSessionState::Failed);
        assert!(!matches!(session.payment_status, PaymentStatus::Refunded));
        let record = claim
            .refund_broadcast
            .expect("refund intent should persist");
        assert!(record.raw_transaction_hash.is_some());
        assert!(record.rpc_tx_hash.is_some());
        assert_eq!(record.receipt_status, Some(true));
    }

    #[test]
    fn claim_escrow_refund_resumed_broadcast_stays_pending_below_confirmation_depth() {
        reset_escrow_factory_state();
        write_state(|state| {
            state.evm_confirmation_depth = 99;
        });
        let session_id = create_refundable_failed_session(2_000, PaymentStatus::Paid);

        assert!(claim_escrow_refund(&session_id, 2_001).is_err());

        let first_record = get_escrow_claim(&session_id)
            .expect("claim should exist")
            .refund_broadcast
            .expect("refund intent should persist");

        let error = claim_escrow_refund(&session_id, 2_002)
            .expect_err("resumed refund receipt check below depth should remain pending");
        assert!(matches!(
            error,
            FactoryError::ManagementCallFailed {
                ref method,
                ref message,
                ..
            } if method == "eth_getTransactionReceipt" && message == "refund transaction remains pending"
        ));

        let second_record = get_escrow_claim(&session_id)
            .expect("claim should persist")
            .refund_broadcast
            .expect("refund intent should persist after retry");

        assert_eq!(second_record.nonce, first_record.nonce);
        assert_eq!(second_record.rpc_tx_hash, first_record.rpc_tx_hash);
        assert_eq!(
            second_record.raw_transaction_hash,
            first_record.raw_transaction_hash
        );
        assert_eq!(
            read_state(|state| {
                let session = state.sessions.get(&session_id).expect("session exists");
                session.payment_status.clone()
            }),
            PaymentStatus::Paid
        );
    }

    #[test]
    fn claim_escrow_refund_finalizes_at_exact_depth_and_is_idempotent() {
        reset_escrow_factory_state();
        write_state(|state| {
            state.evm_confirmation_depth = 12;
        });
        let session_id = create_refundable_failed_session(3_000, PaymentStatus::Paid);

        let first =
            claim_escrow_refund(&session_id, 3_001).expect("refund should confirm at exact depth");
        assert_eq!(first.payment_status, PaymentStatus::Refunded);
        assert!(first.refund_tx_hash.is_some());

        let first_hash = first.refund_tx_hash.clone();
        let first_refunded_at = first.refunded_at;

        let second =
            claim_escrow_refund(&session_id, 3_002).expect("refunded session should be idempotent");
        assert_eq!(second.payment_status, PaymentStatus::Refunded);
        assert_eq!(second.refund_tx_hash, first_hash);
        assert_eq!(second.refunded_at, first_refunded_at);
    }
}
