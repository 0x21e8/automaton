/// Scheduler tick — the heartbeat of the IC-Automaton canister.
///
/// `scheduler_tick` is called by the IC timer every `BASE_TICK_SECS` seconds.
/// Each tick performs three main duties:
///
/// 1. **Lease recovery** — stale mutating leases (jobs that crashed without
///    releasing their lock) are reclaimed so the queue can continue.
/// 2. **Job materialisation** (`refresh_due_jobs`) — for every enabled
///    `TaskKind`, a pending job is enqueued into the mutating lane when the
///    task's `next_due_ns` is in the past.
/// 3. **Job dispatch** — up to `MAX_MUTATING_JOBS_PER_TICK` pending jobs are
///    popped, individually lease-gated, survival-policy checked, dispatched,
///    and their outcomes persisted before the next job is attempted.
///
/// # Task kinds
///
/// | Kind            | Survival gate | Lease TTL             |
/// |-----------------|---------------|-----------------------|
/// | `AgentTurn`     | Inference     | `AGENT_TURN_LEASE_TTL_NS` |
/// | `PollInbox`     | EvmPoll       | `LIGHTWEIGHT_LEASE_TTL_NS` |
/// | `CheckCycles`   | —             | `LIGHTWEIGHT_LEASE_TTL_NS` |
/// | `TopUpCycles`   | —             | `LIGHTWEIGHT_LEASE_TTL_NS` |
/// | `Reconcile`     | —             | `LIGHTWEIGHT_LEASE_TTL_NS` |
///
/// Failed jobs are passed through the recovery policy, which may retry
/// immediately, apply exponential backoff, tune response-byte limits, or
/// escalate to a fault.
use crate::agent::{run_scheduled_turn_job_with_trigger, ScheduledTurnTrigger};
use crate::domain::cycle_admission::{
    affordability_requirements, can_afford_with_reserve, estimate_operation_cost,
    AffordabilityRequirements, OperationClass, DEFAULT_RESERVE_FLOOR_CYCLES,
    DEFAULT_SAFETY_MARGIN_BPS,
};
use crate::domain::mortality::{canonical_runway_seconds, policy_for_tier};
use crate::domain::recovery_policy::decide_recovery_action;
use crate::domain::types::{
    EvmEvent, InboxMessageSource, JobStatus, MemoryFact, MemoryFactSort, OperationFailure,
    OperationFailureKind, PendingStrategyExecutionState, RecoveryContext, RecoveryFailure,
    RecoveryOperation, RecoveryPolicyAction, ResponseLimitAdjustment, ResponseLimitPolicy,
    RuntimeSnapshot, ScheduledJob, StrategyExecutionCallState, SurvivalOperationClass,
    SurvivalTier, TaskKind, TaskLane, TemplateActivationState, TemplateStatus,
};
use crate::features::cycle_topup::{
    TopUpStage, TOPUP_MIN_OPERATIONAL_CYCLES, TOPUP_MIN_USDC_AVAILABLE_RAW,
};
use crate::features::cycle_topup_host::{
    build_cycle_topup, enqueue_topup_cycles_job, topup_cycles_dedupe_key,
};
use crate::features::evm::{
    classify_evm_failure, decode_message_queued_payload, fetch_peer_min_prices,
    fetch_transaction_receipt_status, fetch_wallet_balance_sync_read, HttpEvmRpcClient,
    StrategyReceiptObservation, TransactionReceiptStatus,
};
use crate::features::factory_room::{FactoryPeer, FactoryRoomClient, ReproductionSessionState};
use crate::features::inference::classify_inference_failure;
use crate::features::ThresholdSignerAdapter;
use crate::features::{EvmPoller, HttpEvmPoller};
use crate::storage::stable;
use crate::timing::{self, current_time_ns};
use crate::tools::{
    counterparty_pending_receipt_key, record_counterparty_deal, reproduction_approve_args,
    reproduction_deposit_args,
};
use alloy_primitives::U256;
use canlog::{log, GetLogFilter, LogFilter, LogPriorityLevels};
use serde_json::json;
use std::str::FromStr;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum inbox messages promoted from pending → staged in one `PollInbox` job.
const POLL_INBOX_STAGE_BATCH_SIZE: usize = 50;

/// Keep room reads far less frequent than the main external polling loop.
const ROOM_POLL_INTERVAL_SECS: u64 = timing::DEFAULT_TASK_INTERVAL_SECS * 5;

/// Cap each room catch-up step so chat remains incremental and low-cost.
const ROOM_POLL_PAGE_LIMIT: u64 = 20;
const PEER_DIRECTORY_PAGE_LIMIT: u64 = 20;
const PEER_LOOKUP_HARD_CAP: usize = 500;
const PENDING_RECEIPT_SCAN_LIMIT: usize = 500;

/// Reference workflow-envelope cost (cycles) used by `CheckCycles` to
/// estimate the minimum operational floor.
const CHECKCYCLES_REFERENCE_ENVELOPE_CYCLES: u128 = 5_000_000_000;

/// Liquid-cycles multiple of the critical floor that triggers `LowCycles` tier.
/// A canister with fewer than `required × 15` liquid cycles is classified low.
const CHECKCYCLES_LOW_TIER_MULTIPLIER: u128 = 15;

/// Maximum number of mutating jobs dispatched per scheduler tick.
/// Prevents a single tick from dominating the IC message queue.
const MAX_MUTATING_JOBS_PER_TICK: u8 = 4;

/// Upper bound for the EVM RPC `max_response_bytes` tuning policy.
const EVM_RPC_MAX_RESPONSE_BYTES_POLICY_MAX: u64 = 2 * 1024 * 1024;

/// Lower bound for any response-bytes tuning policy (both EVM and wallet sync).
const RESPONSE_BYTES_POLICY_MIN: u64 = 256;

/// Base interval (seconds) for exponential backoff on job failures.
const RECOVERY_BACKOFF_BASE_SECS: u64 = 1;

/// Upper bound for the wallet-balance sync `max_response_bytes` tuning policy.
const WALLET_SYNC_MAX_RESPONSE_BYTES_RECOVERY_MAX: u64 = 4 * 1024;

/// Minimum wait (seconds) before a failed top-up is automatically retried.
const TOPUP_FAILED_RECOVERY_BACKOFF_SECS: u64 = 120;

/// Maximum number of strategy templates iterated per `Reconcile` job.
const STRATEGY_RECONCILE_MAX_TEMPLATES: usize = 200;
const PENDING_STRATEGY_EXECUTION_SCAN_LIMIT: usize = 20;
/// A transaction with no receipt for one hour is considered dropped.
const STRATEGY_EXECUTION_RECEIPT_TIMEOUT_NS: u64 = 60 * 60 * 1_000_000_000;
const STRATEGY_EXECUTION_RECHECK_NS: u64 = 15 * 1_000_000_000;
const STRATEGY_EXECUTION_MAX_BACKOFF_NS: u64 = 5 * 60 * 1_000_000_000;

/// Templates older than this window (14 days) are disabled by the reconciler.
const STRATEGY_TEMPLATE_FRESHNESS_WINDOW_SECS: u64 = 14 * 24 * 60 * 60;

// ── Log types ────────────────────────────────────────────────────────────────

/// Outcome returned by `dispatch_job` to indicate whether a `TopUpCycles` job
/// needs a follow-up continuation (multi-stage top-up still in progress).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobDispatchOutcome {
    Completed,
    TopUpWaiting,
}

#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum SchedulerLogPriority {
    #[log_level(capacity = 2000, name = "SCHEDULER_INFO")]
    Info,
    #[log_level(capacity = 500, name = "SCHEDULER_ERROR")]
    Error,
}

impl GetLogFilter for SchedulerLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

// ── Tick entry point ─────────────────────────────────────────────────────────

/// Main scheduler heartbeat, invoked by the IC timer every `BASE_TICK_SECS`.
///
/// Sequence:
/// 1. Record tick start timestamp.
/// 2. Recover any stale mutating leases.
/// 3. Return early (no-op) if the scheduler is disabled.
/// 4. Materialise due jobs via `refresh_due_jobs`.
/// 5. Dispatch up to `MAX_MUTATING_JOBS_PER_TICK` pending jobs.
/// 6. Run retention maintenance if its interval has elapsed.
/// 7. Refresh HTTP certification state.
pub async fn scheduler_tick() {
    let now_ns = current_time_ns();
    stable::record_scheduler_tick_start(now_ns);

    // Death is durable and absolute. Even if an upgrade or stale timer leaves
    // the scheduler flag enabled, no queue or recovery path may act again.
    if stable::mortality_is_dead() {
        stable::record_scheduler_tick_end(now_ns, None);
        crate::http::init_certification();
        return;
    }

    log!(
        SchedulerLogPriority::Info,
        "scheduler_tick_start now={now_ns}"
    );

    stable::recover_stale_lease(now_ns);
    let _ = stable::recover_orphaned_turn_lock(now_ns);
    let expired_proxy_jobs = stable::expire_inference_proxy_pending_jobs(
        now_ns,
        stable::INFERENCE_PROXY_PENDING_JOB_TTL_SECS,
    );
    if !expired_proxy_jobs.is_empty() {
        let expired_job_ids = expired_proxy_jobs
            .iter()
            .map(|job| job.job_id.clone())
            .collect::<Vec<_>>()
            .join(",");
        log!(
            SchedulerLogPriority::Info,
            "inference_proxy_job_expired count={} job_ids={}",
            expired_proxy_jobs.len(),
            expired_job_ids,
        );
    }

    if !stable::scheduler_enabled() {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_tick_end disabled now={now_ns}"
        );
        stable::record_scheduler_tick_end(now_ns, None);
        crate::http::init_certification();
        return;
    }

    refresh_due_jobs(now_ns);

    let mut processed_jobs = 0u8;
    let mut terminal_error: Option<String> = None;
    while processed_jobs < MAX_MUTATING_JOBS_PER_TICK {
        match run_one_pending_mutating_job(current_time_ns()).await {
            Ok(true) => processed_jobs = processed_jobs.saturating_add(1),
            Ok(false) => break,
            Err(error) => {
                terminal_error = Some(error);
                break;
            }
        }
    }

    if let Some(pruned) = stable::run_retention_maintenance_if_due(current_time_ns()) {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_retention_maintenance deleted_jobs={} deleted_dedupe={} deleted_inbox={} deleted_outbox={} deleted_turns={} deleted_transitions={} deleted_tools={} generated_session_summaries={} generated_turn_window_summaries={} generated_memory_rollups={} deleted_memory_facts={}",
            pruned.deleted_jobs,
            pruned.deleted_dedupe,
            pruned.deleted_inbox,
            pruned.deleted_outbox,
            pruned.deleted_turns,
            pruned.deleted_transitions,
            pruned.deleted_tools,
            pruned.generated_session_summaries,
            pruned.generated_turn_window_summaries,
            pruned.generated_memory_rollups,
            pruned.deleted_memory_facts
        );
    }

    log!(
        SchedulerLogPriority::Info,
        "scheduler_tick_end processed_jobs={} now={}",
        processed_jobs,
        current_time_ns()
    );
    stable::record_scheduler_tick_end(current_time_ns(), terminal_error);
    crate::http::init_certification();
}

// ── Job dispatch ─────────────────────────────────────────────────────────────

/// Pops the next pending mutating job, acquires its lease, checks the survival
/// policy, dispatches it, and applies the recovery policy on failure.
///
/// Returns `Ok(true)` if a job was processed, `Ok(false)` if the queue is empty
/// or a mutating lease is already active, and `Err` on a terminal lease error.
async fn run_one_pending_mutating_job(now_ns: u64) -> Result<bool, String> {
    if stable::mutating_lease_active(now_ns) {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_tick mutating lease active now={now_ns}",
        );
        return Ok(false);
    }

    let job = match stable::pop_next_pending_job(TaskLane::Mutating, now_ns) {
        Some(job) => {
            log!(
                SchedulerLogPriority::Info,
                "scheduler_tick_dequeue job_id={} kind={:?} lane={:?}",
                job.id,
                job.kind,
                job.lane,
            );
            job
        }
        None => return Ok(false),
    };

    if let Err(error) = stable::acquire_mutating_lease(&job.id, now_ns, lease_ttl_ns(&job.kind)) {
        log!(
            SchedulerLogPriority::Error,
            "scheduler_tick_lease_error job_id={} err={}",
            job.id,
            error
        );
        stable::complete_job(
            &job.id,
            JobStatus::Failed,
            Some(format!("lease acquire failed: {error}")),
            current_time_ns(),
            None,
        );
        return Err(error);
    }

    if let Some(operation_class) = operation_class_for_task(&job.kind) {
        let is_terminal_turn = job.kind == TaskKind::AgentTurn
            && matches!(
                stable::mortality_runtime().phase,
                crate::domain::types::MortalityPhase::TerminalPending
                    | crate::domain::types::MortalityPhase::TerminalInProgress
            );
        if !is_terminal_turn && !stable::can_run_survival_operation(&operation_class, now_ns) {
            let reason = format!(
                "operation blocked by survival policy (operation={:?})",
                operation_class
            );
            stable::complete_job(
                &job.id,
                JobStatus::Skipped,
                Some(reason.clone()),
                current_time_ns(),
                None,
            );
            log!(
                SchedulerLogPriority::Info,
                "scheduler_job_skipped job_id={} kind={:?} operation={:?} reason={reason}",
                job.id,
                job.kind,
                operation_class
            );
            return Ok(true);
        }
    }

    let result = dispatch_job(&job).await;
    match result {
        Ok(outcome) => {
            stable::complete_job(&job.id, JobStatus::Succeeded, None, current_time_ns(), None);
            maybe_enqueue_topup_waiting_continuation(outcome, current_time_ns());
        }
        Err(error) => apply_recovery_policy_for_failed_job(&job, error, current_time_ns()),
    }

    Ok(true)
}

/// Routes a job to the appropriate handler based on its `TaskKind`.
async fn dispatch_job(job: &ScheduledJob) -> Result<JobDispatchOutcome, String> {
    match job.kind {
        TaskKind::AgentTurn => {
            let trigger = if job
                .dedupe_key
                .starts_with("AgentTurn:inference-proxy-resume:")
            {
                ScheduledTurnTrigger::InferenceProxyResume
            } else if job.dedupe_key.starts_with("AgentTurn:plan-continuation:") {
                ScheduledTurnTrigger::PlanContinuation
            } else {
                ScheduledTurnTrigger::Periodic
            };
            run_scheduled_turn_job_with_trigger(trigger).await?;
            Ok(JobDispatchOutcome::Completed)
        }
        TaskKind::PollInbox => {
            run_poll_inbox_job(current_time_ns()).await?;
            Ok(JobDispatchOutcome::Completed)
        }
        TaskKind::CheckCycles => {
            run_check_cycles().await?;
            Ok(JobDispatchOutcome::Completed)
        }
        TaskKind::TopUpCycles => {
            let snapshot = stable::runtime_snapshot();
            let topup = build_cycle_topup(&snapshot)?;
            let done = topup.advance().await?;
            if done {
                Ok(JobDispatchOutcome::Completed)
            } else {
                Ok(JobDispatchOutcome::TopUpWaiting)
            }
        }
        TaskKind::Reconcile => {
            run_reconcile_job(current_time_ns()).await?;
            Ok(JobDispatchOutcome::Completed)
        }
    }
}

/// Runs the strategy reconciliation job: iterates registered templates, disabling
/// stale or provenance-failed entries and activating those that pass dry-run compile.
fn strategy_discovery_exposure_summary() -> String {
    let exposures = stable::list_active_exposures();
    if exposures.is_empty() {
        return "no active exposures tracked".to_string();
    }
    format!("active_exposures={}", exposures.len())
}

fn strategy_discovery_autonomy_summary() -> String {
    let telemetry = stable::cycle_telemetry();
    format!(
        "liquid_cycles={} total_cycles={} survival_tier={:?}",
        telemetry.liquid_cycles,
        telemetry.total_cycles,
        stable::scheduler_survival_tier()
    )
}

async fn run_reconcile_job(now_ns: u64) -> Result<(), String> {
    reconcile_pending_strategy_executions(now_ns).await?;
    let templates = crate::strategy::registry::list_all_templates(STRATEGY_RECONCILE_MAX_TEMPLATES);
    if templates.is_empty() {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_reconcile_strategy empty=true"
        );
    }

    let mut stale_disabled = 0u32;
    let mut provenance_disabled = 0u32;
    let mut dry_run_activated = 0u32;

    for template in templates {
        let key = template.key.clone();

        let age_secs = if now_ns >= template.updated_at_ns {
            now_ns
                .saturating_sub(template.updated_at_ns)
                .checked_div(1_000_000_000)
                .unwrap_or(u64::MAX)
        } else {
            0
        };
        if age_secs > STRATEGY_TEMPLATE_FRESHNESS_WINDOW_SECS {
            let _ = stable::set_strategy_template_activation(TemplateActivationState {
                key: key.clone(),
                enabled: false,
                updated_at_ns: now_ns,
                reason: Some(format!(
                    "stale_template age_secs={age_secs} freshness_window_secs={STRATEGY_TEMPLATE_FRESHNESS_WINDOW_SECS}"
                )),
            });
            stale_disabled = stale_disabled.saturating_add(1);
            continue;
        }

        if let Err(error) = crate::strategy::compiler::dry_run_compile(&key) {
            let _ = stable::set_strategy_template_activation(TemplateActivationState {
                key: key.clone(),
                enabled: false,
                updated_at_ns: now_ns,
                reason: Some(format!("provenance_or_dry_run_failed: {error}")),
            });
            provenance_disabled = provenance_disabled.saturating_add(1);
            continue;
        }

        if !matches!(template.status, TemplateStatus::Active) {
            continue;
        }
        let currently_enabled = stable::strategy_template_activation(&key)
            .map(|state| state.enabled)
            .unwrap_or(false);
        if currently_enabled {
            continue;
        }

        stable::set_strategy_template_activation(TemplateActivationState {
            key,
            enabled: true,
            updated_at_ns: now_ns,
            reason: Some("scheduler dry-run compile passed".to_string()),
        })?;
        dry_run_activated = dry_run_activated.saturating_add(1);
    }

    log!(
        SchedulerLogPriority::Info,
        "scheduler_reconcile_strategy stale_disabled={} provenance_disabled={} dry_run_activated={}",
        stale_disabled,
        provenance_disabled,
        dry_run_activated
    );

    let exposure_status =
        crate::strategy::exposure_reconciliation::reconcile_active_exposures_from_recent_executions(
            now_ns,
        )?;
    log!(
        SchedulerLogPriority::Info,
        "scheduler_reconcile_exposure_state repaired={} recreated={} closed={} drift_reason={:?}",
        exposure_status.repaired_exposures,
        exposure_status.recreated_exposures,
        exposure_status.closed_exposures,
        exposure_status.drift_reason
    );

    let discovery_config = stable::strategy_discovery_worker_config();
    let expired_jobs =
        stable::expire_strategy_discovery_pending_jobs(now_ns, discovery_config.result_ttl_secs);
    if !expired_jobs.is_empty() {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_reconcile_strategy_discovery_expired count={}",
            expired_jobs.len()
        );
    }
    if discovery_config.enabled
        && discovery_config.worker_api_key.is_some()
        && !discovery_config.worker_base_url.trim().is_empty()
        && stable::pending_strategy_discovery_jobs_count() == 0
        && stable::freshest_validated_strategy_discovery_result_for_config(
            &discovery_config,
            now_ns,
        )
        .is_none()
    {
        let pending = crate::features::strategy_discovery::submit_strategy_discovery_job(
            &discovery_config,
            discovery_config.objective.clone(),
            discovery_config.protocol_watchlist.clone(),
            strategy_discovery_exposure_summary(),
            strategy_discovery_autonomy_summary(),
        )
        .await?;
        log!(
            SchedulerLogPriority::Info,
            "scheduler_reconcile_strategy_discovery_submitted job_id={} watchlist_len={}",
            pending.job_id,
            pending.watchlist.len()
        );
    }

    Ok(())
}

