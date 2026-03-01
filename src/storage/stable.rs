/// Stable-memory persistence layer for the ic-automaton canister.
///
/// This is the SQLite-primary rewrite of the storage layer.  All
/// `StableBTreeMap` / `MemoryManager` infrastructure has been removed; every
/// function now delegates to `super::sqlite` (the hot-state + SQLite adapter).
///
/// Public function signatures are intentionally identical to the original so
/// that callers require no changes.
use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, AgentEvent, AgentState, ConversationEntry, ConversationLog,
    ConversationSummary, CycleTelemetry, EvmPollCursor, EvmRouteStateView, InboxMessage,
    InboxMessageStatus, InboxStats, InferenceConfigView, InferenceProvider, JobStatus, MemoryFact,
    MemoryRollup, ObservabilitySnapshot, OutboxMessage, OutboxStats, PromptLayer, PromptLayerView,
    RetentionConfig, RetentionMaintenanceRuntime, RuntimeSnapshot, RuntimeView, ScheduledJob,
    SchedulerLease, SchedulerRuntime, SessionSummary, SkillRecord, StorageGrowthMetrics,
    StoragePressureLevel, StrategyKillSwitchState, StrategyOutcomeEvent, StrategyOutcomeKind,
    StrategyOutcomeStats, StrategyTemplate, StrategyTemplateKey, SurvivalOperationClass,
    SurvivalTier, TaskKind, TaskLane, TaskScheduleConfig, TaskScheduleRuntime,
    TemplateActivationState, TemplateRevocationState, TemplateVersion, ToolCallRecord,
    TransitionLogRecord, TurnRecord, TurnWindowSummary, WalletBalanceSnapshot,
    WalletBalanceSyncConfig, WalletBalanceSyncConfigView, WalletBalanceTelemetryView,
};
pub use crate::domain::types::{
    AutonomyToolFailureCooldown, MemoryFactSort, MemoryFactStats, RetentionPruneStats,
    RuntimeOutcallKind,
};
use crate::features::cycle_topup::TopUpStage;
use crate::prompt;
use candid::Principal;
use canlog::{log, GetLogFilter, LogFilter, LogPriorityLevels};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use super::sqlite;
use super::sqlite::SurvivalOperationRuntimeRecord;

fn now_ns() -> u64 {
    crate::timing::current_time_ns()
}

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Monotonically increasing inbox sequence counter stored in `runtime_scalars`.
const INBOX_SEQ_KEY: &str = "inbox.seq";
/// Monotonically increasing outbox sequence counter stored in `runtime_scalars`.
const OUTBOX_SEQ_KEY: &str = "outbox.seq";
/// Bool flag: `true` once the HTTP allow-list has been explicitly configured.
const HTTP_ALLOWLIST_INITIALIZED_KEY: &str = "http.allowlist.initialized";
/// Serialised `Vec<CycleBalanceSample>` ring buffer stored in `runtime_scalars`.
const CYCLE_BALANCE_SAMPLES_KEY: &str = "cycles.balance.samples";
/// Serialised `Vec<StorageGrowthSample>` ring buffer stored in `runtime_scalars`.
const STORAGE_GROWTH_SAMPLES_KEY: &str = "storage.growth.samples";
const RETENTION_CONFIG_KEY: &str = "retention.config";
const RETENTION_RUNTIME_KEY: &str = "retention.runtime";
const TOPUP_STATE_KEY: &str = "cycle_topup.state";
/// Persisted cadence multiplier for overriding `TICKS_PER_TURN_INTERVAL` at runtime.
#[allow(dead_code)]
const CADENCE_MULTIPLIER_KEY: &str = "timing.cadence_multiplier";
/// Persisted scheduler base tick interval in seconds.
const SCHEDULER_BASE_TICK_SECS_KEY: &str = "timing.scheduler_base_tick_secs";
const WELCOME_MESSAGE_KEY: &str = "ui.welcome_message";
/// Maximum character count accepted by `set_welcome_message`.
pub const MAX_WELCOME_MESSAGE_CHARS: usize = 2_000;

// ── Capacity constants ───────────────────────────────────────────────────────

/// Maximum number of jobs returned by `list_recent_jobs`.
const MAX_RECENT_JOBS: usize = 200;
/// Default number of items returned by the observability snapshot query.
const DEFAULT_OBSERVABILITY_LIMIT: usize = 25;
/// Hard cap on items returned by the observability snapshot query.
const MAX_OBSERVABILITY_LIMIT: usize = 100;
use crate::timing;
/// Nanosecond window over which the cycles burn-rate moving average is computed.
const CYCLES_BURN_MOVING_WINDOW_NS: u64 = timing::CYCLES_BURN_MOVING_WINDOW_NS;
/// Maximum number of cycle-balance samples retained in the ring buffer.
const CYCLES_BURN_MAX_SAMPLES: usize = 450;
/// Trend window (seconds) for the storage-growth rate calculation.
const STORAGE_GROWTH_TREND_WINDOW_SECONDS: u64 = 6 * 60 * 60;
const STORAGE_GROWTH_TREND_WINDOW_NS: u64 = STORAGE_GROWTH_TREND_WINDOW_SECONDS * 1_000_000_000;
/// Maximum number of storage-growth samples retained in the ring buffer.
const STORAGE_GROWTH_MAX_SAMPLES: usize = 360;
/// Utilisation % threshold for `StoragePressureLevel::Elevated`.
const STORAGE_PRESSURE_ELEVATED_PERCENT: u8 = 70;
/// Utilisation % threshold for `StoragePressureLevel::High`.
const STORAGE_PRESSURE_HIGH_PERCENT: u8 = 85;
/// Utilisation % threshold for `StoragePressureLevel::Critical`.
const STORAGE_PRESSURE_CRITICAL_PERCENT: u8 = 95;
/// Storage growth rate (entries/hour) above which a pressure warning is emitted.
const STORAGE_GROWTH_WARNING_ENTRIES_PER_HOUR: i64 = 5_000;
/// USD value of 1 trillion cycles used for burn-rate cost projections.
const CYCLES_USD_PER_TRILLION_ESTIMATE: f64 = 1.35;
/// Maximum conversation entries kept per sender in `conversations`.
const MAX_CONVERSATION_ENTRIES_PER_SENDER: usize = 20;
/// Maximum number of distinct senders tracked in `conversations`.
const MAX_CONVERSATION_SENDERS: usize = 200;
/// Character limit applied to inbox message bodies in conversation logs.
const MAX_CONVERSATION_BODY_CHARS: usize = 500;
/// Character limit applied to agent replies in conversation logs.
const MAX_CONVERSATION_REPLY_CHARS: usize = 500;
/// Maximum value accepted for `evm_cursor.confirmation_depth`.
const MAX_EVM_CONFIRMATION_DEPTH: u64 = 100;
/// Hard cap on the number of memory facts stored in `memory_facts`.
pub const MAX_MEMORY_FACTS: usize = 500;
/// Maximum inbox message body size (in characters) accepted by `post_inbox_message`.
pub const MAX_INBOX_BODY_CHARS: usize = 4_096;
/// Character limit for the `inner_dialogue` field of a stored `TurnRecord`.
const MAX_TURN_INNER_DIALOGUE_CHARS: usize = 12_000;
/// Character limit for tool call `args_json` stored in `tool_calls`.
const MAX_TOOL_ARGS_JSON_CHARS: usize = 4_000;
/// Character limit for tool call `output` stored in `tool_calls`.
const MAX_TOOL_OUTPUT_CHARS: usize = 8_000;
/// Character limit for telemetry error text retained in runtime timing stats.
const MAX_TIMING_ERROR_CHARS: usize = 512;
const MIN_RETENTION_BATCH_SIZE: u32 = 1;
const MAX_RETENTION_BATCH_SIZE: u32 = 1_000;
const MIN_RETENTION_INTERVAL_SECS: u64 = 1;
const MIN_MEMORY_FACT_PRUNE_BATCH_SIZE: u32 = 1;
const MIN_SCHEDULER_BASE_TICK_SECS: u64 = 1;
const MAX_SCHEDULER_BASE_TICK_SECS: u64 = 3_600;
/// 24-hour summary window used for session and turn-window summaries.
const SUMMARY_WINDOW_NS: u64 = 24 * 60 * 60 * 1_000_000_000;
/// Age threshold after which a memory fact is eligible for rollup compression.
const MEMORY_ROLLUP_STALE_NS: u64 = 24 * 60 * 60 * 1_000_000_000;
/// Maximum number of session summaries in `session_summaries`.
const MAX_SESSION_SUMMARIES: usize = 2_000;
/// Maximum number of turn-window summaries in `turn_window_summaries`.
const MAX_TURN_WINDOW_SUMMARIES: usize = 1_000;
/// Maximum number of memory rollups in `memory_rollups`.
const MAX_MEMORY_ROLLUPS: usize = 128;
/// Maximum distinct error strings stored per turn-window summary.
const MAX_TURN_SUMMARY_ERRORS: usize = 5;
/// Maximum source keys stored per memory rollup.
const MAX_MEMORY_ROLLUP_SOURCE_KEYS: usize = 10;
/// Maximum facts sampled per namespace when building a rollup.
const MAX_MEMORY_ROLLUP_FACTS_PER_NAMESPACE: usize = 5;
#[cfg(test)]
const MAX_FIELD_TRUNCATION_MARKER_RESERVE_CHARS: usize = 120;
const AUTONOMY_TOOL_SUCCESS_KEY_PREFIX: &str = "autonomy.tool_success.";
const AUTONOMY_TOOL_FAILURE_KEY_PREFIX: &str = "autonomy.tool_failure.";
const EVM_INGEST_DEDUPE_KEY_PREFIX: &str = "evm.ingest";
#[cfg(not(target_arch = "wasm32"))]
const HOST_TOTAL_CYCLES_OVERRIDE_KEY: &str = "host.total_cycles";
#[cfg(not(target_arch = "wasm32"))]
const HOST_LIQUID_CYCLES_OVERRIDE_KEY: &str = "host.liquid_cycles";

// ── Survival constants ───────────────────────────────────────────────────────

/// Number of consecutive `Normal` cycle observations required before the
/// scheduler downgrades from an elevated `SurvivalTier`.
pub const SURVIVAL_TIER_RECOVERY_CHECKS_REQUIRED: u32 = 3;
/// Maximum exponential backoff (seconds) for the `Inference` operation class.
pub const SURVIVAL_OPERATION_MAX_BACKOFF_SECS_INFERENCE: u64 = 120;
/// Maximum exponential backoff (seconds) for the `EvmPoll` operation class.
pub const SURVIVAL_OPERATION_MAX_BACKOFF_SECS_EVM_POLL: u64 = 120;
/// Maximum exponential backoff (seconds) for the `EvmBroadcast` operation class.
pub const SURVIVAL_OPERATION_MAX_BACKOFF_SECS_EVM_BROADCAST: u64 = 300;
/// Maximum exponential backoff (seconds) for the `ThresholdSign` operation class.
pub const SURVIVAL_OPERATION_MAX_BACKOFF_SECS_THRESHOLD_SIGN: u64 = 120;
/// Maximum exponential backoff (seconds) for the `InterCanisterCall` operation class.
pub const SURVIVAL_OPERATION_MAX_BACKOFF_SECS_INTER_CANISTER_CALL: u64 = 120;
/// Hard upper limit on the EVM RPC response buffer (2 MiB).
const MAX_EVM_RPC_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
#[allow(dead_code)]
const MIN_WALLET_BALANCE_SYNC_INTERVAL_SECS: u64 = 60;
#[allow(dead_code)]
const MAX_WALLET_BALANCE_SYNC_INTERVAL_SECS: u64 = 24 * 60 * 60;
#[allow(dead_code)]
const MIN_WALLET_BALANCE_FRESHNESS_WINDOW_SECS: u64 = 60;
#[allow(dead_code)]
const MAX_WALLET_BALANCE_FRESHNESS_WINDOW_SECS: u64 = 24 * 60 * 60;
#[allow(dead_code)]
const MIN_WALLET_BALANCE_SYNC_RESPONSE_BYTES: u64 = 256;
#[allow(dead_code)]
const MAX_WALLET_BALANCE_SYNC_RESPONSE_BYTES: u64 = 4 * 1024;

#[derive(Clone, Copy, Debug, LogPriorityLevels)]
enum SchedulerStorageLogPriority {
    #[log_level(capacity = 2000, name = "SCHED_STORAGE_INFO")]
    Info,
    #[log_level(capacity = 500, name = "SCHED_STORAGE_WARN")]
    Warn,
    #[log_level(capacity = 100, name = "SCHED_STORAGE_ERROR")]
    Error,
}

