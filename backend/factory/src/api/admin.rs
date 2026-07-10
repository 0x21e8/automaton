use sha2::{Digest, Sha256};

use crate::api::public::get_spawn_session;
use crate::retry::retry_failed_session;
use crate::state::{
    current_canister_balance, ensure_admin_in_state, read_state, write_state, FactoryState,
};
use crate::types::{
    amount_to_string, hex_encode, parse_amount, validate_sha256_hex, validate_version_commit,
    ArtifactUploadStatus, CreationCostQuote, FactoryArtifactSnapshot, FactoryConfigSnapshot,
    FactoryError, FactoryHealthSnapshot, FactoryOperationalConfig, FactoryRuntimeSnapshot,
    FactorySchedulerHealthSnapshot, FactorySchedulerJobCounts, FactorySessionHealthCounts,
    FeeConfig, PendingArtifactUpload, ReleaseBroadcastConfig, SchedulerJob, SchedulerJobStatus,
    SessionAdminView, SessionAuditActor, SessionAuditEntry, SpawnPaymentInstructions, SpawnQuote,
    SpawnSessionState, SpawnSessionStatusResponse, SpawnedAutomatonRecord,
};

pub fn set_fee_config(
    caller: &str,
    mut config: FeeConfig,
    now_ms: u64,
) -> Result<FeeConfig, FactoryError> {
    parse_amount(&config.usdc_fee)?;
    config.updated_at = now_ms;

    write_state(|state| -> Result<(), FactoryError> {
        ensure_admin_in_state(state, caller)?;
        state.fee_config = config.clone();
        Ok(())
    })?;

    Ok(config)
}

pub fn set_creation_cost_quote(
    caller: &str,
    mut config: CreationCostQuote,
    now_ms: u64,
) -> Result<CreationCostQuote, FactoryError> {
    parse_amount(&config.usdc_cost)?;
    config.updated_at = now_ms;

    write_state(|state| -> Result<(), FactoryError> {
        ensure_admin_in_state(state, caller)?;
        state.creation_cost_quote = config.clone();
        Ok(())
    })?;

    Ok(config)
}

pub fn set_release_broadcast_config(
    caller: &str,
    config: ReleaseBroadcastConfig,
) -> Result<ReleaseBroadcastConfig, FactoryError> {
    write_state(|state| -> Result<(), FactoryError> {
        ensure_admin_in_state(state, caller)?;
        state.release_broadcast_config = config.clone();
        if state.child_runtime.evm_chain_id.is_some() {
            state.child_runtime.evm_chain_id = Some(config.chain_id);
        }
        Ok(())
    })?;

    Ok(config)
}

pub fn set_child_runtime_config(
    caller: &str,
    config: crate::types::AutomatonChildRuntimeConfig,
) -> Result<crate::types::AutomatonChildRuntimeConfig, FactoryError> {
    write_state(|state| -> Result<(), FactoryError> {
        ensure_admin_in_state(state, caller)?;
        state.child_runtime = config.clone();
        if let Some(chain_id) = config.evm_chain_id {
            state.release_broadcast_config.chain_id = chain_id;
        }
        Ok(())
    })?;

    Ok(config)
}

pub fn set_operational_config(
    caller: &str,
    config: FactoryOperationalConfig,
) -> Result<FactoryOperationalConfig, FactoryError> {
    write_state(|state| -> Result<(), FactoryError> {
        ensure_admin_in_state(state, caller)?;
        state.cycles_per_spawn = config.cycles_per_spawn;
        state.min_pool_balance = config.min_pool_balance;
        state.estimated_outcall_cycles_per_interval = config.estimated_outcall_cycles_per_interval;
        Ok(())
    })?;

    Ok(config)
}

pub fn set_pause(caller: &str, paused: bool) -> Result<bool, FactoryError> {
    write_state(|state| -> Result<(), FactoryError> {
        ensure_admin_in_state(state, caller)?;
        state.pause = paused;
        Ok(())
    })?;
    Ok(paused)
}

pub fn get_factory_config(caller: &str) -> Result<FactoryConfigSnapshot, FactoryError> {
    read_state(|state| -> Result<FactoryConfigSnapshot, FactoryError> {
        ensure_admin_in_state(state, caller)?;
        Ok(FactoryConfigSnapshot {
            fee_config: state.fee_config.clone(),
            creation_cost_quote: state.creation_cost_quote.clone(),
            release_broadcast_config: state.release_broadcast_config.clone(),
            child_runtime: state.child_runtime.clone(),
            pause: state.pause,
            payment_address: state.payment_address.clone(),
            escrow_contract_address: state.escrow_contract_address.clone(),
            factory_evm_address: state.factory_evm_address.clone(),
            base_rpc_endpoint: state.base_rpc_endpoint.clone(),
            base_rpc_fallback_endpoint: state.base_rpc_fallback_endpoint.clone(),
            cycles_per_spawn: state.cycles_per_spawn,
            min_pool_balance: state.min_pool_balance,
            estimated_outcall_cycles_per_interval: state.estimated_outcall_cycles_per_interval,
            session_ttl_ms: state.session_ttl_ms,
            version_commit: state.version_commit.clone(),
            wasm_sha256: state.wasm_sha256.clone(),
        })
    })
}