async fn reconcile_pending_strategy_executions(now_ns: u64) -> Result<(), String> {
    let pending =
        stable::list_due_pending_strategy_executions(now_ns, PENDING_STRATEGY_EXECUTION_SCAN_LIMIT);
    if pending.is_empty() {
        return Ok(());
    }
    let snapshot = stable::runtime_snapshot();
    let rpc = HttpEvmRpcClient::from_snapshot(&snapshot);
    let required_depth = snapshot.evm_cursor.confirmation_depth.max(1);
    for mut execution in pending {
        // `expected` is the stale-capable snapshot loaded before receipt RPC awaits.
        let expected = execution.clone();
        expire_unattempted_strategy_calls(&mut execution, now_ns);
        let mut transport_error = None;
        for call in execution.calls.iter_mut() {
            if call.state == StrategyExecutionCallState::Unattempted {
                continue;
            }
            if call.state != StrategyExecutionCallState::Submitted {
                continue;
            }
            let Some(tx_hash) = call.tx_hash.as_deref() else {
                continue;
            };
            let observation = match rpc.as_ref() {
                Ok(rpc) => rpc.strategy_receipt_observation(tx_hash).await,
                Err(error) => Err(error.clone()),
            };
            match observation {
                Ok(observation) => {
                    apply_strategy_receipt_observation(
                        call,
                        observation,
                        now_ns,
                        execution.created_at_ns,
                        required_depth,
                    )?;
                }
                Err(error) => {
                    call.last_checked_at_ns = Some(now_ns);
                    call.error = Some(error.clone());
                    transport_error = Some(error);
                    break;
                }
            }
        }
        execution.updated_at_ns = now_ns;
        if let Some(error) = transport_error {
            execution.consecutive_rpc_failures =
                execution.consecutive_rpc_failures.saturating_add(1);
            let backoff = strategy_execution_rpc_backoff_ns(execution.consecutive_rpc_failures);
            execution.next_check_at_ns = now_ns.saturating_add(backoff);
            let _ = stable::compare_and_update_pending_strategy_execution(&expected, execution)?;
            log!(
                SchedulerLogPriority::Error,
                "strategy receipt reconciliation backed off: {error}"
            );
            continue;
        }
        execution.consecutive_rpc_failures = 0;
        let confirmed = execution
            .calls
            .iter()
            .filter(|call| call.state == StrategyExecutionCallState::Confirmed)
            .count();
        let reverted = execution
            .calls
            .iter()
            .any(|call| call.state == StrategyExecutionCallState::Reverted);
        let dropped = execution
            .calls
            .iter()
            .any(|call| call.state == StrategyExecutionCallState::Dropped);
        if reverted || dropped {
            execution.state = if confirmed > 0 {
                PendingStrategyExecutionState::PartialFailure
            } else if reverted {
                PendingStrategyExecutionState::Reverted
            } else {
                PendingStrategyExecutionState::Dropped
            };
            execution.next_check_at_ns = now_ns;
            let reason = if reverted {
                "strategy transaction reverted"
            } else {
                "strategy transaction dropped"
            };
            let kind = if reverted {
                crate::domain::types::StrategyOutcomeKind::DeterministicFailure
            } else {
                crate::domain::types::StrategyOutcomeKind::NondeterministicFailure
            };
            crate::tools::apply_terminal_strategy_failure(&mut execution, kind, reason, now_ns)?;
        } else if confirmed == execution.calls.len() && !execution.calls.is_empty() {
            crate::tools::apply_confirmed_strategy_execution(&mut execution, now_ns)?;
        } else {
            execution.state = PendingStrategyExecutionState::Pending;
            execution.next_check_at_ns = now_ns.saturating_add(STRATEGY_EXECUTION_RECHECK_NS);
            let _ = stable::compare_and_update_pending_strategy_execution(&expected, execution)?;
        }
    }
    Ok(())
}

fn expire_unattempted_strategy_calls(
    execution: &mut crate::domain::types::PendingStrategyExecution,
    now_ns: u64,
) {
    if now_ns.saturating_sub(execution.created_at_ns) < STRATEGY_EXECUTION_RECEIPT_TIMEOUT_NS {
        return;
    }
    for call in &mut execution.calls {
        if call.state == StrategyExecutionCallState::Unattempted
            || (call.state == StrategyExecutionCallState::Submitted && call.tx_hash.is_none())
        {
            call.state = StrategyExecutionCallState::Dropped;
            call.last_checked_at_ns = Some(now_ns);
            call.error = Some("unattempted call recovery timeout exceeded".to_string());
        }
    }
}

fn strategy_execution_rpc_backoff_ns(consecutive_failures: u32) -> u64 {
    let shift = consecutive_failures.min(8);
    STRATEGY_EXECUTION_RECHECK_NS
        .saturating_mul(1u64 << shift)
        .min(STRATEGY_EXECUTION_MAX_BACKOFF_NS)
}

fn apply_strategy_receipt_observation(
    call: &mut crate::domain::types::PendingStrategyExecutionCall,
    observation: StrategyReceiptObservation,
    now_ns: u64,
    execution_created_at_ns: u64,
    required_depth: u64,
) -> Result<(), String> {
    call.last_checked_at_ns = Some(now_ns);
    call.receipt_block_number = observation.block_number;
    call.receipt_block_hash = observation.block_hash;
    call.error = None;
    match observation.status {
        TransactionReceiptStatus::Pending => {
            let submitted = call.submitted_at_ns.unwrap_or(execution_created_at_ns);
            if now_ns.saturating_sub(submitted) >= STRATEGY_EXECUTION_RECEIPT_TIMEOUT_NS {
                call.state = StrategyExecutionCallState::Dropped;
                call.error = Some("receipt timeout exceeded".to_string());
            }
        }
        TransactionReceiptStatus::Reverted => {
            call.state = StrategyExecutionCallState::Reverted;
            call.error = Some("transaction receipt status was 0x0".to_string());
        }
        TransactionReceiptStatus::Confirmed => {
            let receipt_block = observation
                .block_number
                .ok_or_else(|| "confirmed strategy receipt missing block number".to_string())?;
            if receipt_block > observation.latest_block {
                return Err(format!(
                    "receipt block {receipt_block} is greater than latest head {}",
                    observation.latest_block
                ));
            }
            let confirmations = observation
                .latest_block
                .checked_sub(receipt_block)
                .and_then(|distance| distance.checked_add(1))
                .ok_or_else(|| "strategy receipt confirmation arithmetic overflow".to_string())?;
            if confirmations >= required_depth.max(1) {
                call.state = StrategyExecutionCallState::Confirmed;
            }
        }
    }
    Ok(())
}

/// If `outcome` is `TopUpWaiting`, enqueues a continuation `TopUpCycles` job
/// scheduled one task-interval into the future so the multi-stage top-up
/// resumes on the next eligible tick.
fn maybe_enqueue_topup_waiting_continuation(outcome: JobDispatchOutcome, now_ns: u64) {
    if !matches!(outcome, JobDispatchOutcome::TopUpWaiting) {
        return;
    }

    let interval_secs = stable::get_task_config(&TaskKind::TopUpCycles)
        .map(|config| config.interval_secs.max(1))
        .unwrap_or(TaskKind::TopUpCycles.default_interval_secs().max(1));
    let continuation_hint_ns = now_ns.saturating_add(interval_secs.saturating_mul(1_000_000_000));
    let enqueued = enqueue_topup_cycles_job("wait", continuation_hint_ns).is_some();
    log!(
        SchedulerLogPriority::Info,
        "scheduler_topup_waiting_continuation enqueued={} continuation_hint_ns={}",
        enqueued,
        continuation_hint_ns
    );
}

// ── Survival policy helpers ───────────────────────────────────────────────────

/// Maps a task kind + error string to a `RecoveryFailure` classification used
/// by the recovery policy to decide the appropriate action.
fn classify_failure_for_task(kind: &TaskKind, error: &str) -> RecoveryFailure {
    match kind {
        TaskKind::AgentTurn => classify_inference_failure(error),
        TaskKind::PollInbox => classify_evm_failure(error),
        TaskKind::CheckCycles | TaskKind::TopUpCycles | TaskKind::Reconcile => {
            RecoveryFailure::Operation(OperationFailure {
                kind: OperationFailureKind::Unknown,
            })
        }
    }
}

/// Returns `true` when `error` originated from an `eth_getLogs` poll call.
fn is_eth_get_logs_failure(error: &str) -> bool {
    error.to_ascii_lowercase().contains("eth_getlogs")
}

/// Maps a task kind to the `RecoveryOperation` tag used in recovery contexts.
fn recovery_operation_for_task(kind: &TaskKind) -> RecoveryOperation {
    match kind {
        TaskKind::AgentTurn => RecoveryOperation::Inference,
        TaskKind::PollInbox => RecoveryOperation::EvmPoll,
        TaskKind::CheckCycles | TaskKind::TopUpCycles | TaskKind::Reconcile => {
            RecoveryOperation::Unknown
        }
    }
}

/// Builds a `RecoveryContext` for `job`, pulling consecutive-failure counters,
/// backoff caps, and response-limit policies from stable storage.
fn recovery_context_for_task_job(job: &ScheduledJob) -> RecoveryContext {
    let task_runtime = stable::get_task_runtime(&job.kind);
    let task_config = stable::get_task_config(&job.kind)
        .unwrap_or_else(|| crate::domain::types::TaskScheduleConfig::default_for(&job.kind));
    let snapshot = stable::runtime_snapshot();

    let response_limit = if job.kind == TaskKind::PollInbox {
        Some(ResponseLimitPolicy {
            current_bytes: snapshot.evm_rpc_max_response_bytes,
            min_bytes: RESPONSE_BYTES_POLICY_MIN,
            max_bytes: EVM_RPC_MAX_RESPONSE_BYTES_POLICY_MAX,
            tune_multiplier: 2,
        })
    } else {
        None
    };

    RecoveryContext {
        operation: recovery_operation_for_task(&job.kind),
        consecutive_failures: task_runtime.consecutive_failures,
        backoff_base_secs: RECOVERY_BACKOFF_BASE_SECS,
        backoff_max_secs: task_config.max_backoff_secs,
        response_limit,
    }
}

/// Applies a `ResponseLimitAdjustment` for `operation` by persisting the new
/// `max_response_bytes` to stable storage.
fn apply_response_limit_tuning(
    operation: &RecoveryOperation,
    adjustment: &ResponseLimitAdjustment,
) -> Result<(), String> {
    match operation {
        RecoveryOperation::EvmPoll => {
            stable::set_evm_rpc_max_response_bytes(adjustment.to_bytes).map(|_| ())
        }
        RecoveryOperation::WalletBalanceSync => {
            let mut config = stable::wallet_balance_sync_config();
            config.max_response_bytes = adjustment.to_bytes;
            stable::set_wallet_balance_sync_config(config).map(|_| ())
        }
        _ => Err("response limit tuning is not supported for this operation".to_string()),
    }
}

/// Runs the full recovery policy pipeline for a failed job: classifies the
/// failure, decides the action (skip / retry-immediate / backoff / tune
/// response-limit / escalate), and completes the job record accordingly.
fn apply_recovery_policy_for_failed_job(job: &ScheduledJob, error: String, now_ns: u64) {
    let defer_poll_inbox_retry_to_next_slot =
        job.kind == TaskKind::PollInbox && is_eth_get_logs_failure(&error);
    let failure = classify_failure_for_task(&job.kind, &error);
    let context = recovery_context_for_task_job(job);
    let decision = decide_recovery_action(&failure, &context);

    let mut status = JobStatus::Failed;
    let mut retry_after_secs = None;
    let mut final_error = error;

    match decision.action {
        RecoveryPolicyAction::Skip => {
            status = JobStatus::Skipped;
        }
        RecoveryPolicyAction::RetryImmediate => {
            retry_after_secs = Some(0);
        }
        RecoveryPolicyAction::Backoff => {
            retry_after_secs = decision.backoff_secs.or(Some(1));
        }
        RecoveryPolicyAction::TuneResponseLimit => {
            if let Some(adjustment) = decision.response_limit_adjustment.as_ref() {
                if let Err(tune_error) = apply_response_limit_tuning(&context.operation, adjustment)
                {
                    final_error = format!(
                        "{final_error}; response_limit_tune_failed {}->{}: {tune_error}",
                        adjustment.from_bytes, adjustment.to_bytes
                    );
                } else {
                    retry_after_secs = Some(0);
                }
            } else {
                final_error = format!("{final_error}; response limit adjustment missing");
            }
        }
        RecoveryPolicyAction::EscalateFault => {}
    }

    if defer_poll_inbox_retry_to_next_slot {
        status = JobStatus::Skipped;
        retry_after_secs = None;
    }

    log!(
        SchedulerLogPriority::Info,
        "scheduler_job_recovery_decision job_id={} kind={:?} action={:?} reason={:?} retry_after_secs={:?} backoff_secs={:?}",
        job.id,
        job.kind,
        decision.action,
        decision.reason,
        retry_after_secs,
        decision.backoff_secs
    );

    stable::complete_job(&job.id, status, Some(final_error), now_ns, retry_after_secs);
}

// ── Recovery ─────────────────────────────────────────────────────────────────

/// Serialises an `EvmEvent` as a JSON fallback body when ABI decoding fails.
fn evm_event_to_inbox_body(event: &EvmEvent) -> String {
    json!({
        "source": "evm_log",
        "tx_hash": event.tx_hash,
        "chain_id": event.chain_id,
        "block_number": event.block_number,
        "log_index": event.log_index,
        "address": event.source,
        "data": event.payload,
    })
    .to_string()
}

/// Decodes an `EvmEvent` into an `(inbox_body, sender)` pair.
/// Falls back to the raw JSON envelope when ABI decoding fails.
fn evm_event_to_inbox_message(event: &EvmEvent) -> (String, String) {
    match decode_message_queued_payload(&event.payload) {
        Ok(decoded) => {
            let bounded = stable::normalize_inbox_body(&decoded.message)
                .unwrap_or_else(|_| "[invalid decoded message]".to_string());
            (bounded, decoded.sender)
        }
        Err(error) => {
            log!(
                SchedulerLogPriority::Error,
                "scheduler_poll_inbox_decode_failed tx_hash={} log_index={} error={}",
                event.tx_hash,
                event.log_index,
                error
            );
            let fallback = stable::normalize_inbox_body(&evm_event_to_inbox_body(event))
                .unwrap_or_else(|_| "[evm log decode failed]".to_string());
            (fallback, event.source.clone())
        }
    }
}

fn ingest_inbox_message(
    body: String,
    sender: String,
    source: InboxMessageSource,
) -> Result<String, String> {
    stable::post_inbox_message_with_source(body, sender, source)
}

fn ingest_verified_patronage_event(event: &EvmEvent) -> Result<Option<u128>, String> {
    let Ok(amount) = crate::features::evm::decode_patronage_payload(&event.payload) else {
        return Ok(None);
    };
    let amount_raw = amount
        .to_string()
        .parse::<u128>()
        .map_err(|_| "patronage amount exceeds supported counter range".to_string())?;
    stable::record_verified_patronage_usdc(amount_raw)?;
    Ok(Some(amount_raw))
}

/// Ingests a direct steward message through the same inbox path used by EVM
/// event delivery, then promotes pending messages into the staged queue.
pub(crate) fn ingest_steward_direct_message(
    sender: String,
    message: String,
) -> Result<String, String> {
    let inbox_id = ingest_inbox_message(message, sender, InboxMessageSource::StewardDirect)?;
    let staged =
        stable::stage_pending_inbox_messages(POLL_INBOX_STAGE_BATCH_SIZE, current_time_ns());
    log!(
        SchedulerLogPriority::Info,
        "scheduler_steward_direct_ingested id={} staged={}",
        inbox_id,
        staged
    );
    Ok(inbox_id)
}

/// Enqueues an immediate `AgentTurn` job if no non-terminal job already exists
/// for the provided dedupe key.
pub(crate) fn enqueue_immediate_agent_turn_job_if_absent(dedupe_key: String) -> Option<String> {
    stable::enqueue_job_if_absent(
        TaskKind::AgentTurn,
        TaskLane::Mutating,
        dedupe_key,
        current_time_ns(),
        stable::agent_turn_priority(),
    )
}

/// Returns the minimum delay in nanoseconds before the next EVM poll, based on
/// the number of consecutive empty polls and the `EMPTY_POLL_BACKOFF_SCHEDULE_SECS`.
fn empty_poll_backoff_delay_ns(consecutive_empty_polls: u32) -> u64 {
    // Use the first backoff slot for both "no empties yet" and "first empty poll observed".
    // This keeps the first empty-poll retry window at 60s instead of jumping to 120s.
    let schedule_idx = consecutive_empty_polls.saturating_sub(1);
    let idx = usize::try_from(schedule_idx).unwrap_or(usize::MAX);
    let secs = timing::EMPTY_POLL_BACKOFF_SCHEDULE_SECS
        .get(idx)
        .copied()
        .unwrap_or(
            *timing::EMPTY_POLL_BACKOFF_SCHEDULE_SECS
                .last()
                .unwrap_or(&300),
        );
    secs.saturating_mul(1_000_000_000)
}

/// Returns `true` when enough time has elapsed since `last_poll_at_ns` given
/// the current empty-poll backoff level.
fn poll_inbox_rpc_due(now_ns: u64, last_poll_at_ns: u64, consecutive_empty_polls: u32) -> bool {
    if last_poll_at_ns == 0 {
        return true;
    }
    let min_delay_ns = empty_poll_backoff_delay_ns(consecutive_empty_polls);
    now_ns >= last_poll_at_ns.saturating_add(min_delay_ns)
}

fn room_poll_due(now_ns: u64, last_attempted_at_ns: Option<u64>) -> bool {
    let Some(last_attempted_at_ns) = last_attempted_at_ns else {
        return true;
    };
    let interval_ns = ROOM_POLL_INTERVAL_SECS.saturating_mul(1_000_000_000);
    now_ns >= last_attempted_at_ns.saturating_add(interval_ns)
}