impl GetLogFilter for SchedulerStorageLogPriority {
    fn get_log_filter() -> LogFilter {
        LogFilter::ShowAll
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CycleBalanceSample {
    captured_at_ns: u64,
    total_cycles: u128,
    liquid_cycles: u128,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StorageGrowthSample {
    captured_at_ns: u64,
    tracked_entries: u64,
}

// ── Survival / backoff helpers ────────────────────────────────────────────────

fn get_survival_operation_runtime(operation: &SurvivalOperationClass) -> SurvivalOperationRuntimeRecord {
    sqlite::read_survival_operation_runtime(operation)
        .unwrap_or_default()
}

fn set_survival_operation_runtime(
    operation: &SurvivalOperationClass,
    runtime: &SurvivalOperationRuntimeRecord,
) {
    let _ = sqlite::write_survival_operation_runtime(operation, runtime);
}

fn survival_operation_backoff_secs(failures: u32, max_backoff_secs: u64) -> u64 {
    let capped = max_backoff_secs.max(1);
    let exponent = failures.min(20);
    let delay = 1u64.checked_shl(exponent).unwrap_or(u64::MAX);
    delay.min(capped)
}

fn survival_operation_allows_in_tier(
    tier: &SurvivalTier,
    operation: &SurvivalOperationClass,
) -> bool {
    !matches!(
        (tier, operation),
        (SurvivalTier::Critical, _)
            | (SurvivalTier::OutOfCycles, _)
            | (
                SurvivalTier::LowCycles,
                SurvivalOperationClass::ThresholdSign
            )
            | (
                SurvivalTier::LowCycles,
                SurvivalOperationClass::EvmBroadcast
            )
    )
}

// ── Survival / backoff ───────────────────────────────────────────────────────

/// Returns `true` if `operation` is permitted given the current survival tier
/// and its per-operation backoff timer.
pub fn can_run_survival_operation(operation: &SurvivalOperationClass, now_ns: u64) -> bool {
    if !survival_operation_allows_in_tier(&scheduler_survival_tier(), operation) {
        return false;
    }
    get_survival_operation_runtime(operation)
        .backoff_until_ns
        .is_none_or(|until| until <= now_ns)
}

/// Records a failure for `operation`, increments the consecutive-failure count,
/// and sets an exponential backoff deadline.
pub fn record_survival_operation_failure(
    operation: &SurvivalOperationClass,
    now_ns: u64,
    max_backoff_secs: u64,
) {
    let mut runtime = get_survival_operation_runtime(operation);
    runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
    let backoff_ns =
        survival_operation_backoff_secs(runtime.consecutive_failures, max_backoff_secs)
            .saturating_mul(1_000_000_000);
    runtime.backoff_until_ns = now_ns.checked_add(backoff_ns);
    if runtime.backoff_until_ns.is_none() {
        runtime.backoff_until_ns = Some(u64::MAX);
    }
    set_survival_operation_runtime(operation, &runtime);
    log!(
        SchedulerStorageLogPriority::Warn,
        "survival_operation_failure operation={:?} consecutive_failures={} backoff_until_ns={}",
        operation,
        runtime.consecutive_failures,
        runtime.backoff_until_ns.unwrap_or_default()
    );
}

/// Clears the consecutive-failure count and backoff for `operation` after a
/// successful execution.  A no-op if the operation was already clean.
pub fn record_survival_operation_success(operation: &SurvivalOperationClass) {
    let runtime = get_survival_operation_runtime(operation);
    if runtime.consecutive_failures == 0 && runtime.backoff_until_ns.is_none() {
        return;
    }
    log!(
        SchedulerStorageLogPriority::Info,
        "survival_operation_success operation={:?}",
        operation
    );
    set_survival_operation_runtime(operation, &SurvivalOperationRuntimeRecord::default());
}

#[allow(dead_code)]
pub fn survival_operation_backoff_until(operation: &SurvivalOperationClass) -> Option<u64> {
    get_survival_operation_runtime(operation).backoff_until_ns
}

#[allow(dead_code)]
pub fn survival_operation_consecutive_failures(operation: &SurvivalOperationClass) -> u32 {
    get_survival_operation_runtime(operation).consecutive_failures
}

// ── Initialization ───────────────────────────────────────────────────────────

/// One-time storage bootstrap called from `canister_init` and `canister_post_upgrade`.
///
/// Performs idempotent setup: migrates legacy cursor fields, seeds default
/// prompt layers, initialises sequence counters, and calls
/// `init_scheduler_defaults` / `init_retention_defaults`.
pub fn init_storage() {
    let _ = sqlite::init_storage();

    let mut snapshot = runtime_snapshot();
    let mut snapshot_changed = false;
    if snapshot.evm_cursor.contract_address.is_none() {
        if let Some(contract_address) = snapshot.inbox_contract_address.clone() {
            snapshot.evm_cursor.contract_address = Some(contract_address);
            snapshot_changed = true;
        }
    }
    if snapshot.evm_cursor.automaton_address_topic.is_none() {
        if let Some(address) = snapshot.evm_address.as_deref() {
            snapshot.evm_cursor.automaton_address_topic = Some(evm_address_to_topic(address));
            snapshot_changed = true;
        }
    }
    if !snapshot.wallet_balance_bootstrap_pending {
        snapshot.wallet_balance_bootstrap_pending = true;
        snapshot_changed = true;
    }
    if snapshot_changed {
        save_runtime_snapshot(&snapshot);
    }
    seed_default_prompt_layers();
    if runtime_u64(INBOX_SEQ_KEY).is_none() {
        save_runtime_u64(INBOX_SEQ_KEY, 0);
    }
    if runtime_u64(OUTBOX_SEQ_KEY).is_none() {
        save_runtime_u64(OUTBOX_SEQ_KEY, 0);
    }
    if runtime_bool(HTTP_ALLOWLIST_INITIALIZED_KEY).is_none() {
        save_runtime_bool(HTTP_ALLOWLIST_INITIALIZED_KEY, false);
    }
    init_scheduler_defaults(now_ns());
    init_retention_defaults(now_ns());
}

/// Idempotent initialisation of scheduler-runtime and per-task config/runtime
/// entries.  `PollInbox` is scheduled immediately (`next_due_ns = now_ns`);
/// every other task starts at its default interval.
pub fn init_scheduler_defaults(now_ns: u64) {
    if sqlite::read_scheduler_runtime().ok().flatten().is_none() {
        save_scheduler_runtime(&SchedulerRuntime::default());
    }

    // Seed base tick secs if not yet set.
    if sqlite::get_runtime_scalar(SCHEDULER_BASE_TICK_SECS_KEY)
        .ok()
        .flatten()
        .is_none()
    {
        let _ = sqlite::set_runtime_scalar(
            SCHEDULER_BASE_TICK_SECS_KEY,
            &timing::SCHEDULER_TICK_INTERVAL_SECS.to_string(),
        );
    }

    for kind in TaskKind::all() {
        if get_task_config(kind).is_none() {
            upsert_task_config(TaskScheduleConfig::default_for(kind));
        }
        if get_task_runtime_if_any(kind).is_none() {
            let next_due_ns = if *kind == TaskKind::PollInbox {
                now_ns
            } else {
                now_ns.saturating_add(kind.default_interval_secs().saturating_mul(1_000_000_000))
            };
            save_task_runtime(
                kind,
                &TaskScheduleRuntime {
                    kind: kind.clone(),
                    next_due_ns,
                    backoff_until_ns: None,
                    consecutive_failures: 0,
                    pending_job_id: None,
                    last_started_ns: None,
                    last_finished_ns: None,
                    last_error: None,
                },
            );
        }
    }
}

fn init_retention_defaults(now_ns: u64) {
    // Seed retention config if not yet stored.
    if sqlite::read_retention_runtime::<RetentionConfig>()
        .ok()
        .flatten()
        .is_none()
    {
        // Store config under a combined retention runtime record using the key prefix approach:
        // We use get/set_runtime_scalar for the retention config.
        let _ = sqlite::set_runtime_scalar(
            RETENTION_CONFIG_KEY,
            &serde_json::to_string(&RetentionConfig::default()).unwrap_or_default(),
        );
    }

    // Seed retention maintenance runtime if not yet stored.
    if sqlite::read_retention_runtime::<RetentionMaintenanceRuntime>()
        .ok()
        .flatten()
        .is_none()
    {
        let _ = sqlite::write_retention_runtime(&RetentionMaintenanceRuntime {
            next_run_after_ns: now_ns,
            ..RetentionMaintenanceRuntime::default()
        });
    }
}

// ── Retention ────────────────────────────────────────────────────────────────

/// Returns the current retention configuration from stable storage.
pub fn retention_config() -> RetentionConfig {
    // Retention config is stored as a runtime_scalar (JSON-encoded)
    sqlite::get_runtime_scalar(RETENTION_CONFIG_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Validates and persists a new retention configuration.
///
/// Returns the saved config on success, or an error string if any field is
/// out of the permitted range.
pub fn set_retention_config(config: RetentionConfig) -> Result<RetentionConfig, String> {
    if !(MIN_RETENTION_BATCH_SIZE..=MAX_RETENTION_BATCH_SIZE)
        .contains(&config.maintenance_batch_size)
    {
        return Err(format!(
            "maintenance_batch_size must be in range {}..={}",
            MIN_RETENTION_BATCH_SIZE, MAX_RETENTION_BATCH_SIZE
        ));
    }
    if config.maintenance_interval_secs < MIN_RETENTION_INTERVAL_SECS {
        return Err(format!(
            "maintenance_interval_secs must be at least {}",
            MIN_RETENTION_INTERVAL_SECS
        ));
    }
    if config.memory_facts_prune_batch_size < MIN_MEMORY_FACT_PRUNE_BATCH_SIZE {
        return Err(format!(
            "memory_facts_prune_batch_size must be at least {}",
            MIN_MEMORY_FACT_PRUNE_BATCH_SIZE
        ));
    }

    let json = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    let _ = sqlite::set_runtime_scalar(RETENTION_CONFIG_KEY, &json);
    Ok(config)
}

/// Returns the current retention maintenance runtime (scan cursors, last-run
/// timestamps, per-bucket deletion counts).
pub fn retention_maintenance_runtime() -> RetentionMaintenanceRuntime {
    sqlite::read_retention_runtime::<RetentionMaintenanceRuntime>()
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn save_retention_maintenance_runtime(runtime: &RetentionMaintenanceRuntime) {
    let _ = sqlite::write_retention_runtime(runtime);
}

/// Runs one incremental retention pass if `runtime.next_run_after_ns <= now_ns`.
///
/// Returns `None` if the maintenance window has not yet elapsed, or
/// `Some(stats)` with per-bucket deletion counts on success.
pub fn run_retention_maintenance_if_due(now_ns: u64) -> Option<RetentionPruneStats> {
    let runtime = retention_maintenance_runtime();
    if runtime.next_run_after_ns > now_ns {
        return None;
    }
    Some(run_retention_maintenance_once(now_ns))
}

/// Executes one full incremental retention pass regardless of the schedule.
///
/// Uses SQL DELETE queries to prune old records efficiently.
pub fn run_retention_maintenance_once(now_ns: u64) -> RetentionPruneStats {
    let config = retention_config();
    let mut runtime = retention_maintenance_runtime();
    runtime.last_started_ns = Some(now_ns);
    runtime.last_finished_ns = None;
    runtime.last_error = None;
    save_retention_maintenance_runtime(&runtime);

    let jobs_budget = usize::try_from(config.maintenance_batch_size).unwrap_or(usize::MAX);
    let jobs_cutoff_ns =
        now_ns.saturating_sub(config.jobs_max_age_secs.saturating_mul(1_000_000_000));
    let inbox_cutoff_ns =
        now_ns.saturating_sub(config.inbox_max_age_secs.saturating_mul(1_000_000_000));
    let outbox_cutoff_ns =
        now_ns.saturating_sub(config.outbox_max_age_secs.saturating_mul(1_000_000_000));
    let turns_cutoff_ns =
        now_ns.saturating_sub(config.turns_max_age_secs.saturating_mul(1_000_000_000));
    let transitions_cutoff_ns = now_ns.saturating_sub(
        config.transitions_max_age_secs.saturating_mul(1_000_000_000),
    );
    let protected_inbox_ids = protected_conversation_inbox_ids();

    // Delete old completed/failed jobs.
    let deleted_jobs = sqlite::delete_jobs_older_than(jobs_cutoff_ns, jobs_budget)
        .map(|ids| u32::try_from(ids.len()).unwrap_or(u32::MAX))
        .unwrap_or(0);

    // Dedupe entries: use sql_query to purge old dedupe keys from runtime_scalars.
    // In the SQLite primary model there is no separate dedupe table to clean up
    // beyond what delete_jobs_older_than already handles (jobs are the canonical
    // dedup source). For now report 0.
    let deleted_dedupe: u32 = 0;

    // Delete old consumed inbox messages.
    let deleted_inbox = sqlite::delete_inbox_older_than(inbox_cutoff_ns, jobs_budget, &protected_inbox_ids)
        .map(|ids| u32::try_from(ids.len()).unwrap_or(u32::MAX))
        .unwrap_or(0);

    // Delete old outbox messages.
    let deleted_outbox = sqlite::delete_outbox_older_than(outbox_cutoff_ns, jobs_budget, &protected_inbox_ids)
        .map(|ids| u32::try_from(ids.len()).unwrap_or(u32::MAX))
        .unwrap_or(0);

    // Delete old turns (and their tool calls).
    let deleted_turns_records = sqlite::delete_turns_older_than(turns_cutoff_ns, jobs_budget)
        .unwrap_or_default();
    let deleted_turns = u32::try_from(deleted_turns_records.len()).unwrap_or(u32::MAX);

    // Delete old transitions.
    let deleted_transitions_records =
        sqlite::delete_transitions_older_than(transitions_cutoff_ns, jobs_budget)
            .unwrap_or_default();
    let deleted_transitions = u32::try_from(deleted_transitions_records.len()).unwrap_or(u32::MAX);

    // Tool calls are deleted together with their turns; report them as turns.
    let deleted_tools: u32 = 0;

    // Generate session summaries for deleted inbox/outbox.
    let generated_session_summaries: u32 = 0;

    // Generate turn-window summaries from deleted turns/transitions.
    let generated_turn_window_summaries = generate_turn_window_summaries_from_deleted(
        &deleted_turns_records,
        now_ns,
    );

    // Update memory rollups and prune stale facts.
    let generated_memory_rollups = update_memory_rollups(now_ns);
    let deleted_memory_facts = prune_stale_non_critical_memory_facts(
        now_ns,
        config.memory_facts_max_age_secs,
        usize::try_from(config.memory_facts_prune_batch_size).unwrap_or(usize::MAX),
    );

    runtime.job_scan_cursor = None;
    runtime.dedupe_scan_cursor = None;
    runtime.inbox_scan_cursor = None;
    runtime.outbox_scan_cursor = None;
    runtime.turn_scan_cursor = None;
    runtime.transition_scan_cursor = None;
    runtime.last_deleted_jobs = deleted_jobs;
    runtime.last_deleted_dedupe = deleted_dedupe;
    runtime.last_deleted_inbox = deleted_inbox;
    runtime.last_deleted_outbox = deleted_outbox;
    runtime.last_deleted_turns = deleted_turns;
    runtime.last_deleted_transitions = deleted_transitions;
    runtime.last_deleted_tools = deleted_tools;
    runtime.last_generated_session_summaries = generated_session_summaries;
    runtime.last_generated_turn_window_summaries = generated_turn_window_summaries;
    runtime.last_generated_memory_rollups = generated_memory_rollups;
    runtime.last_deleted_memory_facts = deleted_memory_facts;
    runtime.last_finished_ns = Some(now_ns);
    runtime.next_run_after_ns = now_ns.saturating_add(
        config
            .maintenance_interval_secs
            .saturating_mul(1_000_000_000),
    );
    runtime.retention_progress_percent = 100;
    runtime.summarization_progress_percent = 100;
    save_retention_maintenance_runtime(&runtime);

    RetentionPruneStats {
        deleted_jobs,
        deleted_dedupe,
        deleted_inbox,
        deleted_outbox,
        deleted_turns,
        deleted_transitions,
        deleted_tools,
        generated_session_summaries,
        generated_turn_window_summaries,
        generated_memory_rollups,
        deleted_memory_facts,
    }
}

/// Returns `Vec<String>` of inbox message IDs that appear in recent
/// conversation logs and should not be pruned.
fn protected_conversation_inbox_ids() -> Vec<String> {
    // In the SQLite model conversations are not keyed by inbox_id so we return empty.
    Vec::new()
}

/// Generates turn-window summaries for deleted turn records and returns count.
fn generate_turn_window_summaries_from_deleted(
    deleted_turns: &[(String, crate::domain::types::TurnRecord)],
    now_ns: u64,
) -> u32 {
    use std::collections::BTreeMap;
    if deleted_turns.is_empty() {
        return 0;
    }

    // Group turns by 24-hour window.
    let mut by_window: BTreeMap<u64, Vec<&crate::domain::types::TurnRecord>> = BTreeMap::new();
    for (_, turn) in deleted_turns {
        let window = summary_window_start_ns(turn.created_at_ns);
        by_window.entry(window).or_default().push(turn);
    }

    let mut generated = 0u32;
    for (window_start_ns, turns) in &by_window {
        let date_key = turn_window_summary_key(*window_start_ns);
        // Read existing summary if any.
        let mut existing: TurnWindowSummary = sqlite::get_turn_window_summary::<TurnWindowSummary>(&date_key)
            .ok()
            .flatten()
            .unwrap_or(TurnWindowSummary {
                window_start_ns: *window_start_ns,
                window_end_ns: window_start_ns.saturating_add(SUMMARY_WINDOW_NS),
                source_count: 0,
                turn_count: 0,
                transition_count: 0,
                tool_call_count: 0,
                succeeded_turn_count: 0,
                failed_turn_count: 0,
                tool_success_count: 0,
                tool_failure_count: 0,
                top_errors: Vec::new(),
                generated_at_ns: now_ns,
            });

        for turn in turns {
            existing.source_count = existing.source_count.saturating_add(1);
            existing.turn_count = existing.turn_count.saturating_add(1);
            existing.tool_call_count = existing
                .tool_call_count
                .saturating_add(u32::try_from(turn.tool_call_count).unwrap_or(u32::MAX));
            if turn.error.is_none() {
                existing.succeeded_turn_count =
                    existing.succeeded_turn_count.saturating_add(1);
            } else {
                existing.failed_turn_count = existing.failed_turn_count.saturating_add(1);
                if let Some(err) = &turn.error {
                    accumulate_error(&mut existing.top_errors, Some(err.as_str()));
                }
            }
        }
        existing.generated_at_ns = now_ns;
        let _ = sqlite::upsert_turn_window_summary(&date_key, &existing);
        generated = generated.saturating_add(1);
    }
    generated
}

fn accumulate_error(errors: &mut Vec<String>, error: Option<&str>) {
    let Some(error) = error else {
        return;
    };
    let normalized = error.trim();
    if normalized.is_empty() {
        return;
    }
    if errors.iter().any(|existing| existing == normalized) {
        return;
    }
    if errors.len() >= MAX_TURN_SUMMARY_ERRORS {
        return;
    }
    errors.push(normalized.to_string());
}

fn seed_default_prompt_layers() {
    for layer_id in prompt::MUTABLE_LAYER_MIN_ID..=prompt::MUTABLE_LAYER_MAX_ID {
        if get_prompt_layer(layer_id).is_some() {
            continue;
        }
        if let Some(content) = prompt::default_layer_content(layer_id) {
            let _ = save_prompt_layer(&PromptLayer {
                layer_id,
                content: content.to_string(),
                updated_at_ns: now_ns(),
                updated_by_turn: "init".to_string(),
                version: 1,
            });
        }
    }
}

// ── Prompt layers ─────────────────────────────────────────────────────────────

/// Retrieves the mutable prompt layer with `layer_id`, or `None` if not yet set.
pub fn get_prompt_layer(layer_id: u8) -> Option<PromptLayer> {
    sqlite::get_prompt_layer(layer_id).ok().flatten()
}

/// Persists a mutable prompt layer.  Returns an error if `layer_id` falls
/// outside the mutable range `[MUTABLE_LAYER_MIN_ID, MUTABLE_LAYER_MAX_ID]`.
pub fn save_prompt_layer(layer: &PromptLayer) -> Result<(), String> {
    if !(prompt::MUTABLE_LAYER_MIN_ID..=prompt::MUTABLE_LAYER_MAX_ID).contains(&layer.layer_id) {
        return Err(format!(
            "mutable prompt layer id must be in range {}..={}",
            prompt::MUTABLE_LAYER_MIN_ID,
            prompt::MUTABLE_LAYER_MAX_ID
        ));
    }
    sqlite::save_prompt_layer(layer)
}

/// Returns a merged view of all immutable and mutable prompt layers, ordered
/// by `layer_id` ascending.
pub fn list_prompt_layers() -> Vec<PromptLayerView> {
    let mut layers = Vec::with_capacity(
        usize::from(prompt::IMMUTABLE_LAYER_MAX_ID - prompt::IMMUTABLE_LAYER_MIN_ID + 1)
            + usize::from(prompt::MUTABLE_LAYER_MAX_ID - prompt::MUTABLE_LAYER_MIN_ID + 1),
    );

    for layer_id in prompt::IMMUTABLE_LAYER_MIN_ID..=prompt::IMMUTABLE_LAYER_MAX_ID {
        if let Some(content) = prompt::immutable_layer_content(layer_id) {
            layers.push(PromptLayerView {
                layer_id,
                is_mutable: false,
                content: content.to_string(),
                updated_at_ns: None,
                updated_by_turn: None,
                version: None,
            });
        }
    }

    for layer_id in prompt::MUTABLE_LAYER_MIN_ID..=prompt::MUTABLE_LAYER_MAX_ID {
        if let Some(layer) = get_prompt_layer(layer_id) {
            layers.push(PromptLayerView {
                layer_id,
                is_mutable: true,
                content: layer.content,
                updated_at_ns: Some(layer.updated_at_ns),
                updated_by_turn: Some(layer.updated_by_turn),
                version: Some(layer.version),
            });
            continue;
        }

        layers.push(PromptLayerView {
            layer_id,
            is_mutable: true,
            content: prompt::default_layer_content(layer_id)
                .unwrap_or_default()
                .to_string(),
            updated_at_ns: None,
            updated_by_turn: None,
            version: None,
        });
    }

    layers
}

/// Returns the custom TUI welcome message, or `None` if the default is in use.
pub fn get_welcome_message() -> Option<String> {
    sqlite::get_runtime_scalar(WELCOME_MESSAGE_KEY)
        .ok()
        .and_then(|value| value)
}

pub fn set_welcome_message(message: String) -> Result<String, String> {
    let normalized = message.trim().to_string();
    if normalized.chars().count() > MAX_WELCOME_MESSAGE_CHARS {
        return Err(format!(
            "welcome message must not exceed {MAX_WELCOME_MESSAGE_CHARS} characters"
        ));
    }
    if normalized.is_empty() {
        sqlite::delete_runtime_scalar(WELCOME_MESSAGE_KEY)?;
    } else {
        sqlite::set_runtime_scalar(WELCOME_MESSAGE_KEY, &normalized)?;
    }
    Ok(normalized)
}

// ── Conversation ─────────────────────────────────────────────────────────────

/// Appends a conversation entry for `sender`, trimming body/reply fields to
/// `MAX_CONVERSATION_BODY_CHARS` / `MAX_CONVERSATION_REPLY_CHARS`.
pub fn append_conversation_entry(sender: &str, mut entry: ConversationEntry) {
    let sender_key = normalize_conversation_sender(sender);
    if sender_key.is_empty() {
        return;
    }

    entry.sender_body = truncate_to_chars(&entry.sender_body, MAX_CONVERSATION_BODY_CHARS);
    entry.agent_reply = truncate_to_chars(&entry.agent_reply, MAX_CONVERSATION_REPLY_CHARS);

    let _ = sqlite::append_conversation(&sender_key, &entry);
}

/// Returns the full conversation log for `sender`, or `None` if no entries
/// exist yet.
pub fn get_conversation_log(sender: &str) -> Option<ConversationLog> {
    let sender_key = normalize_conversation_sender(sender);
    if sender_key.is_empty() {
        return None;
    }
    sqlite::get_conversation_log(&sender_key).ok().flatten()
}

/// Returns one-line summaries for every tracked sender, sorted by most recent
/// activity descending.
pub fn list_conversation_summaries() -> Vec<ConversationSummary> {
    let raw = sqlite::list_conversation_summaries().unwrap_or_default();
    let mut summaries: Vec<ConversationSummary> = raw
        .into_iter()
        .map(|(sender, last_activity_ns, entry_count)| ConversationSummary {
            sender,
            last_activity_ns,
            entry_count,
        })
        .collect();
    summaries.sort_by(|left, right| {
        right
            .last_activity_ns
            .cmp(&left.last_activity_ns)
            .then_with(|| left.sender.cmp(&right.sender))
    });
    summaries
}

fn summary_window_start_ns(timestamp_ns: u64) -> u64 {
    timestamp_ns.saturating_sub(timestamp_ns % SUMMARY_WINDOW_NS)
}

fn turn_window_summary_key(window_start_ns: u64) -> String {
    format!("turn-window:{window_start_ns:020}")
}

// ── Summaries ─────────────────────────────────────────────────────────────────

/// Returns the most-recent `limit` daily session summaries (newest first).
/// Caps `limit` at `MAX_OBSERVABILITY_LIMIT`.
pub fn list_session_summaries(limit: usize) -> Vec<SessionSummary> {
    if limit == 0 {
        return Vec::new();
    }
    let keep = limit.min(MAX_OBSERVABILITY_LIMIT);
    sqlite::list_session_summaries::<SessionSummary>(keep)
        .unwrap_or_default()
}

/// Returns the most-recent `limit` daily turn-window summaries (newest first).
/// Caps `limit` at `MAX_OBSERVABILITY_LIMIT`.
pub fn list_turn_window_summaries(limit: usize) -> Vec<TurnWindowSummary> {
    if limit == 0 {
        return Vec::new();
    }
    let keep = limit.min(MAX_OBSERVABILITY_LIMIT);
    sqlite::list_turn_window_summaries::<TurnWindowSummary>(keep)
        .unwrap_or_default()
}

/// Returns up to `limit` memory rollups, sorted by generation time descending.
/// Caps `limit` at `MAX_MEMORY_ROLLUPS`.
pub fn list_memory_rollups(limit: usize) -> Vec<MemoryRollup> {
    if limit == 0 {
        return Vec::new();
    }
    let keep = limit.min(MAX_MEMORY_ROLLUPS);
    let mut rollups: Vec<MemoryRollup> = sqlite::list_memory_rollups::<MemoryRollup>(keep)
        .unwrap_or_default();
    rollups.sort_by(|left, right| {
        right
            .generated_at_ns
            .cmp(&left.generated_at_ns)
            .then_with(|| left.namespace.cmp(&right.namespace))
    });
    rollups.truncate(keep);
    rollups
}

fn is_critical_exact_memory_key(key: &str) -> bool {
    key == "balance.eth"
        || key == "balance.eth.last_checked_ns"
        || key.starts_with("balance.eth.")
        || key.starts_with("wallet.")
        || key.starts_with("config.")
}

/// Selects up to `raw_limit` facts for the agent's context prompt.
///
/// Critical keys (e.g. `balance.*`, `config.*`) are always included first;
/// non-critical facts fill the remaining slots.  If the raw limit is reached,
/// `rollup_limit` memory rollups are also returned for additional context.
pub fn list_memory_for_context(
    raw_limit: usize,
    rollup_limit: usize,
) -> (Vec<MemoryFact>, Vec<MemoryRollup>) {
    let all = list_all_memory_facts(MAX_MEMORY_FACTS);
    let mut critical = Vec::new();
    let mut non_critical = Vec::new();
    for fact in all {
        if is_critical_exact_memory_key(&fact.key) {
            critical.push(fact);
        } else {
            non_critical.push(fact);
        }
    }

    let mut selected_raw = critical;
    if selected_raw.len() < raw_limit {
        let remaining = raw_limit.saturating_sub(selected_raw.len());
        selected_raw.extend(non_critical.into_iter().take(remaining));
    }
    selected_raw.truncate(raw_limit);

    let include_rollups = selected_raw.len() >= raw_limit;
    let rollups = if include_rollups {
        list_memory_rollups(rollup_limit)
    } else {
        Vec::new()
    };

    (selected_raw, rollups)
}

// ── Runtime snapshot ─────────────────────────────────────────────────────────

/// Deserialises and returns the current [`RuntimeSnapshot`].
/// Falls back to `Default` if absent (first boot).
pub fn runtime_snapshot() -> RuntimeSnapshot {
    sqlite::read_runtime_snapshot()
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Reads the current cycle top-up pipeline stage.
pub fn read_topup_state() -> Option<TopUpStage> {
    sqlite::read_topup_state().ok().flatten()
}

/// Persists the cycle top-up pipeline stage.
pub fn write_topup_state(state: &TopUpStage) {
    let _ = sqlite::write_topup_state(state);
}

/// Removes the top-up state key, resetting the pipeline.
pub fn clear_topup_state() {
    let _ = sqlite::clear_topup_state();
}

/// Serialises `snapshot` and writes it to persistent storage.
pub fn save_runtime_snapshot(snapshot: &RuntimeSnapshot) {
    let _ = sqlite::write_runtime_snapshot(snapshot);
}

// ── Task / scheduler management ──────────────────────────────────────────────

/// Returns all stored per-task schedule configs, sorted by priority then name.
pub fn list_task_configs() -> Vec<(TaskKind, TaskScheduleConfig)> {
    sqlite::list_task_configs().unwrap_or_default()
}

/// Returns paired `(config, runtime)` tuples for every task kind, sorted by priority.
pub fn list_task_schedules() -> Vec<(TaskScheduleConfig, TaskScheduleRuntime)> {
    let mut schedules = list_task_configs()
        .into_iter()
        .map(|(kind, config)| (config, get_task_runtime(&kind)))
        .collect::<Vec<_>>();
    schedules.sort_by_key(|(config, _)| config.priority);
    schedules
}

/// Inserts or replaces the schedule config for `config.kind`.
pub fn upsert_task_config(config: TaskScheduleConfig) {
    let _ = sqlite::write_task_config(&config);
}

/// Returns the schedule config for `kind`, or `None` if not yet persisted.
pub fn get_task_config(kind: &TaskKind) -> Option<TaskScheduleConfig> {
    sqlite::read_task_config(kind).ok().flatten()
}

/// Updates the recurrence interval for `kind` and advances `next_due_ns`
/// by the new interval from `now`.  Returns an error if `interval_secs` is 0.
pub fn set_task_interval_secs(kind: &TaskKind, interval_secs: u64) -> Result<(), String> {
    if interval_secs == 0 {
        return Err("interval_secs must be greater than 0".to_string());
    }
    let mut config = get_task_config(kind).unwrap_or_else(|| TaskScheduleConfig::default_for(kind));
    config.interval_secs = interval_secs;
    upsert_task_config(config);
    let mut runtime = get_task_runtime(kind);
    runtime.next_due_ns = now_ns().saturating_add(interval_secs.saturating_mul(1_000_000_000));
    save_task_runtime(kind, &runtime);
    Ok(())
}

// ── Scheduler base tick ─────────────────────────────────────────────────────

/// Returns the persisted scheduler base tick interval in seconds.
/// Falls back to the compile-time default when unset.
pub fn get_scheduler_base_tick_secs() -> u64 {
    sqlite::get_runtime_scalar(SCHEDULER_BASE_TICK_SECS_KEY)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(timing::SCHEDULER_TICK_INTERVAL_SECS)
}

/// Persists the scheduler base tick interval (seconds).
/// Returns an error when the value is outside the supported range.
pub fn set_scheduler_base_tick_secs(interval_secs: u64) -> Result<u64, String> {
    if !(MIN_SCHEDULER_BASE_TICK_SECS..=MAX_SCHEDULER_BASE_TICK_SECS).contains(&interval_secs) {
        return Err(format!(
            "scheduler_base_tick_secs must be in range {}..={}",
            MIN_SCHEDULER_BASE_TICK_SECS, MAX_SCHEDULER_BASE_TICK_SECS
        ));
    }
    let _ = sqlite::set_runtime_scalar(SCHEDULER_BASE_TICK_SECS_KEY, &interval_secs.to_string());
    Ok(interval_secs)
}

// ── Cadence multiplier ───────────────────────────────────────────────────────

/// Read the persisted cadence multiplier, falling back to the compile-time default.
#[allow(dead_code)]
pub fn get_cadence_multiplier() -> u64 {
    sqlite::get_runtime_scalar(CADENCE_MULTIPLIER_KEY)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(timing::TICKS_PER_TURN_INTERVAL)
}

/// Persist a new cadence multiplier and re-derive `interval_secs` for every task.
///
/// The multiplier is clamped to `[MIN_CADENCE_MULTIPLIER, MAX_CADENCE_MULTIPLIER]`.
/// Returns the effective (clamped) multiplier.
#[allow(dead_code)]
pub fn set_cadence_multiplier(multiplier: u64) -> u64 {
    let clamped = multiplier.clamp(
        timing::MIN_CADENCE_MULTIPLIER,
        timing::MAX_CADENCE_MULTIPLIER,
    );
    let _ = sqlite::set_runtime_scalar(CADENCE_MULTIPLIER_KEY, &clamped.to_string());
    let new_interval = timing::task_interval_for_multiplier(clamped);
    for kind in TaskKind::all() {
        let _ = set_task_interval_secs(kind, new_interval);
    }
    clamped
}

/// Enables or disables scheduling of `kind` without touching its interval.
pub fn set_task_enabled(kind: &TaskKind, enabled: bool) {
    let mut config = get_task_config(kind).unwrap_or_else(|| TaskScheduleConfig::default_for(kind));
    config.enabled = enabled;
    upsert_task_config(config);
}

/// Returns the schedule runtime for `kind`.  Falls back to a freshly-initialised
/// runtime if the entry is absent from stable storage.
pub fn get_task_runtime(kind: &TaskKind) -> TaskScheduleRuntime {
    sqlite::read_task_runtime(kind)
        .ok()
        .flatten()
        .unwrap_or_else(|| TaskScheduleRuntime {
            kind: kind.clone(),
            next_due_ns: now_ns()
                .saturating_add(kind.default_interval_secs().saturating_mul(1_000_000_000)),
            backoff_until_ns: None,
            consecutive_failures: 0,
            pending_job_id: None,
            last_started_ns: None,
            last_finished_ns: None,
            last_error: None,
        })
}

fn get_task_runtime_if_any(kind: &TaskKind) -> Option<TaskScheduleRuntime> {
    sqlite::read_task_runtime(kind).ok().flatten()
}

/// Persists the schedule runtime for `kind`.
pub fn save_task_runtime(kind: &TaskKind, runtime: &TaskScheduleRuntime) {
    let _ = sqlite::write_task_runtime(kind, runtime);
}

// ── Scheduler runtime management ─────────────────────────────────────────────

/// Returns the full scheduler runtime (exposed for observability queries).
pub fn scheduler_runtime_view() -> SchedulerRuntime {
    scheduler_runtime()
}

/// Enables or disables the scheduler globally.  When disabled the
/// `paused_reason` is set to `"disabled"`.  Returns a status string.
pub fn set_scheduler_enabled(enabled: bool) -> String {
    let mut runtime = scheduler_runtime();
    runtime.enabled = enabled;
    runtime.paused_reason = if enabled {
        None
    } else {
        Some("disabled".to_string())
    };
    save_scheduler_runtime(&runtime);
    format!("scheduler_enabled={enabled}")
}

/// Transitions the scheduler into or out of low-cycles mode.
pub fn set_scheduler_low_cycles_mode(enabled: bool) -> String {
    let previous = scheduler_low_cycles_mode();
    let mut runtime = scheduler_runtime();
    runtime.low_cycles_mode = enabled;
    runtime.survival_tier = if enabled {
        SurvivalTier::LowCycles
    } else {
        SurvivalTier::Normal
    };
    runtime.survival_tier_recovery_checks = 0;
    runtime.paused_reason = if enabled {
        Some("low_cycles".to_string())
    } else {
        None
    };
    if previous != enabled {
        log!(
            SchedulerStorageLogPriority::Info,
            "scheduler_low_cycles_mode transition new={enabled}"
        );
    }
    save_scheduler_runtime(&runtime);
    if enabled {
        "low_cycles_mode=on".to_string()
    } else {
        "low_cycles_mode=off".to_string()
    }
}

/// Updates the scheduler's survival tier with hysteresis recovery.
pub fn set_scheduler_survival_tier(observed_tier: SurvivalTier) {
    let mut runtime = scheduler_runtime();
    let previous_tier = runtime.survival_tier.clone();
    let previous_checks = runtime.survival_tier_recovery_checks;

    let (resolved_tier, resolved_checks) =
        next_survival_tier_with_recovery(previous_tier.clone(), previous_checks, observed_tier);
    let resolved_low_cycles = resolved_tier != SurvivalTier::Normal;
    let resolved_paused_reason = if resolved_low_cycles {
        Some("low_cycles".to_string())
    } else {
        None
    };
    if resolved_tier == previous_tier
        && resolved_checks == previous_checks
        && runtime.low_cycles_mode == resolved_low_cycles
        && runtime.paused_reason == resolved_paused_reason
    {
        return;
    }

    runtime.survival_tier = resolved_tier.clone();
    runtime.survival_tier_recovery_checks = resolved_checks;
    runtime.low_cycles_mode = resolved_low_cycles;
    runtime.paused_reason = resolved_paused_reason;

    log!(
        SchedulerStorageLogPriority::Info,
        "scheduler_survival_tier transition previous_tier={:?} next_tier={:?} recovery_checks={}",
        previous_tier,
        resolved_tier,
        resolved_checks
    );
    save_scheduler_runtime(&runtime);
}

pub fn scheduler_low_cycles_mode() -> bool {
    scheduler_runtime().low_cycles_mode
}

pub fn scheduler_survival_tier() -> SurvivalTier {
    scheduler_runtime().survival_tier
}

pub fn scheduler_survival_tier_recovery_checks() -> u32 {
    scheduler_runtime().survival_tier_recovery_checks
}

pub fn scheduler_enabled() -> bool {
    scheduler_runtime().enabled
}

pub fn mutating_lease_active(now_ns: u64) -> bool {
    scheduler_runtime()
        .active_mutating_lease
        .is_some_and(|lease| lease.expires_at_ns > now_ns)
}

fn next_survival_tier_with_recovery(
    current: SurvivalTier,
    current_checks: u32,
    observed: SurvivalTier,
) -> (SurvivalTier, u32) {
    match observed {
        SurvivalTier::Normal => {
            if current == SurvivalTier::Normal
                || current_checks.saturating_add(1) >= SURVIVAL_TIER_RECOVERY_CHECKS_REQUIRED
            {
                (SurvivalTier::Normal, 0)
            } else {
                (current, current_checks.saturating_add(1))
            }
        }
        _ => (observed, 0),
    }
}

/// Records the start of a scheduler tick: sets `last_tick_started_ns` and
/// clears any prior tick error.
pub fn record_scheduler_tick_start(now_ns: u64) {
    let mut runtime = scheduler_runtime();
    runtime.last_tick_started_ns = now_ns;
    runtime.last_tick_error = None;
    save_scheduler_runtime(&runtime);
}

/// Records the end of a scheduler tick: sets `last_tick_finished_ns` and
/// stores any error that occurred during the tick.
pub fn record_scheduler_tick_end(now_ns: u64, error: Option<String>) {
    let mut runtime = scheduler_runtime();
    runtime.last_tick_finished_ns = now_ns;
    runtime.last_tick_error = error;
    save_scheduler_runtime(&runtime);
}

// ── Private scheduler runtime helpers ────────────────────────────────────────

fn scheduler_runtime() -> SchedulerRuntime {
    sqlite::read_scheduler_runtime()
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn save_scheduler_runtime(runtime: &SchedulerRuntime) {
    let _ = sqlite::write_scheduler_runtime(runtime);
}

fn task_kind_key(kind: &TaskKind) -> String {
    format!("task:{kind:?}")
}

// ── Runtime snapshot mutators ─────────────────────────────────────────────────

/// Toggles the `loop_enabled` flag in the runtime snapshot.
pub fn set_loop_enabled(enabled: bool) {
    let mut snapshot = runtime_snapshot();
    snapshot.loop_enabled = enabled;
    save_runtime_snapshot(&snapshot);
}

#[allow(dead_code)]
pub fn set_turn_lock(in_flight: bool) {
    let mut snapshot = runtime_snapshot();
    snapshot.turn_in_flight = in_flight;
    save_runtime_snapshot(&snapshot);
}

#[allow(dead_code)]
pub fn update_state(state: AgentState) {
    let mut snapshot = runtime_snapshot();
    snapshot.state = state;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
}

/// Replaces the agent's `soul` field and updates `last_transition_at_ns`.
/// Returns the new soul string.
pub fn set_soul(soul: String) -> String {
    let mut snapshot = runtime_snapshot();
    snapshot.soul = soul;
    snapshot.last_transition_at_ns = now_ns();
    let out = snapshot.soul.clone();
    save_runtime_snapshot(&snapshot);
    out
}

pub fn get_soul() -> String {
    runtime_snapshot().soul
}

fn normalize_https_url(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.starts_with("https://") {
        return Ok(trimmed.to_string());
    }
    if lowered.starts_with("http://") && is_local_http_url(&lowered) {
        return Ok(trimmed.to_string());
    }
    Err(format!(
        "{field} must be an https:// URL or localhost http:// URL"
    ))
}

fn is_local_http_url(url: &str) -> bool {
    let without_scheme = match url.strip_prefix("http://") {
        Some(value) => value,
        None => return false,
    };
    let authority = without_scheme.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        return false;
    }

    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or_default()
    };

    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1")
}

fn normalize_evm_hex_address(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    let valid_len = trimmed.len() == 42;
    let valid_prefix = trimmed.starts_with("0x");
    let valid_hex = trimmed
        .as_bytes()
        .iter()
        .skip(2)
        .all(|byte| byte.is_ascii_hexdigit());
    if !(valid_len && valid_prefix && valid_hex) {
        return Err(format!("{field} must be a 0x-prefixed 20-byte hex string"));
    }
    Ok(trimmed)
}

fn evm_address_to_topic(address: &str) -> String {
    let suffix = address.strip_prefix("0x").unwrap_or(address);
    format!("0x{:0>64}", suffix)
}

pub fn set_ecdsa_key_name(key_name: String) -> Result<String, String> {
    let trimmed = key_name.trim();
    if trimmed.is_empty() {
        return Err("ecdsa key name cannot be empty".to_string());
    }

    let mut snapshot = runtime_snapshot();
    snapshot.ecdsa_key_name = trimmed.to_string();
    snapshot.last_transition_at_ns = now_ns();
    let out = snapshot.ecdsa_key_name.clone();
    save_runtime_snapshot(&snapshot);
    Ok(out)
}

#[allow(dead_code)]
pub fn get_ecdsa_key_name() -> String {
    runtime_snapshot().ecdsa_key_name
}

// ── EVM configuration ─────────────────────────────────────────────────────────

/// Sets the agent's EVM address (0x-prefixed 20-byte hex) in the snapshot.
/// Pass `None` to clear.
pub fn set_evm_address(address: Option<String>) -> Result<Option<String>, String> {
    let normalized = match address {
        Some(raw) => Some(normalize_evm_hex_address(&raw, "evm address")?),
        None => None,
    };

    let mut snapshot = runtime_snapshot();
    snapshot.evm_address = normalized.clone();
    snapshot.evm_cursor.automaton_address_topic = normalized.as_deref().map(evm_address_to_topic);
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(normalized)
}

pub fn get_evm_address() -> Option<String> {
    runtime_snapshot().evm_address
}

#[allow(dead_code)]
pub fn get_automaton_evm_address() -> Option<String> {
    get_evm_address()
}

pub fn get_evm_rpc_url() -> String {
    runtime_snapshot().evm_rpc_url
}

pub fn get_discovered_usdc_address() -> Option<String> {
    runtime_snapshot().wallet_balance.usdc_contract_address
}

/// Sets the inbox contract address and mirrors it to `evm_cursor.contract_address`.
pub fn set_inbox_contract_address(address: Option<String>) -> Result<Option<String>, String> {
    let normalized = match address {
        Some(raw) => Some(normalize_evm_hex_address(&raw, "inbox contract address")?),
        None => None,
    };

    let mut snapshot = runtime_snapshot();
    snapshot.inbox_contract_address = normalized.clone();
    snapshot.evm_cursor.contract_address = normalized.clone();
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(normalized)
}

/// Sets the EVM chain ID and resets the poll cursor to block 0.
/// Returns an error if `chain_id` is 0.
pub fn set_evm_chain_id(chain_id: u64) -> Result<u64, String> {
    if chain_id == 0 {
        return Err("evm chain id must be greater than 0".to_string());
    }

    let mut snapshot = runtime_snapshot();
    snapshot.evm_cursor.chain_id = chain_id;
    snapshot.evm_cursor.next_block = 0;
    snapshot.evm_cursor.next_log_index = 0;
    snapshot.evm_cursor.last_poll_at_ns = 0;
    snapshot.evm_cursor.consecutive_empty_polls = 0;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(chain_id)
}

/// Sets the EVM confirmation depth (capped at `MAX_EVM_CONFIRMATION_DEPTH = 100`).
pub fn set_evm_confirmation_depth(confirmation_depth: u64) -> Result<u64, String> {
    if confirmation_depth > MAX_EVM_CONFIRMATION_DEPTH {
        return Err(format!(
            "evm confirmation depth must be <= {MAX_EVM_CONFIRMATION_DEPTH}"
        ));
    }
    let mut snapshot = runtime_snapshot();
    snapshot.evm_cursor.confirmation_depth = confirmation_depth;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(confirmation_depth)
}

/// Sets the initial lookback window (in blocks) used when `evm_cursor.next_block == 0`.
pub fn set_evm_bootstrap_lookback_blocks(lookback_blocks: u64) -> Result<u64, String> {
    let mut snapshot = runtime_snapshot();
    snapshot.evm_bootstrap_lookback_blocks = lookback_blocks;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(lookback_blocks)
}

pub fn set_evm_rpc_url(url: String) -> Result<String, String> {
    let normalized = normalize_https_url(&url, "evm rpc url")?;
    let mut snapshot = runtime_snapshot();
    snapshot.evm_rpc_url = normalized.clone();
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(normalized)
}

pub fn set_evm_rpc_fallback_url(url: Option<String>) -> Result<Option<String>, String> {
    let normalized = match url {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(normalize_https_url(trimmed, "evm rpc fallback url")?)
            }
        }
        None => None,
    };

    let mut snapshot = runtime_snapshot();
    snapshot.evm_rpc_fallback_url = normalized.clone();
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(normalized)
}

pub fn set_evm_rpc_max_response_bytes(max_response_bytes: u64) -> Result<u64, String> {
    if max_response_bytes == 0 {
        return Err("evm rpc max_response_bytes must be greater than 0".to_string());
    }
    if max_response_bytes > MAX_EVM_RPC_RESPONSE_BYTES {
        return Err(format!(
            "evm rpc max_response_bytes must be <= {MAX_EVM_RPC_RESPONSE_BYTES}"
        ));
    }
    let mut snapshot = runtime_snapshot();
    snapshot.evm_rpc_max_response_bytes = max_response_bytes;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(max_response_bytes)
}

/// Sets the active inference provider (e.g. `OpenRouter`, `LLMCanister`).
pub fn set_inference_provider(provider: InferenceProvider) {
    let mut snapshot = runtime_snapshot();
    snapshot.inference_provider = provider;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
}

/// Sets the inference model string (e.g. `"anthropic/claude-opus-4-6"`).
/// Returns an error if the trimmed value is empty.
pub fn set_inference_model(model: String) -> Result<String, String> {
    if model.trim().is_empty() {
        return Err("inference model cannot be empty".to_string());
    }
    let mut snapshot = runtime_snapshot();
    snapshot.inference_model = model.trim().to_string();
    snapshot.last_transition_at_ns = now_ns();
    let out = snapshot.inference_model.clone();
    save_runtime_snapshot(&snapshot);
    Ok(out)
}

/// Sets the ICP canister ID of the on-chain LLM canister.
/// Returns an error if `canister_id` is not a valid `Principal` text.
pub fn set_llm_canister_id(canister_id: String) -> Result<String, String> {
    let trimmed = canister_id.trim();
    if trimmed.is_empty() {
        return Err("llm canister id cannot be empty".to_string());
    }
    let normalized = Principal::from_text(trimmed)
        .map_err(|error| format!("invalid llm canister id: {error}"))?
        .to_text();

    let mut snapshot = runtime_snapshot();
    snapshot.llm_canister_id = normalized.clone();
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(normalized)
}

#[allow(dead_code)]
pub fn get_llm_canister_id() -> String {
    runtime_snapshot().llm_canister_id
}

#[allow(dead_code)]
pub fn set_openrouter_base_url(base_url: String) -> Result<String, String> {
    if base_url.trim().is_empty() {
        return Err("openrouter base url cannot be empty".to_string());
    }
    let mut snapshot = runtime_snapshot();
    snapshot.openrouter_base_url = base_url.trim().trim_end_matches('/').to_string();
    snapshot.last_transition_at_ns = now_ns();
    let out = snapshot.openrouter_base_url.clone();
    save_runtime_snapshot(&snapshot);
    Ok(out)
}

/// Sets (or clears) the OpenRouter API key stored in the runtime snapshot.
/// An empty string is treated as `None`.
pub fn set_openrouter_api_key(api_key: Option<String>) {
    let mut snapshot = runtime_snapshot();
    snapshot.openrouter_api_key = api_key.and_then(|key| {
        let trimmed = key.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
}

// ── Private scalar helpers ────────────────────────────────────────────────────

fn runtime_u64(key: &str) -> Option<u64> {
    sqlite::get_runtime_scalar(key)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
}

fn save_runtime_u64(key: &str, value: u64) {
    let _ = sqlite::set_runtime_scalar(key, &value.to_string());
}

fn runtime_bool(key: &str) -> Option<bool> {
    sqlite::get_runtime_scalar(key)
        .ok()
        .flatten()
        .map(|s| s == "true" || s == "1")
}

fn save_runtime_bool(key: &str, value: bool) {
    let _ = sqlite::set_runtime_scalar(key, if value { "true" } else { "false" });
}

fn evm_ingest_dedupe_key(tx_hash: &str, log_index: u64) -> String {
    format!("{EVM_INGEST_DEDUPE_KEY_PREFIX}:{tx_hash}:{log_index}")
}

fn next_inbox_seq() -> u64 {
    let next = runtime_u64(INBOX_SEQ_KEY).unwrap_or(0).saturating_add(1);
    save_runtime_u64(INBOX_SEQ_KEY, next);
    next
}

fn next_outbox_seq() -> u64 {
    let next = runtime_u64(OUTBOX_SEQ_KEY).unwrap_or(0).saturating_add(1);
    save_runtime_u64(OUTBOX_SEQ_KEY, next);
    next
}

fn normalize_conversation_sender(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn truncate_to_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

fn update_memory_rollups(_now_ns: u64) -> u32 {
    // Full implementation belongs to Part 2 (memory facts section).
    0
}

fn prune_stale_non_critical_memory_facts(
    now_ns: u64,
    max_age_secs: u64,
    limit: usize,
) -> u32 {
    let cutoff_ns = now_ns.saturating_sub(max_age_secs.saturating_mul(1_000_000_000));
    sqlite::prune_memory_facts(None, Some(cutoff_ns), limit)
        .map(|keys| u32::try_from(keys.len()).unwrap_or(u32::MAX))
        .unwrap_or(0)
}
// ── Shared helper functions ──────────────────────────────────────────────────

fn truncate_text_field(value: &str, max_chars: usize) -> String {
    let total_chars = value.chars().count();
    if total_chars <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }

    let truncated = total_chars.saturating_sub(max_chars);
    let digest = Keccak256::digest(value.as_bytes());
    let digest_hex = hex::encode(digest);
    let marker = format!(
        "...[truncated {truncated} chars keccak:{}]",
        &digest_hex[..16]
    );
    let marker_len = marker.chars().count();
    if marker_len >= max_chars {
        return marker.chars().take(max_chars).collect();
    }

    let keep_chars = max_chars.saturating_sub(marker_len);
    let prefix_chars = keep_chars / 2;
    let suffix_chars = keep_chars.saturating_sub(prefix_chars);
    let prefix = value.chars().take(prefix_chars).collect::<String>();
    let suffix = value
        .chars()
        .rev()
        .take(suffix_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{prefix}{marker}{suffix}")
}

// ── Transitions and turns ─────────────────────────────────────────────────────

/// Appends a `TransitionLogRecord` and increments snapshot transition/event sequences.
pub fn record_transition(
    turn_id: &str,
    from: &AgentState,
    to: &AgentState,
    event: &AgentEvent,
    error: Option<String>,
) {
    let mut snapshot = runtime_snapshot();
    snapshot.transition_seq = snapshot.transition_seq.saturating_add(1);
    snapshot.event_seq = snapshot.event_seq.saturating_add(1);
    snapshot.last_transition_at_ns = now_ns();

    let record = TransitionLogRecord {
        id: format!("{:020}-{:020}", snapshot.transition_seq, snapshot.event_seq),
        turn_id: turn_id.to_string(),
        from_state: from.clone(),
        to_state: to.clone(),
        event: format!("{event:?}"),
        error,
        occurred_at_ns: snapshot.last_transition_at_ns,
    };

    let _ = sqlite::upsert_transition(&record);
    save_runtime_snapshot(&snapshot);
}

/// Persists a `TurnRecord` (with `inner_dialogue` truncated) and its tool-call records.
pub fn append_turn_record(record: &TurnRecord, tool_calls: &[ToolCallRecord]) {
    let mut bounded_record = record.clone();
    bounded_record.inner_dialogue = bounded_record
        .inner_dialogue
        .as_ref()
        .map(|dialogue| truncate_text_field(dialogue, MAX_TURN_INNER_DIALOGUE_CHARS));
    let _ = sqlite::upsert_turn(&bounded_record);
    set_tool_records(&bounded_record.id, tool_calls);
}

/// Records aggregate turn duration telemetry in the runtime snapshot.
pub fn record_turn_duration(started_at_ns: u64, finished_at_ns: u64, max_turn_duration_ns: u64) {
    let mut snapshot = runtime_snapshot();
    let duration_ns = finished_at_ns.saturating_sub(started_at_ns);
    let duration_ms = duration_ns / 1_000_000;
    snapshot.timing_telemetry.last_turn_duration_ms = Some(duration_ms);
    snapshot.timing_telemetry.max_turn_duration_ms = snapshot
        .timing_telemetry
        .max_turn_duration_ms
        .max(duration_ms);
    if duration_ns >= max_turn_duration_ns {
        snapshot.timing_telemetry.turns_over_budget = snapshot
            .timing_telemetry
            .turns_over_budget
            .saturating_add(1);
    }
    save_runtime_snapshot(&snapshot);
}

/// Records one inference/http-fetch outcall latency sample in runtime telemetry.
pub fn record_outcall_timing(
    kind: RuntimeOutcallKind,
    started_at_ns: u64,
    finished_at_ns: u64,
    error: Option<&str>,
    timed_out: bool,
) {
    let mut snapshot = runtime_snapshot();
    let duration_ms = finished_at_ns.saturating_sub(started_at_ns) / 1_000_000;
    let stats = match kind {
        RuntimeOutcallKind::Inference => &mut snapshot.timing_telemetry.inference_outcall,
        RuntimeOutcallKind::HttpFetch => &mut snapshot.timing_telemetry.http_fetch_outcall,
    };
    stats.total_calls = stats.total_calls.saturating_add(1);
    if error.is_some() {
        stats.failure_calls = stats.failure_calls.saturating_add(1);
    }
    if timed_out {
        stats.timeout_failures = stats.timeout_failures.saturating_add(1);
    }
    stats.total_duration_ms = stats.total_duration_ms.saturating_add(duration_ms);
    stats.max_duration_ms = stats.max_duration_ms.max(duration_ms);
    stats.last_duration_ms = Some(duration_ms);
    stats.last_started_at_ns = Some(started_at_ns);
    stats.last_finished_at_ns = Some(finished_at_ns);
    stats.last_error = error.map(|message| truncate_text_field(message, MAX_TIMING_ERROR_CHARS));
    save_runtime_snapshot(&snapshot);
}

/// Returns the most-recent `limit` FSM transition records (newest first).
pub fn list_recent_transitions(limit: usize) -> Vec<TransitionLogRecord> {
    if limit == 0 {
        return Vec::new();
    }
    sqlite::list_recent_transitions(limit).unwrap_or_default()
}

/// Returns the most-recent `limit` agent turn records (newest first).
pub fn list_turns(limit: usize) -> Vec<TurnRecord> {
    if limit == 0 {
        return Vec::new();
    }
    sqlite::list_turns(limit).unwrap_or_default()
}

/// Persists the tool-call records for `turn_id`, truncating field sizes.
pub fn set_tool_records(turn_id: &str, tool_calls: &[ToolCallRecord]) {
    let bounded_tool_calls = tool_calls
        .iter()
        .map(|record| {
            let mut bounded = record.clone();
            bounded.args_json = truncate_text_field(&bounded.args_json, MAX_TOOL_ARGS_JSON_CHARS);
            bounded.output = truncate_text_field(&bounded.output, MAX_TOOL_OUTPUT_CHARS);
            bounded
        })
        .collect::<Vec<_>>();
    let _ = sqlite::replace_tool_calls(turn_id, &bounded_tool_calls);
}

/// Marks the current turn as complete.
pub fn complete_turn(state: AgentState, error: Option<String>) {
    let mut snapshot = runtime_snapshot();
    snapshot.turn_in_flight = false;
    snapshot.state = state;
    snapshot.last_error = error;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
}

/// Returns the tool-call records associated with `turn_id`.
pub fn get_tools_for_turn(turn_id: &str) -> Vec<ToolCallRecord> {
    sqlite::get_tools_for_turn(turn_id).unwrap_or_default()
}

/// Replaces `evm_cursor` in the snapshot.
pub fn set_evm_cursor(cursor: &EvmPollCursor) {
    let mut snapshot = runtime_snapshot();
    let mut next_cursor = cursor.clone();
    if next_cursor.contract_address.is_none() {
        next_cursor.contract_address = snapshot.inbox_contract_address.clone();
    }
    if next_cursor.automaton_address_topic.is_none() {
        next_cursor.automaton_address_topic =
            snapshot.evm_address.as_deref().map(evm_address_to_topic);
    }
    snapshot.evm_cursor = next_cursor;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
}

/// Idempotent deduplication for EVM log ingestion.
pub fn try_mark_evm_event_ingested(tx_hash: &str, log_index: u64) -> bool {
    let key = evm_ingest_dedupe_key(tx_hash, log_index);
    if runtime_bool(&key).is_some() {
        return false;
    }
    save_runtime_bool(&key, true);
    true
}

pub fn normalize_inbox_body(raw_body: &str) -> Result<String, String> {
    let trimmed = raw_body.trim();
    if trimmed.is_empty() {
        return Err("message cannot be empty".to_string());
    }
    Ok(truncate_text_field(trimmed, MAX_INBOX_BODY_CHARS))
}

// ── Inbox / Outbox ────────────────────────────────────────────────────────────

/// Creates and stores a new inbox message. Returns the new message ID.
pub fn post_inbox_message(body: String, caller: String) -> Result<String, String> {
    let bounded_body = normalize_inbox_body(&body)?;

    let seq = next_inbox_seq();
    let id = format!("inbox:{seq:020}");
    let message = InboxMessage {
        id: id.clone(),
        seq,
        body: bounded_body,
        posted_at_ns: now_ns(),
        posted_by: caller,
        status: InboxMessageStatus::Pending,
        staged_at_ns: None,
        consumed_at_ns: None,
    };
    let _ = sqlite::upsert_inbox(&message);
    log!(
        SchedulerStorageLogPriority::Info,
        "inbox_posted id={} seq={}",
        id,
        seq
    );
    Ok(id)
}

/// Returns the most-recent `limit` inbox messages (newest first).
pub fn list_inbox_messages(limit: usize) -> Vec<InboxMessage> {
    if limit == 0 {
        return Vec::new();
    }
    sqlite::list_inbox_messages(limit).unwrap_or_default()
}

/// Computes inbox statistics.
pub fn inbox_stats() -> InboxStats {
    let mut stats = InboxStats::default();
    stats.pending_count = sqlite::count_inbox_by_status("Pending").unwrap_or(0);
    stats.staged_count = sqlite::count_inbox_by_status("Staged").unwrap_or(0);
    stats.consumed_count = sqlite::count_inbox_by_status("Consumed").unwrap_or(0);
    stats.total_messages = stats
        .pending_count
        .saturating_add(stats.staged_count)
        .saturating_add(stats.consumed_count);
    stats
}

/// Creates and stores a new outbox message. Returns the new message ID.
pub fn post_outbox_message(
    turn_id: String,
    body: String,
    source_inbox_ids: Vec<String>,
) -> Result<String, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("outbox message cannot be empty".to_string());
    }

    let seq = next_outbox_seq();
    let id = format!("outbox:{seq:020}");
    let message = OutboxMessage {
        id: id.clone(),
        seq,
        turn_id,
        body: trimmed.to_string(),
        created_at_ns: now_ns(),
        source_inbox_ids,
    };
    let _ = sqlite::upsert_outbox(&message);
    Ok(id)
}

/// Returns the most-recent `limit` outbox messages (newest first).
pub fn list_outbox_messages(limit: usize) -> Vec<OutboxMessage> {
    if limit == 0 {
        return Vec::new();
    }
    sqlite::list_outbox_messages(limit).unwrap_or_default()
}

pub fn get_outbox_message(message_id: &str) -> Option<OutboxMessage> {
    sqlite::get_outbox_message(message_id).ok().flatten()
}

/// Computes outbox statistics.
pub fn outbox_stats() -> OutboxStats {
    let total = sqlite::table_count("outbox").unwrap_or(0);
    OutboxStats {
        total_messages: total,
    }
}

/// Moves pending inbox messages to staged status. Returns staged count.
pub fn stage_pending_inbox_messages(batch_size: usize, now_ns: u64) -> usize {
    if batch_size == 0 {
        return 0;
    }
    let pending = match sqlite::list_pending_inbox(batch_size) {
        Ok(msgs) => msgs,
        Err(_) => return 0,
    };
    let mut staged_count = 0usize;
    for mut message in pending {
        if matches!(message.status, InboxMessageStatus::Pending) {
            message.status = InboxMessageStatus::Staged;
            message.staged_at_ns = Some(now_ns);
            let _ = sqlite::upsert_inbox(&message);
            staged_count += 1;
        }
    }
    if staged_count > 0 {
        log!(
            SchedulerStorageLogPriority::Info,
            "inbox_staged count={} now_ns={}",
            staged_count,
            now_ns
        );
    }
    staged_count
}

/// Returns up to `batch_size` messages in `Staged` state, in FIFO order.
pub fn list_staged_inbox_messages(batch_size: usize) -> Vec<InboxMessage> {
    if batch_size == 0 {
        return Vec::new();
    }
    sqlite::list_staged_inbox(batch_size).unwrap_or_default()
}

/// Marks the given staged inbox message IDs as `Consumed`. Returns consumed count.
pub fn consume_staged_inbox_messages(ids: &[String], now_ns: u64) -> usize {
    if ids.is_empty() {
        return 0;
    }
    let mut consumed = 0usize;
    for id in ids {
        let Some(mut message) = sqlite::get_inbox_message(id).ok().flatten() else {
            continue;
        };
        if !matches!(message.status, InboxMessageStatus::Staged) {
            continue;
        }
        message.status = InboxMessageStatus::Consumed;
        message.consumed_at_ns = Some(now_ns);
        let _ = sqlite::upsert_inbox(&message);
        consumed += 1;
    }
    if consumed > 0 {
        log!(
            SchedulerStorageLogPriority::Info,
            "inbox_consumed count={} now_ns={}",
            consumed,
            now_ns
        );
    }
    consumed
}

// ── Memory facts ──────────────────────────────────────────────────────────────

/// Stores a memory fact, evicting the oldest if at capacity.
pub fn set_memory_fact(fact: &MemoryFact) -> Result<(), String> {
    let count = sqlite::count_memory_facts().unwrap_or(0);
    if count >= MAX_MEMORY_FACTS {
        // Only evict if this is a new key (not an update to an existing one)
        if sqlite::get_memory_fact(&fact.key).ok().flatten().is_none() {
            // Find the oldest non-critical fact to evict
            let all_facts = sqlite::list_all_memory_facts(MAX_MEMORY_FACTS).unwrap_or_default();
            let eviction_candidate = all_facts
                .iter()
                .filter(|stored| !is_critical_exact_memory_key(&stored.key))
                .min_by(|left, right| {
                    left.updated_at_ns
                        .cmp(&right.updated_at_ns)
                        .then_with(|| left.key.cmp(&right.key))
                })
                .map(|stored| stored.key.clone());

            let Some(candidate_key) = eviction_candidate else {
                return Err(
                    "memory full: non-evictable capacity reached (all stored facts are critical)"
                        .to_string(),
                );
            };

            let _ = sqlite::delete_memory_fact(&candidate_key);
        }
    }
    sqlite::upsert_memory_fact(fact)
}

pub fn get_memory_fact(key: &str) -> Option<MemoryFact> {
    sqlite::get_memory_fact(key).ok().flatten()
}

pub fn remove_memory_fact(key: &str) -> bool {
    sqlite::delete_memory_fact(key).is_ok()
}

pub fn memory_fact_count() -> usize {
    sqlite::count_memory_facts().unwrap_or(0)
}

pub fn memory_fact_stats() -> MemoryFactStats {
    let total_facts = sqlite::count_memory_facts().unwrap_or(0);
    let config_facts = sqlite::count_memory_facts_by_prefix("config.").unwrap_or(0);
    // Estimate storage bytes from all facts
    let all_facts = sqlite::list_all_memory_facts(usize::MAX).unwrap_or_default();
    let storage_bytes: usize = all_facts
        .iter()
        .map(|f| f.key.len() + f.value.len() + f.source_turn_id.len() + 16)
        .sum();
    MemoryFactStats {
        total_facts,
        storage_bytes,
        config_facts,
    }
}

pub fn count_memory_facts_by_prefix(prefix: &str) -> usize {
    sqlite::count_memory_facts_by_prefix(prefix).unwrap_or(0)
}

pub fn list_all_memory_facts(limit: usize) -> Vec<MemoryFact> {
    list_all_memory_facts_sorted(limit, MemoryFactSort::UpdatedAtDesc)
}

pub fn list_all_memory_facts_sorted(limit: usize, sort: MemoryFactSort) -> Vec<MemoryFact> {
    let mut facts = sqlite::list_all_memory_facts(limit.max(MAX_MEMORY_FACTS)).unwrap_or_default();
    sort_memory_facts(&mut facts, sort);
    facts.truncate(limit);
    facts
}

pub fn list_memory_facts_by_prefix(prefix: &str, limit: usize) -> Vec<MemoryFact> {
    sqlite::list_memory_facts_by_prefix(prefix, limit).unwrap_or_default()
}

pub fn list_memory_facts_by_prefix_sorted(
    prefix: &str,
    limit: usize,
    sort: MemoryFactSort,
) -> Vec<MemoryFact> {
    let mut facts = sqlite::list_memory_facts_by_prefix(prefix, limit.max(MAX_MEMORY_FACTS))
        .unwrap_or_default();
    sort_memory_facts(&mut facts, sort);
    facts.truncate(limit);
    facts
}

pub fn prune_memory_facts(
    prefix: Option<&str>,
    updated_before_ns: Option<u64>,
    limit: usize,
) -> Vec<String> {
    sqlite::prune_memory_facts(prefix, updated_before_ns, limit).unwrap_or_default()
}

fn sort_memory_facts(facts: &mut [MemoryFact], sort: MemoryFactSort) {
    match sort {
        MemoryFactSort::UpdatedAtDesc => facts.sort_by(|a, b| b.updated_at_ns.cmp(&a.updated_at_ns)),
        MemoryFactSort::KeyAsc => facts.sort_by(|a, b| a.key.cmp(&b.key)),
    }
}

// ── Strategy templates ────────────────────────────────────────────────────────

pub fn upsert_strategy_template(template: StrategyTemplate) -> Result<StrategyTemplate, String> {
    if template.key.protocol.trim().is_empty() {
        return Err("strategy template protocol cannot be empty".to_string());
    }
    if template.key.template_id.trim().is_empty() {
        return Err("strategy template template_id cannot be empty".to_string());
    }
    sqlite::upsert_strategy_template(&template)?;
    Ok(template)
}

pub fn strategy_template(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> Option<StrategyTemplate> {
    sqlite::strategy_template(key, version).ok().flatten()
}

pub fn list_strategy_template_versions(key: &StrategyTemplateKey) -> Vec<TemplateVersion> {
    sqlite::list_strategy_template_versions(key).unwrap_or_default()
}

pub fn list_strategy_templates(key: &StrategyTemplateKey, limit: usize) -> Vec<StrategyTemplate> {
    sqlite::list_strategy_templates(key, limit).unwrap_or_default()
}

pub fn list_all_strategy_templates(limit: usize) -> Vec<StrategyTemplate> {
    sqlite::list_all_strategy_templates(limit).unwrap_or_default()
}

pub fn upsert_abi_artifact(artifact: AbiArtifact) -> Result<AbiArtifact, String> {
    if artifact.key.protocol.trim().is_empty() {
        return Err("abi artifact protocol cannot be empty".to_string());
    }
    if artifact.key.role.trim().is_empty() {
        return Err("abi artifact role cannot be empty".to_string());
    }
    sqlite::upsert_abi_artifact(&artifact)?;
    Ok(artifact)
}

pub fn abi_artifact(key: &AbiArtifactKey) -> Option<AbiArtifact> {
    sqlite::abi_artifact(key).ok().flatten()
}

pub fn list_abi_artifact_versions(
    protocol: &str,
    chain_id: u64,
    role: &str,
) -> Vec<TemplateVersion> {
    sqlite::list_abi_artifact_versions(protocol, chain_id, role).unwrap_or_default()
}

// ── Strategy activations / revocations / kill switches / outcome stats ───────

pub fn set_strategy_template_activation(
    state: TemplateActivationState,
) -> Result<TemplateActivationState, String> {
    validate_strategy_template_key(&state.key)?;
    validate_template_version(&state.version)?;
    let record_key = template_state_record_key("activation", &state.key, &state.version);
    sqlite::upsert_strategy_activation(&record_key, &state)?;
    Ok(state)
}

pub fn strategy_template_activation(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> Option<TemplateActivationState> {
    let record_key = template_state_record_key("activation", key, version);
    sqlite::get_strategy_activation::<TemplateActivationState>(&record_key)
        .ok()
        .flatten()
}

pub fn set_strategy_template_revocation(
    state: TemplateRevocationState,
) -> Result<TemplateRevocationState, String> {
    validate_strategy_template_key(&state.key)?;
    validate_template_version(&state.version)?;
    let record_key = template_state_record_key("revocation", &state.key, &state.version);
    sqlite::upsert_strategy_revocation(&record_key, &state)?;
    Ok(state)
}

#[allow(dead_code)]
pub fn strategy_template_revocation(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> Option<TemplateRevocationState> {
    let record_key = template_state_record_key("revocation", key, version);
    sqlite::get_strategy_revocation::<TemplateRevocationState>(&record_key)
        .ok()
        .flatten()
}

pub fn set_strategy_kill_switch(
    state: StrategyKillSwitchState,
) -> Result<StrategyKillSwitchState, String> {
    validate_strategy_template_key(&state.key)?;
    let record_key = strategy_kill_switch_record_key(&state.key);
    sqlite::upsert_strategy_kill_switch(&record_key, &state)?;
    Ok(state)
}

pub fn strategy_kill_switch(key: &StrategyTemplateKey) -> Option<StrategyKillSwitchState> {
    let record_key = strategy_kill_switch_record_key(key);
    sqlite::get_strategy_kill_switch::<StrategyKillSwitchState>(&record_key)
        .ok()
        .flatten()
}

pub fn strategy_outcome_stats(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> Option<StrategyOutcomeStats> {
    let record_key = strategy_outcome_stats_record_key(key, version);
    sqlite::get_strategy_outcome_stats::<StrategyOutcomeStats>(&record_key)
        .ok()
        .flatten()
}

pub fn upsert_strategy_outcome_stats(
    stats: StrategyOutcomeStats,
) -> Result<StrategyOutcomeStats, String> {
    validate_strategy_template_key(&stats.key)?;
    validate_template_version(&stats.version)?;
    let record_key = strategy_outcome_stats_record_key(&stats.key, &stats.version);
    sqlite::upsert_strategy_outcome_stats(&record_key, &stats)?;
    Ok(stats)
}

pub fn strategy_template_budget_spent_wei(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> Option<String> {
    let record_key = strategy_budget_record_key(key, version);
    sqlite::get_strategy_budget(&record_key).ok().flatten()
}

pub fn set_strategy_template_budget_spent_wei(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
    spent_wei: String,
) -> Result<String, String> {
    validate_strategy_template_key(key)?;
    validate_template_version(version)?;
    let normalized = normalize_decimal_string(&spent_wei, "strategy budget spent_wei")?;
    let record_key = strategy_budget_record_key(key, version);
    sqlite::upsert_strategy_budget(&record_key, &normalized)?;
    Ok(normalized)
}

// ── Strategy helper functions ────────────────────────────────────────────────

fn validate_strategy_template_key(key: &StrategyTemplateKey) -> Result<(), String> {
    if key.protocol.trim().is_empty() {
        return Err("strategy protocol must be non-empty".to_string());
    }
    if key.primitive.trim().is_empty() {
        return Err("strategy primitive must be non-empty".to_string());
    }
    if key.chain_id == 0 {
        return Err("strategy chain_id must be greater than zero".to_string());
    }
    if key.template_id.trim().is_empty() {
        return Err("strategy template_id must be non-empty".to_string());
    }
    Ok(())
}

fn validate_template_version(version: &TemplateVersion) -> Result<(), String> {
    if version.major == 0 && version.minor == 0 && version.patch == 0 {
        return Err("template version must not be 0.0.0".to_string());
    }
    Ok(())
}

fn strategy_template_lookup_key(key: &StrategyTemplateKey) -> String {
    let normalized = format!(
        "{}|{}|{}|{}",
        key.protocol.trim().to_ascii_lowercase(),
        key.primitive.trim().to_ascii_lowercase(),
        key.chain_id,
        key.template_id.trim().to_ascii_lowercase()
    );
    lookup_digest("strategy:template", &normalized)
}

fn template_version_sort_key(version: &TemplateVersion) -> String {
    format!(
        "{:05}.{:05}.{:05}",
        version.major, version.minor, version.patch
    )
}

fn template_state_record_key(
    kind: &str,
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> String {
    format!(
        "strategy:{kind}:{}:{}",
        strategy_template_lookup_key(key),
        template_version_sort_key(version)
    )
}

fn strategy_kill_switch_record_key(key: &StrategyTemplateKey) -> String {
    format!("strategy:kill_switch:{}", strategy_template_lookup_key(key))
}

fn strategy_outcome_stats_record_key(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> String {
    format!(
        "strategy:outcome:{}:{}",
        strategy_template_lookup_key(key),
        template_version_sort_key(version)
    )
}

fn strategy_budget_record_key(key: &StrategyTemplateKey, version: &TemplateVersion) -> String {
    format!(
        "strategy:budget:{}:{}",
        strategy_template_lookup_key(key),
        template_version_sort_key(version)
    )
}

fn lookup_digest(prefix: &str, payload: &str) -> String {
    let mut hasher = Keccak256::new();
    hasher.update(payload.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{prefix}:{digest}")
}

fn normalize_decimal_string(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must be non-empty"));
    }
    if !trimmed.as_bytes().iter().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{field} must be a decimal string"));
    }
    Ok(trimmed.to_string())
}

/// Records a strategy execution outcome, updating outcome stats and budget.
pub fn record_strategy_outcome(
    outcome: StrategyOutcomeEvent,
) -> Result<StrategyOutcomeStats, String> {
    validate_strategy_template_key(&outcome.key)?;
    validate_template_version(&outcome.version)?;
    if outcome.action_id.trim().is_empty() {
        return Err("outcome action_id must be non-empty".to_string());
    }

    let mut stats = strategy_outcome_stats(&outcome.key, &outcome.version).unwrap_or_else(|| {
        StrategyOutcomeStats {
            key: outcome.key.clone(),
            version: outcome.version.clone(),
            total_runs: 0,
            success_runs: 0,
            deterministic_failures: 0,
            nondeterministic_failures: 0,
            deterministic_failure_streak: 0,
            confidence_bps: 0,
            ranking_score_bps: 0,
            parameter_priors: crate::domain::types::StrategyParameterPriors::default(),
            last_error: None,
            last_tx_hash: None,
            last_observed_at_ns: None,
        }
    });

    stats.total_runs = stats.total_runs.saturating_add(1);
    match outcome.outcome {
        StrategyOutcomeKind::Success => {
            stats.success_runs = stats.success_runs.saturating_add(1);
            stats.deterministic_failure_streak = 0;
            stats.last_error = None;
        }
        StrategyOutcomeKind::DeterministicFailure => {
            stats.deterministic_failures = stats.deterministic_failures.saturating_add(1);
            stats.deterministic_failure_streak =
                stats.deterministic_failure_streak.saturating_add(1);
            stats.last_error = outcome.error.clone();
        }
        StrategyOutcomeKind::NondeterministicFailure => {
            stats.nondeterministic_failures = stats.nondeterministic_failures.saturating_add(1);
            stats.deterministic_failure_streak = 0;
            stats.last_error = outcome.error.clone();
        }
    }
    stats.last_tx_hash = outcome.tx_hash.clone();
    stats.last_observed_at_ns = Some(outcome.observed_at_ns);
    upsert_strategy_outcome_stats(stats)
}

// ── Autonomy tool tracking ────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AutonomyToolFailureTracker {
    normalized_error: String,
    repeat_count: u32,
    first_failed_at_ns: u64,
    last_failed_at_ns: u64,
    cooldown_until_ns: Option<u64>,
    last_suppressed_at_ns: Option<u64>,
    suppressed_count: u32,
}

pub fn autonomy_tool_last_success_ns(fingerprint: &str) -> Option<u64> {
    let key = format!("{AUTONOMY_TOOL_SUCCESS_KEY_PREFIX}{fingerprint}");
    runtime_u64(&key)
}

pub fn record_autonomy_tool_success(fingerprint: &str, succeeded_at_ns: u64) {
    let key = format!("{AUTONOMY_TOOL_SUCCESS_KEY_PREFIX}{fingerprint}");
    save_runtime_u64(&key, succeeded_at_ns);
}

/// Returns active cooldown metadata for an autonomy tool call fingerprint.
pub fn autonomy_tool_failure_cooldown(
    fingerprint: &str,
    now_ns: u64,
) -> Option<AutonomyToolFailureCooldown> {
    let key = autonomy_tool_failure_key(fingerprint);
    let tracker: AutonomyToolFailureTracker = sqlite::get_autonomy_tool_failure(&key)
        .ok()
        .flatten()?;
    let cooldown_until_ns = tracker.cooldown_until_ns?;
    if cooldown_until_ns <= now_ns {
        return None;
    }
    Some(AutonomyToolFailureCooldown {
        normalized_error: tracker.normalized_error,
        repeat_count: tracker.repeat_count,
        cooldown_until_ns,
    })
}

/// Records an autonomy tool failure and returns a newly-active cooldown when
/// the repeat threshold is reached.
pub fn record_autonomy_tool_failure(
    fingerprint: &str,
    normalized_error: &str,
    failed_at_ns: u64,
) -> Option<AutonomyToolFailureCooldown> {
    let trimmed_error = normalized_error.trim();
    if trimmed_error.is_empty() {
        return None;
    }

    let key = autonomy_tool_failure_key(fingerprint);
    let existing: Option<AutonomyToolFailureTracker> =
        sqlite::get_autonomy_tool_failure(&key).ok().flatten();

    let mut tracker = existing.unwrap_or_default();
    let within_window = failed_at_ns.saturating_sub(tracker.last_failed_at_ns)
        <= timing::AUTONOMY_FAILURE_REPEAT_WINDOW_NS;
    if tracker.normalized_error == trimmed_error && within_window {
        tracker.repeat_count = tracker.repeat_count.saturating_add(1);
    } else {
        tracker.repeat_count = 1;
        tracker.first_failed_at_ns = failed_at_ns;
    }

    tracker.normalized_error = trimmed_error.to_string();
    tracker.last_failed_at_ns = failed_at_ns;
    if tracker.repeat_count >= timing::AUTONOMY_FAILURE_REPEAT_THRESHOLD {
        tracker.cooldown_until_ns =
            Some(failed_at_ns.saturating_add(timing::AUTONOMY_FAILURE_COOLDOWN_NS));
    } else {
        tracker.cooldown_until_ns = None;
    }

    let _ = sqlite::upsert_autonomy_tool_failure(&key, &tracker);

    tracker
        .cooldown_until_ns
        .filter(|cooldown_until_ns| *cooldown_until_ns > failed_at_ns)
        .map(|cooldown_until_ns| AutonomyToolFailureCooldown {
            normalized_error: tracker.normalized_error,
            repeat_count: tracker.repeat_count,
            cooldown_until_ns,
        })
}

/// Clears stored autonomy failure streak/cooldown state for a tool fingerprint.
pub fn clear_autonomy_tool_failure(fingerprint: &str) {
    let _ = sqlite::delete_autonomy_tool_failure(&autonomy_tool_failure_key(fingerprint));
}

/// Records that a failure-cooldown suppression was applied for a fingerprint.
pub fn note_autonomy_tool_failure_suppressed(fingerprint: &str, now_ns: u64) {
    let key = autonomy_tool_failure_key(fingerprint);
    let Some(mut tracker): Option<AutonomyToolFailureTracker> =
        sqlite::get_autonomy_tool_failure(&key).ok().flatten()
    else {
        return;
    };
    tracker.last_suppressed_at_ns = Some(now_ns);
    tracker.suppressed_count = tracker.suppressed_count.saturating_add(1);
    let _ = sqlite::upsert_autonomy_tool_failure(&key, &tracker);
}

fn autonomy_tool_failure_key(fingerprint: &str) -> String {
    format!("{AUTONOMY_TOOL_FAILURE_KEY_PREFIX}{fingerprint}")
}

// ── HTTP allowlist ────────────────────────────────────────────────────────────

pub fn list_allowed_http_domains() -> Vec<String> {
    sqlite::list_http_domains().unwrap_or_default()
}

pub fn is_http_allowlist_enforced() -> bool {
    runtime_bool(HTTP_ALLOWLIST_INITIALIZED_KEY).unwrap_or(false)
}

pub fn set_http_allowed_domains(domains: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::with_capacity(domains.len());
    for raw in &domains {
        normalized.push(normalize_http_allowed_domain(raw)?);
    }
    normalized.sort();
    normalized.dedup();
    sqlite::set_http_domains(&normalized)?;
    save_runtime_bool(HTTP_ALLOWLIST_INITIALIZED_KEY, true);
    Ok(normalized)
}

pub fn add_http_allowed_domain(domain: String) -> Result<String, String> {
    let normalized = normalize_http_allowed_domain(&domain)?;
    sqlite::add_http_domain(&normalized)?;
    save_runtime_bool(HTTP_ALLOWLIST_INITIALIZED_KEY, true);
    Ok(normalized)
}

pub fn remove_http_allowed_domain(domain: String) -> Result<bool, String> {
    let normalized = normalize_http_allowed_domain(&domain)?;
    let removed = sqlite::remove_http_domain(&normalized)?;
    save_runtime_bool(HTTP_ALLOWLIST_INITIALIZED_KEY, true);
    Ok(removed)
}

fn normalize_http_allowed_domain(raw: &str) -> Result<String, String> {
    let domain = raw.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Err("http allowed domain cannot be empty".to_string());
    }
    if domain.contains("://")
        || domain.contains('/')
        || domain.contains('?')
        || domain.contains('#')
        || domain.contains('@')
        || domain.contains(':')
    {
        return Err("http allowed domain must be a bare host without scheme/path/port".to_string());
    }
    if domain.starts_with('.') || domain.ends_with('.') {
        return Err("http allowed domain must not start or end with '.'".to_string());
    }

    for label in domain.split('.') {
        if label.is_empty() {
            return Err("http allowed domain labels must not be empty".to_string());
        }
        let bytes = label.as_bytes();
        let starts_ok = bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
        let ends_ok = bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
        if !starts_ok || !ends_ok {
            return Err(
                "http allowed domain labels must start and end with alphanumeric characters"
                    .to_string(),
            );
        }
        if !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        {
            return Err("http allowed domain labels may only contain [a-z0-9-]".to_string());
        }
    }

    Ok(domain)
}

// ── Wallet balance ────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub fn wallet_balance_snapshot() -> WalletBalanceSnapshot {
    runtime_snapshot().wallet_balance
}

#[allow(dead_code)]
pub fn set_wallet_balance_snapshot(balance: WalletBalanceSnapshot) {
    let mut snapshot = runtime_snapshot();
    snapshot.wallet_balance = balance;
    save_runtime_snapshot(&snapshot);
}

#[allow(dead_code)]
pub fn wallet_balance_sync_config() -> WalletBalanceSyncConfig {
    runtime_snapshot().wallet_balance_sync
}

#[allow(dead_code)]
pub fn set_wallet_balance_sync_config(
    config: WalletBalanceSyncConfig,
) -> Result<WalletBalanceSyncConfig, String> {
    validate_wallet_balance_sync_config(&config)?;
    let mut snapshot = runtime_snapshot();
    snapshot.wallet_balance_sync = config.clone();
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    Ok(config)
}

#[allow(dead_code)]
pub fn wallet_balance_bootstrap_pending() -> bool {
    runtime_snapshot().wallet_balance_bootstrap_pending
}

#[allow(dead_code)]
pub fn set_wallet_balance_bootstrap_pending(pending: bool) {
    let mut snapshot = runtime_snapshot();
    snapshot.wallet_balance_bootstrap_pending = pending;
    save_runtime_snapshot(&snapshot);
}

pub fn record_wallet_balance_sync_success(
    now_ns: u64,
    eth_balance_wei_hex: String,
    usdc_balance_raw_hex: String,
    usdc_contract_address: String,
) -> WalletBalanceSnapshot {
    let mut snapshot = runtime_snapshot();
    snapshot.wallet_balance.eth_balance_wei_hex = Some(eth_balance_wei_hex);
    snapshot.wallet_balance.usdc_balance_raw_hex = Some(usdc_balance_raw_hex);
    snapshot.wallet_balance.usdc_contract_address = Some(usdc_contract_address);
    snapshot.wallet_balance.last_synced_at_ns = Some(now_ns);
    snapshot.wallet_balance.last_synced_block = None;
    snapshot.wallet_balance.last_error = None;
    snapshot.wallet_balance_bootstrap_pending = false;
    let updated = snapshot.wallet_balance.clone();
    save_runtime_snapshot(&snapshot);
    updated
}

pub fn record_wallet_balance_sync_error(error: String) -> WalletBalanceSnapshot {
    let mut snapshot = runtime_snapshot();
    snapshot.wallet_balance.last_error = Some(error);
    let updated = snapshot.wallet_balance.clone();
    save_runtime_snapshot(&snapshot);
    updated
}

pub fn set_last_error(error: Option<String>) {
    let mut snapshot = runtime_snapshot();
    snapshot.last_error = error;
    save_runtime_snapshot(&snapshot);
}

pub fn increment_turn_counter() -> RuntimeSnapshot {
    let mut snapshot = runtime_snapshot();
    snapshot.turn_counter = snapshot.turn_counter.saturating_add(1);
    snapshot.turn_in_flight = true;
    snapshot.last_turn_id = Some(format!("turn-{}", snapshot.turn_counter));
    snapshot.last_error = None;
    snapshot.last_transition_at_ns = now_ns();
    save_runtime_snapshot(&snapshot);
    snapshot
}

pub fn snapshot_to_view() -> RuntimeView {
    RuntimeView::from(&runtime_snapshot())
}

pub fn evm_route_state_view() -> EvmRouteStateView {
    EvmRouteStateView::from(&runtime_snapshot())
}

pub fn wallet_balance_telemetry_view() -> WalletBalanceTelemetryView {
    WalletBalanceTelemetryView::from_snapshot(&runtime_snapshot(), now_ns())
}

pub fn wallet_balance_sync_config_view() -> WalletBalanceSyncConfigView {
    WalletBalanceSyncConfigView::from(&runtime_snapshot().wallet_balance_sync)
}

fn inbox_usdc_discovery_blocked(snapshot: &RuntimeSnapshot) -> bool {
    snapshot
        .wallet_balance
        .last_error
        .as_deref()
        .map(|error| {
            error
                .to_ascii_lowercase()
                .contains("inbox.usdc returned zero address")
        })
        .unwrap_or(false)
}

fn wallet_balance_sync_has_discoverable_usdc_source(snapshot: &RuntimeSnapshot) -> bool {
    snapshot.wallet_balance_sync.discover_usdc_via_inbox
        && snapshot.inbox_contract_address.is_some()
        && !inbox_usdc_discovery_blocked(snapshot)
}

fn wallet_balance_sync_has_usdc_source(snapshot: &RuntimeSnapshot) -> bool {
    snapshot.wallet_balance.usdc_contract_address.is_some()
        || wallet_balance_sync_has_discoverable_usdc_source(snapshot)
}

pub fn wallet_balance_sync_capable(snapshot: &RuntimeSnapshot) -> bool {
    if !snapshot.wallet_balance_sync.enabled {
        return false;
    }
    if snapshot.evm_rpc_url.trim().is_empty() {
        return false;
    }
    if snapshot.evm_address.is_some() {
        if snapshot.wallet_balance.usdc_contract_address.is_some() {
            return true;
        }
        if !snapshot.wallet_balance_sync.discover_usdc_via_inbox {
            return true;
        }
        return wallet_balance_sync_has_discoverable_usdc_source(snapshot);
    }
    if snapshot.ecdsa_key_name.trim().is_empty() {
        return false;
    }
    wallet_balance_sync_has_usdc_source(snapshot)
}

pub fn inference_config_view() -> InferenceConfigView {
    InferenceConfigView::from(&runtime_snapshot())
}

#[allow(dead_code)]
fn validate_wallet_balance_sync_config(config: &WalletBalanceSyncConfig) -> Result<(), String> {
    if config.normal_interval_secs < MIN_WALLET_BALANCE_SYNC_INTERVAL_SECS
        || config.normal_interval_secs > MAX_WALLET_BALANCE_SYNC_INTERVAL_SECS
    {
        return Err(format!(
            "wallet balance sync normal_interval_secs must be in {MIN_WALLET_BALANCE_SYNC_INTERVAL_SECS}..={MAX_WALLET_BALANCE_SYNC_INTERVAL_SECS}"
        ));
    }
    if config.low_cycles_interval_secs < MIN_WALLET_BALANCE_SYNC_INTERVAL_SECS
        || config.low_cycles_interval_secs > MAX_WALLET_BALANCE_SYNC_INTERVAL_SECS
    {
        return Err(format!(
            "wallet balance sync low_cycles_interval_secs must be in {MIN_WALLET_BALANCE_SYNC_INTERVAL_SECS}..={MAX_WALLET_BALANCE_SYNC_INTERVAL_SECS}"
        ));
    }
    if config.low_cycles_interval_secs < config.normal_interval_secs {
        return Err(
            "wallet balance sync low_cycles_interval_secs must be >= normal_interval_secs"
                .to_string(),
        );
    }
    if config.freshness_window_secs < MIN_WALLET_BALANCE_FRESHNESS_WINDOW_SECS
        || config.freshness_window_secs > MAX_WALLET_BALANCE_FRESHNESS_WINDOW_SECS
    {
        return Err(format!(
            "wallet balance sync freshness_window_secs must be in {MIN_WALLET_BALANCE_FRESHNESS_WINDOW_SECS}..={MAX_WALLET_BALANCE_FRESHNESS_WINDOW_SECS}"
        ));
    }
    if config.max_response_bytes < MIN_WALLET_BALANCE_SYNC_RESPONSE_BYTES
        || config.max_response_bytes > MAX_WALLET_BALANCE_SYNC_RESPONSE_BYTES
    {
        return Err(format!(
            "wallet balance sync max_response_bytes must be in {MIN_WALLET_BALANCE_SYNC_RESPONSE_BYTES}..={MAX_WALLET_BALANCE_SYNC_RESPONSE_BYTES}"
        ));
    }
    Ok(())
}

// ── Skills ────────────────────────────────────────────────────────────────────

pub fn upsert_skill(skill: &SkillRecord) {
    let _ = sqlite::upsert_skill(skill);
}

pub fn list_skills() -> Vec<SkillRecord> {
    sqlite::list_skills().unwrap_or_default()
}

pub fn remove_skill(name: &str) -> bool {
    sqlite::delete_skill(name).is_ok()
}

// ── Job queue ─────────────────────────────────────────────────────────────────

pub fn enqueue_job_if_absent(
    kind: TaskKind,
    lane: TaskLane,
    dedupe_key: String,
    scheduled_for_ns: u64,
    priority: u8,
) -> Option<String> {
    // Check dedup
    if let Ok(Some(existing)) = sqlite::find_job_by_dedupe_key(&dedupe_key) {
        if !existing.is_terminal() {
            log!(
                SchedulerStorageLogPriority::Warn,
                "scheduler_dedupe_hit kind={:?} dedupe_key={} existing_job_id={} status={:?}",
                kind,
                dedupe_key,
                existing.id,
                existing.status
            );
            return None;
        }
    }

    let mut runtime = scheduler_runtime();
    let job_seq = runtime.next_job_seq.saturating_add(1);
    runtime.next_job_seq = job_seq;
    save_scheduler_runtime(&runtime);

    let now = now_ns();
    let job_id = format!("job:{:020}:{:020}", job_seq, scheduled_for_ns);
    let job = ScheduledJob {
        id: job_id.clone(),
        kind: kind.clone(),
        lane: lane.clone(),
        dedupe_key: dedupe_key.clone(),
        priority,
        created_at_ns: now,
        scheduled_for_ns,
        started_at_ns: None,
        finished_at_ns: None,
        status: JobStatus::Pending,
        attempts: 0,
        max_attempts: 3,
        last_error: None,
    };

    let _ = sqlite::upsert_job(&job);

    // Update task runtime's pending_job_id
    if let Some(mut task_runtime) = sqlite::read_task_runtime(&kind).ok().flatten() {
        task_runtime.pending_job_id = Some(job_id.clone());
        let _ = sqlite::write_task_runtime(&kind, &task_runtime);
    }

    log!(
        SchedulerStorageLogPriority::Info,
        "scheduler_enqueue_job kind={:?} lane={:?} job_id={} dedupe_key={} scheduled_for={}",
        kind,
        lane,
        job_id,
        dedupe_key,
        scheduled_for_ns
    );

    Some(job_id)
}

pub fn pop_next_pending_job(lane: TaskLane, now_ns_param: u64) -> Option<ScheduledJob> {
    let mut job = sqlite::pop_next_pending_job(lane.as_str(), now_ns_param).ok()??;
    job.started_at_ns = Some(now_ns());
    job.status = JobStatus::InFlight;
    let _ = sqlite::upsert_job(&job);
    log!(
        SchedulerStorageLogPriority::Info,
        "scheduler_pop_pending job_id={} lane={:?}",
        job.id,
        lane
    );
    Some(job)
}

pub fn acquire_mutating_lease(job_id: &str, now_ns: u64, ttl_ns: u64) -> Result<(), String> {
    let mut runtime = scheduler_runtime();
    if runtime
        .active_mutating_lease
        .as_ref()
        .is_some_and(|lease| lease.expires_at_ns > now_ns)
    {
        log!(
            SchedulerStorageLogPriority::Warn,
            "scheduler_lease_active_reject job_id={}",
            job_id
        );
        return Err("mutating lease already active".to_string());
    }
    if sqlite::get_job(job_id).ok().flatten().is_none() {
        log!(
            SchedulerStorageLogPriority::Warn,
            "scheduler_lease_acquire_missing_job job_id={}",
            job_id
        );
        return Err("job not found".to_string());
    }
    runtime.active_mutating_lease = Some(SchedulerLease {
        lane: TaskLane::Mutating,
        job_id: job_id.to_string(),
        acquired_at_ns: now_ns,
        expires_at_ns: now_ns.saturating_add(ttl_ns),
    });
    log!(
        SchedulerStorageLogPriority::Info,
        "scheduler_lease_acquired job_id={} ttl_ns={}",
        job_id,
        ttl_ns
    );
    save_scheduler_runtime(&runtime);
    Ok(())
}

pub fn complete_job(
    job_id: &str,
    status: JobStatus,
    error: Option<String>,
    now_ns: u64,
    retry_after_secs: Option<u64>,
) {
    let mut job = match sqlite::get_job(job_id).ok().flatten() {
        Some(job) => job,
        None => return,
    };
    let old_status = job.status.clone();
    let started_at_ns = job.started_at_ns;
    job.last_error = error.clone();
    job.attempts = job.attempts.saturating_add(1);
    let should_retry = matches!(status, JobStatus::Failed | JobStatus::TimedOut)
        && retry_after_secs.is_some()
        && job.attempts < job.max_attempts;
    let mut retry_at_ns = None;
    let mut retried = false;

    if should_retry {
        let retry_delay_secs = retry_after_secs.unwrap_or_default();
        let scheduled_for_ns =
            now_ns.saturating_add(retry_delay_secs.saturating_mul(1_000_000_000));
        job.status = JobStatus::Pending;
        job.scheduled_for_ns = scheduled_for_ns;
        job.started_at_ns = None;
        job.finished_at_ns = None;
        let _ = sqlite::upsert_job(&job);
        retry_at_ns = Some(scheduled_for_ns);
        retried = true;
    } else {
        job.status = status.clone();
        job.finished_at_ns = Some(now_ns);
        let _ = sqlite::upsert_job(&job);
    }

    let cfg =
        get_task_config(&job.kind).unwrap_or_else(|| TaskScheduleConfig::default_for(&job.kind));
    let mut task_runtime = get_task_runtime(&job.kind);
    task_runtime.last_started_ns = started_at_ns;
    task_runtime.last_finished_ns = Some(now_ns);
    task_runtime.last_error = error.clone();

    if status == JobStatus::Succeeded {
        task_runtime.consecutive_failures = 0;
        task_runtime.backoff_until_ns = None;
    } else if retried {
        task_runtime.consecutive_failures = task_runtime.consecutive_failures.saturating_add(1);
        task_runtime.backoff_until_ns = retry_at_ns;
        task_runtime.pending_job_id = Some(job.id.clone());
    } else if matches!(status, JobStatus::Failed | JobStatus::TimedOut) {
        task_runtime.consecutive_failures = task_runtime.consecutive_failures.saturating_add(1);
        let capped = retry_after_secs.unwrap_or_else(|| {
            let exponent = task_runtime.consecutive_failures.min(20) as u32;
            let base_delay = 1u64 << exponent;
            base_delay.min(cfg.max_backoff_secs.max(1))
        });
        task_runtime.backoff_until_ns = now_ns.checked_add(capped.saturating_mul(1_000_000_000));
    }

    if !retried
        && task_runtime
            .pending_job_id
            .as_ref()
            .is_some_and(|id| id == job_id)
    {
        task_runtime.pending_job_id = None;
    }
    save_task_runtime(&job.kind, &task_runtime);

    let mut runtime = scheduler_runtime();
    if runtime
        .active_mutating_lease
        .as_ref()
        .is_some_and(|lease| lease.job_id == job_id)
    {
        runtime.active_mutating_lease = None;
        log!(
            SchedulerStorageLogPriority::Info,
            "scheduler_lease_released job_id={}",
            job_id
        );
        save_scheduler_runtime(&runtime);
    }

    log!(
        SchedulerStorageLogPriority::Info,
        "scheduler_job_complete job_id={} from={:?} to={:?} attempts={} max_attempts={} retried={} retry_at_ns={:?} error={:?}",
        job_id,
        old_status,
        job.status,
        job.attempts,
        job.max_attempts,
        retried,
        retry_at_ns,
        error
    );
}

pub fn recover_stale_lease(now_ns: u64) {
    let expired_job_id = scheduler_runtime()
        .active_mutating_lease
        .filter(|lease| lease.expires_at_ns <= now_ns)
        .map(|lease| lease.job_id);
    if let Some(job_id) = expired_job_id {
        log!(
            SchedulerStorageLogPriority::Warn,
            "scheduler_recover_stale_lease job_id={}",
            job_id
        );
        complete_job(
            &job_id,
            JobStatus::TimedOut,
            Some("mutating lease expired".to_string()),
            now_ns,
            None,
        );
    }
}

pub fn list_recent_jobs(limit: usize) -> Vec<ScheduledJob> {
    if limit == 0 {
        return Vec::new();
    }
    let keep = limit.min(MAX_RECENT_JOBS);
    sqlite::list_recent_jobs(keep).unwrap_or_default()
}

// ── Observability ─────────────────────────────────────────────────────────────

fn current_total_cycle_balance() -> u128 {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::canister_cycle_balance();

    #[cfg(not(target_arch = "wasm32"))]
    {
        runtime_u128(HOST_TOTAL_CYCLES_OVERRIDE_KEY).unwrap_or_default()
    }
}

fn current_liquid_cycle_balance(total_cycles: u128) -> u128 {
    #[cfg(target_arch = "wasm32")]
    return ic_cdk::api::canister_liquid_cycle_balance().min(total_cycles);

    #[cfg(not(target_arch = "wasm32"))]
    {
        runtime_u128(HOST_LIQUID_CYCLES_OVERRIDE_KEY)
            .unwrap_or(total_cycles)
            .min(total_cycles)
    }
}

fn runtime_u128(key: &str) -> Option<u128> {
    sqlite::get_runtime_scalar(key)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u128>().ok())
}

fn load_cycle_balance_samples() -> Vec<CycleBalanceSample> {
    sqlite::get_runtime_scalar(CYCLE_BALANCE_SAMPLES_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cycle_balance_samples(samples: &[CycleBalanceSample]) {
    if let Ok(json) = serde_json::to_string(samples) {
        let _ = sqlite::set_runtime_scalar(CYCLE_BALANCE_SAMPLES_KEY, &json);
    }
}

fn push_cycle_balance_sample(
    now_ns: u64,
    total_cycles: u128,
    liquid_cycles: u128,
) -> Vec<CycleBalanceSample> {
    let mut samples = load_cycle_balance_samples();
    let sample = CycleBalanceSample {
        captured_at_ns: now_ns,
        total_cycles,
        liquid_cycles,
    };

    if let Some(last) = samples.last_mut() {
        if last.captured_at_ns == now_ns {
            *last = sample;
        } else {
            samples.push(sample);
        }
    } else {
        samples.push(sample);
    }

    let cutoff_ns = now_ns.saturating_sub(CYCLES_BURN_MOVING_WINDOW_NS);
    samples.retain(|entry| entry.captured_at_ns >= cutoff_ns);
    if samples.len() > CYCLES_BURN_MAX_SAMPLES {
        let drop_count = samples.len() - CYCLES_BURN_MAX_SAMPLES;
        samples.drain(0..drop_count);
    }
    save_cycle_balance_samples(&samples);
    samples
}

fn round_f64_to_u128(value: f64) -> Option<u128> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    if value >= u128::MAX as f64 {
        return Some(u128::MAX);
    }
    Some(value.round() as u128)
}

fn cycles_to_usd_estimate(cycles: u128) -> f64 {
    (cycles as f64 / 1_000_000_000_000f64) * CYCLES_USD_PER_TRILLION_ESTIMATE
}

fn calculate_liquid_burn_cycles_per_sec(samples: &[CycleBalanceSample]) -> Option<f64> {
    let first = samples.first()?;
    let last = samples.last()?;
    if last.captured_at_ns <= first.captured_at_ns {
        return None;
    }

    let burned_cycles = samples.windows(2).fold(0u128, |acc, pair| {
        let prev = &pair[0];
        let next = &pair[1];
        if next.captured_at_ns <= prev.captured_at_ns {
            return acc;
        }
        acc.saturating_add(prev.liquid_cycles.saturating_sub(next.liquid_cycles))
    });

    if burned_cycles == 0 {
        return None;
    }

    let elapsed_secs = (last.captured_at_ns.saturating_sub(first.captured_at_ns)) as f64 / 1e9f64;
    if elapsed_secs <= 0.0 {
        return None;
    }
    Some(burned_cycles as f64 / elapsed_secs)
}

fn derive_cycle_telemetry(
    now_ns: u64,
    total_cycles: u128,
    liquid_cycles: u128,
    samples: &[CycleBalanceSample],
) -> CycleTelemetry {
    let freezing_threshold_cycles = total_cycles.saturating_sub(liquid_cycles);
    let window_duration_seconds = samples
        .first()
        .zip(samples.last())
        .map(|(first, last)| {
            last.captured_at_ns
                .saturating_sub(first.captured_at_ns)
                .saturating_div(1_000_000_000)
        })
        .unwrap_or_default();

    let burn_per_sec = calculate_liquid_burn_cycles_per_sec(samples);
    let burn_per_hour = burn_per_sec.and_then(|rate| round_f64_to_u128(rate * 3_600f64));
    let burn_per_day = burn_per_sec.and_then(|rate| round_f64_to_u128(rate * 86_400f64));

    let estimated_seconds_until_freezing_threshold = burn_per_sec.and_then(|rate| {
        if rate <= 0.0 {
            return None;
        }
        let estimate = (liquid_cycles as f64 / rate).floor();
        if !estimate.is_finite() || estimate < 0.0 || estimate > u64::MAX as f64 {
            return None;
        }
        Some(estimate as u64)
    });
    let estimated_freeze_time_ns = estimated_seconds_until_freezing_threshold.and_then(|seconds| {
        seconds
            .checked_mul(1_000_000_000)
            .and_then(|delta_ns| now_ns.checked_add(delta_ns))
    });

    CycleTelemetry {
        total_cycles,
        liquid_cycles,
        freezing_threshold_cycles,
        moving_window_seconds: timing::CYCLES_BURN_MOVING_WINDOW_SECS,
        window_duration_seconds,
        window_sample_count: u32::try_from(samples.len()).unwrap_or(u32::MAX),
        burn_rate_cycles_per_hour: burn_per_hour,
        burn_rate_cycles_per_day: burn_per_day,
        burn_rate_usd_per_hour: burn_per_hour.map(cycles_to_usd_estimate),
        burn_rate_usd_per_day: burn_per_day.map(cycles_to_usd_estimate),
        estimated_seconds_until_freezing_threshold,
        estimated_freeze_time_ns,
        usd_per_trillion_cycles: CYCLES_USD_PER_TRILLION_ESTIMATE,
    }
}

fn load_storage_growth_samples() -> Vec<StorageGrowthSample> {
    sqlite::get_runtime_scalar(STORAGE_GROWTH_SAMPLES_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_storage_growth_samples(samples: &[StorageGrowthSample]) {
    if let Ok(json) = serde_json::to_string(samples) {
        let _ = sqlite::set_runtime_scalar(STORAGE_GROWTH_SAMPLES_KEY, &json);
    }
}

fn push_storage_growth_sample(now_ns: u64, tracked_entries: u64) -> Vec<StorageGrowthSample> {
    let mut samples = load_storage_growth_samples();
    let sample = StorageGrowthSample {
        captured_at_ns: now_ns,
        tracked_entries,
    };

    if let Some(last) = samples.last_mut() {
        if last.captured_at_ns == now_ns {
            *last = sample;
        } else {
            samples.push(sample);
        }
    } else {
        samples.push(sample);
    }

    let cutoff_ns = now_ns.saturating_sub(STORAGE_GROWTH_TREND_WINDOW_NS);
    samples.retain(|entry| entry.captured_at_ns >= cutoff_ns);
    if samples.len() > STORAGE_GROWTH_MAX_SAMPLES {
        let drop_count = samples.len() - STORAGE_GROWTH_MAX_SAMPLES;
        samples.drain(0..drop_count);
    }
    save_storage_growth_samples(&samples);
    samples
}

fn calculate_tracked_entries_delta_per_hour(samples: &[StorageGrowthSample]) -> Option<i64> {
    let first = samples.first()?;
    let last = samples.last()?;
    if last.captured_at_ns <= first.captured_at_ns {
        return None;
    }

    let delta = last.tracked_entries as f64 - first.tracked_entries as f64;
    let elapsed_secs = (last.captured_at_ns.saturating_sub(first.captured_at_ns)) as f64 / 1e9f64;
    if elapsed_secs <= 0.0 {
        return None;
    }

    let per_hour = (delta / elapsed_secs) * 3_600f64;
    if !per_hour.is_finite() {
        return None;
    }

    if per_hour >= i64::MAX as f64 {
        return Some(i64::MAX);
    }
    if per_hour <= i64::MIN as f64 {
        return Some(i64::MIN);
    }
    Some(per_hour.round() as i64)
}

fn utilization_percent(entries: u64, limit: u64) -> u8 {
    if limit == 0 {
        return 0;
    }
    let numerator = entries.saturating_mul(100);
    let rounded = numerator
        .saturating_add(limit.saturating_sub(1))
        .saturating_div(limit)
        .min(100);
    u8::try_from(rounded).unwrap_or(100)
}

fn pressure_level_for_percent(max_utilization_percent: u8) -> StoragePressureLevel {
    if max_utilization_percent >= STORAGE_PRESSURE_CRITICAL_PERCENT {
        StoragePressureLevel::Critical
    } else if max_utilization_percent >= STORAGE_PRESSURE_HIGH_PERCENT {
        StoragePressureLevel::High
    } else if max_utilization_percent >= STORAGE_PRESSURE_ELEVATED_PERCENT {
        StoragePressureLevel::Elevated
    } else {
        StoragePressureLevel::Normal
    }
}

fn storage_growth_metrics(captured_at_ns: u64) -> StorageGrowthMetrics {
    #[cfg(target_arch = "wasm32")]
    let heap_memory_mb = {
        let pages = core::arch::wasm32::memory_size(0) as u64;
        pages as f64 * 65536.0 / 1_048_576.0
    };
    #[cfg(not(target_arch = "wasm32"))]
    let heap_memory_mb = 0.0_f64;

    #[cfg(target_arch = "wasm32")]
    let stable_memory_mb = {
        let pages = ic_cdk::api::stable_size();
        pages as f64 * 65536.0 / 1_048_576.0
    };
    #[cfg(not(target_arch = "wasm32"))]
    let stable_memory_mb = 0.0_f64;

    // Use sqlite table_count for entry counts
    let transition_map_entries = sqlite::table_count("transitions").unwrap_or(0);
    let turn_map_entries = sqlite::table_count("turns").unwrap_or(0);
    let tool_map_entries = sqlite::table_count("tool_calls").unwrap_or(0);
    let job_map_entries = sqlite::table_count("jobs").unwrap_or(0);
    let inbox_map_entries = sqlite::table_count("inbox").unwrap_or(0);
    let outbox_map_entries = sqlite::table_count("outbox").unwrap_or(0);
    let session_summary_entries = sqlite::table_count("session_summaries").unwrap_or(0);
    let turn_window_summary_entries = sqlite::table_count("turn_window_summaries").unwrap_or(0);
    let memory_rollup_entries = sqlite::table_count("memory_rollups").unwrap_or(0);
    let memory_fact_entries = sqlite::table_count("memory_facts").unwrap_or(0);
    let runtime_map_entries = sqlite::table_count("runtime_scalars").unwrap_or(0);
    // These are no longer separate maps — use 0 as placeholders
    let job_queue_map_entries = 0u64;
    let dedupe_map_entries = 0u64;
    let inbox_pending_queue_entries = sqlite::count_inbox_by_status("Pending").unwrap_or(0) as u64;
    let inbox_staged_queue_entries = sqlite::count_inbox_by_status("Staged").unwrap_or(0) as u64;

    let session_summary_limit = u64::try_from(MAX_SESSION_SUMMARIES).unwrap_or(u64::MAX);
    let turn_window_summary_limit = u64::try_from(MAX_TURN_WINDOW_SUMMARIES).unwrap_or(u64::MAX);
    let memory_rollup_limit = u64::try_from(MAX_MEMORY_ROLLUPS).unwrap_or(u64::MAX);
    let memory_fact_limit = u64::try_from(MAX_MEMORY_FACTS).unwrap_or(u64::MAX);

    let tracked_entry_count = runtime_map_entries
        .saturating_add(transition_map_entries)
        .saturating_add(turn_map_entries)
        .saturating_add(tool_map_entries)
        .saturating_add(job_map_entries)
        .saturating_add(inbox_map_entries)
        .saturating_add(outbox_map_entries)
        .saturating_add(session_summary_entries)
        .saturating_add(turn_window_summary_entries)
        .saturating_add(memory_rollup_entries)
        .saturating_add(memory_fact_entries);
    let growth_samples = push_storage_growth_sample(captured_at_ns, tracked_entry_count);
    let tracked_entries_delta_per_hour = calculate_tracked_entries_delta_per_hour(&growth_samples);
    let trend_window_seconds = growth_samples
        .first()
        .zip(growth_samples.last())
        .map(|(first, last)| {
            last.captured_at_ns
                .saturating_sub(first.captured_at_ns)
                .saturating_div(1_000_000_000)
        })
        .unwrap_or_default();
    let trend_sample_count = u32::try_from(growth_samples.len()).unwrap_or(u32::MAX);

    let session_summary_utilization_percent =
        utilization_percent(session_summary_entries, session_summary_limit);
    let turn_window_summary_utilization_percent =
        utilization_percent(turn_window_summary_entries, turn_window_summary_limit);
    let memory_rollup_utilization_percent =
        utilization_percent(memory_rollup_entries, memory_rollup_limit);
    let memory_fact_utilization_percent =
        utilization_percent(memory_fact_entries, memory_fact_limit);

    let max_utilization_percent = [
        session_summary_utilization_percent,
        turn_window_summary_utilization_percent,
        memory_rollup_utilization_percent,
        memory_fact_utilization_percent,
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    let pressure_level = pressure_level_for_percent(max_utilization_percent);

    let mut pressure_warnings = Vec::new();
    if session_summary_utilization_percent >= STORAGE_PRESSURE_HIGH_PERCENT {
        pressure_warnings.push(format!(
            "session summaries at {}% capacity ({}/{})",
            session_summary_utilization_percent, session_summary_entries, session_summary_limit
        ));
    }
    if turn_window_summary_utilization_percent >= STORAGE_PRESSURE_HIGH_PERCENT {
        pressure_warnings.push(format!(
            "turn window summaries at {}% capacity ({}/{})",
            turn_window_summary_utilization_percent,
            turn_window_summary_entries,
            turn_window_summary_limit
        ));
    }
    if memory_rollup_utilization_percent >= STORAGE_PRESSURE_HIGH_PERCENT {
        pressure_warnings.push(format!(
            "memory rollups at {}% capacity ({}/{})",
            memory_rollup_utilization_percent, memory_rollup_entries, memory_rollup_limit
        ));
    }
    if memory_fact_utilization_percent >= STORAGE_PRESSURE_HIGH_PERCENT {
        pressure_warnings.push(format!(
            "memory facts at {}% capacity ({}/{})",
            memory_fact_utilization_percent, memory_fact_entries, memory_fact_limit
        ));
    }
    if tracked_entries_delta_per_hour
        .map(|delta| delta >= STORAGE_GROWTH_WARNING_ENTRIES_PER_HOUR)
        .unwrap_or(false)
    {
        pressure_warnings.push(format!(
            "tracked entries growing quickly ({} entries/hour)",
            tracked_entries_delta_per_hour.unwrap_or_default()
        ));
    }
    let near_limit = max_utilization_percent >= STORAGE_PRESSURE_HIGH_PERCENT;

    let retention_runtime = retention_maintenance_runtime();
    let retention_config = retention_config();

    StorageGrowthMetrics {
        runtime_map_entries,
        transition_map_entries,
        turn_map_entries,
        tool_map_entries,
        job_map_entries,
        job_queue_map_entries,
        dedupe_map_entries,
        inbox_map_entries,
        inbox_pending_queue_entries,
        inbox_staged_queue_entries,
        outbox_map_entries,
        session_summary_entries,
        session_summary_limit,
        turn_window_summary_entries,
        turn_window_summary_limit,
        memory_rollup_entries,
        memory_rollup_limit,
        memory_fact_entries,
        memory_fact_limit,
        session_summary_utilization_percent,
        turn_window_summary_utilization_percent,
        memory_rollup_utilization_percent,
        memory_fact_utilization_percent,
        memory_fact_retention_max_age_secs: retention_config.memory_facts_max_age_secs,
        memory_fact_prune_batch_size: retention_config.memory_facts_prune_batch_size,
        last_deleted_memory_facts: retention_runtime.last_deleted_memory_facts,
        near_limit,
        pressure_level,
        pressure_warnings,
        tracked_entry_count,
        tracked_entries_delta_per_hour,
        trend_window_seconds,
        trend_sample_count,
        retention_progress_percent: retention_runtime.retention_progress_percent,
        summarization_progress_percent: retention_runtime.summarization_progress_percent,
        heap_memory_mb,
        stable_memory_mb,
    }
}

pub fn observability_snapshot(limit: usize) -> ObservabilitySnapshot {
    let bounded_limit = if limit == 0 {
        DEFAULT_OBSERVABILITY_LIMIT
    } else {
        limit.min(MAX_OBSERVABILITY_LIMIT)
    };
    let captured_at_ns = now_ns();
    let total_cycles = current_total_cycle_balance();
    let liquid_cycles = current_liquid_cycle_balance(total_cycles);
    let cycle_samples = push_cycle_balance_sample(captured_at_ns, total_cycles, liquid_cycles);
    let cycles =
        derive_cycle_telemetry(captured_at_ns, total_cycles, liquid_cycles, &cycle_samples);
    let mut conversation_summaries = list_conversation_summaries();
    conversation_summaries.truncate(bounded_limit);
    let session_summaries = list_session_summaries(bounded_limit);
    let turn_window_summaries = list_turn_window_summaries(bounded_limit);
    let memory_rollups = list_memory_rollups(bounded_limit);

    ObservabilitySnapshot {
        captured_at_ns,
        runtime: snapshot_to_view(),
        scheduler: scheduler_runtime_view(),
        storage_growth: storage_growth_metrics(captured_at_ns),
        inbox_stats: inbox_stats(),
        inbox_messages: list_inbox_messages(bounded_limit),
        outbox_stats: outbox_stats(),
        outbox_messages: list_outbox_messages(bounded_limit),
        prompt_layers: list_prompt_layers(),
        conversation_summaries,
        session_summaries,
        turn_window_summaries,
        memory_rollups,
        cycles,
        recent_turns: list_turns(bounded_limit),
        recent_transitions: list_recent_transitions(bounded_limit),
        recent_jobs: list_recent_jobs(bounded_limit),
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────────

#[cfg(test)]
pub fn save_job_for_tests(job: ScheduledJob) {
    let _ = sqlite::upsert_job(&job);
}

#[cfg(test)]
pub fn insert_dedupe_for_tests(_dedupe_key: String, _job_id: String) {
    // Dedup is handled inline by the jobs table in SQLite - no separate map needed.
    // This function exists only for test compatibility.
}