fn build_artifact_snapshot(state: &FactoryState) -> FactoryArtifactSnapshot {
    FactoryArtifactSnapshot {
        loaded: state.wasm_bytes.is_some(),
        wasm_sha256: state.wasm_sha256.clone(),
        version_commit: state
            .wasm_bytes
            .as_ref()
            .map(|_| state.version_commit.clone()),
        wasm_size_bytes: state.wasm_bytes.as_ref().map(|bytes| bytes.len() as u64),
    }
}

fn build_artifact_upload_status(state: &FactoryState) -> ArtifactUploadStatus {
    match &state.pending_artifact_upload {
        Some(upload) => ArtifactUploadStatus {
            in_progress: true,
            expected_sha256: Some(upload.expected_sha256.clone()),
            version_commit: Some(upload.version_commit.clone()),
            total_size_bytes: Some(upload.total_size_bytes),
            received_size_bytes: upload.wasm_bytes.len() as u64,
        },
        None => ArtifactUploadStatus {
            in_progress: false,
            expected_sha256: None,
            version_commit: None,
            total_size_bytes: None,
            received_size_bytes: 0,
        },
    }
}

fn store_artifact(
    state: &mut FactoryState,
    wasm_bytes: Vec<u8>,
    wasm_sha256: String,
    version_commit: String,
) -> FactoryArtifactSnapshot {
    state.wasm_bytes = Some(wasm_bytes);
    state.wasm_sha256 = Some(wasm_sha256);
    state.pending_artifact_upload = None;
    state.version_commit = version_commit;
    build_artifact_snapshot(state)
}

fn build_scheduler_job_counts(state: &FactoryState) -> (FactorySchedulerJobCounts, u64) {
    let mut counts = FactorySchedulerJobCounts::default();
    let mut retry_queue_count = 0u64;

    for job in state.scheduler_jobs.values() {
        counts.total += 1;
        match job.status {
            SchedulerJobStatus::Pending => counts.pending += 1,
            SchedulerJobStatus::Running => counts.running += 1,
            SchedulerJobStatus::Completed => counts.completed += 1,
            SchedulerJobStatus::Backoff => counts.backoff += 1,
            SchedulerJobStatus::Skipped => counts.skipped += 1,
            SchedulerJobStatus::Terminal => counts.terminal += 1,
        }

        if job.last_error.is_some() {
            counts.with_last_error += 1;
        }

        if job.consecutive_failure_count > 0 && job.next_run_at_ms.is_some() {
            retry_queue_count += 1;
        }
    }

    (counts, retry_queue_count)
}

fn build_scheduler_health_snapshot(state: &FactoryState) -> FactorySchedulerHealthSnapshot {
    let (job_counts, retry_queue_count) = build_scheduler_job_counts(state);

    FactorySchedulerHealthSnapshot {
        last_tick_started_ms: state.scheduler_runtime.last_tick_started_ms,
        last_tick_finished_ms: state.scheduler_runtime.last_tick_finished_ms,
        last_tick_error: state.scheduler_runtime.last_tick_error.clone(),
        active_job_ids: state.scheduler_runtime.active_job_ids.clone(),
        job_counts,
        retry_queue_count,
    }
}

fn job_recency_ms(job: &SchedulerJob) -> u64 {
    [
        job.last_finished_at_ms,
        job.last_started_at_ms,
        job.leased_at_ms,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0)
}

fn sort_jobs_by_recency_desc(jobs: &mut [SchedulerJob]) {
    jobs.sort_by_cached_key(|job| std::cmp::Reverse((job_recency_ms(job), job.job_id.clone())));
}