async fn refresh_peer_directory(
    now_ns: u64,
    snapshot: &RuntimeSnapshot,
    client: &FactoryRoomClient,
) {
    let Ok(page) = client.list_peers(None, PEER_DIRECTORY_PAGE_LIMIT).await else {
        return;
    };
    let mut peers = Vec::with_capacity(page.items.len());
    for peer in page.items {
        let price = match fetch_peer_min_prices(snapshot, &peer.evm_address).await {
            Ok(value) => {
                json!({ "status": "verified", "usdc_min_raw": value.usdc_min_raw, "eth_min_wei": value.eth_min_wei, "uses_default": value.uses_default })
            }
            Err(error) => {
                json!({ "status": "unavailable", "usdc_min_raw": null, "eth_min_wei": null, "error": error })
            }
        };
        peers.push(json!({ "canister_id": peer.canister_id, "name": peer.name, "evm_address": peer.evm_address, "alive": peer.death_cause.is_none(), "parent_id": peer.parent_id, "generation": peer.generation, "price_of_attention": price }));
    }
    let value =
        json!({ "peers": peers, "bounded": true, "next_cursor": page.next_cursor }).to_string();
    let created_at_ns = stable::get_memory_fact("society.peer_directory")
        .map(|fact| fact.created_at_ns)
        .unwrap_or(now_ns);
    let _ = stable::set_memory_fact(&MemoryFact {
        key: "society.peer_directory".to_string(),
        value,
        created_at_ns,
        updated_at_ns: now_ns,
        source_turn_id: "room_poll".to_string(),
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReproductionPaymentRecoveryAction {
    Wait,
    BroadcastApproval,
    BroadcastDeposit,
}

fn reproduction_payment_recovery_action(
    factory_state: &ReproductionSessionState,
    approve_hash: Option<&str>,
    approve_receipt: Option<TransactionReceiptStatus>,
    deposit_hash: Option<&str>,
    deposit_receipt: Option<TransactionReceiptStatus>,
) -> ReproductionPaymentRecoveryAction {
    if !matches!(factory_state, ReproductionSessionState::AwaitingPayment) {
        return ReproductionPaymentRecoveryAction::Wait;
    }
    match (approve_hash, approve_receipt) {
        (None, _) | (Some(_), Some(TransactionReceiptStatus::Reverted)) => {
            ReproductionPaymentRecoveryAction::BroadcastApproval
        }
        (Some(_), Some(TransactionReceiptStatus::Confirmed)) => match deposit_hash {
            Some(_) if deposit_receipt == Some(TransactionReceiptStatus::Reverted) => {
                ReproductionPaymentRecoveryAction::BroadcastDeposit
            }
            Some(_) => ReproductionPaymentRecoveryAction::Wait,
            None => ReproductionPaymentRecoveryAction::BroadcastDeposit,
        },
        _ => ReproductionPaymentRecoveryAction::Wait,
    }
}

fn prepare_reproduction_payment_retry_value(
    value: &serde_json::Value,
    action: ReproductionPaymentRecoveryAction,
) -> serde_json::Value {
    let mut prepared = value.clone();
    if action == ReproductionPaymentRecoveryAction::BroadcastApproval {
        if let Some(object) = prepared.as_object_mut() {
            // A deposit signed after a reverted approval necessarily depended
            // on stale allowance/nonce state and must not suppress reapproval.
            object.remove("approve_tx_hash");
            object.remove("deposit_tx_hash");
        }
    }
    prepared
}

fn persist_reproduction_payment_value(
    fact: &MemoryFact,
    mut value: serde_json::Value,
    now_ns: u64,
    status: &str,
    field: Option<(&str, String)>,
    error: Option<String>,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("status".to_string(), json!(status));
    if let Some((name, field_value)) = field {
        object.insert(name.to_string(), json!(field_value));
    }
    if let Some(error) = error {
        object.insert("last_error".to_string(), json!(error));
    } else {
        object.remove("last_error");
    }
    let _ = stable::set_memory_fact(&MemoryFact {
        value: value.to_string(),
        updated_at_ns: now_ns,
        source_turn_id: "reproduction_payment_recovery".to_string(),
        ..fact.clone()
    });
}

async fn recover_reproduction_payment(
    now_ns: u64,
    snapshot: &RuntimeSnapshot,
    fact: &MemoryFact,
    value: &serde_json::Value,
    factory_state: &ReproductionSessionState,
) {
    let approve_hash = value
        .get("approve_tx_hash")
        .and_then(serde_json::Value::as_str);
    let deposit_hash = value
        .get("deposit_tx_hash")
        .and_then(serde_json::Value::as_str);
    let approve_receipt = match approve_hash {
        Some(hash) => fetch_transaction_receipt_status(snapshot, hash).await.ok(),
        None => None,
    };
    let deposit_receipt = match deposit_hash {
        Some(hash) => fetch_transaction_receipt_status(snapshot, hash).await.ok(),
        None => None,
    };
    let action = reproduction_payment_recovery_action(
        factory_state,
        approve_hash,
        approve_receipt,
        deposit_hash,
        deposit_receipt,
    );
    if action == ReproductionPaymentRecoveryAction::Wait {
        return;
    }
    let recovery_value = prepare_reproduction_payment_retry_value(value, action);
    if action == ReproductionPaymentRecoveryAction::BroadcastApproval {
        // Persist the cleared dependency chain before the outcall so a crash
        // cannot revive a stale deposit hash on the next scheduler tick.
        persist_reproduction_payment_value(
            fact,
            recovery_value.clone(),
            now_ns,
            "approval_rebroadcasting",
            None,
            None,
        );
    }
    let Some(token) = value.get("token").and_then(serde_json::Value::as_str) else {
        return;
    };
    let Some(escrow) = value.get("escrow").and_then(serde_json::Value::as_str) else {
        return;
    };
    let Some(claim_id) = value.get("claim_id").and_then(serde_json::Value::as_str) else {
        return;
    };
    let Some(gross) = value
        .get("gross_amount")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| U256::from_str(raw).ok())
    else {
        return;
    };
    if gross.is_zero() {
        return;
    }
    let args = match action {
        ReproductionPaymentRecoveryAction::BroadcastApproval => {
            reproduction_approve_args(token, escrow, gross)
        }
        ReproductionPaymentRecoveryAction::BroadcastDeposit => {
            reproduction_deposit_args(escrow, claim_id, gross)
        }
        ReproductionPaymentRecoveryAction::Wait => return,
    };
    let args = match args {
        Ok(args) => args,
        Err(error) => {
            persist_reproduction_payment_value(
                fact,
                recovery_value.clone(),
                now_ns,
                "payment_recovery_failed",
                None,
                Some(error),
            );
            return;
        }
    };
    let signer = ThresholdSignerAdapter::new(snapshot.ecdsa_key_name.clone());
    match crate::features::evm::send_eth_tool(&args, &signer).await {
        Ok(hash) => {
            let (status, field) = match action {
                ReproductionPaymentRecoveryAction::BroadcastApproval => {
                    ("approval_broadcast", "approve_tx_hash")
                }
                ReproductionPaymentRecoveryAction::BroadcastDeposit => {
                    ("deposit_broadcast", "deposit_tx_hash")
                }
                ReproductionPaymentRecoveryAction::Wait => return,
            };
            persist_reproduction_payment_value(
                fact,
                recovery_value.clone(),
                now_ns,
                status,
                Some((field, hash)),
                None,
            );
        }
        Err(error) => persist_reproduction_payment_value(
            fact,
            recovery_value,
            now_ns,
            "payment_recovery_retryable",
            None,
            Some(error),
        ),
    }
}

async fn reconcile_reproduction_sessions(
    now_ns: u64,
    snapshot: &RuntimeSnapshot,
    client: &FactoryRoomClient,
) {
    for fact in stable::list_memory_facts_by_prefix_sorted(
        "counterparty.child.pending.",
        8,
        MemoryFactSort::UpdatedAtDesc,
    ) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&fact.value) else {
            continue;
        };
        let Some(session_id) = value
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(status) = client.get_reproduction_session(&session_id).await else {
            continue;
        };
        match status.session.state {
            ReproductionSessionState::Complete => {
                let Some(child_id) = status.session.automaton_canister_id else {
                    continue;
                };
                let _ = stable::set_memory_fact(&MemoryFact {
                    key: format!("counterparty.child.{}.birth.{}", child_id.to_ascii_lowercase(), session_id),
                    value: json!({ "kind": "reproduction", "session_id": session_id, "child_canister_id": child_id, "generation": status.session.generation, "status": "born" }).to_string(),
                    created_at_ns: fact.created_at_ns,
                    updated_at_ns: now_ns,
                    source_turn_id: "reproduction_reconciler".to_string(),
                });
                stable::remove_memory_fact(&fact.key);
            }
            ReproductionSessionState::Failed | ReproductionSessionState::Expired => {
                let _ = stable::set_memory_fact(&MemoryFact {
                    value: json!({ "kind": "reproduction", "session_id": session_id, "status": format!("{:?}", status.session.state).to_ascii_lowercase() }).to_string(),
                    updated_at_ns: now_ns,
                    ..fact
                });
            }
            ReproductionSessionState::AwaitingPayment => {
                recover_reproduction_payment(
                    now_ns,
                    snapshot,
                    &fact,
                    &value,
                    &status.session.state,
                )
                .await;
            }
            _ => {}
        }
    }
}

async fn reconcile_submitted_peer_payments(now_ns: u64, snapshot: &RuntimeSnapshot) {
    for fact in stable::list_memory_facts_by_prefix_sorted(
        "counterparty_pending_receipt.",
        PENDING_RECEIPT_SCAN_LIMIT,
        MemoryFactSort::UpdatedAtDesc,
    ) {
        let Ok(alias) = serde_json::from_str::<serde_json::Value>(&fact.value) else {
            continue;
        };
        let Some(tx_hash) = alias
            .get("tx_hash")
            .and_then(|item| item.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        if counterparty_pending_receipt_key(&tx_hash) != fact.key {
            continue;
        }
        let Ok(status) = fetch_transaction_receipt_status(snapshot, &tx_hash).await else {
            continue;
        };
        let _ = reconcile_pending_receipt_alias(&fact.key, status, now_ns);
    }
}

fn reconcile_pending_receipt_alias(
    alias_key: &str,
    status: TransactionReceiptStatus,
    now_ns: u64,
) -> bool {
    let Some(alias) = stable::get_memory_fact(alias_key) else {
        return false;
    };
    let Ok(alias_value) = serde_json::from_str::<serde_json::Value>(&alias.value) else {
        return false;
    };
    let Some(deal_key) = alias_value.get("deal_key").and_then(|item| item.as_str()) else {
        return false;
    };
    let Some(deal) = stable::get_memory_fact(deal_key) else {
        return false;
    };
    let Ok(mut deal_value) = serde_json::from_str::<serde_json::Value>(&deal.value) else {
        return false;
    };
    if status == TransactionReceiptStatus::Pending {
        return false;
    }
    if !persist_peer_payment_receipt_status(&mut deal_value, status, now_ns) {
        return false;
    }
    stable::remove_memory_fact(alias_key)
}

fn persist_peer_payment_receipt_status(
    value: &mut serde_json::Value,
    status: TransactionReceiptStatus,
    now_ns: u64,
) -> bool {
    if !apply_peer_payment_receipt_status(value, status) {
        return false;
    }
    let peer_id = value
        .get("peer_id")
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    let tx_hash = value
        .get("tx_hash")
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    !peer_id.is_empty()
        && !tx_hash.is_empty()
        && record_counterparty_deal(
            peer_id,
            tx_hash,
            &value.to_string(),
            now_ns,
            "payment_reconcile",
        )
        .is_ok()
}

fn apply_peer_payment_receipt_status(
    value: &mut serde_json::Value,
    status: TransactionReceiptStatus,
) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    if object.get("status").and_then(|item| item.as_str()) != Some("submitted") {
        return false;
    }
    match status {
        TransactionReceiptStatus::Pending => false,
        TransactionReceiptStatus::Confirmed => {
            object.insert("status".to_string(), json!("paid"));
            object.insert("assessment".to_string(), json!("awaiting_delivery"));
            true
        }
        TransactionReceiptStatus::Reverted => {
            object.insert("status".to_string(), json!("failed"));
            object.insert("assessment".to_string(), json!("transaction_reverted"));
            true
        }
    }
}

async fn load_peers_for_incoming_events(client: &FactoryRoomClient) -> Vec<FactoryPeer> {
    let mut peers = Vec::new();
    let mut cursor = None;
    while peers.len() < PEER_LOOKUP_HARD_CAP {
        let Ok(page) = client.list_peers(cursor, 50).await else {
            break;
        };
        peers.extend(
            page.items
                .into_iter()
                .take(PEER_LOOKUP_HARD_CAP - peers.len()),
        );
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    peers
}

async fn maybe_poll_factory_room(now_ns: u64, snapshot: &RuntimeSnapshot) {
    if snapshot
        .factory_principal
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        return;
    }

    if !room_poll_due(now_ns, snapshot.room_poll.last_attempted_at_ns) {
        let next_due_ns = snapshot
            .room_poll
            .last_attempted_at_ns
            .unwrap_or(0)
            .saturating_add(ROOM_POLL_INTERVAL_SECS.saturating_mul(1_000_000_000));
        log!(
            SchedulerLogPriority::Info,
            "scheduler_room_poll_skipped reason=due_not_reached now_ns={} next_due_ns={} last_seen_seq={:?}",
            now_ns,
            next_due_ns,
            snapshot.room_poll.last_seen_seq
        );
        return;
    }

    let client = match FactoryRoomClient::from_runtime() {
        Ok(client) => client,
        Err(error) => {
            stable::record_room_poll_error(now_ns, error.clone());
            log!(
                SchedulerLogPriority::Error,
                "scheduler_room_poll_error stage=client_init last_seen_seq={:?} error={}",
                snapshot.room_poll.last_seen_seq,
                error
            );
            return;
        }
    };

    if snapshot.inbox_contract_address.is_some() && !snapshot.evm_rpc_url.trim().is_empty() {
        refresh_peer_directory(now_ns, snapshot, &client).await;
        reconcile_submitted_peer_payments(now_ns, snapshot).await;
    }
    reconcile_reproduction_sessions(now_ns, snapshot, &client).await;

    match client
        .list_my_room_messages(snapshot.room_poll.last_seen_seq, Some(ROOM_POLL_PAGE_LIMIT))
        .await
    {
        Ok(page) => {
            let fetched = page.messages.len();
            let observations_loaded = stable::store_room_observations(&page.messages);
            let last_seen_seq = if page.next_after_seq.is_none() {
                page.latest_seq
                    .or_else(|| page.messages.last().map(|message| message.seq))
                    .or(snapshot.room_poll.last_seen_seq)
            } else {
                page.messages
                    .last()
                    .map(|message| message.seq)
                    .or(snapshot.room_poll.last_seen_seq)
            };
            let room_head_gap = page
                .latest_seq
                .zip(last_seen_seq)
                .map(|(latest_seq, seen_seq)| latest_seq.saturating_sub(seen_seq));
            stable::record_room_poll_success(now_ns, last_seen_seq, page.latest_seq, fetched);
            log!(
                SchedulerLogPriority::Info,
                "scheduler_room_poll_success fetched={} observations_loaded={} last_seen_seq={:?} latest_seq={:?} room_head_gap={:?}",
                fetched,
                observations_loaded,
                last_seen_seq,
                page.latest_seq,
                room_head_gap
            );
        }
        Err(error) => {
            stable::record_room_poll_error(now_ns, error.clone());
            log!(
                SchedulerLogPriority::Error,
                "scheduler_room_poll_error stage=list_my_room_messages last_seen_seq={:?} error={}",
                snapshot.room_poll.last_seen_seq,
                error
            );
        }
    }
}

/// Returns the wallet-balance sync interval for the current survival tier,
/// or `None` if syncing is disabled or the tier prohibits it (Critical / OutOfCycles).
fn wallet_balance_sync_interval_secs(
    snapshot: &RuntimeSnapshot,
    tier: &SurvivalTier,
) -> Option<u64> {
    if !snapshot.wallet_balance_sync.enabled {
        return None;
    }

    match tier {
        SurvivalTier::Normal => Some(snapshot.wallet_balance_sync.normal_interval_secs),
        SurvivalTier::LowCycles => Some(snapshot.wallet_balance_sync.low_cycles_interval_secs),
        SurvivalTier::Critical | SurvivalTier::OutOfCycles => None,
    }
}

/// Returns `true` when a wallet balance sync should be attempted now —
/// either because the bootstrap gate is pending or because the freshness
/// interval has elapsed since the last successful sync.
fn wallet_balance_sync_due(snapshot: &RuntimeSnapshot, now_ns: u64, interval_secs: u64) -> bool {
    if snapshot.wallet_balance_bootstrap_pending {
        return true;
    }

    let Some(last_synced_at_ns) = snapshot.wallet_balance.last_synced_at_ns else {
        return true;
    };
    let due_ns = interval_secs.saturating_mul(1_000_000_000);
    now_ns >= last_synced_at_ns.saturating_add(due_ns)
}

/// Builds a `RecoveryContext` for wallet-balance sync failures, wiring in
/// the current `max_response_bytes` and the sync-specific backoff cap.
fn wallet_sync_recovery_context(snapshot: &RuntimeSnapshot) -> RecoveryContext {
    let consecutive_failures = u32::from(snapshot.wallet_balance.last_error.is_some());
    RecoveryContext {
        operation: RecoveryOperation::WalletBalanceSync,
        consecutive_failures,
        backoff_base_secs: RECOVERY_BACKOFF_BASE_SECS,
        backoff_max_secs: stable::SURVIVAL_OPERATION_MAX_BACKOFF_SECS_EVM_POLL,
        response_limit: Some(ResponseLimitPolicy {
            current_bytes: snapshot.wallet_balance_sync.max_response_bytes,
            min_bytes: RESPONSE_BYTES_POLICY_MIN,
            max_bytes: WALLET_SYNC_MAX_RESPONSE_BYTES_RECOVERY_MAX,
            tune_multiplier: 2,
        }),
    }
}

/// Conditionally fetches and persists the latest wallet balances.
///
/// Skips when syncing is disabled, the canister is on a Critical/OutOfCycles
/// tier, the required address routes are not yet configured, or the freshness
/// interval has not elapsed.  On failure, applies the sync recovery policy
/// (response-limit tuning / immediate retry) before persisting the error.
async fn maybe_sync_wallet_balances(now_ns: u64, snapshot: &RuntimeSnapshot) {
    let tier = stable::scheduler_survival_tier();
    let Some(interval_secs) = wallet_balance_sync_interval_secs(snapshot, &tier) else {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_wallet_balance_sync_skipped reason=disabled_or_tier tier={:?}",
            tier
        );
        return;
    };
    if !stable::wallet_balance_sync_capable(snapshot) {
        if snapshot.wallet_balance_bootstrap_pending {
            stable::set_wallet_balance_bootstrap_pending(false);
        }
        log!(
            SchedulerLogPriority::Info,
            "scheduler_wallet_balance_sync_skipped reason=not_configured_or_incomplete_route tier={:?}",
            tier
        );
        return;
    }
    if !wallet_balance_sync_due(snapshot, now_ns, interval_secs) {
        let next_due_ns = snapshot
            .wallet_balance
            .last_synced_at_ns
            .unwrap_or(0)
            .saturating_add(interval_secs.saturating_mul(1_000_000_000));
        log!(
            SchedulerLogPriority::Info,
            "scheduler_wallet_balance_sync_skipped reason=due_not_reached now_ns={} next_due_ns={} tier={:?}",
            now_ns,
            next_due_ns,
            tier
        );
        return;
    }

    let mut sync_snapshot = snapshot.clone();
    if sync_snapshot.evm_address.is_none() && !sync_snapshot.ecdsa_key_name.trim().is_empty() {
        match crate::features::threshold_signer::derive_and_cache_evm_address(
            &sync_snapshot.ecdsa_key_name,
        )
        .await
        {
            Ok(derived_address) => {
                sync_snapshot.evm_address = Some(derived_address);
            }
            Err(error) => {
                stable::record_wallet_balance_sync_error(format!(
                    "wallet address derivation failed: {error}"
                ));
                log!(
                    SchedulerLogPriority::Error,
                    "scheduler_wallet_balance_sync_error stage=derive_wallet_address error={}",
                    error
                );
                return;
            }
        }
    }

    match fetch_wallet_balance_sync_read(&sync_snapshot).await {
        Ok(read) => {
            stable::record_wallet_balance_sync_success(
                now_ns,
                read.eth_balance_wei_hex,
                read.usdc_balance_raw_hex,
                read.usdc_contract_address,
            );
            log!(
                SchedulerLogPriority::Info,
                "scheduler_wallet_balance_sync_success now_ns={} bootstrap_pending_cleared={}",
                now_ns,
                snapshot.wallet_balance_bootstrap_pending
            );
        }
        Err(error) => {
            let failure = classify_evm_failure(&error);
            let decision =
                decide_recovery_action(&failure, &wallet_sync_recovery_context(&sync_snapshot));
            let mut retry_snapshot: Option<RuntimeSnapshot> = None;
            let mut retry_reason = "none";
            let mut final_error = error;

            match decision.action {
                RecoveryPolicyAction::TuneResponseLimit => {
                    if let Some(adjustment) = decision.response_limit_adjustment.as_ref() {
                        if let Err(tune_error) = apply_response_limit_tuning(
                            &RecoveryOperation::WalletBalanceSync,
                            adjustment,
                        ) {
                            final_error = format!(
                                "{final_error}; wallet_sync_response_limit_tune_failed {}->{}: {tune_error}",
                                adjustment.from_bytes, adjustment.to_bytes
                            );
                        } else {
                            let mut updated = sync_snapshot.clone();
                            updated.wallet_balance_sync.max_response_bytes = adjustment.to_bytes;
                            retry_snapshot = Some(updated);
                            retry_reason = "tune_response_limit";
                        }
                    } else {
                        final_error =
                            format!("{final_error}; wallet sync tune action missing adjustment");
                    }
                }
                RecoveryPolicyAction::RetryImmediate => {
                    retry_snapshot = Some(sync_snapshot.clone());
                    retry_reason = "retry_immediate";
                }
                RecoveryPolicyAction::Backoff
                | RecoveryPolicyAction::EscalateFault
                | RecoveryPolicyAction::Skip => {}
            }

            log!(
                SchedulerLogPriority::Info,
                "scheduler_wallet_balance_sync_recovery_decision action={:?} reason={:?} retry_reason={} backoff_secs={:?}",
                decision.action,
                decision.reason,
                retry_reason,
                decision.backoff_secs
            );

            if let Some(retry_snapshot) = retry_snapshot {
                match fetch_wallet_balance_sync_read(&retry_snapshot).await {
                    Ok(read) => {
                        stable::record_wallet_balance_sync_success(
                            now_ns,
                            read.eth_balance_wei_hex,
                            read.usdc_balance_raw_hex,
                            read.usdc_contract_address,
                        );
                        log!(
                            SchedulerLogPriority::Info,
                            "scheduler_wallet_balance_sync_recovered now_ns={} retry_reason={} bootstrap_pending_cleared={}",
                            now_ns,
                            retry_reason,
                            snapshot.wallet_balance_bootstrap_pending
                        );
                        return;
                    }
                    Err(retry_error) => {
                        final_error =
                            format!("{final_error}; retry({retry_reason}) failed: {retry_error}");
                    }
                }
            }

            stable::record_wallet_balance_sync_error(final_error.clone());
            log!(
                SchedulerLogPriority::Error,
                "scheduler_wallet_balance_sync_error stage=fetch_balances error={}",
                final_error
            );
        }
    }
}

/// Forces the ordinary wallet-balance synchronization path for the local
/// generations observatory. Authorization is enforced by the caller-facing
/// canister endpoint before this helper is reached.
pub(crate) async fn run_evaluation_wallet_balance_sync() -> Result<String, String> {
    stable::set_wallet_balance_bootstrap_pending(true);
    let snapshot = stable::runtime_snapshot();
    maybe_sync_wallet_balances(current_time_ns(), &snapshot).await;
    let balance = stable::wallet_balance_snapshot();
    if let Some(error) = balance.last_error {
        return Err(error);
    }
    balance
        .usdc_balance_raw_hex
        .ok_or_else(|| "wallet balance sync returned no USDC balance".to_string())
}

