use crate::state::{record_session_audit, FactoryState};
use crate::types::{
    FactoryError, PaymentStatus, SessionAuditActor, SpawnSession, SpawnSessionState,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SpawnSessionEvent {
    SessionCreated,
    PaymentObserved,
    SpawnStarted,
    InstallSucceeded,
    ReleaseBroadcast,
    SpawnFailed,
    RetryRequested,
    SessionExpired,
}

impl SpawnSessionEvent {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SessionCreated => "session_created",
            Self::PaymentObserved => "payment_observed",
            Self::SpawnStarted => "spawn_started",
            Self::InstallSucceeded => "install_succeeded",
            Self::ReleaseBroadcast => "release_broadcast",
            Self::SpawnFailed => "spawn_failed",
            Self::RetryRequested => "retry_requested",
            Self::SessionExpired => "session_expired",
        }
    }
}

fn is_retryable_payment_status(payment_status: &PaymentStatus) -> bool {
    *payment_status == PaymentStatus::Paid
}

fn is_refundable_payment_status(payment_status: &PaymentStatus) -> bool {
    matches!(payment_status, PaymentStatus::Partial | PaymentStatus::Paid)
}

fn derive_retryable(session: &SpawnSession, now_ms: u64) -> bool {
    session.state == SpawnSessionState::Failed
        && is_retryable_payment_status(&session.payment_status)
        && now_ms <= session.expires_at
}

fn derive_refundable(session: &SpawnSession) -> bool {
    session.state == SpawnSessionState::Expired
        && is_refundable_payment_status(&session.payment_status)
}

fn transition_error(
    session_id: &str,
    from_state: &SpawnSessionState,
    event: &SpawnSessionEvent,
) -> FactoryError {
    FactoryError::IllegalSessionTransition {
        session_id: session_id.to_string(),
        from_state: from_state.clone(),
        event: event.as_str().to_string(),
    }
}

pub(crate) fn sync_session_derived_flags_in_state(
    state: &mut FactoryState,
    session_id: &str,
    now_ms: u64,
) -> Result<SpawnSession, FactoryError> {
    let refundable =
        {
            let session = state.sessions.get_mut(session_id).ok_or_else(|| {
                FactoryError::SessionNotFound {
                    session_id: session_id.to_string(),
                }
            })?;
            session.retryable = derive_retryable(session, now_ms);
            session.refundable = derive_refundable(session);
            session.refundable
        };

    if let Some(claim) = state.escrow_claims.get_mut(session_id) {
        claim.refundable = refundable;
        claim.updated_at = now_ms;
    }

    Ok(state
        .sessions
        .get(session_id)
        .cloned()
        .expect("session should remain stored"))
}