pub fn get_factory_health() -> FactoryHealthSnapshot {
    read_state(|state| {
        let mut active_sessions = FactorySessionHealthCounts::default();

        for session in state.sessions.values() {
            match session.state {
                SpawnSessionState::AwaitingPayment => active_sessions.awaiting_payment += 1,
                SpawnSessionState::PaymentDetected => active_sessions.payment_detected += 1,
                SpawnSessionState::Spawning => active_sessions.spawning += 1,
                SpawnSessionState::BroadcastingRelease => active_sessions.broadcasting_release += 1,
                SpawnSessionState::Failed if session.retryable => {
                    active_sessions.retryable_failed += 1
                }
                SpawnSessionState::Complete
                | SpawnSessionState::Failed
                | SpawnSessionState::Expired => {}
            }
        }
        FactoryHealthSnapshot {
            current_canister_balance: current_canister_balance(),
            pause: state.pause,
            cycles_per_spawn: state.cycles_per_spawn,
            min_pool_balance: state.min_pool_balance,
            estimated_outcall_cycles_per_interval: state.estimated_outcall_cycles_per_interval,
            escrow_contract_address: state.escrow_contract_address.clone(),
            factory_evm_address: state.factory_evm_address.clone(),
            artifact: build_artifact_snapshot(state),
            active_sessions,
            scheduler: build_scheduler_health_snapshot(state),
        }
    })
}

fn job_failure_ms(job: &SchedulerJob) -> u64 {
    job.last_error.as_ref().map_or(0, |f| f.occurred_at)
}

pub fn get_factory_runtime(
    caller: &str,
    recent_job_limit: usize,
) -> Result<FactoryRuntimeSnapshot, FactoryError> {
    if recent_job_limit == 0 {
        return Err(FactoryError::InvalidPaginationLimit {
            limit: recent_job_limit,
        });
    }

    read_state(|state| -> Result<FactoryRuntimeSnapshot, FactoryError> {
        ensure_admin_in_state(state, caller)?;

        let scheduler = build_scheduler_health_snapshot(state);

        let active_jobs = state
            .scheduler_runtime
            .active_job_ids
            .iter()
            .filter_map(|job_id| state.scheduler_jobs.get(job_id).cloned())
            .collect::<Vec<_>>();

        let mut retry_queue = state
            .scheduler_jobs
            .values()
            .filter(|job| job.consecutive_failure_count > 0 && job.next_run_at_ms.is_some())
            .cloned()
            .collect::<Vec<_>>();
        retry_queue.sort_by(|left, right| {
            left.next_run_at_ms
                .cmp(&right.next_run_at_ms)
                .then_with(|| left.job_id.cmp(&right.job_id))
        });

        let mut recent_jobs = state.scheduler_jobs.values().cloned().collect::<Vec<_>>();
        sort_jobs_by_recency_desc(&mut recent_jobs);
        recent_jobs.truncate(recent_job_limit);

        let mut failed_jobs = state
            .scheduler_jobs
            .values()
            .filter(|job| job.last_error.is_some())
            .cloned()
            .collect::<Vec<_>>();
        failed_jobs.sort_by(|left, right| {
            job_failure_ms(right)
                .cmp(&job_failure_ms(left))
                .then_with(|| right.job_id.cmp(&left.job_id))
        });
        failed_jobs.truncate(recent_job_limit);

        Ok(FactoryRuntimeSnapshot {
            scheduler,
            active_jobs,
            retry_queue,
            recent_jobs,
            failed_jobs,
        })
    })
}

pub fn update_artifact(
    caller: &str,
    wasm_bytes: Vec<u8>,
    expected_sha256: String,
    version_commit: String,
) -> Result<FactoryArtifactSnapshot, FactoryError> {
    validate_sha256_hex(&expected_sha256)?;
    validate_version_commit(&version_commit)?;

    let actual_sha256 = hex_encode(Sha256::digest(&wasm_bytes).as_slice());
    if actual_sha256 != expected_sha256 {
        return Err(FactoryError::ArtifactHashMismatch {
            expected: expected_sha256,
            actual: actual_sha256,
        });
    }

    write_state(|state| -> Result<FactoryArtifactSnapshot, FactoryError> {
        ensure_admin_in_state(state, caller)?;
        Ok(store_artifact(
            state,
            wasm_bytes,
            actual_sha256,
            version_commit,
        ))
    })
}

pub fn begin_artifact_upload(
    caller: &str,
    expected_sha256: String,
    version_commit: String,
    total_size_bytes: u64,
) -> Result<ArtifactUploadStatus, FactoryError> {
    validate_sha256_hex(&expected_sha256)?;
    validate_version_commit(&version_commit)?;

    write_state(|state| -> Result<ArtifactUploadStatus, FactoryError> {
        ensure_admin_in_state(state, caller)?;
        state.pending_artifact_upload = Some(PendingArtifactUpload {
            expected_sha256,
            version_commit,
            total_size_bytes,
            wasm_bytes: Vec::new(),
        });
        Ok(build_artifact_upload_status(state))
    })
}

