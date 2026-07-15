#![cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]

use crate::base_rpc::{configured_rpc_endpoints, endpoint_has_scheme};
use crate::state::{read_state, write_state, FactoryState};
use crate::types::{
    FactoryError, PaymentStatus, SchedulerFailureAction, SchedulerFailureSource, SchedulerJob,
    SchedulerJobFailure, SchedulerJobKind, SchedulerJobStatus, SpawnExecutionReceipt, SpawnSession,
    SpawnSessionState,
};

pub const PAYMENT_POLL_INTERVAL_MS: u64 = 30_000;
const JOB_LEASE_DURATION_MS: u64 = 60_000;
const INITIAL_BACKOFF_MS: u64 = 5_000;
const MAX_BACKOFF_MS: u64 = 5 * 60_000;
const MAX_JOBS_PER_TICK: usize = 8;

pub const PAYMENT_POLL_JOB_ID: &str = "payment-poll:base";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerRunReport {
    pub job_id: String,
    pub kind: SchedulerJobKind,
    pub spawn_receipt: Option<SpawnExecutionReceipt>,
    pub error: Option<FactoryError>,
}

#[derive(Clone, Debug)]
struct JobExecutionFailure {
    #[cfg(not(target_arch = "wasm32"))]
    error: Option<Box<FactoryError>>,
    failure: SchedulerJobFailure,
}

pub(crate) fn session_needs_payment_poll(session: &SpawnSession) -> bool {
    session.state == SpawnSessionState::AwaitingPayment
        && !matches!(
            session.payment_status,
            PaymentStatus::Paid | PaymentStatus::Refunded
        )
}

pub(crate) fn spawn_job_id(session_id: &str) -> String {
    format!("spawn-execution:{session_id}")
}

fn payment_poll_job(now_ms: u64) -> SchedulerJob {
    SchedulerJob {
        job_id: PAYMENT_POLL_JOB_ID.to_string(),
        kind: SchedulerJobKind::PaymentPoll,
        status: SchedulerJobStatus::Pending,
        next_run_at_ms: Some(now_ms),
        leased_at_ms: None,
        leased_until_ms: None,
        last_started_at_ms: None,
        last_finished_at_ms: None,
        attempt_count: 0,
        consecutive_failure_count: 0,
        success_count: 0,
        last_error: None,
    }
}

fn spawn_execution_job(session_id: &str, now_ms: u64) -> SchedulerJob {
    SchedulerJob {
        job_id: spawn_job_id(session_id),
        kind: SchedulerJobKind::SpawnExecution {
            session_id: session_id.to_string(),
        },
        status: SchedulerJobStatus::Pending,
        next_run_at_ms: Some(now_ms),
        leased_at_ms: None,
        leased_until_ms: None,
        last_started_at_ms: None,
        last_finished_at_ms: None,
        attempt_count: 0,
        consecutive_failure_count: 0,
        success_count: 0,
        last_error: None,
    }
}

fn job_has_live_lease(job: &SchedulerJob, now_ms: u64) -> bool {
    job.leased_until_ms
        .map(|leased_until_ms| leased_until_ms > now_ms)
        .unwrap_or(false)
}

fn job_is_due(job: &SchedulerJob, now_ms: u64) -> bool {
    job.next_run_at_ms
        .map(|next_run_at_ms| next_run_at_ms <= now_ms)
        .unwrap_or(false)
        && !job_has_live_lease(job, now_ms)
}

fn refresh_active_job_ids(state: &mut FactoryState, now_ms: u64) {
    let mut active_job_ids = state
        .scheduler_jobs
        .values()
        .filter(|job| job_has_live_lease(job, now_ms))
        .map(|job| job.job_id.clone())
        .collect::<Vec<_>>();
    active_job_ids.sort();
    state.scheduler_runtime.active_job_ids = active_job_ids;
}

fn start_tick(now_ms: u64) {
    write_state(|state| {
        state.scheduler_runtime.last_tick_started_ms = Some(now_ms);
        state.scheduler_runtime.last_tick_error = None;
        refresh_active_job_ids(state, now_ms);
    });
}