/// Executes the `PollInbox` job: polls the EVM inbox contract for new
/// `MessageQueued` events, ingests them as inbox messages, advances the cursor,
/// then calls `maybe_sync_wallet_balances` and stages any pending messages.
async fn run_poll_inbox_job(now_ns: u64) -> Result<(), String> {
    let snapshot = stable::runtime_snapshot();
    let mut fetched_events = 0usize;
    let mut ingested_events = 0usize;
    let mut skipped_duplicate_events = 0usize;

    if snapshot.evm_rpc_url.trim().is_empty() {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_poll_inbox_rpc_unconfigured skipping_evm_rpc=true"
        );
    } else if snapshot.inbox_contract_address.is_none() {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_poll_inbox_contract_unconfigured skipping_evm_rpc=true"
        );
    } else if snapshot.evm_address.is_none() {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_poll_inbox_agent_address_unavailable skipping_evm_rpc=true"
        );
    } else if !poll_inbox_rpc_due(
        now_ns,
        snapshot.evm_cursor.last_poll_at_ns,
        snapshot.evm_cursor.consecutive_empty_polls,
    ) {
        let next_due_ns =
            snapshot
                .evm_cursor
                .last_poll_at_ns
                .saturating_add(empty_poll_backoff_delay_ns(
                    snapshot.evm_cursor.consecutive_empty_polls,
                ));
        log!(
            SchedulerLogPriority::Info,
            "scheduler_poll_inbox_backoff_skip now_ns={} last_poll_at_ns={} empty_polls={} next_due_ns={}",
            now_ns,
            snapshot.evm_cursor.last_poll_at_ns,
            snapshot.evm_cursor.consecutive_empty_polls,
            next_due_ns
        );
    } else {
        let poller = HttpEvmPoller::from_snapshot(&snapshot)?;
        let poll = poller
            .poll(&snapshot.evm_cursor)
            .await
            .inspect_err(|error| {
                if is_eth_get_logs_failure(error) {
                    log!(
                        SchedulerLogPriority::Info,
                        "scheduler_poll_inbox_retry_deferred reason=eth_getLogs_failure"
                    );
                } else {
                    stable::record_survival_operation_failure(
                        &SurvivalOperationClass::EvmPoll,
                        now_ns,
                        stable::SURVIVAL_OPERATION_MAX_BACKOFF_SECS_EVM_POLL,
                    );
                }
            })?;

        fetched_events = poll.events.len();
        let peer_directory = if !poll.events.is_empty() {
            if let Ok(client) = FactoryRoomClient::from_runtime() {
                load_peers_for_incoming_events(&client).await
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        for event in &poll.events {
            if !stable::try_mark_evm_event_ingested(&event.tx_hash, event.log_index) {
                skipped_duplicate_events = skipped_duplicate_events.saturating_add(1);
                continue;
            }
            if ingest_verified_patronage_event(event)?.is_some() {
                // Patronage is telemetry, not paid correspondence: deliberately
                // do not create an inbox item or schedule an autonomy turn.
                ingested_events = ingested_events.saturating_add(1);
                continue;
            }
            let (body, sender) = evm_event_to_inbox_message(event);
            ingest_inbox_message(body, sender, InboxMessageSource::EvmInbox)?;
            if let Ok(payment) = crate::features::evm::decode_message_queued_payload(&event.payload)
            {
                if !payment.eth_amount.is_zero() || !payment.usdc_amount.is_zero() {
                    if let Some(peer) = peer_directory
                        .iter()
                        .find(|peer| peer.evm_address.eq_ignore_ascii_case(&payment.sender))
                    {
                        let value = serde_json::json!({ "peer_id": peer.canister_id, "promise": payment.message, "asset": if payment.usdc_amount.is_zero() { "eth" } else { "usdc" }, "amount_raw": if payment.usdc_amount.is_zero() { payment.eth_amount.to_string() } else { payment.usdc_amount.to_string() }, "tx_hash": event.tx_hash, "delivered": null, "assessment": "commission_received", "status": "received", "direction": "incoming" }).to_string();
                        record_counterparty_deal(
                            &peer.canister_id,
                            &event.tx_hash,
                            &value,
                            now_ns,
                            "evm_inbox",
                        )?;
                    }
                }
            }
            ingested_events = ingested_events.saturating_add(1);
        }

        let mut next_cursor = poll.cursor.clone();
        next_cursor.last_poll_at_ns = now_ns;
        if ingested_events > 0 {
            next_cursor.consecutive_empty_polls = 0;
        } else {
            next_cursor.consecutive_empty_polls = snapshot
                .evm_cursor
                .consecutive_empty_polls
                .saturating_add(1);
        }
        stable::set_evm_cursor(&next_cursor);
        stable::record_survival_operation_success(&SurvivalOperationClass::EvmPoll);
    }

    maybe_sync_wallet_balances(now_ns, &snapshot).await;

    let staged =
        stable::stage_pending_inbox_messages(POLL_INBOX_STAGE_BATCH_SIZE, current_time_ns());
    log!(
        SchedulerLogPriority::Info,
        "scheduler_poll_inbox_staged count={} evm_events_fetched={} evm_events_ingested={} evm_events_duplicate_skipped={}",
        staged,
        fetched_events,
        ingested_events,
        skipped_duplicate_events
    );

    maybe_poll_factory_room(now_ns, &snapshot).await;
    Ok(())
}

/// Executes the `CheckCycles` job: reads the canister cycle balances, classifies
/// the survival tier, persists it, and — when conditions are met — triggers or
/// recovers an automated cycle top-up.
async fn run_check_cycles() -> Result<(), String> {
    let now_ns = current_time_ns();
    let total_cycles = ic_cdk::api::canister_cycle_balance();
    let liquid_cycles = ic_cdk::api::canister_liquid_cycle_balance();
    let expected = classify_survival_tier(total_cycles, liquid_cycles)?;
    let requirements = check_cycles_requirements()?;
    let snapshot = stable::runtime_snapshot();
    let telemetry = stable::cycle_telemetry();
    let cached_usdc_balance_raw =
        parse_hex_quantity_u64(snapshot.wallet_balance.usdc_balance_raw_hex.as_deref());
    let mut topup_state = stable::read_topup_state();
    let mut topup_triggered = false;

    stable::set_scheduler_survival_tier(expected.clone());
    let runtime_tier = stable::scheduler_survival_tier();
    let recovery_checks = stable::scheduler_survival_tier_recovery_checks();
    let runway_seconds = canonical_runway_seconds(
        liquid_cycles,
        telemetry.burn_rate_cycles_per_day,
        parse_hex_quantity_u64(snapshot.wallet_balance.usdc_balance_raw_hex.as_deref()),
        snapshot.wallet_balance.usdc_decimals,
        telemetry.usd_per_trillion_cycles,
    );
    let mortality = stable::observe_mortality_resources(liquid_cycles, runway_seconds, now_ns);
    let mortality_policy = policy_for_tier(mortality.tier);
    if mortality.phase == crate::domain::types::MortalityPhase::TerminalPending {
        let _ = enqueue_immediate_agent_turn_job_if_absent("AgentTurn:terminal".to_string());
    }

    if snapshot.cycle_topup.enabled
        && maybe_recover_failed_topup(&snapshot, topup_state.as_ref(), now_ns)
    {
        topup_triggered = true;
        topup_state = stable::read_topup_state();
    }

    if !topup_triggered
        && should_trigger_cycle_topup(
            total_cycles,
            &snapshot,
            topup_state.as_ref(),
            cached_usdc_balance_raw,
        )
    {
        match build_cycle_topup(&snapshot) {
            Ok(topup) => match topup.start() {
                Ok(()) => {
                    let _ = enqueue_topup_cycles_job("auto", now_ns);
                    topup_triggered = true;
                }
                Err(error) => {
                    log!(
                        SchedulerLogPriority::Error,
                        "scheduler_checkcycles_topup_start_rejected error={error}",
                    );
                }
            },
            Err(error) => {
                log!(
                    SchedulerLogPriority::Error,
                    "scheduler_checkcycles_topup_config_error error={error}",
                );
            }
        }
    }

    log!(
        SchedulerLogPriority::Info,
    "scheduler_checkcycles total_cycles={} liquid_cycles={} reserve_floor_cycles={} required_cycles={} low_tier_limit={} observed_tier={:?} runtime_tier={:?} recovery_checks={} mortality_tier={:?} runway_seconds={:?} cadence_multiplier={} reasoning={:?} cached_usdc_balance_raw={:?} topup_triggered={} topup_state={:?}",
        total_cycles,
        liquid_cycles,
        DEFAULT_RESERVE_FLOOR_CYCLES,
        requirements.required_cycles,
        requirements.required_cycles.saturating_mul(CHECKCYCLES_LOW_TIER_MULTIPLIER),
        expected,
        runtime_tier,
        recovery_checks,
        mortality.tier,
        mortality.runway_seconds,
        mortality_policy.cadence_multiplier,
        mortality_policy.reasoning_level,
        cached_usdc_balance_raw,
        topup_triggered,
        topup_state
    );
    Ok(())
}

/// Runs the ordinary mortality observation immediately for the guarded local
/// evaluation starvation fixture instead of waiting for the periodic cadence.
pub(crate) async fn run_evaluation_mortality_check() -> Result<(), String> {
    run_check_cycles().await
}

/// Parses an optional hex string (with or without `0x` prefix) into a `u64`.
/// Returns `None` when the input is absent or unparseable.
fn parse_hex_quantity_u64(raw: Option<&str>) -> Option<u64> {
    let raw = raw?;
    let normalized = raw.trim();
    let without_prefix = normalized
        .strip_prefix("0x")
        .or_else(|| normalized.strip_prefix("0X"))
        .unwrap_or(normalized);
    if without_prefix.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(without_prefix, 16).ok()
}

/// Returns `true` when the top-up state machine is idle (no stage in progress),
/// allowing a new automated top-up to be started.
fn topup_state_allows_auto_start(state: Option<&TopUpStage>) -> bool {
    matches!(state, None | Some(TopUpStage::Completed { .. }))
}

/// Returns `true` when the top-up is in a `Failed` state and the
/// `TOPUP_FAILED_RECOVERY_BACKOFF_SECS` window has elapsed since failure.
fn topup_failed_recovery_due(state: Option<&TopUpStage>, now_ns: u64) -> bool {
    let Some(TopUpStage::Failed { failed_at_ns, .. }) = state else {
        return false;
    };
    let backoff_ns = TOPUP_FAILED_RECOVERY_BACKOFF_SECS.saturating_mul(1_000_000_000);
    now_ns >= failed_at_ns.saturating_add(backoff_ns)
}

/// Attempts to recover a failed top-up if the backoff window has passed.
/// Resets the state machine, re-starts the top-up, and enqueues a
/// continuation job.  Returns `true` on a successful recovery start.
fn maybe_recover_failed_topup(
    snapshot: &RuntimeSnapshot,
    topup_state: Option<&TopUpStage>,
    now_ns: u64,
) -> bool {
    let Some(TopUpStage::Failed {
        stage,
        error,
        failed_at_ns,
        attempts,
    }) = topup_state
    else {
        return false;
    };

    let retry_at_ns = failed_at_ns
        .saturating_add(TOPUP_FAILED_RECOVERY_BACKOFF_SECS.saturating_mul(1_000_000_000));
    if !topup_failed_recovery_due(topup_state, now_ns) {
        log!(
            SchedulerLogPriority::Info,
            "scheduler_checkcycles_topup_recovery_backoff active=true retry_at_ns={} failed_stage={} attempts={}",
            retry_at_ns,
            stage,
            attempts
        );
        return false;
    }

    let topup = match build_cycle_topup(snapshot) {
        Ok(topup) => topup,
        Err(recover_error) => {
            log!(
                SchedulerLogPriority::Error,
                "scheduler_checkcycles_topup_recovery_config_error error={recover_error}",
            );
            return false;
        }
    };

    if let Err(recover_error) = topup.reset() {
        log!(
            SchedulerLogPriority::Error,
            "scheduler_checkcycles_topup_recovery_reset_error error={recover_error}",
        );
        return false;
    }
    if let Err(recover_error) = topup.start() {
        log!(
            SchedulerLogPriority::Error,
            "scheduler_checkcycles_topup_recovery_start_error error={recover_error}",
        );
        return false;
    }

    let enqueued = enqueue_topup_cycles_job("auto-recover", now_ns).is_some();
    log!(
        SchedulerLogPriority::Info,
        "scheduler_checkcycles_topup_recovery_started enqueued={} failed_stage={} failed_error={} previous_attempts={}",
        enqueued,
        stage,
        error,
        attempts
    );
    true
}

/// Returns `true` when all conditions for an automated cycle top-up are met:
/// top-up is enabled, cycles are below the threshold but above the operational
/// floor, no top-up is already in progress, and the USDC balance is sufficient.
fn should_trigger_cycle_topup(
    total_cycles: u128,
    snapshot: &RuntimeSnapshot,
    topup_state: Option<&TopUpStage>,
    cached_usdc_balance_raw: Option<u64>,
) -> bool {
    if !snapshot.cycle_topup.enabled {
        return false;
    }
    if total_cycles <= TOPUP_MIN_OPERATIONAL_CYCLES {
        return false;
    }
    if total_cycles >= snapshot.cycle_topup.auto_topup_cycle_threshold {
        return false;
    }
    if !topup_state_allows_auto_start(topup_state) {
        return false;
    }

    let min_required = snapshot
        .cycle_topup
        .min_usdc_reserve
        .saturating_add(TOPUP_MIN_USDC_AVAILABLE_RAW);
    cached_usdc_balance_raw.unwrap_or_default() >= min_required
}

/// Computes the minimum cycles required to sustain one workflow envelope,
/// used as the affordability baseline by `classify_survival_tier`.
fn check_cycles_requirements() -> Result<AffordabilityRequirements, String> {
    let operation_cost = estimate_operation_cost(&OperationClass::WorkflowEnvelope {
        envelope_cycles: CHECKCYCLES_REFERENCE_ENVELOPE_CYCLES,
    })?;
    Ok(affordability_requirements(
        operation_cost,
        DEFAULT_SAFETY_MARGIN_BPS,
        0,
    ))
}

/// Classifies the current canister into `Critical`, `LowCycles`, or `Normal`
/// based on whether liquid cycles satisfy the reserve floor and the low-tier
/// multiplier threshold.
fn classify_survival_tier(total_cycles: u128, liquid_cycles: u128) -> Result<SurvivalTier, String> {
    let can_cover_critical_floor = can_afford_with_reserve(
        total_cycles,
        &OperationClass::WorkflowEnvelope {
            envelope_cycles: CHECKCYCLES_REFERENCE_ENVELOPE_CYCLES,
        },
        DEFAULT_SAFETY_MARGIN_BPS,
        DEFAULT_RESERVE_FLOOR_CYCLES,
    )?;
    if !can_cover_critical_floor {
        return Ok(SurvivalTier::Critical);
    }

    let requirements = check_cycles_requirements()?;
    if liquid_cycles < requirements.required_cycles {
        return Ok(SurvivalTier::Critical);
    }

    let low_threshold = requirements
        .required_cycles
        .saturating_mul(CHECKCYCLES_LOW_TIER_MULTIPLIER);
    if liquid_cycles < low_threshold {
        return Ok(SurvivalTier::LowCycles);
    }

    Ok(SurvivalTier::Normal)
}

// ── Lease helpers ────────────────────────────────────────────────────────────

/// Materialises pending jobs for every enabled task whose `next_due_ns` has
/// elapsed, using slot-aligned dedupe keys to prevent duplicate enqueuing.
///
/// After enqueuing, `next_due_ns` is advanced by one interval — but never
/// placed before the current slot start — to avoid bursty catch-up when the
/// canister is behind wall-clock time.
pub fn refresh_due_jobs(now_ns: u64) {
    let mut schedules = stable::list_task_configs();
    schedules.sort_by_key(|(_kind, config)| (config.priority, config.kind.as_str().to_string()));
    let low_cycles = stable::scheduler_low_cycles_mode();

    for (kind, config) in schedules {
        if !config.enabled {
            continue;
        }
        if low_cycles && !config.essential {
            continue;
        }
        let interval_ns = config.interval_secs.saturating_mul(1_000_000_000);
        if interval_ns == 0 {
            continue;
        }

        let mut runtime = stable::get_task_runtime(&kind);
        if runtime.pending_job_id.is_some() {
            continue;
        }
        if runtime.backoff_until_ns.is_some_and(|until| until > now_ns) {
            continue;
        }
        if runtime.next_due_ns > now_ns {
            continue;
        }

        let slot_start_ns = now_ns - (now_ns % interval_ns);
        let dedupe_key = if kind == TaskKind::TopUpCycles {
            topup_cycles_dedupe_key()
        } else {
            format!("{}:{}", kind.as_str(), slot_start_ns)
        };
        if let Some(job_id) = stable::enqueue_job_if_absent(
            kind.clone(),
            TaskLane::Mutating,
            dedupe_key,
            slot_start_ns,
            config.priority,
        ) {
            log!(
                SchedulerLogPriority::Info,
                "scheduler_job_enqueued kind={:?} job_id={} scheduled_for={} priority={}",
                kind,
                job_id,
                slot_start_ns,
                config.priority,
            );
        }

        // Prevent bursty "catch-up" scheduling when runtime is far behind wall-clock.
        // We advance by one interval, but never leave next_due_ns behind the current slot.
        let advanced_due_ns = runtime.next_due_ns.saturating_add(interval_ns);
        let aligned_due_ns = slot_start_ns.saturating_add(interval_ns);
        runtime.next_due_ns = advanced_due_ns.max(aligned_due_ns);
        stable::save_task_runtime(&kind, &runtime);
    }
}

/// Returns the `SurvivalOperationClass` that gates execution of `kind`, or
/// `None` for tasks that are not gated by the survival policy.
fn operation_class_for_task(kind: &TaskKind) -> Option<SurvivalOperationClass> {
    match kind {
        TaskKind::AgentTurn => Some(SurvivalOperationClass::Inference),
        TaskKind::PollInbox => Some(SurvivalOperationClass::EvmPoll),
        _ => None,
    }
}

/// Returns the lease TTL in nanoseconds for `kind`.
/// Agent turns use the extended TTL to accommodate multi-round inference;
/// all other tasks use the lightweight TTL.
fn lease_ttl_ns(kind: &TaskKind) -> u64 {
    match kind {
        TaskKind::AgentTurn => timing::AGENT_TURN_LEASE_TTL_NS,
        _ => timing::LIGHTWEIGHT_LEASE_TTL_NS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{
        AbiArtifact, AbiArtifactKey, AbiFunctionSpec, AbiTypeSpec, ActionSpec, ContractRoleBinding,
        EvmEvent, InboxMessageSource, InboxMessageStatus, RecoveryOperation, RecoveryPolicyAction,
        ResponseLimitAdjustment, RetentionConfig, RoomContentType, RoomMessage, RoomMessagePage,
        SpawnBootstrapView, StrategyTemplate, StrategyTemplateKey, SurvivalOperationClass,
        TaskScheduleConfig, TaskScheduleRuntime, TemplateActivationState, TemplateStatus,
        WalletBalanceSnapshot, WalletBalanceSyncConfig,
    };
    use crate::storage::stable;
    use crate::util::block_on_with_spin;
    use candid::Principal;
    use std::cell::Cell;
    use std::rc::Rc;

    fn settle_pending_topup_jobs(now_ns: u64) {
        for job in stable::list_recent_jobs(500)
            .into_iter()
            .filter(|job| job.kind == TaskKind::TopUpCycles && !job.is_terminal())
        {
            stable::complete_job(&job.id, JobStatus::Succeeded, None, now_ns, None);
        }
    }

    fn topup_job_count() -> usize {
        stable::list_recent_jobs(500)
            .into_iter()
            .filter(|job| job.kind == TaskKind::TopUpCycles)
            .count()
    }

    fn strategy_execution_test_call(
        submitted_at_ns: u64,
    ) -> crate::domain::types::PendingStrategyExecutionCall {
        crate::domain::types::PendingStrategyExecutionCall {
            index: 0,
            call: crate::domain::types::StrategyExecutionCall {
                role: "pool".into(),
                to: "0x1111111111111111111111111111111111111111".into(),
                value_wei: "0".into(),
                data: "0x".into(),
            },
            tx_hash: Some("0xabc".into()),
            state: StrategyExecutionCallState::Submitted,
            receipt_block_number: None,
            receipt_block_hash: None,
            submitted_at_ns: Some(submitted_at_ns),
            last_checked_at_ns: None,
            error: None,
        }
    }

    #[test]
    fn strategy_execution_receipt_state_matrix_enforces_depth_revert_timeout_and_head_order() {
        assert_eq!(strategy_execution_rpc_backoff_ns(1), 30 * 1_000_000_000);
        assert_eq!(
            strategy_execution_rpc_backoff_ns(100),
            STRATEGY_EXECUTION_MAX_BACKOFF_NS
        );
        let mut call = strategy_execution_test_call(100);
        apply_strategy_receipt_observation(
            &mut call,
            StrategyReceiptObservation {
                status: TransactionReceiptStatus::Pending,
                block_number: None,
                block_hash: None,
                latest_block: 20,
            },
            200,
            100,
            3,
        )
        .unwrap();
        assert_eq!(call.state, StrategyExecutionCallState::Submitted);

        apply_strategy_receipt_observation(
            &mut call,
            StrategyReceiptObservation {
                status: TransactionReceiptStatus::Confirmed,
                block_number: Some(19),
                block_hash: Some("0xblock".into()),
                latest_block: 20,
            },
            201,
            100,
            3,
        )
        .unwrap();
        assert_eq!(
            call.state,
            StrategyExecutionCallState::Submitted,
            "two confirmations are below depth three"
        );
        apply_strategy_receipt_observation(
            &mut call,
            StrategyReceiptObservation {
                status: TransactionReceiptStatus::Confirmed,
                block_number: Some(18),
                block_hash: Some("0xblock".into()),
                latest_block: 20,
            },
            202,
            100,
            3,
        )
        .unwrap();
        assert_eq!(call.state, StrategyExecutionCallState::Confirmed);

        let mut reverted = strategy_execution_test_call(100);
        apply_strategy_receipt_observation(
            &mut reverted,
            StrategyReceiptObservation {
                status: TransactionReceiptStatus::Reverted,
                block_number: Some(20),
                block_hash: Some("0xblock".into()),
                latest_block: 20,
            },
            202,
            100,
            3,
        )
        .unwrap();
        assert_eq!(reverted.state, StrategyExecutionCallState::Reverted);

        let mut dropped = strategy_execution_test_call(100);
        apply_strategy_receipt_observation(
            &mut dropped,
            StrategyReceiptObservation {
                status: TransactionReceiptStatus::Pending,
                block_number: None,
                block_hash: None,
                latest_block: 20,
            },
            100 + STRATEGY_EXECUTION_RECEIPT_TIMEOUT_NS,
            100,
            3,
        )
        .unwrap();
        assert_eq!(dropped.state, StrategyExecutionCallState::Dropped);

        let mut future = strategy_execution_test_call(100);
        let error = apply_strategy_receipt_observation(
            &mut future,
            StrategyReceiptObservation {
                status: TransactionReceiptStatus::Confirmed,
                block_number: Some(21),
                block_hash: Some("0xblock".into()),
                latest_block: 20,
            },
            202,
            100,
            3,
        )
        .unwrap_err();
        assert!(error.contains("greater than latest head"));

        let mut reloaded = crate::domain::types::PendingStrategyExecution {
            execution_id: "crash-before-broadcast".into(),
            turn_id: "turn".into(),
            key: StrategyTemplateKey {
                protocol: "p".into(),
                primitive: "q".into(),
                chain_id: 8453,
                template_id: "t".into(),
            },
            action_id: "enter_supply".into(),
            plan_digest: "digest".into(),
            asset_effects: vec![],
            calls: vec![crate::domain::types::PendingStrategyExecutionCall {
                state: StrategyExecutionCallState::Unattempted,
                tx_hash: None,
                ..strategy_execution_test_call(100)
            }],
            state: PendingStrategyExecutionState::Pending,
            created_at_ns: 100,
            updated_at_ns: 100,
            next_check_at_ns: 100,
            consecutive_rpc_failures: 0,
            bookkeeping_applied: false,
            terminal_bookkeeping_applied: false,
        };
        expire_unattempted_strategy_calls(
            &mut reloaded,
            100 + STRATEGY_EXECUTION_RECEIPT_TIMEOUT_NS - 1,
        );
        assert_eq!(
            reloaded.calls[0].state,
            StrategyExecutionCallState::Unattempted
        );
        expire_unattempted_strategy_calls(
            &mut reloaded,
            100 + STRATEGY_EXECUTION_RECEIPT_TIMEOUT_NS,
        );
        assert_eq!(reloaded.calls[0].state, StrategyExecutionCallState::Dropped);
        reloaded.calls[0].state = StrategyExecutionCallState::Submitted;
        reloaded.calls[0].tx_hash = None;
        expire_unattempted_strategy_calls(
            &mut reloaded,
            100 + STRATEGY_EXECUTION_RECEIPT_TIMEOUT_NS,
        );
        assert_eq!(reloaded.calls[0].state, StrategyExecutionCallState::Dropped);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn strategy_receipt_reconciliation_persists_malformed_and_transport_backoff_end_to_end() {
        fn pending(id: &str, now_ns: u64) -> crate::domain::types::PendingStrategyExecution {
            crate::domain::types::PendingStrategyExecution {
                execution_id: id.into(),
                turn_id: format!("turn-{id}"),
                key: StrategyTemplateKey {
                    protocol: "receipt-e2e".into(),
                    primitive: "lend".into(),
                    chain_id: 8453,
                    template_id: id.into(),
                },
                action_id: "enter_supply".into(),
                plan_digest: format!("digest-{id}"),
                asset_effects: vec![],
                calls: vec![strategy_execution_test_call(now_ns)],
                state: PendingStrategyExecutionState::Pending,
                created_at_ns: now_ns,
                updated_at_ns: now_ns,
                next_check_at_ns: now_ns,
                consecutive_rpc_failures: 0,
                bookkeeping_applied: false,
                terminal_bookkeeping_applied: false,
            }
        }

        stable::init_storage();
        let malformed_id = "receipt-malformed-json-e2e";
        let now_ns = 91_000_000_000;
        stable::insert_pending_strategy_execution(pending(malformed_id, now_ns)).unwrap();
        with_host_stub_env(
            &[("IC_AUTOMATON_EVM_RPC_STUB_FORCE_BODY", Some("{malformed"))],
            || block_on_with_spin(reconcile_pending_strategy_executions(now_ns)).unwrap(),
        );
        let first = stable::pending_strategy_execution(malformed_id).unwrap();
        assert_eq!(first.consecutive_rpc_failures, 1);
        assert_eq!(first.calls[0].last_checked_at_ns, Some(now_ns));
        assert!(first.calls[0]
            .error
            .as_deref()
            .unwrap()
            .contains("parse eth_getTransactionReceipt response JSON"));
        assert_eq!(
            first.next_check_at_ns,
            now_ns + strategy_execution_rpc_backoff_ns(1)
        );
        let retry_ns = first.next_check_at_ns;
        with_host_stub_env(
            &[("IC_AUTOMATON_EVM_RPC_STUB_FORCE_BODY", Some("{malformed"))],
            || block_on_with_spin(reconcile_pending_strategy_executions(retry_ns)).unwrap(),
        );
        let second = stable::pending_strategy_execution(malformed_id).unwrap();
        assert_eq!(second.consecutive_rpc_failures, 2);
        assert_eq!(second.calls[0].last_checked_at_ns, Some(retry_ns));
        assert_eq!(
            second.next_check_at_ns,
            retry_ns + strategy_execution_rpc_backoff_ns(2)
        );

        let transport_id = "receipt-transport-e2e";
        stable::insert_pending_strategy_execution(pending(transport_id, now_ns)).unwrap();
        with_host_stub_env(
            &[
                ("IC_AUTOMATON_EVM_RPC_STUB_FORCE_STATUS", Some("503")),
                ("IC_AUTOMATON_EVM_RPC_STUB_FORCE_BODY", Some("unavailable")),
            ],
            || block_on_with_spin(reconcile_pending_strategy_executions(now_ns)).unwrap(),
        );
        let transport = stable::pending_strategy_execution(transport_id).unwrap();
        assert_eq!(transport.consecutive_rpc_failures, 1);
        assert_eq!(transport.calls[0].last_checked_at_ns, Some(now_ns));
        assert!(transport.calls[0].error.as_deref().unwrap().contains("503"));
        assert!(transport.next_check_at_ns <= now_ns + STRATEGY_EXECUTION_MAX_BACKOFF_NS);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn with_host_stub_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        crate::test_support::with_locked_host_env(vars, f);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn with_clean_host_stub_env(f: impl FnOnce()) {
        with_host_stub_env(
            &[
                ("IC_AUTOMATON_EVM_RPC_STUB_FORCE_STATUS", None),
                ("IC_AUTOMATON_EVM_RPC_STUB_FORCE_BODY", None),
                ("IC_AUTOMATON_EVM_RPC_STUB_MAX_LOG_BLOCK_SPAN", None),
            ],
            f,
        );
    }

    fn init_scheduler_scope() {
        stable::init_scheduler_defaults(0);
        let target_kind = TaskKind::PollInbox;
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, kind == &target_kind);
        }

        let mut config =
            stable::get_task_config(&target_kind).unwrap_or_else(|| TaskScheduleConfig {
                kind: target_kind.clone(),
                enabled: true,
                essential: true,
                interval_secs: 10,
                priority: 0,
                max_backoff_secs: 120,
            });
        config.enabled = true;
        config.interval_secs = 10;
        config.essential = true;
        config.priority = 0;
        stable::upsert_task_config(config);
    }

    fn test_factory_principal() -> Principal {
        Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").expect("test principal should parse")
    }

    fn sample_factory_peer(index: usize) -> FactoryPeer {
        FactoryPeer {
            name: Some(format!("Peer {index}")),
            constitution_hash: None,
            canister_id: format!("peer-{index}-cai"),
            steward_address: "0x0000000000000000000000000000000000000000".to_string(),
            evm_address: format!("0x{index:040x}"),
            chain: crate::features::factory_room::FactoryPeerChain::Base,
            session_id: format!("session-{index}"),
            parent_id: None,
            generation: Some(0),
            parent_constitution_hash: None,
            child_ids: vec![],
            created_at: 1,
            version_commit: "0".repeat(40),
            controllers: None,
            control_status: None,
            control_verified_at: None,
            death_cause: None,
            died_at: None,
            estate_disposition: None,
            death_recorded_by: None,
            death_incident_reference: None,
        }
    }

    #[test]
    fn reproduction_payment_recovery_is_restart_safe_and_idempotent_after_deposit() {
        assert_eq!(
            reproduction_payment_recovery_action(
                &ReproductionSessionState::AwaitingPayment,
                Some("0xapprove"),
                Some(TransactionReceiptStatus::Confirmed),
                None,
                None,
            ),
            ReproductionPaymentRecoveryAction::BroadcastDeposit,
            "restart after approval must resume the same session at deposit"
        );
        assert_eq!(
            reproduction_payment_recovery_action(
                &ReproductionSessionState::AwaitingPayment,
                Some("0xapprove"),
                Some(TransactionReceiptStatus::Confirmed),
                Some("0xdeposit"),
                Some(TransactionReceiptStatus::Reverted),
            ),
            ReproductionPaymentRecoveryAction::BroadcastDeposit,
            "a reverted deposit submission is retried without reapproval"
        );
        assert_eq!(
            reproduction_payment_recovery_action(
                &ReproductionSessionState::AwaitingPayment,
                Some("0xapprove"),
                Some(TransactionReceiptStatus::Confirmed),
                Some("0xdeposit"),
                Some(TransactionReceiptStatus::Confirmed),
            ),
            ReproductionPaymentRecoveryAction::Wait,
            "a mined deposit waits for factory log reconciliation"
        );
        assert_eq!(
            reproduction_payment_recovery_action(
                &ReproductionSessionState::PaymentDetected,
                None,
                None,
                None,
                None,
            ),
            ReproductionPaymentRecoveryAction::Wait,
            "factory-observed deposits are never rebroadcast even if local persistence was lost"
        );
    }

    #[test]
    fn reverted_approval_clears_dependent_deposit_before_persisted_retry() {
        stable::init_storage();
        let fact = MemoryFact {
            key: "counterparty.child.pending.recovery-regression".to_string(),
            value: json!({
                "session_id": "recovery-regression",
                "token": "0x1111111111111111111111111111111111111111",
                "escrow": "0x2222222222222222222222222222222222222222",
                "claim_id": format!("0x{}", "33".repeat(32)),
                "gross_amount": "50000000",
                "approve_tx_hash": "0xold-approve",
                "deposit_tx_hash": "0xold-dependent-deposit"
            })
            .to_string(),
            created_at_ns: 1,
            updated_at_ns: 1,
            source_turn_id: "test".to_string(),
        };
        stable::set_memory_fact(&fact).unwrap();
        let value: serde_json::Value = serde_json::from_str(&fact.value).unwrap();
        let action = reproduction_payment_recovery_action(
            &ReproductionSessionState::AwaitingPayment,
            Some("0xold-approve"),
            Some(TransactionReceiptStatus::Reverted),
            Some("0xold-dependent-deposit"),
            Some(TransactionReceiptStatus::Reverted),
        );
        assert_eq!(action, ReproductionPaymentRecoveryAction::BroadcastApproval);

        let cleared = prepare_reproduction_payment_retry_value(&value, action);
        persist_reproduction_payment_value(
            &fact,
            cleared,
            2,
            "approval_rebroadcasting",
            None,
            None,
        );
        let persisted = stable::get_memory_fact(&fact.key).unwrap();
        let mut persisted_value: serde_json::Value =
            serde_json::from_str(&persisted.value).unwrap();
        assert!(persisted_value.get("approve_tx_hash").is_none());
        assert!(persisted_value.get("deposit_tx_hash").is_none());

        persisted_value["approve_tx_hash"] = json!("0xnew-approve");
        persist_reproduction_payment_value(
            &persisted,
            persisted_value,
            3,
            "approval_broadcast",
            None,
            None,
        );
        let resumed: serde_json::Value =
            serde_json::from_str(&stable::get_memory_fact(&fact.key).unwrap().value).unwrap();
        assert_eq!(
            reproduction_payment_recovery_action(
                &ReproductionSessionState::AwaitingPayment,
                resumed
                    .get("approve_tx_hash")
                    .and_then(serde_json::Value::as_str),
                Some(TransactionReceiptStatus::Confirmed),
                resumed
                    .get("deposit_tx_hash")
                    .and_then(serde_json::Value::as_str),
                None,
            ),
            ReproductionPaymentRecoveryAction::BroadcastDeposit
        );
    }

    #[test]
    fn receipt_reconciliation_keeps_pending_and_marks_confirmed_or_reverted() {
        let original = json!({ "status": "submitted", "assessment": "awaiting_confirmation" });
        let mut pending = original.clone();
        assert!(!apply_peer_payment_receipt_status(
            &mut pending,
            TransactionReceiptStatus::Pending
        ));
        assert_eq!(pending, original);

        let mut confirmed = original.clone();
        assert!(apply_peer_payment_receipt_status(
            &mut confirmed,
            TransactionReceiptStatus::Confirmed
        ));
        assert_eq!(confirmed["status"], "paid");
        assert_eq!(confirmed["assessment"], "awaiting_delivery");

        let mut reverted = original;
        assert!(apply_peer_payment_receipt_status(
            &mut reverted,
            TransactionReceiptStatus::Reverted
        ));
        assert_eq!(reverted["status"], "failed");
        assert_eq!(reverted["assessment"], "transaction_reverted");

        stable::init_storage();
        let tx1 = format!("0x{}", "11".repeat(32));
        let tx2 = format!("0x{}", "22".repeat(32));
        let first = json!({ "peer_id": "peer-cai", "tx_hash": tx1, "status": "submitted", "assessment": "awaiting_confirmation" });
        let second = json!({ "peer_id": "peer-cai", "tx_hash": tx2, "status": "submitted", "assessment": "awaiting_confirmation" });
        record_counterparty_deal("peer-cai", &tx1, &first.to_string(), 1, "turn-1").unwrap();
        record_counterparty_deal("peer-cai", &tx2, &second.to_string(), 2, "turn-2").unwrap();
        let alias1 = crate::tools::record_pending_receipt("peer-cai", &tx1, 1, "turn-1").unwrap();
        let alias2 = crate::tools::record_pending_receipt("peer-cai", &tx2, 2, "turn-2").unwrap();
        for index in 0..12 {
            let peer = format!("unrelated-{index}-cai");
            let tx = format!("0x{index:064x}");
            let value = json!({ "peer_id": peer, "tx_hash": tx, "status": "received" }).to_string();
            record_counterparty_deal(&peer, &tx, &value, 10 + index, "unrelated").unwrap();
        }
        let pending_aliases = stable::list_memory_facts_by_prefix_sorted(
            "counterparty_pending_receipt.",
            PENDING_RECEIPT_SCAN_LIMIT,
            MemoryFactSort::UpdatedAtDesc,
        );
        assert_eq!(pending_aliases.len(), 2);
        assert!(pending_aliases.iter().any(|fact| fact.key == alias1));

        assert!(!reconcile_pending_receipt_alias(
            &alias1,
            TransactionReceiptStatus::Pending,
            30
        ));
        assert!(stable::get_memory_fact(&alias1).is_some());
        assert!(reconcile_pending_receipt_alias(
            &alias1,
            TransactionReceiptStatus::Confirmed,
            31
        ));
        assert!(stable::get_memory_fact(&alias1).is_none());
        assert!(reconcile_pending_receipt_alias(
            &alias2,
            TransactionReceiptStatus::Reverted,
            32
        ));
        assert!(stable::get_memory_fact(&alias2).is_none());
        let first_saved =
            stable::get_memory_fact(&crate::tools::counterparty_deal_key("peer-cai", &tx1))
                .unwrap();
        let second_saved =
            stable::get_memory_fact(&crate::tools::counterparty_deal_key("peer-cai", &tx2))
                .unwrap();
        assert!(first_saved.value.contains("\"status\":\"paid\""));
        assert!(second_saved.value.contains("\"status\":\"failed\""));
        let standing = stable::get_memory_fact("counterparty.peer-cai.standing").unwrap();
        assert!(standing.value.contains("\"latest_status\":\"failed\""));
    }

    #[test]
    fn incoming_peer_lookup_paginates_beyond_first_fifty_with_hard_cap() {
        crate::features::factory_room::clear_mock_factory_room_call();
        let calls = Rc::new(Cell::new(0u32));
        let calls_for_mock = Rc::clone(&calls);
        crate::features::factory_room::set_mock_factory_room_call(
            move |_canister, method, args| {
                assert_eq!(method, "list_spawned_automatons");
                let (cursor, limit): (Option<String>, u64) = candid::decode_args(args).unwrap();
                assert_eq!(limit, 50);
                calls_for_mock.set(calls_for_mock.get() + 1);
                let (items, next_cursor) = if cursor.is_none() {
                    (
                        (0..50).map(sample_factory_peer).collect(),
                        Some("page-2".to_string()),
                    )
                } else {
                    (vec![sample_factory_peer(50)], None)
                };
                candid::encode_one(crate::features::factory_room::FactoryRoomCallResult::Ok(
                    crate::features::factory_room::FactoryPeerPage { items, next_cursor },
                ))
                .map_err(|error| error.to_string())
            },
        );
        let peers = block_on_with_spin(load_peers_for_incoming_events(&FactoryRoomClient::new(
            test_factory_principal(),
        )));
        assert_eq!(peers.len(), 51);
        assert_eq!(peers.last().unwrap().canister_id, "peer-50-cai");
        assert_eq!(calls.get(), 2);
        crate::features::factory_room::clear_mock_factory_room_call();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn room_refresh_stores_registry_identity_and_verified_inbox_prices() {
        with_clean_host_stub_env(|| {
            stable::init_storage();
            stable::set_evm_rpc_url("https://mainnet.base.org".to_string()).unwrap();
            stable::set_inbox_contract_address(Some(
                "0x3333333333333333333333333333333333333333".to_string(),
            ))
            .unwrap();
            crate::features::factory_room::set_mock_factory_room_call(
                move |_canister, method, args| {
                    assert_eq!(method, "list_spawned_automatons");
                    let (cursor, limit): (Option<String>, u64) = candid::decode_args(args).unwrap();
                    assert!(cursor.is_none());
                    assert_eq!(limit, PEER_DIRECTORY_PAGE_LIMIT);
                    candid::encode_one(crate::features::factory_room::FactoryRoomCallResult::Ok(
                        crate::features::factory_room::FactoryPeerPage {
                            items: vec![sample_factory_peer(7)],
                            next_cursor: None,
                        },
                    ))
                    .map_err(|error| error.to_string())
                },
            );
            let snapshot = stable::runtime_snapshot();
            block_on_with_spin(refresh_peer_directory(
                99,
                &snapshot,
                &FactoryRoomClient::new(test_factory_principal()),
            ));
            let value: serde_json::Value = serde_json::from_str(
                &stable::get_memory_fact("society.peer_directory")
                    .unwrap()
                    .value,
            )
            .unwrap();
            assert_eq!(value["peers"][0]["canister_id"], "peer-7-cai");
            assert_eq!(value["peers"][0]["name"], "Peer 7");
            assert_eq!(value["peers"][0]["alive"], true);
            assert_eq!(
                value["peers"][0]["price_of_attention"]["status"], "verified",
                "{value}"
            );
            assert_eq!(
                value["peers"][0]["price_of_attention"]["usdc_min_raw"],
                "1000000"
            );
            assert_eq!(
                value["peers"][0]["price_of_attention"]["eth_min_wei"],
                "500000000000000"
            );
            crate::features::factory_room::clear_mock_factory_room_call();
        });
    }

    fn configure_factory_room_access() {
        stable::set_spawn_bootstrap_metadata(SpawnBootstrapView {
            contract_version: None,
            name: None,
            constitution: None,
            session_id: None,
            parent_id: None,
            generation: 0,
            factory_principal: Some(test_factory_principal()),
            risk: None,
            strategies: Vec::new(),
            skills: Vec::new(),
            version_commit: None,
        });
    }

    fn sample_room_message(seq: u64) -> RoomMessage {
        RoomMessage {
            message_id: format!("room-message-{seq}"),
            seq,
            author_canister_id: "um5iw-rqaaa-aaaaq-qaaba-cai".to_string(),
            created_at: 123_456_789,
            body: "untrusted room body".to_string(),
            mentions: vec!["rrkah-fqaaa-aaaaa-aaaaq-cai".to_string()],
            content_type: RoomContentType::TextPlain,
        }
    }

    fn strategy_key(template_id: &str) -> StrategyTemplateKey {
        StrategyTemplateKey {
            protocol: "erc20".to_string(),
            primitive: "transfer".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        }
    }

    fn seed_strategy_for_reconcile(
        template_id: &str,
        updated_at_ns: u64,
        call_selector_hex: &str,
        status: TemplateStatus,
        activation_enabled: bool,
    ) {
        let key = strategy_key(template_id);
        let function = AbiFunctionSpec {
            role: "token".to_string(),
            name: "transfer".to_string(),
            selector_hex: call_selector_hex.to_string(),
            inputs: vec![
                AbiTypeSpec {
                    name: "to".to_string(),
                    kind: "address".to_string(),
                    components: Vec::new(),
                },
                AbiTypeSpec {
                    name: "amount".to_string(),
                    kind: "uint256".to_string(),
                    components: Vec::new(),
                },
            ],
            outputs: vec![AbiTypeSpec {
                name: "success".to_string(),
                kind: "bool".to_string(),
                components: Vec::new(),
            }],
            state_mutability: "nonpayable".to_string(),
        };
        crate::strategy::registry::upsert_template(StrategyTemplate {
            key: key.clone(),
            status,
            contract_roles: vec![ContractRoleBinding {
                role: "token".to_string(),
                address: "0x2222222222222222222222222222222222222222".to_string(),
                source_ref: "https://example.com/token-address".to_string(),
                codehash: None,
            }],
            actions: vec![ActionSpec {
                action_id: "transfer".to_string(),
                call_sequence: vec![function.clone()],
                preconditions: vec!["allowance_ok".to_string()],
                postconditions: vec!["balance_delta_positive".to_string()],
                risk_checks: vec!["max_notional".to_string()],
            }],
            constraints_json: "{}".to_string(),
            created_at_ns: updated_at_ns,
            updated_at_ns,
        })
        .expect("template should persist");
        crate::strategy::registry::upsert_abi_artifact(AbiArtifact {
            key: AbiArtifactKey {
                protocol: key.protocol.clone(),
                chain_id: key.chain_id,
                role: "token".to_string(),
            },
            source_ref: "https://example.com/token-abi".to_string(),
            codehash: None,
            abi_json: "[]".to_string(),
            functions: vec![function],
            created_at_ns: updated_at_ns,
            updated_at_ns,
        })
        .expect("abi should persist");
        crate::strategy::registry::set_activation(TemplateActivationState {
            key,
            enabled: activation_enabled,
            updated_at_ns,
            reason: Some("seed".to_string()),
        })
        .expect("activation should persist");
    }

    fn encode_message_queued_payload_for_test(
        sender: &str,
        message: &str,
        usdc_amount: u128,
        eth_amount: u128,
    ) -> String {
        let sender_hex = sender.trim().to_ascii_lowercase();
        let sender_hex = sender_hex.trim_start_matches("0x");
        let message_hex = hex::encode(message.as_bytes());
        let padded_message = if message_hex.len().is_multiple_of(64) {
            message_hex.clone()
        } else {
            format!(
                "{}{}",
                message_hex,
                "0".repeat(64 - (message_hex.len() % 64))
            )
        };

        format!(
            "0x{:0>64}{:064x}{:064x}{:064x}{:064x}{}",
            sender_hex,
            128u128,
            usdc_amount,
            eth_amount,
            message.len(),
            padded_message
        )
    }

    #[test]
    fn evm_event_to_inbox_message_decodes_sender_and_message_payload() {
        let event = EvmEvent {
            tx_hash: "0xfab6ba6ed49ad8b578b64692b324c5935d3216185eddc30411a0f29ba9485c6f"
                .to_string(),
            chain_id: 31_337,
            block_number: 9,
            log_index: 0,
            source: "0x5fc8d32690cc91d4c39d9d3abcbd16989f875707".to_string(),
            payload: encode_message_queued_payload_for_test(
                "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                "message=who are you?",
                0,
                500_000_000_000_000,
            ),
        };

        let (body, sender) = evm_event_to_inbox_message(&event);
        assert_eq!(body, "message=who are you?");
        assert_eq!(sender, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    }

    #[test]
    fn evm_event_to_inbox_message_falls_back_to_raw_event_body_on_decode_error() {
        let event = EvmEvent {
            tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            chain_id: 31_337,
            block_number: 7,
            log_index: 1,
            source: "0x5fc8d32690cc91d4c39d9d3abcbd16989f875707".to_string(),
            payload: "0x1234".to_string(),
        };

        let (body, sender) = evm_event_to_inbox_message(&event);
        assert_eq!(sender, event.source);
        assert!(
            body.contains("\"source\":\"evm_log\""),
            "fallback body should preserve raw evm event envelope"
        );
        assert!(
            body.contains(&event.tx_hash),
            "fallback body should preserve transaction hash for debugging"
        );
    }

    #[test]
    fn evm_event_fallback_payload_is_truncated_under_oversized_burst_inputs() {
        let oversized_payload = format!("0x{}", "ab".repeat(16_000));
        let event = EvmEvent {
            tx_hash: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            chain_id: 31_337,
            block_number: 88,
            log_index: 4,
            source: "0x5fc8d32690cc91d4c39d9d3abcbd16989f875707".to_string(),
            payload: oversized_payload,
        };

        let (body, sender) = evm_event_to_inbox_message(&event);
        assert_eq!(sender, event.source);
        assert!(
            body.chars().count() <= crate::storage::stable::MAX_INBOX_BODY_CHARS,
            "scheduler fallback body must remain capped for oversized decode failures"
        );
        assert!(
            body.contains("[truncated"),
            "scheduler fallback body should include truncation marker for diagnostics"
        );
        assert!(
            body.contains(&event.tx_hash),
            "scheduler fallback body should still include tx hash metadata"
        );
    }

    #[test]
    fn ingest_steward_direct_message_routes_through_inbox_pipeline_with_source_tag() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        let sender = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let message = "message from active steward".to_string();
        let inbox_id = ingest_steward_direct_message(sender.clone(), message.clone())
            .expect("steward direct message should ingest");

        let inbox = stable::list_inbox_messages(1);
        assert_eq!(inbox.len(), 1);
        let stored = &inbox[0];
        assert_eq!(stored.id, inbox_id);
        assert_eq!(stored.posted_by, sender);
        assert_eq!(stored.body, message);
        assert_eq!(stored.source, InboxMessageSource::StewardDirect);
        assert_eq!(stored.status, InboxMessageStatus::Staged);

        let stats = stable::inbox_stats();
        assert_eq!(stats.pending_count, 0);
        assert_eq!(stats.staged_count, 1);
        assert_eq!(stats.consumed_count, 0);
    }

    #[test]
    fn refresh_due_jobs_advances_single_interval_once() {
        init_scheduler_scope();
        let now_ns = 2_500u64;
        let interval_secs = 3u64;
        let interval_ns = interval_secs.saturating_mul(1_000_000_000);

        let mut config =
            stable::get_task_config(&TaskKind::PollInbox).expect("poll inbox config should exist");
        config.interval_secs = interval_secs;
        stable::upsert_task_config(config);

        stable::save_task_runtime(
            &TaskKind::PollInbox,
            &TaskScheduleRuntime {
                kind: TaskKind::PollInbox,
                next_due_ns: now_ns,
                backoff_until_ns: None,
                consecutive_failures: 0,
                pending_job_id: None,
                last_started_ns: None,
                last_finished_ns: None,
                last_error: None,
            },
        );

        let slot_start_ns = now_ns - (now_ns % interval_ns);
        let dedupe_key = format!("PollInbox:{slot_start_ns}");

        refresh_due_jobs(now_ns);
        let first = stable::list_recent_jobs(200)
            .into_iter()
            .find(|job| job.dedupe_key == dedupe_key)
            .expect("initial slot should be materialized");
        assert_eq!(first.status, JobStatus::Pending);

        let runtime_after_first = stable::get_task_runtime(&TaskKind::PollInbox);
        assert_eq!(
            runtime_after_first.next_due_ns,
            now_ns.saturating_add(interval_ns)
        );

        stable::complete_job(
            &first.id,
            JobStatus::Succeeded,
            None,
            now_ns.saturating_add(1),
            None,
        );

        let burst_check_now_ns = now_ns.saturating_add(3 * interval_ns);
        refresh_due_jobs(burst_check_now_ns);
        let second_slot_start_ns = burst_check_now_ns - (burst_check_now_ns % interval_ns);
        let second_dedupe_key = format!("PollInbox:{second_slot_start_ns}");
        let second = stable::list_recent_jobs(200)
            .into_iter()
            .find(|job| job.dedupe_key == second_dedupe_key)
            .expect("next slot should be materialized once");
        assert_eq!(second.status, JobStatus::Pending);

        let runtime_after_second = stable::get_task_runtime(&TaskKind::PollInbox);
        assert_eq!(
            runtime_after_second.next_due_ns,
            second_slot_start_ns.saturating_add(interval_ns)
        );
    }

    #[test]
    fn refresh_due_jobs_does_not_duplicate_slot_jobs() {
        init_scheduler_scope();
        let now_ns = 0u64;
        stable::save_task_runtime(
            &TaskKind::PollInbox,
            &TaskScheduleRuntime {
                kind: TaskKind::PollInbox,
                next_due_ns: now_ns,
                backoff_until_ns: None,
                consecutive_failures: 0,
                pending_job_id: None,
                last_started_ns: None,
                last_finished_ns: None,
                last_error: None,
            },
        );

        let interval_ns = 1_000_000_000u64; // 1 second
        let mut config =
            stable::get_task_config(&TaskKind::PollInbox).expect("poll inbox config should exist");
        config.interval_secs = 1;
        stable::upsert_task_config(config);
        let slot_start_ns = now_ns - (now_ns % interval_ns);
        let dedupe_key = format!("PollInbox:{slot_start_ns}");

        refresh_due_jobs(now_ns);
        let first_slot_count = stable::list_recent_jobs(200)
            .into_iter()
            .filter(|job| job.dedupe_key == dedupe_key)
            .count();
        assert_eq!(first_slot_count, 1);

        refresh_due_jobs(now_ns);
        let second_slot_count = stable::list_recent_jobs(200)
            .into_iter()
            .filter(|job| job.dedupe_key == dedupe_key)
            .count();
        assert_eq!(second_slot_count, 1);
    }

    #[test]
    fn topup_jobs_use_singleton_dedupe_across_periodic_and_explicit_triggers() {
        init_scheduler_scope();
        let now_ns = 30_000_000_000u64;

        let mut topup_config =
            stable::get_task_config(&TaskKind::TopUpCycles).expect("top-up config should exist");
        topup_config.enabled = true;
        topup_config.interval_secs = 30;
        stable::upsert_task_config(topup_config);

        let mut topup_runtime = stable::get_task_runtime(&TaskKind::TopUpCycles);
        topup_runtime.next_due_ns = now_ns;
        topup_runtime.pending_job_id = None;
        topup_runtime.backoff_until_ns = None;
        stable::save_task_runtime(&TaskKind::TopUpCycles, &topup_runtime);

        refresh_due_jobs(now_ns);
        let dedupe_key = topup_cycles_dedupe_key();
        let singleton_jobs: Vec<_> = stable::list_recent_jobs(200)
            .into_iter()
            .filter(|job| job.kind == TaskKind::TopUpCycles && job.dedupe_key == dedupe_key)
            .collect();
        assert_eq!(singleton_jobs.len(), 1, "periodic path should enqueue once");

        assert!(
            enqueue_topup_cycles_job("auto", now_ns).is_none(),
            "auto trigger should dedupe against pending singleton job"
        );
        assert!(
            enqueue_topup_cycles_job("tool", now_ns.saturating_add(1)).is_none(),
            "tool trigger should dedupe against pending singleton job"
        );

        let all_topup_jobs = stable::list_recent_jobs(200)
            .into_iter()
            .filter(|job| job.kind == TaskKind::TopUpCycles)
            .count();
        assert_eq!(
            all_topup_jobs, 1,
            "only one top-up job should remain queued"
        );
    }

    #[test]
    fn topup_waiting_outcome_enqueues_continuation_job() {
        stable::init_storage();
        settle_pending_topup_jobs(100);
        let before = topup_job_count();
        maybe_enqueue_topup_waiting_continuation(JobDispatchOutcome::TopUpWaiting, 100);
        let after = topup_job_count();
        assert_eq!(after, before.saturating_add(1));
        let continuation = stable::list_recent_jobs(500)
            .into_iter()
            .find(|job| job.kind == TaskKind::TopUpCycles && !job.is_terminal())
            .expect("continuation topup job should be pending");
        assert_eq!(continuation.dedupe_key, topup_cycles_dedupe_key());
        assert!(
            continuation.scheduled_for_ns > 100,
            "continuation should be scheduled in the future"
        );
    }

    #[test]
    fn topup_completed_outcome_does_not_enqueue_continuation_job() {
        stable::init_storage();
        settle_pending_topup_jobs(100);
        let before = topup_job_count();
        maybe_enqueue_topup_waiting_continuation(JobDispatchOutcome::Completed, 100);
        let after = topup_job_count();
        assert_eq!(after, before);
    }

    #[test]
    fn checkcycles_classifies_survival_tier_by_liquid_cycles() {
        let requirements = check_cycles_requirements().expect("requirements should compute");
        let low_threshold = requirements
            .required_cycles
            .saturating_mul(CHECKCYCLES_LOW_TIER_MULTIPLIER);

        let below_critical = requirements.required_cycles.saturating_sub(1);
        assert_eq!(
            classify_survival_tier(
                below_critical.saturating_add(DEFAULT_RESERVE_FLOOR_CYCLES),
                below_critical,
            )
            .expect("tier should classify"),
            SurvivalTier::Critical
        );

        let low_tier = requirements
            .required_cycles
            .saturating_add((low_threshold.saturating_sub(requirements.required_cycles)) / 2);
        assert_eq!(
            classify_survival_tier(
                low_tier.saturating_add(DEFAULT_RESERVE_FLOOR_CYCLES),
                low_tier
            )
            .expect("tier should classify"),
            SurvivalTier::LowCycles
        );

        assert_eq!(
            classify_survival_tier(
                low_threshold.saturating_add(DEFAULT_RESERVE_FLOOR_CYCLES),
                low_threshold,
            )
            .expect("tier should classify"),
            SurvivalTier::Normal
        );
    }

    #[test]
    fn checkcycles_survival_tier_recovery_hysteresis() {
        init_scheduler_scope();
        stable::set_scheduler_survival_tier(SurvivalTier::Critical);
        let mut runtime = stable::scheduler_runtime_view();
        assert_eq!(runtime.survival_tier, SurvivalTier::Critical);
        assert_eq!(runtime.survival_tier_recovery_checks, 0);

        stable::set_scheduler_survival_tier(SurvivalTier::Normal);
        runtime = stable::scheduler_runtime_view();
        assert_eq!(runtime.survival_tier, SurvivalTier::Critical);
        assert_eq!(runtime.survival_tier_recovery_checks, 1);

        stable::set_scheduler_survival_tier(SurvivalTier::Normal);
        runtime = stable::scheduler_runtime_view();
        assert_eq!(runtime.survival_tier, SurvivalTier::Critical);
        assert_eq!(runtime.survival_tier_recovery_checks, 2);

        stable::set_scheduler_survival_tier(SurvivalTier::Normal);
        runtime = stable::scheduler_runtime_view();
        assert_eq!(runtime.survival_tier, SurvivalTier::Normal);
        assert_eq!(runtime.survival_tier_recovery_checks, 0);
    }

    #[test]
    fn checkcycles_topup_trigger_requires_threshold_usdc_and_idle_state() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.cycle_topup.enabled = true;
        snapshot.cycle_topup.auto_topup_cycle_threshold = 2_000_000_000_000;
        snapshot.cycle_topup.min_usdc_reserve = 10_000_000;

        assert!(should_trigger_cycle_topup(
            1_900_000_000_000,
            &snapshot,
            None,
            Some(20_000_001)
        ));
        assert!(!should_trigger_cycle_topup(
            2_100_000_000_000,
            &snapshot,
            None,
            Some(20_000_001)
        ));
        assert!(!should_trigger_cycle_topup(
            249_000_000_000,
            &snapshot,
            None,
            Some(20_000_001)
        ));
        assert!(!should_trigger_cycle_topup(
            1_900_000_000_000,
            &snapshot,
            Some(&TopUpStage::Preflight),
            Some(20_000_001)
        ));
        assert!(!should_trigger_cycle_topup(
            1_900_000_000_000,
            &snapshot,
            Some(&TopUpStage::Failed {
                stage: "Preflight".to_string(),
                error: "boom".to_string(),
                failed_at_ns: 1,
                attempts: 1,
            }),
            Some(20_000_001)
        ));
        assert!(should_trigger_cycle_topup(
            1_900_000_000_000,
            &snapshot,
            Some(&TopUpStage::Completed {
                cycles_minted: 1,
                usdc_spent: 1,
                completed_at_ns: 1,
            }),
            Some(20_000_000)
        ));
    }

    #[test]
    fn topup_failed_recovery_due_only_after_backoff_window() {
        let failed_at_ns = 1_000_000_000_u64;
        let failed_state = TopUpStage::Failed {
            stage: "Preflight".to_string(),
            error: "boom".to_string(),
            failed_at_ns,
            attempts: 1,
        };
        let backoff_ns = TOPUP_FAILED_RECOVERY_BACKOFF_SECS.saturating_mul(1_000_000_000);

        assert!(!topup_failed_recovery_due(None, failed_at_ns));
        assert!(!topup_failed_recovery_due(
            Some(&failed_state),
            failed_at_ns.saturating_add(backoff_ns).saturating_sub(1),
        ));
        assert!(topup_failed_recovery_due(
            Some(&failed_state),
            failed_at_ns.saturating_add(backoff_ns),
        ));
    }

    #[test]
    fn maybe_recover_failed_topup_resets_and_restarts_when_backoff_elapsed() {
        stable::init_storage();
        stable::clear_topup_state();

        let now_ns = TOPUP_FAILED_RECOVERY_BACKOFF_SECS
            .saturating_mul(1_000_000_000)
            .saturating_add(100);
        let failed_state = TopUpStage::Failed {
            stage: "Preflight".to_string(),
            error: "boom".to_string(),
            failed_at_ns: 0,
            attempts: 1,
        };
        stable::write_topup_state(&failed_state);

        let snapshot = RuntimeSnapshot {
            evm_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            cycle_topup: crate::domain::types::CycleTopUpConfig {
                enabled: true,
                ..crate::domain::types::CycleTopUpConfig::default()
            },
            ..RuntimeSnapshot::default()
        };

        assert!(maybe_recover_failed_topup(
            &snapshot,
            Some(&failed_state),
            now_ns
        ));
        assert!(matches!(
            stable::read_topup_state(),
            Some(TopUpStage::Preflight)
        ));
    }

    #[test]
    fn maybe_recover_failed_topup_respects_backoff_window() {
        stable::init_storage();
        stable::clear_topup_state();

        let failed_at_ns = 10_000u64;
        let failed_state = TopUpStage::Failed {
            stage: "Preflight".to_string(),
            error: "boom".to_string(),
            failed_at_ns,
            attempts: 1,
        };
        stable::write_topup_state(&failed_state);

        let snapshot = RuntimeSnapshot {
            evm_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            cycle_topup: crate::domain::types::CycleTopUpConfig {
                enabled: true,
                ..crate::domain::types::CycleTopUpConfig::default()
            },
            ..RuntimeSnapshot::default()
        };

        let now_ns = failed_at_ns.saturating_add(1);
        assert!(!maybe_recover_failed_topup(
            &snapshot,
            Some(&failed_state),
            now_ns,
        ));
        assert!(matches!(
            stable::read_topup_state(),
            Some(TopUpStage::Failed { .. })
        ));
    }

    #[test]
    fn operation_class_mapping_for_scheduler_tasks_is_stable() {
        assert_eq!(
            operation_class_for_task(&TaskKind::AgentTurn),
            Some(SurvivalOperationClass::Inference)
        );
        assert_eq!(
            operation_class_for_task(&TaskKind::PollInbox),
            Some(SurvivalOperationClass::EvmPoll)
        );
        assert_eq!(operation_class_for_task(&TaskKind::CheckCycles), None);
        assert_eq!(operation_class_for_task(&TaskKind::TopUpCycles), None);
        assert_eq!(operation_class_for_task(&TaskKind::Reconcile), None);
    }

    #[test]
    fn lease_ttl_for_agent_turn_is_extended_for_continuation_rounds() {
        assert_eq!(
            lease_ttl_ns(&TaskKind::AgentTurn),
            timing::AGENT_TURN_LEASE_TTL_NS
        );
        assert_eq!(
            lease_ttl_ns(&TaskKind::PollInbox),
            timing::LIGHTWEIGHT_LEASE_TTL_NS
        );
        assert_eq!(
            lease_ttl_ns(&TaskKind::CheckCycles),
            timing::LIGHTWEIGHT_LEASE_TTL_NS
        );
        assert_eq!(
            lease_ttl_ns(&TaskKind::TopUpCycles),
            timing::LIGHTWEIGHT_LEASE_TTL_NS
        );
        assert_eq!(
            lease_ttl_ns(&TaskKind::Reconcile),
            timing::LIGHTWEIGHT_LEASE_TTL_NS
        );
    }

    #[test]
    fn scheduler_tick_recovers_orphaned_turn_lock_after_stale_agent_turn_lease_timeout() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        let _ = stable::increment_turn_counter();
        let dedupe_key = "AgentTurn:stale-lease-recovery".to_string();
        let job_id = stable::enqueue_job_if_absent(
            TaskKind::AgentTurn,
            TaskLane::Mutating,
            dedupe_key.clone(),
            0,
            0,
        )
        .expect("agent turn job should enqueue");
        let popped = stable::pop_next_pending_job(TaskLane::Mutating, 0).expect("job should pop");
        assert_eq!(popped.id, job_id);
        stable::acquire_mutating_lease(&job_id, 0, 1).expect("lease should acquire");

        block_on_with_spin(scheduler_tick());

        assert!(
            !stable::runtime_snapshot().turn_in_flight,
            "scheduler tick should clear orphaned turn lock after stale lease timeout"
        );
        let jobs = stable::list_recent_jobs(20);
        let timed_out = jobs
            .iter()
            .find(|job| job.dedupe_key == dedupe_key)
            .expect("timed-out job should be retained in recent jobs");
        assert_eq!(timed_out.status, JobStatus::TimedOut);
        assert!(
            timed_out
                .last_error
                .as_deref()
                .unwrap_or_default()
                .contains("mutating lease expired"),
            "stale lease timeout should preserve diagnostic reason"
        );
    }

    #[test]
    fn low_tier_policy_allows_only_inference_and_evm_poll_operations() {
        init_scheduler_scope();
        stable::set_scheduler_survival_tier(SurvivalTier::LowCycles);

        assert!(stable::can_run_survival_operation(
            &SurvivalOperationClass::Inference,
            10
        ));
        assert!(stable::can_run_survival_operation(
            &SurvivalOperationClass::EvmPoll,
            10
        ));
        assert!(!stable::can_run_survival_operation(
            &SurvivalOperationClass::EvmBroadcast,
            10
        ));
        assert!(!stable::can_run_survival_operation(
            &SurvivalOperationClass::ThresholdSign,
            10
        ));
    }

    #[test]
    fn scheduler_blocks_inference_job_when_survival_operation_backoff_active() {
        init_scheduler_scope();
        let now_ns = 10u64;
        stable::set_scheduler_survival_tier(SurvivalTier::Normal);
        stable::record_survival_operation_failure(&SurvivalOperationClass::Inference, now_ns, 1);

        assert!(!stable::can_run_survival_operation(
            &SurvivalOperationClass::Inference,
            now_ns.saturating_add(500),
        ));
        assert_eq!(
            stable::survival_operation_consecutive_failures(&SurvivalOperationClass::Inference),
            1
        );
    }

    #[test]
    fn scheduler_blocks_operation_class_on_critical_tier() {
        init_scheduler_scope();
        stable::set_scheduler_survival_tier(SurvivalTier::Critical);

        assert!(!stable::can_run_survival_operation(
            &SurvivalOperationClass::Inference,
            10
        ));
        assert!(!stable::can_run_survival_operation(
            &SurvivalOperationClass::EvmPoll,
            10
        ));
    }

    #[test]
    fn scheduler_allows_operations_after_low_tier_backoff_cooldown() {
        init_scheduler_scope();
        stable::set_scheduler_survival_tier(SurvivalTier::Normal);
        let now_ns = 10u64;
        stable::record_survival_operation_failure(&SurvivalOperationClass::Inference, now_ns, 1);

        assert!(!stable::can_run_survival_operation(
            &SurvivalOperationClass::Inference,
            now_ns.saturating_add(500),
        ));

        let cooldown = stable::survival_operation_backoff_until(&SurvivalOperationClass::Inference)
            .expect("backoff should be active");
        assert!(
            !stable::can_run_survival_operation(
                &SurvivalOperationClass::Inference,
                cooldown.saturating_sub(1)
            ),
            "operation should remain blocked at backoff boundary"
        );
        let after_backoff = cooldown.saturating_add(1);
        assert!(
            stable::can_run_survival_operation(&SurvivalOperationClass::Inference, after_backoff),
            "operation should be runnable after backoff window"
        );
    }

    #[test]
    fn scheduler_tick_drains_multiple_pending_jobs_in_one_tick() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        let poll_job = stable::enqueue_job_if_absent(
            TaskKind::PollInbox,
            TaskLane::Mutating,
            "PollInbox:manual-1".to_string(),
            0,
            0,
        );
        let reconcile_job = stable::enqueue_job_if_absent(
            TaskKind::Reconcile,
            TaskLane::Mutating,
            "Reconcile:manual-1".to_string(),
            0,
            1,
        );
        assert!(poll_job.is_some(), "poll job should enqueue");
        assert!(reconcile_job.is_some(), "reconcile job should enqueue");

        block_on_with_spin(scheduler_tick());

        let jobs = stable::list_recent_jobs(10);
        let poll = jobs
            .iter()
            .find(|job| job.dedupe_key == "PollInbox:manual-1")
            .expect("poll job should be present");
        let reconcile = jobs
            .iter()
            .find(|job| job.dedupe_key == "Reconcile:manual-1")
            .expect("reconcile job should be present");
        assert_eq!(poll.status, JobStatus::Succeeded);
        assert_eq!(reconcile.status, JobStatus::Succeeded);
    }

    #[test]
    fn reconcile_job_activates_template_when_dry_run_compile_passes() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        seed_strategy_for_reconcile(
            "reconcile-activate",
            current_time_ns(),
            "0xa9059cbb",
            TemplateStatus::Active,
            false,
        );

        let reconcile_job = stable::enqueue_job_if_absent(
            TaskKind::Reconcile,
            TaskLane::Mutating,
            "Reconcile:strategy-activate".to_string(),
            0,
            0,
        );
        assert!(reconcile_job.is_some(), "reconcile job should enqueue");
        block_on_with_spin(scheduler_tick());

        let activation = crate::strategy::registry::activation(&strategy_key("reconcile-activate"))
            .expect("activation should exist");
        assert!(activation.enabled);
        assert!(activation
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("dry-run"));
    }

    #[test]
    fn reconcile_job_disables_stale_template_activation() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        let stale_updated_at_ns = current_time_ns().saturating_sub(
            STRATEGY_TEMPLATE_FRESHNESS_WINDOW_SECS
                .saturating_add(1)
                .saturating_mul(1_000_000_000),
        );
        seed_strategy_for_reconcile(
            "reconcile-stale",
            stale_updated_at_ns,
            "0xa9059cbb",
            TemplateStatus::Active,
            true,
        );

        let reconcile_job = stable::enqueue_job_if_absent(
            TaskKind::Reconcile,
            TaskLane::Mutating,
            "Reconcile:strategy-stale".to_string(),
            0,
            0,
        );
        assert!(reconcile_job.is_some(), "reconcile job should enqueue");
        block_on_with_spin(scheduler_tick());

        let activation = crate::strategy::registry::activation(&strategy_key("reconcile-stale"))
            .expect("activation should exist");
        assert!(!activation.enabled);
        assert!(activation
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("stale_template"));
    }

    #[test]
    fn reconcile_job_disables_activation_when_provenance_fails() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        seed_strategy_for_reconcile(
            "reconcile-provenance",
            current_time_ns(),
            "0xdeadbeef",
            TemplateStatus::Active,
            true,
        );

        let reconcile_job = stable::enqueue_job_if_absent(
            TaskKind::Reconcile,
            TaskLane::Mutating,
            "Reconcile:strategy-provenance".to_string(),
            0,
            0,
        );
        assert!(reconcile_job.is_some(), "reconcile job should enqueue");
        block_on_with_spin(scheduler_tick());

        let activation =
            crate::strategy::registry::activation(&strategy_key("reconcile-provenance"))
                .expect("activation should exist");
        assert!(!activation.enabled);
        assert!(activation
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("provenance_or_dry_run_failed"));
    }

    #[test]
    fn exposure_reconciliation_repairs_missing_state_after_execution() {
        let _ = crate::storage::sqlite::close_storage();
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        let strategy_id = "morpho-v1:supply:8453:morpho-enter-supply".to_string();
        use crate::domain::types::{
            PendingStrategyExecution, PendingStrategyExecutionCall, StrategyAssetDirection,
            StrategyAssetEffect, StrategyExecutionCall,
        };
        stable::insert_pending_strategy_execution(PendingStrategyExecution {
            execution_id: "strategy:turn-enter:digest".into(),
            turn_id: "turn-enter".into(),
            key: StrategyTemplateKey {
                protocol: "morpho-v1".into(),
                primitive: "supply".into(),
                chain_id: 8453,
                template_id: "morpho-enter-supply".into(),
            },
            action_id: "enter_supply".into(),
            plan_digest: "digest".into(),
            asset_effects: vec![StrategyAssetEffect {
                chain_id: 8453,
                asset_address: None,
                asset_symbol: "USDC".into(),
                decimals: 18,
                amount_raw: "50000000000000000".into(),
                direction: StrategyAssetDirection::Debit,
            }],
            calls: vec![PendingStrategyExecutionCall {
                index: 0,
                call: StrategyExecutionCall {
                    role: "pool".into(),
                    to: "0x1111111111111111111111111111111111111111".into(),
                    value_wei: "0".into(),
                    data: "0x".into(),
                },
                tx_hash: Some("0xabc".into()),
                state: StrategyExecutionCallState::Confirmed,
                receipt_block_number: Some(1),
                receipt_block_hash: Some("0xblock".into()),
                submitted_at_ns: Some(1_000),
                last_checked_at_ns: Some(1_000),
                error: None,
            }],
            state: PendingStrategyExecutionState::Confirmed,
            created_at_ns: 1_000,
            updated_at_ns: 1_000,
            next_check_at_ns: 1_000,
            consecutive_rpc_failures: 0,
            bookkeeping_applied: true,
            terminal_bookkeeping_applied: false,
        })
        .unwrap();
        assert!(
            stable::active_exposure(&strategy_id).is_none(),
            "test setup should start without local exposure"
        );

        futures::executor::block_on(run_reconcile_job(current_time_ns()))
            .expect("reconcile job should succeed");

        let exposure = stable::active_exposure(&strategy_id)
            .expect("reconcile job should rebuild the missing exposure");
        assert_eq!(exposure.protocol, "morpho-v1");
        assert_eq!(exposure.chain_id, 8453);
        assert_eq!(exposure.asset_symbol, "USDC");
        assert_eq!(exposure.notional_wei, Some(50_000_000_000_000_000));

        let status = stable::exposure_reconciliation_status();
        assert_eq!(status.recreated_exposures, 1);
        assert_eq!(status.repaired_exposures, 0);
        assert_eq!(status.closed_exposures, 0);
        assert!(status
            .drift_reason
            .as_deref()
            .unwrap_or_default()
            .contains("recreated_missing_exposure"));
    }

    #[test]
    fn scheduler_tick_runs_retention_maintenance_in_low_priority_lane() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }
        stable::set_retention_config(RetentionConfig {
            jobs_max_age_secs: 1,
            jobs_max_records: 0,
            dedupe_max_age_secs: 1,
            turns_max_age_secs: 7 * 24 * 60 * 60,
            transitions_max_age_secs: 7 * 24 * 60 * 60,
            tools_max_age_secs: 7 * 24 * 60 * 60,
            inbox_max_age_secs: 14 * 24 * 60 * 60,
            outbox_max_age_secs: 14 * 24 * 60 * 60,
            memory_facts_max_age_secs: 3 * 24 * 60 * 60,
            memory_facts_prune_batch_size: 25,
            maintenance_batch_size: 50,
            maintenance_interval_secs: 30,
        })
        .expect("retention config should persist");
        let now_ns = current_time_ns();
        let old_scheduled_for_ns = now_ns.saturating_sub(5_000_000_000);
        let old_job_id = "job:00000000000000000001:00000000000000000001".to_string();
        stable::save_job_for_tests(ScheduledJob {
            id: old_job_id.clone(),
            kind: TaskKind::PollInbox,
            lane: TaskLane::Mutating,
            dedupe_key: format!("PollInbox:{}", old_scheduled_for_ns),
            priority: 1,
            created_at_ns: old_scheduled_for_ns,
            scheduled_for_ns: old_scheduled_for_ns,
            started_at_ns: Some(old_scheduled_for_ns),
            finished_at_ns: Some(old_scheduled_for_ns.saturating_add(1)),
            status: JobStatus::Succeeded,
            attempts: 1,
            max_attempts: 3,
            last_error: None,
        });
        stable::insert_dedupe_for_tests(
            format!("PollInbox:{}", old_scheduled_for_ns),
            old_job_id.clone(),
        );

        let poll_job = stable::enqueue_job_if_absent(
            TaskKind::PollInbox,
            TaskLane::Mutating,
            "PollInbox:maintenance-order".to_string(),
            now_ns,
            0,
        );
        assert!(poll_job.is_some(), "poll job should enqueue");

        block_on_with_spin(scheduler_tick());

        let recent = stable::list_recent_jobs(200);
        let manual = recent
            .iter()
            .find(|job| job.dedupe_key == "PollInbox:maintenance-order")
            .expect("manual poll job should still run before maintenance");
        assert_eq!(manual.status, JobStatus::Succeeded);
        assert!(
            recent.iter().all(|job| job.id != old_job_id),
            "retention maintenance should prune old terminal jobs"
        );
    }

    #[test]
    fn poll_inbox_job_skips_evm_poll_when_inbox_contract_is_unset() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        stable::set_evm_cursor(&crate::domain::types::EvmPollCursor {
            chain_id: 8453,
            next_block: 0,
            next_log_index: 0,
            ..crate::domain::types::EvmPollCursor::default()
        });

        let poll_job = stable::enqueue_job_if_absent(
            TaskKind::PollInbox,
            TaskLane::Mutating,
            "PollInbox:evm-cursor".to_string(),
            0,
            0,
        );
        assert!(poll_job.is_some(), "poll job should enqueue");

        block_on_with_spin(scheduler_tick());

        let cursor = stable::runtime_snapshot().evm_cursor;
        assert_eq!(cursor.next_block, 0);
        assert_eq!(
            stable::survival_operation_consecutive_failures(&SurvivalOperationClass::EvmPoll),
            0
        );

        let jobs = stable::list_recent_jobs(10);
        let poll = jobs
            .iter()
            .find(|job| job.dedupe_key == "PollInbox:evm-cursor")
            .expect("poll job should be present");
        assert_eq!(poll.status, JobStatus::Succeeded);
    }

    #[test]
    fn poll_inbox_job_stages_pending_messages_without_running_agent_turn() {
        stable::init_storage();
        stable::init_scheduler_defaults(0);
        for kind in TaskKind::all() {
            stable::set_task_enabled(kind, false);
        }

        stable::post_inbox_message(
            "first staged by poll".to_string(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        )
        .expect("first inbox message should be accepted");
        stable::post_inbox_message(
            "second staged by poll".to_string(),
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        )
        .expect("second inbox message should be accepted");

        let turn_counter_before = stable::runtime_snapshot().turn_counter;
        let poll_job = stable::enqueue_job_if_absent(
            TaskKind::PollInbox,
            TaskLane::Mutating,
            "PollInbox:stage-only".to_string(),
            0,
            0,
        );
        assert!(poll_job.is_some(), "poll job should enqueue");

        block_on_with_spin(scheduler_tick());

        let stats = stable::inbox_stats();
        assert_eq!(stats.pending_count, 0);
        assert_eq!(stats.staged_count, 2);
        assert_eq!(stats.consumed_count, 0);
        let staged = stable::list_staged_inbox_messages(10);
        assert_eq!(staged.len(), 2);
        assert!(
            staged
                .iter()
                .all(|message| message.source == InboxMessageSource::EvmInbox),
            "poll path must keep EVM-ingested messages tagged as evm_inbox"
        );
        assert_eq!(stable::runtime_snapshot().turn_counter, turn_counter_before);
        assert!(
            stable::list_outbox_messages(10).is_empty(),
            "poll job must not emit an outbox reply"
        );
    }

    #[test]
    fn verified_patronage_increments_telemetry_without_creating_inbox_attention() {
        stable::init_storage();
        let inbox_before = stable::inbox_stats().total_messages;
        let turns_before = stable::runtime_snapshot().turn_counter;
        let patronage_before = stable::runtime_snapshot()
            .lifetime_patronage_usdc_raw
            .parse::<u128>()
            .expect("patronage counter should be numeric");
        let event = EvmEvent {
            tx_hash: format!("0x{}", "ab".repeat(32)),
            chain_id: 8453,
            block_number: 42,
            log_index: 7,
            source: "0x2222222222222222222222222222222222222222".to_string(),
            payload: format!("0x{:064x}", U256::from(2_250_000_u64)),
        };

        assert_eq!(
            ingest_verified_patronage_event(&event).unwrap(),
            Some(2_250_000)
        );
        let snapshot = stable::runtime_snapshot();
        assert_eq!(
            snapshot
                .lifetime_patronage_usdc_raw
                .parse::<u128>()
                .unwrap(),
            patronage_before + 2_250_000
        );
        assert_eq!(stable::inbox_stats().total_messages, inbox_before);
        assert_eq!(snapshot.turn_counter, turns_before);
    }

    #[test]
    fn poll_inbox_job_advances_evm_cursor_when_filters_are_configured() {
        with_host_stub_env(
            &[("IC_AUTOMATON_EVM_RPC_STUB_MAX_LOG_BLOCK_SPAN", Some("10"))],
            || {
                stable::init_storage();
                stable::init_scheduler_defaults(0);
                for kind in TaskKind::all() {
                    stable::set_task_enabled(kind, false);
                }

                stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
                    .expect("rpc url should be configurable");
                stable::set_evm_address(Some(
                    "0x1111111111111111111111111111111111111111".to_string(),
                ))
                .expect("evm address should be configurable");
                stable::set_inbox_contract_address(Some(
                    "0x2222222222222222222222222222222222222222".to_string(),
                ))
                .expect("inbox contract should be configurable");
                stable::set_evm_cursor(&crate::domain::types::EvmPollCursor {
                    chain_id: 8453,
                    next_block: 0,
                    next_log_index: 0,
                    ..crate::domain::types::EvmPollCursor::default()
                });

                let poll_job = stable::enqueue_job_if_absent(
                    TaskKind::PollInbox,
                    TaskLane::Mutating,
                    "PollInbox:evm-cursor".to_string(),
                    0,
                    0,
                );
                assert!(poll_job.is_some(), "poll job should enqueue");

                block_on_with_spin(scheduler_tick());

                let cursor = stable::runtime_snapshot().evm_cursor;
                assert_eq!(cursor.next_block, 1);
                assert_eq!(cursor.consecutive_empty_polls, 1);
                assert!(cursor.last_poll_at_ns > 0);

                let jobs = stable::list_recent_jobs(10);
                let poll = jobs
                    .iter()
                    .find(|job| job.dedupe_key == "PollInbox:evm-cursor")
                    .expect("poll job should be present");
                assert_eq!(poll.status, JobStatus::Succeeded);
            },
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn poll_inbox_eth_get_logs_failures_wait_for_next_scheduling_slot() {
        stable::init_storage();
        init_scheduler_scope();
        stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
            .expect("rpc url should be configurable");
        stable::set_evm_address(Some(
            "0x1111111111111111111111111111111111111111".to_string(),
        ))
        .expect("evm address should be configurable");
        stable::set_inbox_contract_address(Some(
            "0x2222222222222222222222222222222222222222".to_string(),
        ))
        .expect("inbox contract should be configurable");

        let dedupe_key = "PollInbox:no-retry-eth-getlogs".to_string();
        with_host_stub_env(
            &[("IC_AUTOMATON_EVM_RPC_STUB_MAX_LOG_BLOCK_SPAN", Some("0"))],
            || {
                let poll_job = stable::enqueue_job_if_absent(
                    TaskKind::PollInbox,
                    TaskLane::Mutating,
                    dedupe_key.clone(),
                    0,
                    0,
                );
                assert!(poll_job.is_some(), "poll job should enqueue");

                let processed = block_on_with_spin(run_one_pending_mutating_job(0))
                    .expect("poll job should be processed");
                assert!(processed, "one job should be processed");
            },
        );

        let jobs = stable::list_recent_jobs(20);
        let poll = jobs
            .iter()
            .find(|job| job.dedupe_key == dedupe_key)
            .expect("poll job should be present");
        assert_eq!(poll.status, JobStatus::Skipped);
        assert!(
            poll.last_error
                .as_deref()
                .unwrap_or_default()
                .contains("eth_getLogs failed"),
            "failed eth_getLogs error should be preserved for diagnostics"
        );
        assert_eq!(
            stable::survival_operation_consecutive_failures(&SurvivalOperationClass::EvmPoll),
            0
        );

        let runtime = stable::get_task_runtime(&TaskKind::PollInbox);
        assert!(runtime.pending_job_id.is_none());
        assert!(runtime.backoff_until_ns.is_none());
        assert_eq!(runtime.consecutive_failures, 0);
        assert!(
            !jobs.iter().any(|job| {
                job.kind == TaskKind::PollInbox && matches!(job.status, JobStatus::Pending)
            }),
            "poll inbox failure should not enqueue immediate/backoff retries"
        );
    }

    #[test]
    fn poll_inbox_job_skips_rpc_outcall_until_empty_poll_backoff_expires() {
        stable::init_storage();
        stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
            .expect("rpc url should be configurable");
        stable::set_evm_address(Some(
            "0x1111111111111111111111111111111111111111".to_string(),
        ))
        .expect("evm address should be configurable");
        stable::set_inbox_contract_address(Some(
            "0x2222222222222222222222222222222222222222".to_string(),
        ))
        .expect("inbox contract should be configurable");

        let now_ns = 500_000_000_000u64;
        // Gap must be shorter than the backoff for consecutive_empty_polls=2.
        // Backoff index 1 = BASE_TICK_SECS * 4 seconds.
        let gap_ns = 1_000_000_000u64; // 1 second — well within any backoff window
        stable::set_evm_cursor(&crate::domain::types::EvmPollCursor {
            chain_id: 8453,
            next_block: 10,
            next_log_index: 0,
            last_poll_at_ns: now_ns.saturating_sub(gap_ns),
            consecutive_empty_polls: 2,
            ..crate::domain::types::EvmPollCursor::default()
        });

        block_on_with_spin(run_poll_inbox_job(now_ns)).expect("poll job should not fail");
        let after = stable::runtime_snapshot().evm_cursor;
        assert_eq!(
            after.next_block, 10,
            "cursor must not advance when backoff window is active"
        );
        assert_eq!(
            after.last_poll_at_ns,
            now_ns.saturating_sub(gap_ns),
            "last poll timestamp must stay unchanged when skipping rpc outcall"
        );
    }

    #[test]
    fn poll_inbox_job_syncs_wallet_balances_and_clears_bootstrap_pending() {
        with_clean_host_stub_env(|| {
            stable::init_storage();
            stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
                .expect("rpc url should be configurable");
            stable::set_evm_address(Some(
                "0x1111111111111111111111111111111111111111".to_string(),
            ))
            .expect("evm address should be configurable");
            stable::set_inbox_contract_address(Some(
                "0x2222222222222222222222222222222222222222".to_string(),
            ))
            .expect("inbox contract should be configurable");
            stable::set_wallet_balance_bootstrap_pending(true);

            let now_ns = 123_000_000_000u64;
            block_on_with_spin(run_poll_inbox_job(now_ns)).expect("poll job should not fail");

            let balance = stable::wallet_balance_snapshot();
            assert_eq!(balance.eth_balance_wei_hex.as_deref(), Some("0x1"));
            assert_eq!(balance.usdc_balance_raw_hex.as_deref(), Some("0x2a"));
            assert_eq!(
                balance.usdc_contract_address.as_deref(),
                Some("0x3333333333333333333333333333333333333333")
            );
            assert_eq!(balance.last_synced_at_ns, Some(now_ns));
            assert!(balance.last_error.is_none());
            assert!(
                !stable::wallet_balance_bootstrap_pending(),
                "successful first sync should clear bootstrap gate"
            );
        });
    }

    #[test]
    fn wallet_sync_recovery_policy_tunes_response_limit_for_oversized_errors() {
        let snapshot = RuntimeSnapshot {
            wallet_balance_sync: WalletBalanceSyncConfig {
                max_response_bytes: 256,
                ..WalletBalanceSyncConfig::default()
            },
            ..RuntimeSnapshot::default()
        };
        let failure = classify_evm_failure(
            "eth_call failed: evm rpc outcall failed: call rejected: 1 - Http body exceeds size limit of 256 bytes.",
        );
        let decision = decide_recovery_action(&failure, &wallet_sync_recovery_context(&snapshot));

        assert_eq!(decision.action, RecoveryPolicyAction::TuneResponseLimit);
        assert_eq!(
            decision
                .response_limit_adjustment
                .as_ref()
                .map(|adjustment| (adjustment.from_bytes, adjustment.to_bytes)),
            Some((256, 512))
        );
    }

    #[test]
    fn wallet_sync_response_limit_recovery_persists_in_runtime_config() {
        stable::init_storage();
        let config = WalletBalanceSyncConfig {
            max_response_bytes: 256,
            ..WalletBalanceSyncConfig::default()
        };
        stable::set_wallet_balance_sync_config(config).expect("wallet sync config should persist");

        apply_response_limit_tuning(
            &RecoveryOperation::WalletBalanceSync,
            &ResponseLimitAdjustment {
                from_bytes: 256,
                to_bytes: 512,
            },
        )
        .expect("wallet sync max response bytes should tune");

        assert_eq!(stable::wallet_balance_sync_config().max_response_bytes, 512);
    }

    #[test]
    fn poll_inbox_job_wallet_sync_uses_tier_aware_due_windows_after_bootstrap() {
        with_clean_host_stub_env(|| {
            stable::init_storage();
            stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
                .expect("rpc url should be configurable");
            stable::set_evm_address(Some(
                "0x1111111111111111111111111111111111111111".to_string(),
            ))
            .expect("evm address should be configurable");
            stable::set_inbox_contract_address(Some(
                "0x2222222222222222222222222222222222222222".to_string(),
            ))
            .expect("inbox contract should be configurable");
            stable::set_wallet_balance_sync_config(WalletBalanceSyncConfig {
                normal_interval_secs: 300,
                low_cycles_interval_secs: 900,
                ..WalletBalanceSyncConfig::default()
            })
            .expect("wallet sync config should persist");
            stable::set_wallet_balance_snapshot(WalletBalanceSnapshot {
                eth_balance_wei_hex: Some("0xaaaa".to_string()),
                usdc_balance_raw_hex: Some("0xbbbb".to_string()),
                usdc_decimals: 6,
                usdc_contract_address: Some(
                    "0x3333333333333333333333333333333333333333".to_string(),
                ),
                last_synced_at_ns: Some(1_000_000_000_000),
                last_synced_block: None,
                last_error: None,
            });
            stable::set_wallet_balance_bootstrap_pending(false);

            stable::set_scheduler_survival_tier(SurvivalTier::Normal);
            block_on_with_spin(run_poll_inbox_job(1_250_000_000_000))
                .expect("normal-tier pre-due poll should succeed");
            let after_normal_skip = stable::wallet_balance_snapshot();
            assert_eq!(
                after_normal_skip.eth_balance_wei_hex.as_deref(),
                Some("0xaaaa")
            );
            assert_eq!(after_normal_skip.last_synced_at_ns, Some(1_000_000_000_000));

            stable::set_scheduler_survival_tier(SurvivalTier::LowCycles);
            block_on_with_spin(run_poll_inbox_job(1_600_000_000_000))
                .expect("low-tier pre-due poll should succeed");
            let after_low_skip = stable::wallet_balance_snapshot();
            assert_eq!(
                after_low_skip.eth_balance_wei_hex.as_deref(),
                Some("0xaaaa")
            );
            assert_eq!(after_low_skip.last_synced_at_ns, Some(1_000_000_000_000));

            block_on_with_spin(run_poll_inbox_job(1_901_000_000_000))
                .expect("low-tier due poll should succeed");
            let after_due = stable::wallet_balance_snapshot();
            assert_eq!(after_due.eth_balance_wei_hex.as_deref(), Some("0x1"));
            assert_eq!(after_due.usdc_balance_raw_hex.as_deref(), Some("0x2a"));
            assert_eq!(after_due.last_synced_at_ns, Some(1_901_000_000_000));
        });
    }

    #[test]
    fn poll_inbox_job_wallet_sync_failure_is_non_fatal_and_preserves_last_good_snapshot() {
        with_clean_host_stub_env(|| {
            stable::init_storage();
            stable::set_evm_rpc_url("https://mainnet.base.org".to_string())
                .expect("rpc url should be configurable");
            stable::set_evm_address(Some(
                "0x1111111111111111111111111111111111111111".to_string(),
            ))
            .expect("evm address should be configurable");
            stable::set_wallet_balance_sync_config(WalletBalanceSyncConfig {
                discover_usdc_via_inbox: false,
                ..WalletBalanceSyncConfig::default()
            })
            .expect("wallet sync config should persist");
            stable::set_wallet_balance_snapshot(WalletBalanceSnapshot {
                eth_balance_wei_hex: Some("0x9999".to_string()),
                usdc_balance_raw_hex: Some("0x8888".to_string()),
                usdc_decimals: 6,
                usdc_contract_address: None,
                last_synced_at_ns: Some(55),
                last_synced_block: Some(7),
                last_error: None,
            });
            stable::set_wallet_balance_bootstrap_pending(true);

            block_on_with_spin(run_poll_inbox_job(777)).expect("poll job should not fail");

            let balance = stable::wallet_balance_snapshot();
            assert_eq!(balance.eth_balance_wei_hex.as_deref(), Some("0x9999"));
            assert_eq!(balance.usdc_balance_raw_hex.as_deref(), Some("0x8888"));
            assert_eq!(balance.last_synced_at_ns, Some(55));
            assert_eq!(balance.last_synced_block, Some(7));
            assert_eq!(
                balance.last_error.as_deref(),
                Some("usdc contract address is not configured")
            );
            assert!(
                stable::wallet_balance_bootstrap_pending(),
                "bootstrap gate must remain set until a successful sync occurs"
            );
        });
    }

    #[test]
    fn poll_inbox_job_incrementally_advances_room_cursor() {
        crate::features::factory_room::clear_mock_factory_room_call();
        stable::init_storage();
        configure_factory_room_access();

        let call_count = Rc::new(Cell::new(0u32));
        let call_count_for_mock = Rc::clone(&call_count);
        crate::features::factory_room::set_mock_factory_room_call(
            move |canister_id, method, encoded_args| {
                call_count_for_mock.set(call_count_for_mock.get().saturating_add(1));
                assert_eq!(canister_id, test_factory_principal());
                assert_eq!(method, "list_my_room_messages");
                let (after_seq, limit): (Option<u64>, Option<u64>) =
                    candid::decode_args(encoded_args).expect("query args should decode");
                assert_eq!(after_seq, None);
                assert_eq!(limit, Some(ROOM_POLL_PAGE_LIMIT));

                candid::encode_one(crate::features::factory_room::FactoryRoomCallResult::Ok(
                    RoomMessagePage {
                        messages: vec![sample_room_message(7), sample_room_message(9)],
                        next_after_seq: Some(9),
                        latest_seq: Some(12),
                    },
                ))
                .map_err(|error| error.to_string())
            },
        );

        block_on_with_spin(run_poll_inbox_job(1_000)).expect("poll job should not fail");

        let room_poll = stable::room_poll_state();
        let room_observations = stable::runtime_snapshot().room_observations;
        assert_eq!(call_count.get(), 1);
        assert_eq!(room_poll.last_seen_seq, Some(9));
        assert_eq!(room_poll.last_attempted_at_ns, Some(1_000));
        assert_eq!(room_poll.last_succeeded_at_ns, Some(1_000));
        assert_eq!(room_poll.last_known_latest_seq, Some(12));
        assert_eq!(room_poll.last_batch_count, 2);
        assert_eq!(room_poll.consecutive_failures, 0);
        assert!(room_poll.last_error.is_none());
        assert_eq!(room_observations.len(), 2);
        assert_eq!(room_observations[0].seq, 7);
        assert_eq!(room_observations[1].seq, 9);
        assert_eq!(room_observations[0].body, "untrusted room body");

        crate::features::factory_room::clear_mock_factory_room_call();
    }

    #[test]
    fn poll_inbox_job_advances_room_cursor_to_head_when_filtered_read_is_caught_up() {
        crate::features::factory_room::clear_mock_factory_room_call();
        stable::init_storage();
        configure_factory_room_access();
        stable::record_room_poll_success(500, Some(5), Some(5), 1);

        crate::features::factory_room::set_mock_factory_room_call(
            move |canister_id, method, encoded_args| {
                assert_eq!(canister_id, test_factory_principal());
                assert_eq!(method, "list_my_room_messages");
                let (after_seq, limit): (Option<u64>, Option<u64>) =
                    candid::decode_args(encoded_args).expect("query args should decode");
                assert_eq!(after_seq, Some(5));
                assert_eq!(limit, Some(ROOM_POLL_PAGE_LIMIT));

                candid::encode_one(crate::features::factory_room::FactoryRoomCallResult::Ok(
                    RoomMessagePage {
                        messages: vec![sample_room_message(9)],
                        next_after_seq: None,
                        latest_seq: Some(15),
                    },
                ))
                .map_err(|error| error.to_string())
            },
        );

        let due_now_ns = 500 + ROOM_POLL_INTERVAL_SECS.saturating_mul(1_000_000_000);
        block_on_with_spin(run_poll_inbox_job(due_now_ns)).expect("poll job should not fail");

        let room_poll = stable::room_poll_state();
        assert_eq!(room_poll.last_seen_seq, Some(15));
        assert_eq!(room_poll.last_attempted_at_ns, Some(due_now_ns));
        assert_eq!(room_poll.last_succeeded_at_ns, Some(due_now_ns));
        assert_eq!(room_poll.last_known_latest_seq, Some(15));
        assert_eq!(room_poll.last_batch_count, 1);
        assert_eq!(room_poll.consecutive_failures, 0);
        assert!(room_poll.last_error.is_none());

        crate::features::factory_room::clear_mock_factory_room_call();
    }

    #[test]
    fn poll_inbox_job_room_poll_tracks_room_head_when_filtered_page_is_empty() {
        crate::features::factory_room::clear_mock_factory_room_call();
        stable::init_storage();
        configure_factory_room_access();
        stable::record_room_poll_success(500, Some(9), Some(12), 1);

        crate::features::factory_room::set_mock_factory_room_call(
            move |canister_id, method, encoded_args| {
                assert_eq!(canister_id, test_factory_principal());
                assert_eq!(method, "list_my_room_messages");
                let (after_seq, limit): (Option<u64>, Option<u64>) =
                    candid::decode_args(encoded_args).expect("query args should decode");
                assert_eq!(after_seq, Some(9));
                assert_eq!(limit, Some(ROOM_POLL_PAGE_LIMIT));

                candid::encode_one(crate::features::factory_room::FactoryRoomCallResult::Ok(
                    RoomMessagePage {
                        messages: Vec::new(),
                        next_after_seq: None,
                        latest_seq: Some(15),
                    },
                ))
                .map_err(|error| error.to_string())
            },
        );

        let due_now_ns = 500 + ROOM_POLL_INTERVAL_SECS.saturating_mul(1_000_000_000);
        block_on_with_spin(run_poll_inbox_job(due_now_ns)).expect("poll job should not fail");

        let room_poll = stable::room_poll_state();
        assert_eq!(room_poll.last_seen_seq, Some(15));
        assert_eq!(room_poll.last_attempted_at_ns, Some(due_now_ns));
        assert_eq!(room_poll.last_succeeded_at_ns, Some(due_now_ns));
        assert_eq!(room_poll.last_known_latest_seq, Some(15));
        assert_eq!(room_poll.last_batch_count, 0);
        assert_eq!(room_poll.consecutive_failures, 0);
        assert!(room_poll.last_error.is_none());

        crate::features::factory_room::clear_mock_factory_room_call();
    }

    #[test]
    fn poll_inbox_job_skips_room_poll_until_interval_elapses() {
        crate::features::factory_room::clear_mock_factory_room_call();
        stable::init_storage();
        configure_factory_room_access();
        stable::record_room_poll_success(1_000, Some(9), Some(12), 1);

        let call_count = Rc::new(Cell::new(0u32));
        let call_count_for_mock = Rc::clone(&call_count);
        crate::features::factory_room::set_mock_factory_room_call(
            move |_canister_id, _method, _encoded_args| {
                call_count_for_mock.set(call_count_for_mock.get().saturating_add(1));
                Err("room poll should not have been attempted".to_string())
            },
        );

        let not_due_now_ns = 1_000 + ROOM_POLL_INTERVAL_SECS.saturating_mul(1_000_000_000) - 1;
        block_on_with_spin(run_poll_inbox_job(not_due_now_ns)).expect("poll job should not fail");

        let room_poll = stable::room_poll_state();
        assert_eq!(call_count.get(), 0);
        assert_eq!(room_poll.last_seen_seq, Some(9));
        assert_eq!(room_poll.last_attempted_at_ns, Some(1_000));
        assert_eq!(room_poll.last_succeeded_at_ns, Some(1_000));
        assert_eq!(room_poll.last_known_latest_seq, Some(12));
        assert_eq!(room_poll.last_batch_count, 1);
        assert_eq!(room_poll.consecutive_failures, 0);
        assert!(room_poll.last_error.is_none());

        crate::features::factory_room::clear_mock_factory_room_call();
    }

    #[test]
    fn poll_inbox_job_room_read_failures_are_non_fatal() {
        crate::features::factory_room::clear_mock_factory_room_call();
        stable::init_storage();
        configure_factory_room_access();
        stable::record_room_poll_success(500, Some(9), Some(12), 1);

        crate::features::factory_room::set_mock_factory_room_call(
            move |canister_id, method, encoded_args| {
                assert_eq!(canister_id, test_factory_principal());
                assert_eq!(method, "list_my_room_messages");
                let (after_seq, limit): (Option<u64>, Option<u64>) =
                    candid::decode_args(encoded_args).expect("query args should decode");
                assert_eq!(after_seq, Some(9));
                assert_eq!(limit, Some(ROOM_POLL_PAGE_LIMIT));
                Err(
                    "factory room call rejected: code=5 msg=temporary room read failure"
                        .to_string(),
                )
            },
        );

        let due_now_ns = 500 + ROOM_POLL_INTERVAL_SECS.saturating_mul(1_000_000_000);
        block_on_with_spin(run_poll_inbox_job(due_now_ns)).expect("poll job should not fail");

        let room_poll = stable::room_poll_state();
        assert_eq!(room_poll.last_seen_seq, Some(9));
        assert_eq!(room_poll.last_attempted_at_ns, Some(due_now_ns));
        assert_eq!(room_poll.last_succeeded_at_ns, Some(500));
        assert_eq!(room_poll.last_known_latest_seq, Some(12));
        assert_eq!(room_poll.last_batch_count, 0);
        assert_eq!(room_poll.consecutive_failures, 1);
        assert_eq!(
            room_poll.last_error.as_deref(),
            Some("factory room call rejected: code=5 msg=temporary room read failure")
        );

        crate::features::factory_room::clear_mock_factory_room_call();
    }
}