pub fn append_artifact_chunk(
    caller: &str,
    chunk: Vec<u8>,
) -> Result<ArtifactUploadStatus, FactoryError> {
    write_state(|state| -> Result<ArtifactUploadStatus, FactoryError> {
        ensure_admin_in_state(state, caller)?;

        let upload = state
            .pending_artifact_upload
            .as_mut()
            .ok_or(FactoryError::NoPendingArtifactUpload)?;
        let attempted = upload.wasm_bytes.len() as u64 + chunk.len() as u64;
        if attempted > upload.total_size_bytes {
            return Err(FactoryError::ArtifactUploadTooLarge {
                expected: upload.total_size_bytes,
                attempted,
            });
        }

        upload.wasm_bytes.extend(chunk);
        Ok(build_artifact_upload_status(state))
    })
}

pub fn get_artifact_upload_status(caller: &str) -> Result<ArtifactUploadStatus, FactoryError> {
    read_state(|state| -> Result<ArtifactUploadStatus, FactoryError> {
        ensure_admin_in_state(state, caller)?;
        Ok(build_artifact_upload_status(state))
    })
}

pub fn commit_artifact_upload(caller: &str) -> Result<FactoryArtifactSnapshot, FactoryError> {
    write_state(|state| -> Result<FactoryArtifactSnapshot, FactoryError> {
        ensure_admin_in_state(state, caller)?;

        {
            let upload = state
                .pending_artifact_upload
                .as_ref()
                .ok_or(FactoryError::NoPendingArtifactUpload)?;
            let received = upload.wasm_bytes.len() as u64;
            if received != upload.total_size_bytes {
                return Err(FactoryError::ArtifactUploadIncomplete {
                    expected: upload.total_size_bytes,
                    received,
                });
            }
            let actual_sha256 = hex_encode(Sha256::digest(&upload.wasm_bytes).as_slice());
            if actual_sha256 != upload.expected_sha256 {
                return Err(FactoryError::ArtifactHashMismatch {
                    expected: upload.expected_sha256.clone(),
                    actual: actual_sha256,
                });
            }
        }

        let upload = state.pending_artifact_upload.take().unwrap();
        Ok(store_artifact(
            state,
            upload.wasm_bytes,
            upload.expected_sha256,
            upload.version_commit,
        ))
    })
}

#[cfg(target_arch = "wasm32")]
pub async fn derive_factory_evm_address(caller: &str) -> Result<String, FactoryError> {
    read_state(|state| ensure_admin_in_state(state, caller))?;
    crate::evm::derive_factory_evm_address().await
}

pub fn get_session_admin(caller: &str, session_id: &str) -> Result<SessionAdminView, FactoryError> {
    read_state(|state| -> Result<SessionAdminView, FactoryError> {
        ensure_admin_in_state(state, caller)?;

        let session = state.sessions.get(session_id).cloned().ok_or_else(|| {
            FactoryError::SessionNotFound {
                session_id: session_id.to_string(),
            }
        })?;
        let audit: Vec<SessionAuditEntry> =
            state.audit_log.get(session_id).cloned().unwrap_or_default();
        let quoted_total_amount = amount_to_string(
            parse_amount(state.fee_config.amount_for(&session.asset))?
                + parse_amount(state.creation_cost_quote.amount_for(&session.asset))?,
        );
        let quote = SpawnQuote {
            session_id: session.session_id.clone(),
            chain: session.chain.clone(),
            asset: session.asset.clone(),
            gross_amount: session.gross_amount.clone(),
            platform_fee: session.platform_fee.clone(),
            creation_cost: session.creation_cost.clone(),
            net_forward_amount: session.net_forward_amount.clone(),
            quote_terms_hash: session.quote_terms_hash.clone(),
            expires_at: session.expires_at,
            payment: SpawnPaymentInstructions::from_session(&session, &state.payment_address),
        };
        let escrow_claim = state
            .escrow_claims
            .get(session_id)
            .cloned()
            .ok_or_else(|| FactoryError::EscrowClaimNotFound {
                session_id: session_id.to_string(),
            })?;
        let registry_record: Option<SpawnedAutomatonRecord> = session
            .automaton_canister_id
            .as_ref()
            .and_then(|canister_id| state.registry.get(canister_id).cloned());
        let runtime_record = session
            .automaton_canister_id
            .as_ref()
            .and_then(|canister_id| state.runtimes.get(canister_id).cloned());

        Ok(SessionAdminView {
            session,
            audit,
            quote,
            escrow_claim,
            runtime_record,
            registry_record,
            pause: state.pause,
            quoted_total_amount,
        })
    })
}

pub fn retry_session_admin(
    caller: &str,
    session_id: &str,
    now_ms: u64,
) -> Result<SpawnSessionStatusResponse, FactoryError> {
    read_state(|state| ensure_admin_in_state(state, caller))?;
    retry_failed_session(
        session_id,
        SessionAuditActor::Admin,
        now_ms,
        "retry requested by admin",
    )?;
    get_spawn_session(session_id)
}