pub(crate) fn apply_session_event_in_state(
    state: &mut FactoryState,
    session_id: &str,
    actor: SessionAuditActor,
    now_ms: u64,
    event: SpawnSessionEvent,
    reason: &str,
) -> Result<SpawnSession, FactoryError> {
    let snapshot =
        state
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| FactoryError::SessionNotFound {
                session_id: session_id.to_string(),
            })?;
    let from_state = snapshot.state.clone();

    let (audit_from_state, to_state, reset_expiry) = match event {
        SpawnSessionEvent::SessionCreated => {
            if snapshot.state != SpawnSessionState::AwaitingPayment
                || state
                    .audit_log
                    .get(session_id)
                    .map(|entries| !entries.is_empty())
                    .unwrap_or(false)
            {
                return Err(transition_error(session_id, &from_state, &event));
            }
            (None, SpawnSessionState::AwaitingPayment, false)
        }
        SpawnSessionEvent::PaymentObserved => {
            if snapshot.state != SpawnSessionState::AwaitingPayment {
                return Err(transition_error(session_id, &from_state, &event));
            }
            if snapshot.payment_status != PaymentStatus::Paid {
                return Err(FactoryError::PaymentNotSettled {
                    session_id: session_id.to_string(),
                    status: snapshot.payment_status,
                });
            }
            (
                Some(from_state.clone()),
                SpawnSessionState::PaymentDetected,
                false,
            )
        }
        SpawnSessionEvent::SpawnStarted => {
            if snapshot.state != SpawnSessionState::PaymentDetected {
                return Err(transition_error(session_id, &from_state, &event));
            }
            (Some(from_state.clone()), SpawnSessionState::Spawning, false)
        }
        SpawnSessionEvent::InstallSucceeded => {
            if snapshot.state != SpawnSessionState::Spawning {
                return Err(transition_error(session_id, &from_state, &event));
            }
            (
                Some(from_state.clone()),
                SpawnSessionState::BroadcastingRelease,
                false,
            )
        }
        SpawnSessionEvent::ReleaseBroadcast => {
            if snapshot.state != SpawnSessionState::BroadcastingRelease {
                return Err(transition_error(session_id, &from_state, &event));
            }
            (Some(from_state.clone()), SpawnSessionState::Complete, false)
        }
        SpawnSessionEvent::SpawnFailed => {
            if !matches!(
                snapshot.state,
                SpawnSessionState::PaymentDetected
                    | SpawnSessionState::Spawning
                    | SpawnSessionState::BroadcastingRelease
            ) {
                return Err(transition_error(session_id, &from_state, &event));
            }
            (Some(from_state.clone()), SpawnSessionState::Failed, false)
        }
        SpawnSessionEvent::RetryRequested => {
            if snapshot.state != SpawnSessionState::Failed || !snapshot.retryable {
                return Err(FactoryError::SessionNotRetryable {
                    session_id: session_id.to_string(),
                    state: snapshot.state,
                });
            }
            if snapshot.payment_status != PaymentStatus::Paid {
                return Err(FactoryError::PaymentNotSettled {
                    session_id: session_id.to_string(),
                    status: snapshot.payment_status,
                });
            }
            (
                Some(from_state.clone()),
                SpawnSessionState::PaymentDetected,
                true,
            )
        }
        SpawnSessionEvent::SessionExpired => {
            if matches!(
                snapshot.state,
                SpawnSessionState::Complete | SpawnSessionState::Expired
            ) || snapshot.payment_status == PaymentStatus::Refunded
                || now_ms <= snapshot.expires_at
            {
                return Ok(snapshot);
            }
            (Some(from_state.clone()), SpawnSessionState::Expired, false)
        }
    };

    let session_ttl_ms = state.session_ttl_ms;
    {
        let session = state
            .sessions
            .get_mut(session_id)
            .expect("session should exist");
        session.state = to_state.clone();
        if reset_expiry {
            session.expires_at = now_ms + session_ttl_ms;
        }
        session.updated_at = now_ms;
    }

    let session = sync_session_derived_flags_in_state(state, session_id, now_ms)?;
    record_session_audit(
        state,
        session_id,
        audit_from_state,
        to_state,
        actor,
        now_ms,
        reason,
    );

    Ok(session)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::state::FactoryState;
    use crate::types::{
        FactoryError, InferenceTransport, OpenRouterReasoningLevel, PaymentStatus, ProviderConfig,
        SessionAuditActor, SpawnAsset, SpawnChain, SpawnConfig, SpawnSession, SpawnSessionState,
    };

    use super::{
        apply_session_event_in_state, sync_session_derived_flags_in_state, SpawnSessionEvent,
    };

    fn sample_session(state: SpawnSessionState) -> SpawnSession {
        SpawnSession {
            session_id: "session-1".to_string(),
            claim_id: "claim-1".to_string(),
            steward_address: "0xsteward".to_string(),
            chain: SpawnChain::Base,
            asset: SpawnAsset::Usdc,
            gross_amount: "60000000".to_string(),
            platform_fee: "5000000".to_string(),
            creation_cost: "45000000".to_string(),
            net_forward_amount: "10000000".to_string(),
            quote_terms_hash: "quote-hash".to_string(),
            expires_at: 10_000,
            state,
            retryable: false,
            refundable: false,
            payment_status: PaymentStatus::Paid,
            last_scanned_block: None,
            automaton_canister_id: None,
            automaton_evm_address: None,
            release_tx_hash: None,
            release_broadcast_at: None,
            release_broadcast: None,
            parent_id: None,
            child_ids: Vec::new(),
            selected_strategies: Vec::new(),
            config: SpawnConfig {
                chain: SpawnChain::Base,
                risk: 7,
                strategies: vec!["base-aave-usdc-reserve-01".to_string()],
                skills: vec!["search".to_string()],
                provider: ProviderConfig {
                    model: Some("openrouter/auto".to_string()),
                    inference_transport: InferenceTransport::OpenrouterDirect,
                    open_router_reasoning_level: OpenRouterReasoningLevel::Default,
                },
            },
            created_at: 1,
            updated_at: 1,
        }
    }

    fn state_with_session(session: SpawnSession) -> FactoryState {
        FactoryState {
            sessions: BTreeMap::from([(session.session_id.clone(), session)]),
            ..FactoryState::default()
        }
    }

    #[test]
    fn allows_all_legal_transition_edges() {
        let cases = [
            (
                SpawnSessionState::AwaitingPayment,
                PaymentStatus::Unpaid,
                5_000,
                SpawnSessionEvent::SessionCreated,
                SpawnSessionState::AwaitingPayment,
            ),
            (
                SpawnSessionState::AwaitingPayment,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::PaymentObserved,
                SpawnSessionState::PaymentDetected,
            ),
            (
                SpawnSessionState::PaymentDetected,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::SpawnStarted,
                SpawnSessionState::Spawning,
            ),
            (
                SpawnSessionState::Spawning,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::InstallSucceeded,
                SpawnSessionState::BroadcastingRelease,
            ),
            (
                SpawnSessionState::BroadcastingRelease,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::ReleaseBroadcast,
                SpawnSessionState::Complete,
            ),
            (
                SpawnSessionState::PaymentDetected,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::SpawnFailed,
                SpawnSessionState::Failed,
            ),
            (
                SpawnSessionState::Spawning,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::SpawnFailed,
                SpawnSessionState::Failed,
            ),
            (
                SpawnSessionState::BroadcastingRelease,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::SpawnFailed,
                SpawnSessionState::Failed,
            ),
            (
                SpawnSessionState::Failed,
                PaymentStatus::Paid,
                5_000,
                SpawnSessionEvent::RetryRequested,
                SpawnSessionState::PaymentDetected,
            ),
            (
                SpawnSessionState::AwaitingPayment,
                PaymentStatus::Unpaid,
                11_000,
                SpawnSessionEvent::SessionExpired,
                SpawnSessionState::Expired,
            ),
            (
                SpawnSessionState::PaymentDetected,
                PaymentStatus::Paid,
                11_000,
                SpawnSessionEvent::SessionExpired,
                SpawnSessionState::Expired,
            ),
            (
                SpawnSessionState::Failed,
                PaymentStatus::Paid,
                11_000,
                SpawnSessionEvent::SessionExpired,
                SpawnSessionState::Expired,
            ),
            (
                SpawnSessionState::Spawning,
                PaymentStatus::Paid,
                11_000,
                SpawnSessionEvent::SessionExpired,
                SpawnSessionState::Expired,
            ),
            (
                SpawnSessionState::BroadcastingRelease,
                PaymentStatus::Paid,
                11_000,
                SpawnSessionEvent::SessionExpired,
                SpawnSessionState::Expired,
            ),
        ];

        for (initial_state, payment_status, now_ms, event, expected_state) in cases {
            let mut session = sample_session(initial_state.clone());
            session.payment_status = payment_status;
            if initial_state == SpawnSessionState::Failed {
                session.retryable = true;
            }
            let mut state = state_with_session(session);

            let updated = apply_session_event_in_state(
                &mut state,
                "session-1",
                SessionAuditActor::System,
                now_ms,
                event,
                "transition",
            )
            .expect("transition should succeed");

            assert_eq!(updated.state, expected_state);
        }
    }

    #[test]
    fn rejects_illegal_transitions() {
        let cases = [
            (
                SpawnSessionState::AwaitingPayment,
                SpawnSessionEvent::SpawnStarted,
            ),
            (
                SpawnSessionState::AwaitingPayment,
                SpawnSessionEvent::InstallSucceeded,
            ),
            (
                SpawnSessionState::AwaitingPayment,
                SpawnSessionEvent::ReleaseBroadcast,
            ),
            (
                SpawnSessionState::PaymentDetected,
                SpawnSessionEvent::ReleaseBroadcast,
            ),
            (
                SpawnSessionState::Spawning,
                SpawnSessionEvent::RetryRequested,
            ),
            (SpawnSessionState::Complete, SpawnSessionEvent::SpawnFailed),
            (
                SpawnSessionState::Expired,
                SpawnSessionEvent::RetryRequested,
            ),
        ];

        for (initial_state, event) in cases {
            let mut state = state_with_session(sample_session(initial_state));
            let error = apply_session_event_in_state(
                &mut state,
                "session-1",
                SessionAuditActor::System,
                5_000,
                event,
                "transition",
            )
            .expect_err("transition should be rejected");

            assert!(matches!(
                error,
                FactoryError::IllegalSessionTransition { .. }
                    | FactoryError::SessionNotRetryable { .. }
            ));
        }
    }

    #[test]
    fn retry_and_expiry_cannot_skip_required_intermediate_states() {
        let mut retry_state = state_with_session({
            let mut session = sample_session(SpawnSessionState::Failed);
            session.retryable = true;
            session
        });

        apply_session_event_in_state(
            &mut retry_state,
            "session-1",
            SessionAuditActor::User,
            5_000,
            SpawnSessionEvent::RetryRequested,
            "retry",
        )
        .expect("retry should succeed");

        let retry_error = apply_session_event_in_state(
            &mut retry_state,
            "session-1",
            SessionAuditActor::System,
            5_001,
            SpawnSessionEvent::ReleaseBroadcast,
            "skip install",
        )
        .expect_err("retry should not jump to completion");
        assert!(matches!(
            retry_error,
            FactoryError::IllegalSessionTransition { .. }
        ));

        let mut expiry_state =
            state_with_session(sample_session(SpawnSessionState::PaymentDetected));
        let expired = apply_session_event_in_state(
            &mut expiry_state,
            "session-1",
            SessionAuditActor::System,
            11_000,
            SpawnSessionEvent::SessionExpired,
            "expired",
        )
        .expect("expiry should succeed");
        assert_eq!(expired.state, SpawnSessionState::Expired);

        let expired_retry = apply_session_event_in_state(
            &mut expiry_state,
            "session-1",
            SessionAuditActor::User,
            11_100,
            SpawnSessionEvent::RetryRequested,
            "retry expired",
        )
        .expect_err("expired session should not retry");
        assert!(matches!(
            expired_retry,
            FactoryError::SessionNotRetryable { .. }
        ));
    }

    #[test]
    fn derives_retryable_and_refundable_flags_from_state_and_payment_status() {
        let mut failed = state_with_session(sample_session(SpawnSessionState::Failed));
        let updated_failed =
            sync_session_derived_flags_in_state(&mut failed, "session-1", 5_000).unwrap();
        assert!(updated_failed.retryable);
        assert!(!updated_failed.refundable);

        let mut expired_session = sample_session(SpawnSessionState::Expired);
        expired_session.payment_status = PaymentStatus::Partial;
        let mut expired = state_with_session(expired_session);
        let updated_expired =
            sync_session_derived_flags_in_state(&mut expired, "session-1", 11_000).unwrap();
        assert!(!updated_expired.retryable);
        assert!(updated_expired.refundable);
    }
}