fn finish_tick(now_ms: u64) {
    write_state(|state| {
        state.scheduler_runtime.last_tick_finished_ms = Some(now_ms);
        refresh_active_job_ids(state, now_ms);
    });
}

fn base_rpc_missing_or_invalid_failure(
    primary_endpoint: Option<String>,
    fallback_endpoint: Option<String>,
    now_ms: u64,
) -> Option<SchedulerJobFailure> {
    let endpoints = configured_rpc_endpoints(primary_endpoint, fallback_endpoint);
    if endpoints.is_empty() {
        return Some(SchedulerJobFailure {
            action: SchedulerFailureAction::Skip,
            source: SchedulerFailureSource::MissingConfig,
            message: "base RPC endpoint is not configured".to_string(),
            occurred_at: now_ms,
        });
    }

    if endpoints
        .iter()
        .all(|endpoint| !endpoint_has_scheme(endpoint))
    {
        return Some(SchedulerJobFailure {
            action: SchedulerFailureAction::Skip,
            source: SchedulerFailureSource::InvalidConfig,
            message: format!(
                "base RPC endpoints have no scheme: {}",
                endpoints.join(", ")
            ),
            occurred_at: now_ms,
        });
    }

    None
}

fn payment_poll_prerequisite_failure(now_ms: u64) -> Option<SchedulerJobFailure> {
    let (endpoint, fallback_endpoint, escrow_contract_address) = read_state(|state| {
        (
            state.base_rpc_endpoint.clone(),
            state.base_rpc_fallback_endpoint.clone(),
            state.escrow_contract_address.clone(),
        )
    });

    if let Some(failure) = base_rpc_missing_or_invalid_failure(endpoint, fallback_endpoint, now_ms)
    {
        return Some(failure);
    }

    if escrow_contract_address.trim().is_empty() {
        return Some(SchedulerJobFailure {
            action: SchedulerFailureAction::Skip,
            source: SchedulerFailureSource::MissingConfig,
            message: "escrow contract address is not configured".to_string(),
            occurred_at: now_ms,
        });
    }

    None
}

fn backoff_delay_ms(consecutive_failures: u32) -> u64 {
    let shift = consecutive_failures.saturating_sub(1).min(16);
    let multiplier = 1_u64 << shift;
    INITIAL_BACKOFF_MS
        .saturating_mul(multiplier)
        .min(MAX_BACKOFF_MS)
}

pub(crate) fn sync_payment_poll_job_in_state(state: &mut FactoryState, now_ms: u64) {
    let has_active_sessions = state.sessions.values().any(session_needs_payment_poll);

    if !has_active_sessions {
        if let Some(job) = state.scheduler_jobs.get_mut(PAYMENT_POLL_JOB_ID) {
            if !job_has_live_lease(job, now_ms) {
                job.status = SchedulerJobStatus::Completed;
                job.next_run_at_ms = None;
                job.leased_at_ms = None;
                job.leased_until_ms = None;
            }
        }
        return;
    }

    let job = state
        .scheduler_jobs
        .entry(PAYMENT_POLL_JOB_ID.to_string())
        .or_insert_with(|| payment_poll_job(now_ms));

    if !job_has_live_lease(job, now_ms) {
        job.next_run_at_ms = Some(job.next_run_at_ms.unwrap_or(now_ms).min(now_ms));
        job.status = SchedulerJobStatus::Pending;
        job.leased_at_ms = None;
        job.leased_until_ms = None;
    }
}

pub(crate) fn enqueue_payment_poll(now_ms: u64) {
    write_state(|state| {
        sync_payment_poll_job_in_state(state, now_ms);
    });
}

pub(crate) fn enqueue_spawn_execution_in_state(
    state: &mut FactoryState,
    session_id: &str,
    now_ms: u64,
) {
    let job_id = spawn_job_id(session_id);
    let job = state
        .scheduler_jobs
        .entry(job_id)
        .or_insert_with(|| spawn_execution_job(session_id, now_ms));

    if !job_has_live_lease(job, now_ms) {
        job.status = SchedulerJobStatus::Pending;
        job.next_run_at_ms = Some(now_ms);
        job.leased_at_ms = None;
        job.leased_until_ms = None;
    }
}

fn lease_due_jobs(now_ms: u64, limit: usize) -> Vec<SchedulerJob> {
    write_state(|state| {
        refresh_active_job_ids(state, now_ms);

        let mut due_jobs = state
            .scheduler_jobs
            .values()
            .filter(|job| job_is_due(job, now_ms))
            .map(|job| (job.next_run_at_ms.unwrap_or(u64::MAX), job.job_id.clone()))
            .collect::<Vec<_>>();
        due_jobs.sort();
        due_jobs.truncate(limit);

        let mut leased = Vec::with_capacity(due_jobs.len());
        for (_, job_id) in due_jobs {
            let Some(job) = state.scheduler_jobs.get_mut(&job_id) else {
                continue;
            };
            job.status = SchedulerJobStatus::Running;
            job.leased_at_ms = Some(now_ms);
            job.leased_until_ms = Some(now_ms + JOB_LEASE_DURATION_MS);
            job.last_started_at_ms = Some(now_ms);
            job.attempt_count = job.attempt_count.saturating_add(1);
            leased.push(job.clone());
        }

        refresh_active_job_ids(state, now_ms);
        leased
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn lease_due_jobs_for_test(now_ms: u64, limit: usize) -> Vec<SchedulerJob> {
    lease_due_jobs(now_ms, limit)
}

fn finalize_job_success(job_id: &str, now_ms: u64) {
    write_state(|state| {
        let poll_remains_active = state.sessions.values().any(session_needs_payment_poll);
        let Some(job) = state.scheduler_jobs.get_mut(job_id) else {
            return;
        };

        job.last_finished_at_ms = Some(now_ms);
        job.leased_at_ms = None;
        job.leased_until_ms = None;
        job.success_count = job.success_count.saturating_add(1);
        job.consecutive_failure_count = 0;
        job.last_error = None;

        match job.kind {
            SchedulerJobKind::PaymentPoll => {
                if poll_remains_active {
                    job.status = SchedulerJobStatus::Pending;
                    job.next_run_at_ms = Some(now_ms + PAYMENT_POLL_INTERVAL_MS);
                } else {
                    job.status = SchedulerJobStatus::Completed;
                    job.next_run_at_ms = None;
                }
            }
            SchedulerJobKind::SpawnExecution { .. } => {
                job.status = SchedulerJobStatus::Completed;
                job.next_run_at_ms = None;
            }
        }

        refresh_active_job_ids(state, now_ms);
    });
}

fn finalize_job_failure(job_id: &str, failure: SchedulerJobFailure, now_ms: u64) {
    let error_message = format!("{job_id}: {}", failure.message);

    write_state(|state| {
        let poll_remains_active = state.sessions.values().any(session_needs_payment_poll);
        let Some(job) = state.scheduler_jobs.get_mut(job_id) else {
            return;
        };

        job.last_finished_at_ms = Some(now_ms);
        job.leased_at_ms = None;
        job.leased_until_ms = None;
        job.consecutive_failure_count = job.consecutive_failure_count.saturating_add(1);
        job.last_error = Some(failure.clone());

        match failure.action {
            SchedulerFailureAction::Retry => {
                job.status = SchedulerJobStatus::Pending;
                job.next_run_at_ms = Some(now_ms);
            }
            SchedulerFailureAction::Backoff => {
                job.status = SchedulerJobStatus::Backoff;
                job.next_run_at_ms = Some(now_ms + backoff_delay_ms(job.consecutive_failure_count));
            }
            SchedulerFailureAction::Skip => {
                job.status = SchedulerJobStatus::Skipped;
                job.next_run_at_ms = match job.kind {
                    SchedulerJobKind::PaymentPoll if poll_remains_active => {
                        Some(now_ms + PAYMENT_POLL_INTERVAL_MS)
                    }
                    _ => None,
                };
            }
            SchedulerFailureAction::Terminal => {
                job.status = SchedulerJobStatus::Terminal;
                job.next_run_at_ms = None;
            }
        }

        refresh_active_job_ids(state, now_ms);
        state.scheduler_runtime.last_tick_error = Some(error_message);
        state.scheduler_runtime.last_tick_finished_ms = Some(now_ms);
    });
}

fn classify_payment_poll_error(error: &FactoryError, now_ms: u64) -> SchedulerJobFailure {
    match error {
        FactoryError::RpcRequestFailed { message, .. }
        | FactoryError::ManagementCallFailed { message, .. } => SchedulerJobFailure {
            action: SchedulerFailureAction::Backoff,
            source: SchedulerFailureSource::Transient,
            message: message.clone(),
            occurred_at: now_ms,
        },
        FactoryError::InvalidAmount { value } => SchedulerJobFailure {
            action: SchedulerFailureAction::Skip,
            source: SchedulerFailureSource::Deterministic,
            message: format!("invalid payment log amount: {value}"),
            occurred_at: now_ms,
        },
        other => SchedulerJobFailure {
            action: SchedulerFailureAction::Skip,
            source: SchedulerFailureSource::Deterministic,
            message: other.to_string(),
            occurred_at: now_ms,
        },
    }
}

fn classify_spawn_error(error: &FactoryError, now_ms: u64) -> SchedulerJobFailure {
    match error {
        FactoryError::MissingChildRuntimeConfig { field } => SchedulerJobFailure {
            action: SchedulerFailureAction::Skip,
            source: SchedulerFailureSource::MissingConfig,
            message: format!("missing child runtime config: {field}"),
            occurred_at: now_ms,
        },
        FactoryError::ManagementCallFailed { message, .. }
            if message.contains("not configured") =>
        {
            SchedulerJobFailure {
                action: SchedulerFailureAction::Skip,
                source: SchedulerFailureSource::MissingConfig,
                message: message.clone(),
                occurred_at: now_ms,
            }
        }
        FactoryError::RpcRequestFailed { message, .. }
        | FactoryError::ManagementCallFailed { message, .. } => SchedulerJobFailure {
            action: SchedulerFailureAction::Backoff,
            source: SchedulerFailureSource::Transient,
            message: message.clone(),
            occurred_at: now_ms,
        },
        FactoryError::InsufficientCyclesPool {
            available,
            required,
        } => SchedulerJobFailure {
            action: SchedulerFailureAction::Backoff,
            source: SchedulerFailureSource::Transient,
            message: format!(
                "insufficient cycles pool: available={available}, required={required}"
            ),
            occurred_at: now_ms,
        },
        FactoryError::InsufficientCyclesForOperation {
            operation,
            available,
            required,
        } => SchedulerJobFailure {
            action: SchedulerFailureAction::Backoff,
            source: SchedulerFailureSource::Transient,
            message: format!(
                "insufficient cycles for operation {operation}: available={available}, required={required}"
            ),
            occurred_at: now_ms,
        },
        FactoryError::SessionExpired {
            session_id,
            expires_at,
        } => SchedulerJobFailure {
            action: SchedulerFailureAction::Terminal,
            source: SchedulerFailureSource::Deterministic,
            message: format!("session expired: {session_id} at {expires_at}"),
            occurred_at: now_ms,
        },
        FactoryError::SessionNotReadyForSpawn { .. } | FactoryError::PaymentNotSettled { .. } => {
            SchedulerJobFailure {
                action: SchedulerFailureAction::Skip,
                source: SchedulerFailureSource::Deterministic,
                message: error.to_string(),
                occurred_at: now_ms,
            }
        }
        FactoryError::SessionNotFound { .. } => SchedulerJobFailure {
            action: SchedulerFailureAction::Terminal,
            source: SchedulerFailureSource::Deterministic,
            message: error.to_string(),
            occurred_at: now_ms,
        },
        other => SchedulerJobFailure {
            action: SchedulerFailureAction::Skip,
            source: SchedulerFailureSource::Deterministic,
            message: other.to_string(),
            occurred_at: now_ms,
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_job_sync(
    job: &SchedulerJob,
    now_ms: u64,
) -> Result<Option<SpawnExecutionReceipt>, JobExecutionFailure> {
    match &job.kind {
        SchedulerJobKind::PaymentPoll => {
            if let Some(failure) = payment_poll_prerequisite_failure(now_ms) {
                return Err(JobExecutionFailure {
                    #[cfg(not(target_arch = "wasm32"))]
                    error: None,
                    failure,
                });
            }

            crate::escrow::poll_escrow_payments(now_ms)
                .map(|_| None)
                .map_err(|error| JobExecutionFailure {
                    failure: classify_payment_poll_error(&error, now_ms),
                    #[cfg(not(target_arch = "wasm32"))]
                    error: Some(Box::new(error)),
                })
        }
        SchedulerJobKind::SpawnExecution { session_id } => {
            crate::spawn::execute_spawn(session_id, now_ms)
                .map(Some)
                .map_err(|error| JobExecutionFailure {
                    failure: classify_spawn_error(&error, now_ms),
                    #[cfg(not(target_arch = "wasm32"))]
                    error: Some(Box::new(error)),
                })
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_scheduler_tick(now_ms: u64) -> Vec<SchedulerRunReport> {
    if read_state(|state| state.pause) {
        start_tick(now_ms);
        finish_tick(now_ms);
        return Vec::new();
    }

    start_tick(now_ms);
    let mut reports = Vec::new();

    for _ in 0..MAX_JOBS_PER_TICK {
        let leased_jobs = lease_due_jobs(now_ms, MAX_JOBS_PER_TICK);
        if leased_jobs.is_empty() {
            break;
        }

        for job in leased_jobs {
            match execute_job_sync(&job, now_ms) {
                Ok(spawn_receipt) => {
                    finalize_job_success(&job.job_id, now_ms);
                    reports.push(SchedulerRunReport {
                        job_id: job.job_id.clone(),
                        kind: job.kind.clone(),
                        spawn_receipt,
                        error: None,
                    });
                }
                Err(error) => {
                    finalize_job_failure(&job.job_id, error.failure.clone(), now_ms);
                    reports.push(SchedulerRunReport {
                        job_id: job.job_id.clone(),
                        kind: job.kind.clone(),
                        spawn_receipt: None,
                        error: error.error.map(|error| *error),
                    });
                }
            }
        }
    }

    finish_tick(now_ms);
    reports
}

#[cfg(target_arch = "wasm32")]
use crate::now_ms as current_time_ms;

#[cfg(target_arch = "wasm32")]
async fn execute_job_async(
    job: &SchedulerJob,
    now_ms: u64,
) -> Result<Option<SpawnExecutionReceipt>, JobExecutionFailure> {
    match &job.kind {
        SchedulerJobKind::PaymentPoll => {
            if let Some(failure) = payment_poll_prerequisite_failure(now_ms) {
                return Err(JobExecutionFailure {
                    #[cfg(not(target_arch = "wasm32"))]
                    error: None,
                    failure,
                });
            }

            crate::escrow::poll_escrow_payments(now_ms)
                .await
                .map(|_| None)
                .map_err(|error| JobExecutionFailure {
                    failure: classify_payment_poll_error(&error, now_ms),
                    #[cfg(not(target_arch = "wasm32"))]
                    error: Some(Box::new(error)),
                })
        }
        SchedulerJobKind::SpawnExecution { session_id } => {
            crate::spawn::execute_spawn(session_id, now_ms)
                .await
                .map(Some)
                .map_err(|error| JobExecutionFailure {
                    failure: classify_spawn_error(&error, now_ms),
                    #[cfg(not(target_arch = "wasm32"))]
                    error: Some(Box::new(error)),
                })
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn schedule_due_jobs(now_ms: u64) {
    if read_state(|state| state.pause) {
        return;
    }

    start_tick(now_ms);
    let leased_jobs = lease_due_jobs(now_ms, MAX_JOBS_PER_TICK);
    finish_tick(now_ms);

    for job in leased_jobs {
        ic_cdk::futures::spawn_017_compat(async move {
            match execute_job_async(&job, now_ms).await {
                Ok(_) => finalize_job_success(&job.job_id, current_time_ms()),
                Err(error) => finalize_job_failure(&job.job_id, error.failure, current_time_ms()),
            }
            schedule_due_jobs(current_time_ms());
        });
    }
}
