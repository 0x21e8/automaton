//! SQLite storage adapter used during the StableBTreeMap -> SQL migration.
//!
//! Phase 1 keeps stable maps as source of truth and dual-writes historical
//! collections into SQLite for parity checks and backfill validation.

use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, ActiveExposure, AutonomyPolicy, ConversationEntry,
    ConversationLog, DecisionRecord, ExposureReconciliationStatus, GoalRecord, InboxMessage,
    JournalDealClaim, JournalEntry, MemoryFact, OutboxMessage, PendingStrategyDiscoveryJob,
    PendingStrategyExecution, PlanRecord, ReflectionMemoryRecord, RuntimeSnapshot, ScheduledJob,
    SchedulerRuntime, SkillRecord, StrategyDiscoveryCallbackRecord, StrategyOutcomeStats,
    StrategyQuarantine, StrategyTemplate, StrategyTemplateKey, SurvivalOperationClass, TaskKind,
    TaskScheduleConfig, TaskScheduleRuntime, TemplateActivationState, ToolCallRecord,
    TransitionLogRecord, TurnRecord,
};
use crate::features::cycle_topup::TopUpStage;
#[cfg(target_arch = "wasm32")]
use ic_rusqlite::rusqlite::params;
#[cfg(target_arch = "wasm32")]
use ic_rusqlite::rusqlite::types::ValueRef as SqlValueRef;
#[cfg(not(target_arch = "wasm32"))]
use rusqlite::params;
#[cfg(not(target_arch = "wasm32"))]
use rusqlite::types::ValueRef as SqlValueRef;
use serde_json::Value;
use serde_json::{Map as JsonMap, Number as JsonNumber};
use std::cell::RefCell;
use std::collections::BTreeMap;

const CURRENT_SCHEMA_VERSION: i64 = 10;

const MIGRATION_001_BASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ns INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS transitions (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    event TEXT NOT NULL,
    error TEXT,
    occurred_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_transitions_occurred_at ON transitions(occurred_at_ns);

CREATE TABLE IF NOT EXISTS turns (
    id TEXT PRIMARY KEY,
    created_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_turns_created_at ON turns(created_at_ns);

CREATE TABLE IF NOT EXISTS journal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id TEXT NOT NULL,
    timestamp_ns INTEGER NOT NULL,
    genesis INTEGER NOT NULL DEFAULT 0,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_journal_timestamp ON journal(timestamp_ns DESC, id DESC);

CREATE TABLE IF NOT EXISTS tool_calls (
    turn_id TEXT NOT NULL,
    call_index INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    success INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (turn_id, call_index)
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_name ON tool_calls(tool_name);

CREATE TABLE IF NOT EXISTS inbox (
    id TEXT PRIMARY KEY,
    seq INTEGER NOT NULL,
    posted_by TEXT NOT NULL,
    status TEXT NOT NULL,
    posted_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_inbox_status ON inbox(status);
CREATE INDEX IF NOT EXISTS idx_inbox_posted_at ON inbox(posted_at_ns);

CREATE TABLE IF NOT EXISTS outbox (
    id TEXT PRIMARY KEY,
    seq INTEGER NOT NULL,
    turn_id TEXT NOT NULL,
    created_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_outbox_created_at ON outbox(created_at_ns);

CREATE TABLE IF NOT EXISTS conversations (
    sender TEXT NOT NULL,
    entry_seq INTEGER NOT NULL,
    timestamp_ns INTEGER NOT NULL,
    turn_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (sender, entry_seq)
);
CREATE INDEX IF NOT EXISTS idx_conversations_sender_time ON conversations(sender, timestamp_ns);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    lane TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER NOT NULL,
    created_at_ns INTEGER NOT NULL,
    scheduled_for_ns INTEGER NOT NULL DEFAULT 0,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_status_created ON jobs(status, created_at_ns);
CREATE INDEX IF NOT EXISTS idx_jobs_pending_lane ON jobs(status, lane, scheduled_for_ns, priority);

CREATE TABLE IF NOT EXISTS memory_facts (
    key TEXT PRIMARY KEY,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_facts_updated_at ON memory_facts(updated_at_ns);

CREATE TABLE IF NOT EXISTS skills (
    name TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strategy_templates (
    template_id TEXT PRIMARY KEY,
    protocol TEXT NOT NULL,
    primitive TEXT NOT NULL,
    chain_id INTEGER NOT NULL,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strategy_templates_updated ON strategy_templates(updated_at_ns);

CREATE TABLE IF NOT EXISTS abi_artifacts (
    artifact_id TEXT PRIMARY KEY,
    protocol TEXT NOT NULL,
    role TEXT NOT NULL,
    chain_id INTEGER NOT NULL,
    created_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_abi_artifacts_created ON abi_artifacts(created_at_ns);

CREATE TABLE IF NOT EXISTS backfill_markers (
    marker_key TEXT PRIMARY KEY,
    completed_at_ns INTEGER NOT NULL
);
"#;

const MIGRATION_002_HOT_STATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS hot_runtime_snapshot (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hot_scheduler_runtime (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hot_task_configs (
    task_kind TEXT PRIMARY KEY,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hot_task_runtimes (
    task_kind TEXT PRIMARY KEY,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hot_topup_state (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hot_survival_operation_runtime (
    operation_key TEXT PRIMARY KEY,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
"#;

const MIGRATION_003_REMAINING_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS http_domain_allowlist (
    domain TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS prompt_layers (
    layer_id INTEGER PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS retention_runtime (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_summaries (
    date_key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS turn_window_summaries (
    date_key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_rollups (
    rollup_key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strategy_activations (
    version_key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strategy_revocations (
    version_key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strategy_kill_switches (
    key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strategy_outcome_stats (
    key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strategy_budgets (
    key TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS autonomy_tool_failures (
    tool_name TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runtime_scalars (
    key TEXT PRIMARY KEY,
    value_text TEXT NOT NULL
);
"#;

const MIGRATION_004_REFLECTION_MEMORY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reflection_memory (
    key TEXT PRIMARY KEY,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reflection_memory_updated_at ON reflection_memory(updated_at_ns);
"#;

const MIGRATION_005_AUTONOMY_RUNTIME_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS autonomy_policy (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS active_exposures (
    strategy_id TEXT PRIMARY KEY,
    protocol TEXT NOT NULL,
    chain_id INTEGER NOT NULL,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_active_exposures_protocol ON active_exposures(protocol);
CREATE INDEX IF NOT EXISTS idx_active_exposures_updated_at ON active_exposures(updated_at_ns);

CREATE TABLE IF NOT EXISTS strategy_quarantines (
    strategy_id TEXT PRIMARY KEY,
    quarantined_at_ns INTEGER NOT NULL,
    release_after_ns INTEGER,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strategy_quarantines_quarantined_at
    ON strategy_quarantines(quarantined_at_ns);

CREATE TABLE IF NOT EXISTS decision_records (
    turn_id TEXT PRIMARY KEY,
    timestamp_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_decision_records_timestamp ON decision_records(timestamp_ns);

CREATE TABLE IF NOT EXISTS exposure_reconciliation_status (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    last_attempted_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
"#;

const MIGRATION_006_GOALS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS goals (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL DEFAULT 'active',
    priority TEXT NOT NULL DEFAULT 'medium',
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_goals_status ON goals(status);
CREATE INDEX IF NOT EXISTS idx_goals_updated_at ON goals(updated_at_ns);
"#;

const MIGRATION_007_PLANS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plans (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL DEFAULT 'active',
    goal_id TEXT,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_plans_status ON plans(status);
CREATE INDEX IF NOT EXISTS idx_plans_updated_at ON plans(updated_at_ns);
 "#;

const MIGRATION_008_STRATEGY_DISCOVERY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS strategy_discovery_pending_jobs (
    job_id TEXT PRIMARY KEY,
    submitted_at_ns INTEGER NOT NULL,
    objective TEXT NOT NULL,
    watchlist_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strategy_discovery_pending_jobs_submitted
    ON strategy_discovery_pending_jobs(submitted_at_ns);

CREATE TABLE IF NOT EXISTS strategy_discovery_results (
    job_id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    accepted_at_ns INTEGER NOT NULL,
    validated_at_ns INTEGER,
    completed_at_ns INTEGER NOT NULL,
    objective TEXT NOT NULL,
    watchlist_hash TEXT NOT NULL,
    result_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strategy_discovery_results_status_validated
    ON strategy_discovery_results(status, validated_at_ns);
CREATE INDEX IF NOT EXISTS idx_strategy_discovery_results_accepted
    ON strategy_discovery_results(accepted_at_ns);

CREATE TABLE IF NOT EXISTS strategy_discovery_completed_callback_jobs (
    job_id TEXT PRIMARY KEY,
    accepted_at_ns INTEGER NOT NULL,
    result_hash TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strategy_discovery_completed_callback_jobs_accepted
    ON strategy_discovery_completed_callback_jobs(accepted_at_ns);
"#;

const MIGRATION_009_JOURNAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS journal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id TEXT NOT NULL,
    timestamp_ns INTEGER NOT NULL,
    genesis INTEGER NOT NULL DEFAULT 0,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_journal_timestamp ON journal(timestamp_ns DESC, id DESC);
"#;

const MIGRATION_010_PENDING_STRATEGY_EXECUTIONS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS pending_strategy_executions (
    execution_id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    next_check_at_ns INTEGER NOT NULL,
    created_at_ns INTEGER NOT NULL,
    updated_at_ns INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pending_strategy_executions_due
    ON pending_strategy_executions(state, next_check_at_ns, created_at_ns);
"#;

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use super::{
        CURRENT_SCHEMA_VERSION, MIGRATION_001_BASE_SCHEMA, MIGRATION_002_HOT_STATE_SCHEMA,
        MIGRATION_003_REMAINING_SCHEMA, MIGRATION_004_REFLECTION_MEMORY_SCHEMA,
        MIGRATION_005_AUTONOMY_RUNTIME_SCHEMA, MIGRATION_006_GOALS_SCHEMA,
        MIGRATION_007_PLANS_SCHEMA, MIGRATION_008_STRATEGY_DISCOVERY_SCHEMA,
        MIGRATION_009_JOURNAL_SCHEMA, MIGRATION_010_PENDING_STRATEGY_EXECUTIONS_SCHEMA,
    };
    use rusqlite::{params, Connection};
    use std::cell::RefCell;

    thread_local! {
        static SQLITE: RefCell<Option<Connection>> = const { RefCell::new(None) };
    }

    pub type SqlResult<T> = Result<T, String>;

    pub fn init() -> SqlResult<()> {
        with_connection(|_conn| Ok(()))
    }

    pub fn close() -> SqlResult<()> {
        SQLITE.with(|slot| {
            *slot.borrow_mut() = None;
        });
        Ok(())
    }

    pub fn with_connection<T, F>(f: F) -> SqlResult<T>
    where
        F: FnOnce(&Connection) -> SqlResult<T>,
    {
        SQLITE.with(|slot| {
            if slot.borrow().is_none() {
                let conn = Connection::open_in_memory().map_err(|err| err.to_string())?;
                conn.pragma_update(None, "journal_mode", "WAL")
                    .map_err(|err| err.to_string())?;
                conn.pragma_update(None, "foreign_keys", "ON")
                    .map_err(|err| err.to_string())?;
                apply_migrations(&conn)?;
                *slot.borrow_mut() = Some(conn);
            }
            let borrow = slot.borrow();
            let conn = borrow
                .as_ref()
                .ok_or_else(|| "sqlite connection unavailable".to_string())?;
            f(conn)
        })
    }

    fn apply_migrations(conn: &Connection) -> SqlResult<()> {
        conn.execute_batch(MIGRATION_001_BASE_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_002_HOT_STATE_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_003_REMAINING_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_004_REFLECTION_MEMORY_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_005_AUTONOMY_RUNTIME_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_006_GOALS_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_007_PLANS_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_008_STRATEGY_DISCOVERY_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_009_JOURNAL_SCHEMA)
            .map_err(|err| err.to_string())?;
        conn.execute_batch(MIGRATION_010_PENDING_STRATEGY_EXECUTIONS_SCHEMA)
            .map_err(|err| err.to_string())?;
        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let now = crate::timing::current_time_ns() as i64;
        for v in (version + 1)..=CURRENT_SCHEMA_VERSION {
            conn.execute(
                "INSERT INTO schema_migrations(version, applied_at_ns) VALUES(?1, ?2)",
                params![v, now],
            )
            .map_err(|err| err.to_string())?;
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod backend {
    use super::{
        CURRENT_SCHEMA_VERSION, MIGRATION_001_BASE_SCHEMA, MIGRATION_002_HOT_STATE_SCHEMA,
        MIGRATION_003_REMAINING_SCHEMA, MIGRATION_004_REFLECTION_MEMORY_SCHEMA,
        MIGRATION_005_AUTONOMY_RUNTIME_SCHEMA, MIGRATION_006_GOALS_SCHEMA,
        MIGRATION_007_PLANS_SCHEMA, MIGRATION_008_STRATEGY_DISCOVERY_SCHEMA,
        MIGRATION_009_JOURNAL_SCHEMA, MIGRATION_010_PENDING_STRATEGY_EXECUTIONS_SCHEMA,
    };

    pub type SqlResult<T> = Result<T, String>;

    pub fn init() -> SqlResult<()> {
        ic_rusqlite::with_connection(|conn| {
            conn.execute_batch(MIGRATION_001_BASE_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_002_HOT_STATE_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_003_REMAINING_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_004_REFLECTION_MEMORY_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_005_AUTONOMY_RUNTIME_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_006_GOALS_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_007_PLANS_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_008_STRATEGY_DISCOVERY_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_009_JOURNAL_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_010_PENDING_STRATEGY_EXECUTIONS_SCHEMA)
                .map_err(|err| err.to_string())?;
            let version: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| row.get(0),
                )
                .map_err(|err| err.to_string())?;
            let now = crate::timing::current_time_ns() as i64;
            for v in (version + 1)..=CURRENT_SCHEMA_VERSION {
                conn.execute(
                    "INSERT INTO schema_migrations(version, applied_at_ns) VALUES(?1, ?2)",
                    [v, now],
                )
                .map_err(|err| err.to_string())?;
            }
            Ok::<(), String>(())
        })
    }

    pub fn close() -> SqlResult<()> {
        Ok(())
    }

    pub fn with_connection<T, F>(f: F) -> SqlResult<T>
    where
        F: FnOnce(&ic_rusqlite::rusqlite::Connection) -> SqlResult<T>,
    {
        ic_rusqlite::with_connection(|conn| f(&*conn))
    }
}

fn row_payload<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| err.to_string())
}

fn bounded_limit(limit: usize, fallback: usize, hard_cap: usize) -> usize {
    if limit == 0 {
        fallback.min(hard_cap)
    } else {
        limit.min(hard_cap)
    }
}

fn from_payload_json<T: serde::de::DeserializeOwned>(payload_json: String) -> Result<T, String> {
    serde_json::from_str::<T>(&payload_json).map_err(|err| err.to_string())
}

fn strategy_template_pk(key: &StrategyTemplateKey) -> String {
    format!(
        "{}:{}:{}:{}",
        key.protocol, key.primitive, key.chain_id, key.template_id
    )
}

fn abi_artifact_pk(key: &AbiArtifactKey) -> String {
    format!("{}:{}:{}", key.protocol, key.chain_id, key.role)
}

fn normalized_sql_query(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("query cannot be empty".to_string());
    }
    let without_trailing_semicolon = trimmed.trim_end_matches(';').trim();
    if without_trailing_semicolon.contains(';') {
        return Err("only a single SELECT statement is allowed".to_string());
    }

    let lowered = without_trailing_semicolon.to_ascii_lowercase();
    if !lowered.starts_with("select ") && lowered != "select" {
        return Err("only SELECT statements are allowed".to_string());
    }
    let padded = format!(" {lowered} ");
    for forbidden in [
        " insert ",
        " update ",
        " delete ",
        " create ",
        " drop ",
        " alter ",
        " pragma ",
        " attach ",
        " detach ",
        " vacuum ",
        " reindex ",
        " replace ",
        " begin ",
        " commit ",
        " rollback ",
    ] {
        if padded.contains(forbidden) {
            return Err("query contains forbidden SQL keyword".to_string());
        }
    }
    Ok(without_trailing_semicolon.to_string())
}

fn sqlite_value_to_json(value: SqlValueRef<'_>) -> Value {
    match value {
        SqlValueRef::Null => Value::Null,
        SqlValueRef::Integer(v) => Value::Number(JsonNumber::from(v)),
        SqlValueRef::Real(v) => JsonNumber::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        SqlValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).to_string()),
        SqlValueRef::Blob(v) => Value::String(format!("0x{}", hex::encode(v))),
    }
}

#[cfg(target_arch = "wasm32")]
fn sql_instruction_budget_exceeded(start_counter: u64, max_delta: u64) -> bool {
    ic_cdk::api::performance_counter(0).saturating_sub(start_counter) > max_delta
}

#[cfg(not(target_arch = "wasm32"))]
fn sql_instruction_budget_exceeded(_start_counter: u64, _max_delta: u64) -> bool {
    false
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SurvivalOperationRuntimeRecord {
    pub consecutive_failures: u32,
    pub backoff_until_ns: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct HotStateCache {
    runtime_snapshot: Option<RuntimeSnapshot>,
    scheduler_runtime: Option<SchedulerRuntime>,
    task_configs: BTreeMap<String, TaskScheduleConfig>,
    task_runtimes: BTreeMap<String, TaskScheduleRuntime>,
    topup_state: Option<TopUpStage>,
    survival_runtime: BTreeMap<String, SurvivalOperationRuntimeRecord>,
}

thread_local! {
    static HOT_STATE_CACHE: RefCell<HotStateCache> = RefCell::new(HotStateCache::default());
}

fn reset_hot_state_cache() {
    HOT_STATE_CACHE.with(|cache| {
        *cache.borrow_mut() = HotStateCache::default();
    });
}

fn task_kind_key(kind: &TaskKind) -> String {
    kind.as_str().to_string()
}

fn hydrate_hot_state_cache() -> Result<(), String> {
    backend::with_connection(|conn| {
        let runtime_snapshot = conn
            .query_row(
                "SELECT payload_json FROM hot_runtime_snapshot WHERE singleton_id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .map(from_payload_json)
            .transpose()?;

        let scheduler_runtime = conn
            .query_row(
                "SELECT payload_json FROM hot_scheduler_runtime WHERE singleton_id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .map(from_payload_json)
            .transpose()?;

        let mut task_configs = BTreeMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT task_kind, payload_json FROM hot_task_configs")
                .map_err(|err| err.to_string())?;
            let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
            while let Some(row) = rows.next().map_err(|err| err.to_string())? {
                let kind = row.get::<_, String>(0).map_err(|err| err.to_string())?;
                let payload_json = row.get::<_, String>(1).map_err(|err| err.to_string())?;
                let config: TaskScheduleConfig = from_payload_json(payload_json)?;
                task_configs.insert(kind, config);
            }
        }

        let mut task_runtimes = BTreeMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT task_kind, payload_json FROM hot_task_runtimes")
                .map_err(|err| err.to_string())?;
            let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
            while let Some(row) = rows.next().map_err(|err| err.to_string())? {
                let kind = row.get::<_, String>(0).map_err(|err| err.to_string())?;
                let payload_json = row.get::<_, String>(1).map_err(|err| err.to_string())?;
                let runtime: TaskScheduleRuntime = from_payload_json(payload_json)?;
                task_runtimes.insert(kind, runtime);
            }
        }

        let topup_state = conn
            .query_row(
                "SELECT payload_json FROM hot_topup_state WHERE singleton_id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .map(from_payload_json)
            .transpose()?;

        let mut survival_runtime = BTreeMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT operation_key, payload_json FROM hot_survival_operation_runtime")
                .map_err(|err| err.to_string())?;
            let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
            while let Some(row) = rows.next().map_err(|err| err.to_string())? {
                let operation_key = row.get::<_, String>(0).map_err(|err| err.to_string())?;
                let payload_json = row.get::<_, String>(1).map_err(|err| err.to_string())?;
                let runtime: SurvivalOperationRuntimeRecord = from_payload_json(payload_json)?;
                survival_runtime.insert(operation_key, runtime);
            }
        }

        HOT_STATE_CACHE.with(|cache| {
            *cache.borrow_mut() = HotStateCache {
                runtime_snapshot,
                scheduler_runtime,
                task_configs,
                task_runtimes,
                topup_state,
                survival_runtime,
            };
        });
        Ok(())
    })
}

pub fn init_storage() -> Result<(), String> {
    backend::init()?;
    hydrate_hot_state_cache()
}

pub fn close_storage() -> Result<(), String> {
    reset_hot_state_cache();
    backend::close()
}

pub fn reopen_storage() -> Result<(), String> {
    close_storage()?;
    init_storage()
}

pub fn schema_version() -> Result<u64, String> {
    backend::with_connection(|conn| {
        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(u64::try_from(version).unwrap_or(0))
    })
}

pub fn read_runtime_snapshot() -> Result<Option<RuntimeSnapshot>, String> {
    HOT_STATE_CACHE.with(|cache| Ok(cache.borrow().runtime_snapshot.clone()))
}

pub fn write_runtime_snapshot(snapshot: &RuntimeSnapshot) -> Result<(), String> {
    let payload_json = row_payload(snapshot)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO hot_runtime_snapshot(singleton_id, updated_at_ns, payload_json)
             VALUES(1, ?1, ?2)",
            (crate::timing::current_time_ns() as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;
        HOT_STATE_CACHE.with(|cache| {
            cache.borrow_mut().runtime_snapshot = Some(snapshot.clone());
        });
        Ok(())
    })
}

pub fn read_scheduler_runtime() -> Result<Option<SchedulerRuntime>, String> {
    HOT_STATE_CACHE.with(|cache| Ok(cache.borrow().scheduler_runtime.clone()))
}

pub fn write_scheduler_runtime(runtime: &SchedulerRuntime) -> Result<(), String> {
    let payload_json = row_payload(runtime)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO hot_scheduler_runtime(singleton_id, updated_at_ns, payload_json)
             VALUES(1, ?1, ?2)",
            (crate::timing::current_time_ns() as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;
        HOT_STATE_CACHE.with(|cache| {
            cache.borrow_mut().scheduler_runtime = Some(runtime.clone());
        });
        Ok(())
    })
}

pub fn list_task_configs() -> Result<Vec<(TaskKind, TaskScheduleConfig)>, String> {
    HOT_STATE_CACHE.with(|cache| {
        let mut entries = cache
            .borrow()
            .task_configs
            .iter()
            .filter_map(|(kind_key, config)| {
                kind_key
                    .parse()
                    .ok()
                    .map(|kind: TaskKind| (kind, config.clone()))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(kind, cfg)| (cfg.priority, kind.as_str().to_string()));
        Ok(entries)
    })
}

pub fn read_task_config(kind: &TaskKind) -> Result<Option<TaskScheduleConfig>, String> {
    let key = task_kind_key(kind);
    HOT_STATE_CACHE.with(|cache| Ok(cache.borrow().task_configs.get(&key).cloned()))
}

pub fn write_task_config(config: &TaskScheduleConfig) -> Result<(), String> {
    let task_key = task_kind_key(&config.kind);
    let payload_json = row_payload(config)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO hot_task_configs(task_kind, updated_at_ns, payload_json)
             VALUES(?1, ?2, ?3)",
            (
                task_key.as_str(),
                crate::timing::current_time_ns() as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        HOT_STATE_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .task_configs
                .insert(task_key, config.clone());
        });
        Ok(())
    })
}

pub fn read_task_runtime(kind: &TaskKind) -> Result<Option<TaskScheduleRuntime>, String> {
    let key = task_kind_key(kind);
    HOT_STATE_CACHE.with(|cache| Ok(cache.borrow().task_runtimes.get(&key).cloned()))
}

pub fn write_task_runtime(kind: &TaskKind, runtime: &TaskScheduleRuntime) -> Result<(), String> {
    let task_key = task_kind_key(kind);
    let payload_json = row_payload(runtime)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO hot_task_runtimes(task_kind, updated_at_ns, payload_json)
             VALUES(?1, ?2, ?3)",
            (
                task_key.as_str(),
                crate::timing::current_time_ns() as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        HOT_STATE_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .task_runtimes
                .insert(task_key, runtime.clone());
        });
        Ok(())
    })
}

pub fn read_topup_state() -> Result<Option<TopUpStage>, String> {
    HOT_STATE_CACHE.with(|cache| Ok(cache.borrow().topup_state.clone()))
}

pub fn write_topup_state(state: &TopUpStage) -> Result<(), String> {
    let payload_json = row_payload(state)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO hot_topup_state(singleton_id, updated_at_ns, payload_json)
             VALUES(1, ?1, ?2)",
            (crate::timing::current_time_ns() as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;
        HOT_STATE_CACHE.with(|cache| {
            cache.borrow_mut().topup_state = Some(state.clone());
        });
        Ok(())
    })
}

pub fn clear_topup_state() -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM hot_topup_state WHERE singleton_id = 1", [])
            .map_err(|err| err.to_string())?;
        HOT_STATE_CACHE.with(|cache| {
            cache.borrow_mut().topup_state = None;
        });
        Ok(())
    })
}

pub fn read_survival_operation_runtime(
    operation: &SurvivalOperationClass,
) -> Result<SurvivalOperationRuntimeRecord, String> {
    let key = operation.as_str().to_string();
    HOT_STATE_CACHE.with(|cache| {
        Ok(cache
            .borrow()
            .survival_runtime
            .get(&key)
            .cloned()
            .unwrap_or_default())
    })
}

pub fn write_survival_operation_runtime(
    operation: &SurvivalOperationClass,
    runtime: &SurvivalOperationRuntimeRecord,
) -> Result<(), String> {
    let operation_key = operation.as_str().to_string();
    let payload_json = row_payload(runtime)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO hot_survival_operation_runtime(operation_key, updated_at_ns, payload_json)
             VALUES(?1, ?2, ?3)",
            (
                operation_key.as_str(),
                crate::timing::current_time_ns() as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        HOT_STATE_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .survival_runtime
                .insert(operation_key, runtime.clone());
        });
        Ok(())
    })
}

pub fn is_backfill_done(marker: &str) -> Result<bool, String> {
    let key = marker.trim();
    if key.is_empty() {
        return Ok(false);
    }
    backend::with_connection(|conn| {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM backfill_markers WHERE marker_key = ?1",
                [key],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(exists > 0)
    })
}

pub fn mark_backfill_done(marker: &str) -> Result<(), String> {
    let key = marker.trim();
    if key.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO backfill_markers(marker_key, completed_at_ns) VALUES(?1, ?2)",
            (key, crate::timing::current_time_ns() as i64),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_transition(record: &TransitionLogRecord) -> Result<(), String> {
    let payload_json = row_payload(record)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO transitions(id, turn_id, from_state, to_state, event, error, occurred_at_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                &record.id,
                &record.turn_id,
                format!("{:?}", record.from_state),
                format!("{:?}", record.to_state),
                &record.event,
                &record.error,
                record.occurred_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_turn(record: &TurnRecord) -> Result<(), String> {
    let payload_json = row_payload(record)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO turns(id, created_at_ns, payload_json) VALUES(?1, ?2, ?3)",
            (&record.id, record.created_at_ns as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn replace_tool_calls(turn_id: &str, tool_calls: &[ToolCallRecord]) -> Result<(), String> {
    let turn = turn_id.trim();
    if turn.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM tool_calls WHERE turn_id = ?1", [turn])
            .map_err(|err| err.to_string())?;
        for (index, tool_call) in tool_calls.iter().enumerate() {
            let payload_json = row_payload(tool_call)?;
            let success = if tool_call.success { 1_i64 } else { 0_i64 };
            conn.execute(
                "INSERT INTO tool_calls(turn_id, call_index, tool_name, success, payload_json) VALUES(?1, ?2, ?3, ?4, ?5)",
                (turn, i64::try_from(index).unwrap_or(i64::MAX), &tool_call.tool, success, payload_json),
            )
            .map_err(|err| err.to_string())?;
        }
        Ok(())
    })
}

pub fn upsert_inbox(message: &InboxMessage) -> Result<(), String> {
    let payload_json = row_payload(message)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO inbox(id, seq, posted_by, status, posted_at_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            (
                &message.id,
                message.seq as i64,
                &message.posted_by,
                format!("{:?}", message.status),
                message.posted_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_outbox(message: &OutboxMessage) -> Result<(), String> {
    let payload_json = row_payload(message)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO outbox(id, seq, turn_id, created_at_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            (
                &message.id,
                message.seq as i64,
                &message.turn_id,
                message.created_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn append_conversation(sender: &str, entry: &ConversationEntry) -> Result<(), String> {
    let normalized = sender.trim();
    if normalized.is_empty() {
        return Ok(());
    }
    let payload_json = row_payload(entry)?;
    backend::with_connection(|conn| {
        let next_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(entry_seq), 0) + 1 FROM conversations WHERE sender = ?1",
                [normalized],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        conn.execute(
            "INSERT INTO conversations(sender, entry_seq, timestamp_ns, turn_id, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            (
                normalized,
                next_seq,
                entry.timestamp_ns as i64,
                &entry.turn_id,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_job(job: &ScheduledJob) -> Result<(), String> {
    let payload_json = row_payload(job)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO jobs(id, kind, lane, status, priority, created_at_ns, scheduled_for_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                &job.id,
                format!("{:?}", job.kind),
                job.lane.as_str(),
                format!("{:?}", job.status),
                i64::from(job.priority),
                job.created_at_ns as i64,
                job.scheduled_for_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_memory_fact(fact: &MemoryFact) -> Result<(), String> {
    let payload_json = row_payload(fact)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO memory_facts(key, updated_at_ns, payload_json) VALUES(?1, ?2, ?3)",
            (&fact.key, fact.updated_at_ns as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_memory_fact(key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM memory_facts WHERE key = ?1", [trimmed])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_skill(skill: &SkillRecord) -> Result<(), String> {
    let payload_json = row_payload(skill)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO skills(name, enabled, payload_json) VALUES(?1, ?2, ?3)",
            (
                &skill.name,
                if skill.enabled { 1_i64 } else { 0_i64 },
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_skill(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM skills WHERE name = ?1", [trimmed])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// ── Goals ─────────────────────────────────────────────────────────────────────

pub fn upsert_goal(goal: &GoalRecord) -> Result<(), String> {
    let payload_json = row_payload(goal)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO goals(id, status, priority, updated_at_ns, payload_json) VALUES(?1, ?2, ?3, ?4, ?5)",
            (
                goal.id.trim(),
                goal.status.to_string(),
                goal.priority.as_str(),
                goal.updated_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn get_goal(id: &str) -> Result<Option<GoalRecord>, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM goals WHERE id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([trimmed], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn delete_goal(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM goals WHERE id = ?1", [trimmed])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_goals_by_status(status: &str, limit: usize) -> Result<Vec<GoalRecord>, String> {
    let keep = bounded_limit(limit, 50, 100);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM goals WHERE status = ?1 ORDER BY updated_at_ns DESC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([status, &(keep as i64).to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_all_goals(limit: usize) -> Result<Vec<GoalRecord>, String> {
    let keep = bounded_limit(limit, 50, 100);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM goals ORDER BY
                 CASE status WHEN 'active' THEN 0 WHEN 'completed' THEN 1 ELSE 2 END,
                 updated_at_ns DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_goals() -> Result<usize, String> {
    backend::with_connection(|conn| {
        let count: i64 = conn
            .query_row("SELECT COUNT(1) FROM goals", [], |row| row.get(0))
            .map_err(|err| err.to_string())?;
        Ok(count as usize)
    })
}

pub fn count_goals_by_status(status: &str) -> Result<usize, String> {
    backend::with_connection(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM goals WHERE status = ?1",
                [status],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(count as usize)
    })
}

pub fn delete_oldest_completed_or_abandoned_goal() -> Result<bool, String> {
    backend::with_connection(|conn| {
        let deleted = conn
            .execute(
                "DELETE FROM goals WHERE id = (
                    SELECT id FROM goals
                    WHERE status IN ('completed', 'abandoned')
                    ORDER BY updated_at_ns ASC
                    LIMIT 1
                )",
                [],
            )
            .map_err(|err| err.to_string())?;
        Ok(deleted > 0)
    })
}

// ── Plans ─────────────────────────────────────────────────────────────────────

pub fn upsert_plan(plan: &PlanRecord) -> Result<(), String> {
    let payload_json = row_payload(plan)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO plans(id, status, goal_id, updated_at_ns, payload_json) VALUES(?1, ?2, ?3, ?4, ?5)",
            (
                plan.id.trim(),
                plan.status.to_string(),
                plan.goal_id.as_deref().unwrap_or(""),
                plan.updated_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn get_plan(id: &str) -> Result<Option<PlanRecord>, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM plans WHERE id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([trimmed], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn delete_plan(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM plans WHERE id = ?1", [trimmed])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_plans_by_status(status: &str, limit: usize) -> Result<Vec<PlanRecord>, String> {
    let keep = bounded_limit(limit, 50, 100);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM plans WHERE status = ?1 ORDER BY updated_at_ns DESC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([status, &(keep as i64).to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_all_plans(limit: usize) -> Result<Vec<PlanRecord>, String> {
    let keep = bounded_limit(limit, 50, 100);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM plans ORDER BY
                 CASE status WHEN 'active' THEN 0 WHEN 'paused' THEN 1 WHEN 'completed' THEN 2 ELSE 3 END,
                 updated_at_ns DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_plans() -> Result<usize, String> {
    backend::with_connection(|conn| {
        let count: i64 = conn
            .query_row("SELECT COUNT(1) FROM plans", [], |row| row.get(0))
            .map_err(|err| err.to_string())?;
        Ok(count as usize)
    })
}

pub fn delete_oldest_terminal_plan() -> Result<bool, String> {
    backend::with_connection(|conn| {
        let deleted = conn
            .execute(
                "DELETE FROM plans WHERE id = (
                    SELECT id FROM plans
                    WHERE status IN ('completed', 'abandoned')
                    ORDER BY updated_at_ns ASC
                    LIMIT 1
                )",
                [],
            )
            .map_err(|err| err.to_string())?;
        Ok(deleted > 0)
    })
}

// ── Strategy templates ────────────────────────────────────────────────────────

pub fn upsert_strategy_template(template: &StrategyTemplate) -> Result<(), String> {
    let payload_json = row_payload(template)?;
    let template_id = strategy_template_pk(&template.key);
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_templates(template_id, protocol, primitive, chain_id, updated_at_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            (
                template_id,
                &template.key.protocol,
                &template.key.primitive,
                template.key.chain_id as i64,
                template.updated_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_abi_artifact(artifact: &AbiArtifact) -> Result<(), String> {
    let payload_json = row_payload(artifact)?;
    let artifact_id = abi_artifact_pk(&artifact.key);
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO abi_artifacts(artifact_id, protocol, role, chain_id, created_at_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            (
                artifact_id,
                &artifact.key.protocol,
                &artifact.key.role,
                artifact.key.chain_id as i64,
                artifact.created_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_recent_transitions(limit: usize) -> Result<Vec<TransitionLogRecord>, String> {
    let keep = bounded_limit(limit, 25, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM transitions
                 ORDER BY occurred_at_ns DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_turns(limit: usize) -> Result<Vec<TurnRecord>, String> {
    let keep = bounded_limit(limit, 25, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM turns
                 ORDER BY created_at_ns DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn get_tools_for_turn(turn_id: &str) -> Result<Vec<ToolCallRecord>, String> {
    let trimmed = turn_id.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM tool_calls
                 WHERE turn_id = ?1
                 ORDER BY call_index ASC",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([trimmed], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_inbox_messages(limit: usize) -> Result<Vec<InboxMessage>, String> {
    let keep = bounded_limit(limit, 25, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM inbox
                 ORDER BY seq DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_outbox_messages(limit: usize) -> Result<Vec<OutboxMessage>, String> {
    let keep = bounded_limit(limit, 25, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM outbox
                 ORDER BY seq DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn get_conversation_log(sender: &str) -> Result<Option<ConversationLog>, String> {
    let normalized = sender.trim();
    if normalized.is_empty() {
        return Ok(None);
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM conversations
                 WHERE sender = ?1
                 ORDER BY entry_seq ASC",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([normalized], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut entries = Vec::<ConversationEntry>::new();
        for row in rows {
            entries.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        if entries.is_empty() {
            return Ok(None);
        }
        let last_activity_ns = entries.last().map(|entry| entry.timestamp_ns).unwrap_or(0);
        Ok(Some(ConversationLog {
            sender: normalized.to_string(),
            entries,
            last_activity_ns,
        }))
    })
}

pub fn list_recent_jobs(limit: usize) -> Result<Vec<ScheduledJob>, String> {
    let keep = bounded_limit(limit, 25, 200);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM jobs
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_skills() -> Result<Vec<SkillRecord>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM skills
                 ORDER BY name ASC
                 LIMIT 1000",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn strategy_template(key: &StrategyTemplateKey) -> Result<Option<StrategyTemplate>, String> {
    let pk = strategy_template_pk(key);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_templates WHERE template_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([&pk], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn list_strategy_templates(
    key: &StrategyTemplateKey,
    limit: usize,
) -> Result<Vec<StrategyTemplate>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let keep = bounded_limit(limit, 25, 1_000);
    let template_id = strategy_template_pk(key);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_templates
                 WHERE template_id = ?1
                 ORDER BY updated_at_ns DESC
                 LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((template_id.as_str(), keep as i64), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let mut templates = Vec::new();
        for row in rows {
            templates.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(templates)
    })
}

pub fn list_all_strategy_templates(limit: usize) -> Result<Vec<StrategyTemplate>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let keep = bounded_limit(limit, 25, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_templates
                 ORDER BY updated_at_ns DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn abi_artifact(key: &AbiArtifactKey) -> Result<Option<AbiArtifact>, String> {
    let pk = abi_artifact_pk(key);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM abi_artifacts WHERE artifact_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([&pk], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_strategy_discovery_pending_job(
    job: &PendingStrategyDiscoveryJob,
    watchlist_hash: &str,
) -> Result<(), String> {
    let payload_json = row_payload(job)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_discovery_pending_jobs(job_id, submitted_at_ns, objective, watchlist_hash, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            (
                &job.job_id,
                job.submitted_at_ns as i64,
                &job.objective,
                watchlist_hash,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn strategy_discovery_pending_job(
    job_id: &str,
) -> Result<Option<PendingStrategyDiscoveryJob>, String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_discovery_pending_jobs WHERE job_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([trimmed], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn list_strategy_discovery_pending_jobs(
    limit: usize,
) -> Result<Vec<PendingStrategyDiscoveryJob>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let keep = bounded_limit(limit, 25, 500);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_discovery_pending_jobs
                 ORDER BY submitted_at_ns DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn strategy_discovery_pending_jobs_count() -> Result<u64, String> {
    backend::with_connection(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM strategy_discovery_pending_jobs",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(count.max(0) as u64)
    })
}

pub fn delete_strategy_discovery_pending_job(job_id: &str) -> Result<(), String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute(
            "DELETE FROM strategy_discovery_pending_jobs WHERE job_id = ?1",
            [trimmed],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn upsert_strategy_discovery_result(
    record: &StrategyDiscoveryCallbackRecord,
    status_key: &str,
    watchlist_hash: &str,
) -> Result<(), String> {
    let payload_json = row_payload(record)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_discovery_results(job_id, status, accepted_at_ns, validated_at_ns, completed_at_ns, objective, watchlist_hash, result_hash, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                &record.job_id,
                status_key,
                record.accepted_at_ns as i64,
                record.validated_at_ns.map(|value| value as i64),
                record.completed_at_ns as i64,
                &record.objective,
                watchlist_hash,
                &record.result_hash,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn strategy_discovery_result(
    job_id: &str,
) -> Result<Option<StrategyDiscoveryCallbackRecord>, String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_discovery_results WHERE job_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([trimmed], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn list_strategy_discovery_results(
    limit: usize,
) -> Result<Vec<StrategyDiscoveryCallbackRecord>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let keep = bounded_limit(limit, 25, 500);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_discovery_results
                 ORDER BY accepted_at_ns DESC
                 LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn strategy_discovery_results_count() -> Result<u64, String> {
    backend::with_connection(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM strategy_discovery_results",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(count.max(0) as u64)
    })
}

pub fn freshest_validated_strategy_discovery_result(
) -> Result<Option<StrategyDiscoveryCallbackRecord>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_discovery_results
                 WHERE status = 'validated' AND validated_at_ns IS NOT NULL
                 ORDER BY validated_at_ns DESC, accepted_at_ns DESC
                 LIMIT 1",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn remember_strategy_discovery_completed_callback_job(
    job_id: &str,
    accepted_at_ns: u64,
    result_hash: &str,
) -> Result<(), String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_discovery_completed_callback_jobs(job_id, accepted_at_ns, result_hash)
             VALUES(?1, ?2, ?3)",
            (trimmed, accepted_at_ns as i64, result_hash),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn strategy_discovery_completed_callback_job(
    job_id: &str,
) -> Result<Option<(u64, String)>, String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT accepted_at_ns, result_hash
                 FROM strategy_discovery_completed_callback_jobs
                 WHERE job_id = ?1",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([trimmed], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => {
                let (accepted_at_ns, result_hash) = row.map_err(|err| err.to_string())?;
                Ok(Some((accepted_at_ns.max(0) as u64, result_hash)))
            }
            None => Ok(None),
        }
    })
}

pub fn sql_query_read_only(query: &str, row_limit: usize) -> Result<String, String> {
    let normalized_query = normalized_sql_query(query)?;
    let enforced_limit = bounded_limit(row_limit, 100, 500);
    let wrapped_query = format!(
        "SELECT * FROM ({normalized_query}) AS _automaton_sql_query LIMIT {enforced_limit}"
    );
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(&wrapped_query)
            .map_err(|err| err.to_string())?;
        let column_names = stmt
            .column_names()
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
        let mut output = Vec::<Value>::new();
        #[cfg(target_arch = "wasm32")]
        let start_counter = ic_cdk::api::performance_counter(0);
        #[cfg(not(target_arch = "wasm32"))]
        let start_counter = 0_u64;
        const MAX_SQL_INSTRUCTION_DELTA: u64 = 18_000_000_000;

        while let Some(row) = rows.next().map_err(|err| err.to_string())? {
            if output.len().is_multiple_of(25)
                && sql_instruction_budget_exceeded(start_counter, MAX_SQL_INSTRUCTION_DELTA)
            {
                return Err("sql query aborted due to instruction budget".to_string());
            }

            let mut record = JsonMap::new();
            for (index, name) in column_names.iter().enumerate() {
                let value = row
                    .get_ref(index)
                    .map_err(|err| err.to_string())
                    .map(sqlite_value_to_json)?;
                record.insert(name.clone(), value);
            }
            output.push(Value::Object(record));
        }

        serde_json::to_string(&output).map_err(|err| err.to_string())
    })
}

pub fn table_count(table: &str) -> Result<u64, String> {
    let table_name = match table {
        "transitions"
        | "turns"
        | "journal"
        | "tool_calls"
        | "inbox"
        | "outbox"
        | "conversations"
        | "jobs"
        | "memory_facts"
        | "skills"
        | "strategy_templates"
        | "abi_artifacts"
        | "strategy_discovery_pending_jobs"
        | "strategy_discovery_results"
        | "strategy_discovery_completed_callback_jobs"
        | "hot_runtime_snapshot"
        | "hot_scheduler_runtime"
        | "hot_task_configs"
        | "hot_task_runtimes"
        | "hot_topup_state"
        | "hot_survival_operation_runtime" => table,
        _ => return Err(format!("unsupported table: {table}")),
    };
    backend::with_connection(|conn| {
        let query = format!("SELECT COUNT(1) FROM {table_name}");
        let count: i64 = conn
            .query_row(&query, [], |row| row.get(0))
            .map_err(|err| err.to_string())?;
        Ok(u64::try_from(count).unwrap_or(0))
    })
}

// ── New table CRUD (Migration 003) ───────────────────────────────────────────

// -- HTTP domain allowlist --

pub fn list_http_domains() -> Result<Vec<String>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT domain FROM http_domain_allowlist ORDER BY domain ASC")
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut domains = Vec::new();
        for row in rows {
            domains.push(row.map_err(|err| err.to_string())?);
        }
        Ok(domains)
    })
}

pub fn set_http_domains(domains: &[String]) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM http_domain_allowlist", [])
            .map_err(|err| err.to_string())?;
        for domain in domains {
            conn.execute(
                "INSERT OR REPLACE INTO http_domain_allowlist(domain) VALUES(?1)",
                [domain.as_str()],
            )
            .map_err(|err| err.to_string())?;
        }
        Ok(())
    })
}

pub fn add_http_domain(domain: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO http_domain_allowlist(domain) VALUES(?1)",
            [domain],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn remove_http_domain(domain: &str) -> Result<bool, String> {
    backend::with_connection(|conn| {
        let deleted = conn
            .execute(
                "DELETE FROM http_domain_allowlist WHERE domain = ?1",
                [domain],
            )
            .map_err(|err| err.to_string())?;
        Ok(deleted > 0)
    })
}

// -- Prompt layers --

pub fn get_prompt_layer(layer_id: u8) -> Result<Option<crate::domain::types::PromptLayer>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM prompt_layers WHERE layer_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([i64::from(layer_id)], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn save_prompt_layer(layer: &crate::domain::types::PromptLayer) -> Result<(), String> {
    let payload_json = row_payload(layer)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO prompt_layers(layer_id, payload_json) VALUES(?1, ?2)",
            (i64::from(layer.layer_id), payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_prompt_layer(layer_id: u8) -> Result<bool, String> {
    backend::with_connection(|conn| {
        let deleted = conn
            .execute(
                "DELETE FROM prompt_layers WHERE layer_id = ?1",
                [i64::from(layer_id)],
            )
            .map_err(|err| err.to_string())?;
        Ok(deleted > 0)
    })
}

pub fn list_prompt_layers() -> Result<Vec<crate::domain::types::PromptLayer>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM prompt_layers ORDER BY layer_id ASC LIMIT 100")
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

// -- Retention runtime --

pub fn read_retention_runtime<T: serde::de::DeserializeOwned>() -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM retention_runtime WHERE singleton_id = 1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn write_retention_runtime<T: serde::Serialize>(value: &T) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO retention_runtime(singleton_id, payload_json) VALUES(1, ?1)",
            [payload_json],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Session summaries --

pub fn get_session_summary<T: serde::de::DeserializeOwned>(
    date_key: &str,
) -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM session_summaries WHERE date_key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([date_key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_session_summary<T: serde::Serialize>(
    date_key: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO session_summaries(date_key, payload_json) VALUES(?1, ?2)",
            (date_key, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_session_summaries<T: serde::de::DeserializeOwned>(
    limit: usize,
) -> Result<Vec<T>, String> {
    let keep = bounded_limit(limit, 25, 100);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM session_summaries ORDER BY date_key DESC LIMIT ?1")
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_session_summaries() -> Result<u64, String> {
    table_count("session_summaries")
}

pub fn delete_oldest_session_summaries(keep_max: usize) -> Result<u32, String> {
    backend::with_connection(|conn| {
        let deleted = conn
            .execute(
                "DELETE FROM session_summaries WHERE date_key NOT IN (
                    SELECT date_key FROM session_summaries ORDER BY date_key DESC LIMIT ?1
                )",
                [keep_max as i64],
            )
            .map_err(|err| err.to_string())?;
        Ok(u32::try_from(deleted).unwrap_or(u32::MAX))
    })
}

// -- Turn window summaries --

pub fn get_turn_window_summary<T: serde::de::DeserializeOwned>(
    date_key: &str,
) -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM turn_window_summaries WHERE date_key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([date_key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_turn_window_summary<T: serde::Serialize>(
    date_key: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO turn_window_summaries(date_key, payload_json) VALUES(?1, ?2)",
            (date_key, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_turn_window_summaries<T: serde::de::DeserializeOwned>(
    limit: usize,
) -> Result<Vec<T>, String> {
    let keep = bounded_limit(limit, 25, 100);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM turn_window_summaries ORDER BY date_key DESC LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn delete_oldest_turn_window_summaries(keep_max: usize) -> Result<u32, String> {
    backend::with_connection(|conn| {
        let deleted = conn
            .execute(
                "DELETE FROM turn_window_summaries WHERE date_key NOT IN (
                    SELECT date_key FROM turn_window_summaries ORDER BY date_key DESC LIMIT ?1
                )",
                [keep_max as i64],
            )
            .map_err(|err| err.to_string())?;
        Ok(u32::try_from(deleted).unwrap_or(u32::MAX))
    })
}

// -- Memory rollups --

pub fn upsert_memory_rollup<T: serde::Serialize>(
    rollup_key: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO memory_rollups(rollup_key, payload_json) VALUES(?1, ?2)",
            (rollup_key, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_memory_rollups<T: serde::de::DeserializeOwned>(limit: usize) -> Result<Vec<T>, String> {
    let keep = bounded_limit(limit, 25, 128);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM memory_rollups ORDER BY rollup_key DESC LIMIT ?1")
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn delete_oldest_memory_rollups(keep_max: usize) -> Result<u32, String> {
    backend::with_connection(|conn| {
        let deleted = conn
            .execute(
                "DELETE FROM memory_rollups WHERE rollup_key NOT IN (
                    SELECT rollup_key FROM memory_rollups ORDER BY rollup_key DESC LIMIT ?1
                )",
                [keep_max as i64],
            )
            .map_err(|err| err.to_string())?;
        Ok(u32::try_from(deleted).unwrap_or(u32::MAX))
    })
}

// -- Strategy activations --

pub fn get_strategy_activation<T: serde::de::DeserializeOwned>(
    version_key: &str,
) -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_activations WHERE version_key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([version_key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_strategy_activation<T: serde::Serialize>(
    version_key: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_activations(version_key, payload_json) VALUES(?1, ?2)",
            (version_key, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Strategy revocations --

pub fn get_strategy_revocation<T: serde::de::DeserializeOwned>(
    version_key: &str,
) -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_revocations WHERE version_key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([version_key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_strategy_revocation<T: serde::Serialize>(
    version_key: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_revocations(version_key, payload_json) VALUES(?1, ?2)",
            (version_key, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Strategy kill switches --

pub fn get_strategy_kill_switch<T: serde::de::DeserializeOwned>(
    key: &str,
) -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_kill_switches WHERE key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_strategy_kill_switch<T: serde::Serialize>(
    key: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_kill_switches(key, payload_json) VALUES(?1, ?2)",
            (key, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Strategy outcome stats --

pub fn get_strategy_outcome_stats<T: serde::de::DeserializeOwned>(
    key: &str,
) -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_outcome_stats WHERE key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_strategy_outcome_stats<T: serde::Serialize>(
    key: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_outcome_stats(key, payload_json) VALUES(?1, ?2)",
            (key, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Strategy budgets --

pub fn get_strategy_budget(key: &str) -> Result<Option<String>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_budgets WHERE key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(|err| err.to_string())?)),
            None => Ok(None),
        }
    })
}

pub fn upsert_strategy_budget(key: &str, value: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_budgets(key, payload_json) VALUES(?1, ?2)",
            (key, value),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Autonomy tool failures --

pub fn get_autonomy_tool_failure<T: serde::de::DeserializeOwned>(
    tool_name: &str,
) -> Result<Option<T>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM autonomy_tool_failures WHERE tool_name = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([tool_name], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_autonomy_tool_failure<T: serde::Serialize>(
    tool_name: &str,
    value: &T,
) -> Result<(), String> {
    let payload_json = row_payload(value)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO autonomy_tool_failures(tool_name, payload_json) VALUES(?1, ?2)",
            (tool_name, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_autonomy_tool_failure(tool_name: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute(
            "DELETE FROM autonomy_tool_failures WHERE tool_name = ?1",
            [tool_name],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_autonomy_tool_failures<T: serde::de::DeserializeOwned>(
) -> Result<Vec<(String, T)>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT tool_name, payload_json FROM autonomy_tool_failures ORDER BY tool_name ASC LIMIT 500")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().map_err(|err| err.to_string())? {
            let name = row.get::<_, String>(0).map_err(|err| err.to_string())?;
            let payload_json = row.get::<_, String>(1).map_err(|err| err.to_string())?;
            let value: T = from_payload_json(payload_json)?;
            records.push((name, value));
        }
        Ok(records)
    })
}

// -- Runtime scalars (replaces RUNTIME_MAP scalar keys) --

pub fn get_runtime_scalar(key: &str) -> Result<Option<String>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT value_text FROM runtime_scalars WHERE key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => Ok(Some(row.map_err(|err| err.to_string())?)),
            None => Ok(None),
        }
    })
}

pub fn set_runtime_scalar(key: &str, value: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO runtime_scalars(key, value_text) VALUES(?1, ?2)",
            (key, value),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_runtime_scalar(key: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM runtime_scalars WHERE key = ?1", [key])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Extended memory fact queries --

pub fn list_all_memory_facts(limit: usize) -> Result<Vec<MemoryFact>, String> {
    let keep = bounded_limit(limit, 500, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM memory_facts ORDER BY key ASC LIMIT ?1")
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_memory_facts_by_prefix(prefix: &str, limit: usize) -> Result<Vec<MemoryFact>, String> {
    let keep = bounded_limit(limit, 500, 1_000);
    let pattern = format!("{prefix}%");
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM memory_facts WHERE key LIKE ?1 ORDER BY key ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((pattern.as_str(), keep as i64), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn get_memory_fact(key: &str) -> Result<Option<MemoryFact>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM memory_facts WHERE key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn count_memory_facts() -> Result<usize, String> {
    table_count("memory_facts").map(|c| c as usize)
}

pub fn count_memory_facts_by_prefix(prefix: &str) -> Result<usize, String> {
    let pattern = format!("{prefix}%");
    backend::with_connection(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM memory_facts WHERE key LIKE ?1",
                [pattern.as_str()],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(count as usize)
    })
}

pub fn prune_memory_facts(
    prefix: Option<&str>,
    updated_before_ns: Option<u64>,
    limit: usize,
) -> Result<Vec<String>, String> {
    if matches!((prefix, updated_before_ns), (None, None)) {
        return Ok(Vec::new());
    }
    backend::with_connection(|conn| {
        // Collect the keys to prune, then delete them in a single subquery statement.
        let keys: Vec<String> = match (prefix, updated_before_ns) {
            (Some(p), Some(ts)) => {
                let pattern = format!("{p}%");
                let mut stmt = conn
                    .prepare(
                        "SELECT key FROM memory_facts WHERE key LIKE ?1 AND updated_at_ns < ?2 ORDER BY updated_at_ns ASC LIMIT ?3",
                    )
                    .map_err(|err| err.to_string())?;
                let rows = stmt
                    .query_map((pattern.as_str(), ts as i64, limit as i64), |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|err| err.to_string())?;
                rows.map(|r| r.map_err(|err| err.to_string()))
                    .collect::<Result<_, _>>()?
            }
            (Some(p), None) => {
                let pattern = format!("{p}%");
                let mut stmt = conn
                    .prepare(
                        "SELECT key FROM memory_facts WHERE key LIKE ?1 ORDER BY updated_at_ns ASC LIMIT ?2",
                    )
                    .map_err(|err| err.to_string())?;
                let rows = stmt
                    .query_map((pattern.as_str(), limit as i64), |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|err| err.to_string())?;
                rows.map(|r| r.map_err(|err| err.to_string()))
                    .collect::<Result<_, _>>()?
            }
            (None, Some(ts)) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT key FROM memory_facts WHERE updated_at_ns < ?1 ORDER BY updated_at_ns ASC LIMIT ?2",
                    )
                    .map_err(|err| err.to_string())?;
                let rows = stmt
                    .query_map((ts as i64, limit as i64), |row| row.get::<_, String>(0))
                    .map_err(|err| err.to_string())?;
                rows.map(|r| r.map_err(|err| err.to_string()))
                    .collect::<Result<_, _>>()?
            }
            (None, None) => unreachable!(),
        };
        if !keys.is_empty() {
            match (prefix, updated_before_ns) {
                (Some(p), Some(ts)) => {
                    let pattern = format!("{p}%");
                    conn.execute(
                        "DELETE FROM memory_facts WHERE key IN (SELECT key FROM memory_facts WHERE key LIKE ?1 AND updated_at_ns < ?2 ORDER BY updated_at_ns ASC LIMIT ?3)",
                        (pattern.as_str(), ts as i64, limit as i64),
                    ).map_err(|err| err.to_string())?;
                }
                (Some(p), None) => {
                    let pattern = format!("{p}%");
                    conn.execute(
                        "DELETE FROM memory_facts WHERE key IN (SELECT key FROM memory_facts WHERE key LIKE ?1 ORDER BY updated_at_ns ASC LIMIT ?2)",
                        (pattern.as_str(), limit as i64),
                    ).map_err(|err| err.to_string())?;
                }
                (None, Some(ts)) => {
                    conn.execute(
                        "DELETE FROM memory_facts WHERE key IN (SELECT key FROM memory_facts WHERE updated_at_ns < ?1 ORDER BY updated_at_ns ASC LIMIT ?2)",
                        (ts as i64, limit as i64),
                    ).map_err(|err| err.to_string())?;
                }
                (None, None) => unreachable!(),
            }
        }
        Ok(keys)
    })
}

// -- Reflection memory queries --

pub fn upsert_reflection_memory(record: &ReflectionMemoryRecord) -> Result<(), String> {
    let payload_json = row_payload(record)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO reflection_memory(key, updated_at_ns, payload_json) VALUES(?1, ?2, ?3)",
            (&record.key, record.updated_at_ns as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_reflection_memory(key: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM reflection_memory WHERE key = ?1", [key])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn get_reflection_memory(key: &str) -> Result<Option<ReflectionMemoryRecord>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM reflection_memory WHERE key = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([key], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn list_reflection_memory(limit: usize) -> Result<Vec<ReflectionMemoryRecord>, String> {
    let keep = bounded_limit(limit, 64, 512);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM reflection_memory ORDER BY updated_at_ns DESC, key ASC LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_reflection_memory() -> Result<usize, String> {
    table_count_extended("reflection_memory").map(|count| count as usize)
}

pub fn prune_reflection_memory(
    updated_before_ns: u64,
    limit: usize,
) -> Result<Vec<String>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT key FROM reflection_memory WHERE updated_at_ns < ?1 ORDER BY updated_at_ns ASC, key ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((updated_before_ns as i64, limit as i64), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let keys: Vec<String> = rows
            .map(|row| row.map_err(|err| err.to_string()))
            .collect::<Result<_, _>>()?;
        if !keys.is_empty() {
            conn.execute(
                "DELETE FROM reflection_memory WHERE key IN (SELECT key FROM reflection_memory WHERE updated_at_ns < ?1 ORDER BY updated_at_ns ASC, key ASC LIMIT ?2)",
                (updated_before_ns as i64, limit as i64),
            )
            .map_err(|err| err.to_string())?;
        }
        Ok(keys)
    })
}

pub fn delete_oldest_reflection_memory(keep_max: usize) -> Result<u32, String> {
    backend::with_connection(|conn| {
        let deleted = if keep_max == 0 {
            conn.execute("DELETE FROM reflection_memory", [])
                .map_err(|err| err.to_string())?
        } else {
            conn.execute(
                "DELETE FROM reflection_memory WHERE key NOT IN (
                    SELECT key FROM reflection_memory ORDER BY updated_at_ns DESC, key ASC LIMIT ?1
                )",
                [keep_max as i64],
            )
            .map_err(|err| err.to_string())?
        };
        Ok(u32::try_from(deleted).unwrap_or(u32::MAX))
    })
}

// -- Autonomy runtime queries --

pub fn read_autonomy_policy() -> Result<Option<AutonomyPolicy>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM autonomy_policy WHERE singleton_id = 1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
        match rows.next().map_err(|err| err.to_string())? {
            Some(row) => {
                from_payload_json(row.get::<_, String>(0).map_err(|err| err.to_string())?).map(Some)
            }
            None => Ok(None),
        }
    })
}

pub fn write_autonomy_policy(policy: &AutonomyPolicy) -> Result<(), String> {
    let payload_json = row_payload(policy)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO autonomy_policy(singleton_id, updated_at_ns, payload_json)
             VALUES(1, ?1, ?2)",
            (policy.updated_at_ns as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn get_active_exposure(strategy_id: &str) -> Result<Option<ActiveExposure>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM active_exposures WHERE strategy_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([strategy_id], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn insert_pending_strategy_execution(
    execution: &PendingStrategyExecution,
) -> Result<PendingStrategyExecution, String> {
    let payload = row_payload(execution)?;
    let state = strategy_execution_state_text(&execution.state);
    backend::with_connection(|conn| {
        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO pending_strategy_executions
                 (execution_id, state, next_check_at_ns, created_at_ns, updated_at_ns, payload_json)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    execution.execution_id,
                    state,
                    execution.next_check_at_ns,
                    execution.created_at_ns,
                    execution.updated_at_ns,
                    payload
                ],
            )
            .map_err(|err| err.to_string())?;
        if changed == 0 {
            let existing: String = conn
                .query_row(
                    "SELECT payload_json FROM pending_strategy_executions WHERE execution_id = ?1",
                    params![execution.execution_id],
                    |row| row.get(0),
                )
                .map_err(|err| err.to_string())?;
            let existing = serde_json::from_str::<PendingStrategyExecution>(&existing)
                .map_err(|err| err.to_string())?;
            if existing.plan_digest != execution.plan_digest {
                return Err("pending strategy execution id collision".to_string());
            }
            return Ok(existing);
        }
        Ok(execution.clone())
    })
}

pub fn get_pending_strategy_execution(
    execution_id: &str,
) -> Result<Option<PendingStrategyExecution>, String> {
    backend::with_connection(|conn| {
        let mut statement = conn
            .prepare("SELECT payload_json FROM pending_strategy_executions WHERE execution_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = statement
            .query(params![execution_id])
            .map_err(|err| err.to_string())?;
        let Some(row) = rows.next().map_err(|err| err.to_string())? else {
            return Ok(None);
        };
        let payload: String = row.get(0).map_err(|err| err.to_string())?;
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(|err| err.to_string())
    })
}

pub fn update_pending_strategy_execution(
    execution: &PendingStrategyExecution,
) -> Result<(), String> {
    let payload = row_payload(execution)?;
    backend::with_connection(|conn| {
        let changed = conn
            .execute(
                "UPDATE pending_strategy_executions SET state = ?2, next_check_at_ns = ?3,
             updated_at_ns = ?4, payload_json = ?5 WHERE execution_id = ?1",
                params![
                    execution.execution_id,
                    strategy_execution_state_text(&execution.state),
                    execution.next_check_at_ns,
                    execution.updated_at_ns,
                    payload
                ],
            )
            .map_err(|err| err.to_string())?;
        if changed != 1 {
            return Err("pending strategy execution was not found".to_string());
        }
        Ok(())
    })
}

/// Persist nonterminal progress only when the durable row still equals the
/// stale-capable snapshot loaded by the caller. A lost race is a safe no-op.
///
/// Strategy execution transition table:
/// - Pending -> Pending: receipt/submission progress; CAS required; no terminal bookkeeping.
/// - Pending -> Confirmed: success bookkeeping in the guarded confirmation transaction.
/// - Pending -> PartialFailure/Reverted/Dropped: failure bookkeeping in the guarded failure transaction.
/// - Any terminal -> any state: forbidden (duplicate same-terminal calls and competing outcomes no-op).
///
/// Confirmed and failure terminals are monotonic, mutually exclusive, and each applies its side effects once.
pub fn compare_and_update_pending_strategy_execution(
    expected: &PendingStrategyExecution,
    desired: &PendingStrategyExecution,
) -> Result<bool, String> {
    if expected.execution_id != desired.execution_id
        || expected.state != crate::domain::types::PendingStrategyExecutionState::Pending
        || desired.state != crate::domain::types::PendingStrategyExecutionState::Pending
        || expected.bookkeeping_applied
        || expected.terminal_bookkeeping_applied
        || desired.bookkeeping_applied
        || desired.terminal_bookkeeping_applied
    {
        return Ok(false);
    }
    let expected_payload = row_payload(expected)?;
    let desired_payload = row_payload(desired)?;
    backend::with_connection(|conn| {
        let changed = conn
            .execute(
                "UPDATE pending_strategy_executions SET state = 'pending', next_check_at_ns = ?3,
             updated_at_ns = ?4, payload_json = ?5
             WHERE execution_id = ?1 AND state = 'pending' AND payload_json = ?2",
                params![
                    desired.execution_id,
                    expected_payload,
                    desired.next_check_at_ns,
                    desired.updated_at_ns,
                    desired_payload
                ],
            )
            .map_err(|err| err.to_string())?;
        Ok(changed == 1)
    })
}

pub fn list_due_pending_strategy_executions(
    now_ns: u64,
    limit: usize,
) -> Result<Vec<PendingStrategyExecution>, String> {
    backend::with_connection(|conn| {
        let mut statement = conn
            .prepare(
                "SELECT payload_json FROM pending_strategy_executions
             WHERE state = 'pending' AND next_check_at_ns <= ?1
             ORDER BY next_check_at_ns ASC, created_at_ns ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = statement
            .query_map(params![now_ns, limit as u64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        rows.map(|row| {
            let payload = row.map_err(|err| err.to_string())?;
            serde_json::from_str(&payload).map_err(|err| err.to_string())
        })
        .collect()
    })
}

pub fn list_confirmed_strategy_executions_page(
    after: Option<(u64, &str)>,
    limit: usize,
) -> Result<Vec<PendingStrategyExecution>, String> {
    backend::with_connection(|conn| {
        let query = if after.is_some() {
            "SELECT payload_json FROM pending_strategy_executions
             WHERE state = 'confirmed' AND (created_at_ns > ?1 OR (created_at_ns = ?1 AND execution_id > ?2))
             ORDER BY created_at_ns ASC, execution_id ASC LIMIT ?3"
        } else {
            "SELECT payload_json FROM pending_strategy_executions
             WHERE state = 'confirmed' ORDER BY created_at_ns ASC, execution_id ASC LIMIT ?3"
        };
        let mut statement = conn.prepare(query).map_err(|err| err.to_string())?;
        let mut rows = match after {
            Some((created_at_ns, execution_id)) => {
                statement.query(params![created_at_ns, execution_id, limit as u64])
            }
            None => statement.query(params![0u64, "", limit as u64]),
        }
        .map_err(|err| err.to_string())?;
        let mut executions = Vec::new();
        while let Some(row) = rows.next().map_err(|err| err.to_string())? {
            let payload: String = row.get(0).map_err(|err| err.to_string())?;
            executions.push(serde_json::from_str(&payload).map_err(|err| err.to_string())?);
        }
        Ok(executions)
    })
}

fn strategy_execution_state_text(
    state: &crate::domain::types::PendingStrategyExecutionState,
) -> &'static str {
    use crate::domain::types::PendingStrategyExecutionState::*;
    match state {
        Pending => "pending",
        Confirmed => "confirmed",
        PartialFailure => "partial_failure",
        Reverted => "reverted",
        Dropped => "dropped",
    }
}

/// Commit receipt confirmation and all success-side bookkeeping in one SQLite transaction.
pub fn confirm_strategy_execution_atomically(
    execution: &PendingStrategyExecution,
    exposure_change: Option<&Option<ActiveExposure>>,
    outcome_record_key: &str,
    outcome_stats: &StrategyOutcomeStats,
    budget_change: Option<(&str, &str)>,
    strategy_id: &str,
) -> Result<bool, String> {
    let execution_payload = row_payload(execution)?;
    let stats_payload = row_payload(outcome_stats)?;
    backend::with_connection(|conn| {
        let transaction = conn
            .unchecked_transaction()
            .map_err(|err| err.to_string())?;
        let current_payload: String = transaction
            .query_row(
                "SELECT payload_json FROM pending_strategy_executions WHERE execution_id = ?1",
                params![execution.execution_id],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let current: PendingStrategyExecution =
            serde_json::from_str(&current_payload).map_err(|err| err.to_string())?;
        if current.state != crate::domain::types::PendingStrategyExecutionState::Pending
            || current.bookkeeping_applied
            || current.terminal_bookkeeping_applied
        {
            return Ok(false);
        }
        if let Some(change) = exposure_change {
            if let Some(exposure) = change {
                transaction.execute(
                    "INSERT OR REPLACE INTO active_exposures(strategy_id, protocol, chain_id, updated_at_ns, payload_json)
                     VALUES(?1, ?2, ?3, ?4, ?5)",
                    params![exposure.strategy_id, exposure.protocol, exposure.chain_id, exposure.updated_at_ns, row_payload(exposure)?],
                ).map_err(|err| err.to_string())?;
            } else {
                transaction
                    .execute(
                        "DELETE FROM active_exposures WHERE strategy_id = ?1",
                        params![strategy_id],
                    )
                    .map_err(|err| err.to_string())?;
            }
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO strategy_outcome_stats(key, payload_json) VALUES(?1, ?2)",
                params![outcome_record_key, stats_payload],
            )
            .map_err(|err| err.to_string())?;
        if let Some((budget_key, budget)) = budget_change {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO strategy_budgets(key, payload_json) VALUES(?1, ?2)",
                    params![budget_key, budget],
                )
                .map_err(|err| err.to_string())?;
        }
        transaction
            .execute(
                "DELETE FROM strategy_quarantines WHERE strategy_id = ?1",
                params![strategy_id],
            )
            .map_err(|err| err.to_string())?;
        transaction
            .execute(
                "UPDATE pending_strategy_executions SET state = 'confirmed', next_check_at_ns = ?2,
             updated_at_ns = ?3, payload_json = ?4 WHERE execution_id = ?1",
                params![
                    execution.execution_id,
                    execution.next_check_at_ns,
                    execution.updated_at_ns,
                    execution_payload
                ],
            )
            .map_err(|err| err.to_string())?;
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(true)
    })
}

/// Commit a terminal failure and its learner/quarantine bookkeeping exactly once.
pub fn fail_strategy_execution_atomically(
    execution: &PendingStrategyExecution,
    outcome_record_key: &str,
    outcome_stats: &StrategyOutcomeStats,
    quarantine: &StrategyQuarantine,
    activation_change: Option<(&str, &TemplateActivationState)>,
) -> Result<bool, String> {
    let execution_payload = row_payload(execution)?;
    let stats_payload = row_payload(outcome_stats)?;
    let quarantine_payload = row_payload(quarantine)?;
    backend::with_connection(|conn| {
        let transaction = conn
            .unchecked_transaction()
            .map_err(|err| err.to_string())?;
        let current_payload: String = transaction
            .query_row(
                "SELECT payload_json FROM pending_strategy_executions WHERE execution_id = ?1",
                params![execution.execution_id],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let current: PendingStrategyExecution =
            serde_json::from_str(&current_payload).map_err(|err| err.to_string())?;
        if current.state != crate::domain::types::PendingStrategyExecutionState::Pending
            || current.bookkeeping_applied
            || current.terminal_bookkeeping_applied
        {
            return Ok(false);
        }
        transaction
            .execute(
                "INSERT OR REPLACE INTO strategy_outcome_stats(key, payload_json) VALUES(?1, ?2)",
                params![outcome_record_key, stats_payload],
            )
            .map_err(|err| err.to_string())?;
        transaction.execute(
            "INSERT OR REPLACE INTO strategy_quarantines(strategy_id, quarantined_at_ns, release_after_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4)",
            params![quarantine.strategy_id, quarantine.quarantined_at_ns, quarantine.release_after_ns, quarantine_payload],
        ).map_err(|err| err.to_string())?;
        if let Some((activation_key, activation)) = activation_change {
            transaction.execute(
                "INSERT OR REPLACE INTO strategy_activations(version_key, payload_json) VALUES(?1, ?2)",
                params![activation_key, row_payload(activation)?],
            ).map_err(|err| err.to_string())?;
        }
        transaction
            .execute(
                "UPDATE pending_strategy_executions SET state = ?2, next_check_at_ns = ?3,
             updated_at_ns = ?4, payload_json = ?5 WHERE execution_id = ?1",
                params![
                    execution.execution_id,
                    strategy_execution_state_text(&execution.state),
                    execution.next_check_at_ns,
                    execution.updated_at_ns,
                    execution_payload
                ],
            )
            .map_err(|err| err.to_string())?;
        transaction.commit().map_err(|err| err.to_string())?;
        Ok(true)
    })
}

pub fn upsert_active_exposure(exposure: &ActiveExposure) -> Result<(), String> {
    let payload_json = row_payload(exposure)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO active_exposures(strategy_id, protocol, chain_id, updated_at_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            (
                &exposure.strategy_id,
                &exposure.protocol,
                exposure.chain_id as i64,
                exposure.updated_at_ns as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_active_exposure(strategy_id: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute(
            "DELETE FROM active_exposures WHERE strategy_id = ?1",
            [strategy_id],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_active_exposures(limit: usize) -> Result<Vec<ActiveExposure>, String> {
    let keep = bounded_limit(limit, 100, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM active_exposures
                 ORDER BY updated_at_ns DESC, strategy_id ASC LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_active_exposures() -> Result<usize, String> {
    table_count_extended("active_exposures").map(|count| count as usize)
}

pub fn get_strategy_quarantine(strategy_id: &str) -> Result<Option<StrategyQuarantine>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM strategy_quarantines WHERE strategy_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([strategy_id], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn upsert_strategy_quarantine(quarantine: &StrategyQuarantine) -> Result<(), String> {
    let payload_json = row_payload(quarantine)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO strategy_quarantines(strategy_id, quarantined_at_ns, release_after_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4)",
            (
                &quarantine.strategy_id,
                quarantine.quarantined_at_ns as i64,
                quarantine.release_after_ns.map(|value| value as i64),
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_strategy_quarantine(strategy_id: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute(
            "DELETE FROM strategy_quarantines WHERE strategy_id = ?1",
            [strategy_id],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn list_strategy_quarantines(limit: usize) -> Result<Vec<StrategyQuarantine>, String> {
    let keep = bounded_limit(limit, 100, 1_000);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_quarantines
                 ORDER BY quarantined_at_ns DESC, strategy_id ASC LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_strategy_quarantines() -> Result<usize, String> {
    table_count_extended("strategy_quarantines").map(|count| count as usize)
}

pub fn append_journal_entry(
    turn_id: &str,
    timestamp_ns: u64,
    text: &str,
    genesis: bool,
    deal_claim: Option<JournalDealClaim>,
    keep_max: usize,
) -> Result<JournalEntry, String> {
    backend::with_connection(|conn| {
        let next_id: i64 = conn
            .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM journal", [], |row| {
                row.get(0)
            })
            .map_err(|err| err.to_string())?;
        let entry = JournalEntry {
            id: next_id as u64,
            turn_id: turn_id.to_string(),
            timestamp_ns,
            text: text.to_string(),
            genesis,
            deal_claim,
        };
        let payload_json = row_payload(&entry)?;
        conn.execute(
            "INSERT INTO journal(id, turn_id, timestamp_ns, genesis, payload_json) VALUES(?1, ?2, ?3, ?4, ?5)",
            (next_id, turn_id, timestamp_ns as i64, i64::from(genesis), payload_json),
        )
        .map_err(|err| err.to_string())?;
        conn.execute(
            "DELETE FROM journal WHERE id NOT IN (SELECT id FROM journal ORDER BY id DESC LIMIT ?1)",
            [keep_max.min(i64::MAX as usize) as i64],
        )
        .map_err(|err| err.to_string())?;
        Ok(entry)
    })
}

pub fn list_recent_journal_entries(limit: usize) -> Result<Vec<JournalEntry>, String> {
    let keep = bounded_limit(limit, 25, 200);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM journal ORDER BY id DESC LIMIT ?1")
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(entries)
    })
}

pub fn count_journal_entries() -> Result<usize, String> {
    table_count_extended("journal").map(|count| count as usize)
}

pub fn append_decision_record(record: &DecisionRecord, keep_max: usize) -> Result<(), String> {
    let payload_json = row_payload(record)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO decision_records(turn_id, timestamp_ns, payload_json)
             VALUES(?1, ?2, ?3)",
            (&record.turn_id, record.timestamp_ns as i64, payload_json),
        )
        .map_err(|err| err.to_string())?;

        let keep_max = keep_max.min(i64::MAX as usize) as i64;
        let total: i64 = conn
            .query_row("SELECT COUNT(1) FROM decision_records", [], |row| {
                row.get(0)
            })
            .map_err(|err| err.to_string())?;
        if total > keep_max {
            let delete_count = total - keep_max;
            conn.execute(
                "DELETE FROM decision_records
                 WHERE turn_id IN (
                     SELECT turn_id FROM decision_records
                     ORDER BY timestamp_ns ASC, turn_id ASC
                     LIMIT ?1
                 )",
                [delete_count],
            )
            .map_err(|err| err.to_string())?;
        }
        Ok(())
    })
}

pub fn get_decision_record(turn_id: &str) -> Result<Option<DecisionRecord>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM decision_records WHERE turn_id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([turn_id], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn list_recent_decision_records(limit: usize) -> Result<Vec<DecisionRecord>, String> {
    let keep = bounded_limit(limit, 25, 200);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM decision_records
                 ORDER BY timestamp_ns DESC, turn_id DESC LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_decision_records() -> Result<usize, String> {
    table_count_extended("decision_records").map(|count| count as usize)
}

pub fn read_exposure_reconciliation_status() -> Result<Option<ExposureReconciliationStatus>, String>
{
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM exposure_reconciliation_status WHERE singleton_id = 1",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
        match rows.next().map_err(|err| err.to_string())? {
            Some(row) => {
                from_payload_json(row.get::<_, String>(0).map_err(|err| err.to_string())?).map(Some)
            }
            None => Ok(None),
        }
    })
}

pub fn write_exposure_reconciliation_status(
    status: &ExposureReconciliationStatus,
) -> Result<(), String> {
    let payload_json = row_payload(status)?;
    backend::with_connection(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO exposure_reconciliation_status(singleton_id, last_attempted_at_ns, payload_json)
             VALUES(1, ?1, ?2)",
            (
                status.last_attempted_at_ns.unwrap_or_default() as i64,
                payload_json,
            ),
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Extended inbox queries --

pub fn list_pending_inbox(limit: usize) -> Result<Vec<InboxMessage>, String> {
    let keep = bounded_limit(limit, 25, 200);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM inbox WHERE status = 'Pending' ORDER BY seq ASC LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn list_staged_inbox(limit: usize) -> Result<Vec<InboxMessage>, String> {
    let keep = bounded_limit(limit, 25, 200);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM inbox WHERE status = 'Staged' ORDER BY seq ASC LIMIT ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([keep as i64], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        for row in rows {
            records.push(from_payload_json(row.map_err(|err| err.to_string())?)?);
        }
        Ok(records)
    })
}

pub fn count_inbox_by_status(status: &str) -> Result<u64, String> {
    backend::with_connection(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM inbox WHERE status = ?1",
                [status],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(u64::try_from(count).unwrap_or(0))
    })
}

pub fn count_inbox_total() -> Result<u64, String> {
    table_count("inbox")
}

pub fn get_inbox_message(id: &str) -> Result<Option<InboxMessage>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM inbox WHERE id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

// -- Extended outbox queries --

pub fn get_outbox_message(id: &str) -> Result<Option<OutboxMessage>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM outbox WHERE id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn count_outbox_total() -> Result<u64, String> {
    table_count("outbox")
}

// -- Extended job queries --

pub fn get_job(id: &str) -> Result<Option<ScheduledJob>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM jobs WHERE id = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn find_job_by_dedupe_key(dedupe_key: &str) -> Result<Option<ScheduledJob>, String> {
    backend::with_connection(|conn| {
        // Only find active (non-terminal) jobs
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM jobs WHERE payload_json LIKE ?1 AND (status = 'Pending' OR status = 'Running') LIMIT 1",
            )
            .map_err(|err| err.to_string())?;
        let pattern = format!("%\"dedupe_key\":\"{dedupe_key}\"%");
        let mut rows = stmt
            .query_map([pattern.as_str()], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => {
                let job: ScheduledJob = from_payload_json(row.map_err(|err| err.to_string())?)?;
                if job.dedupe_key == dedupe_key {
                    Ok(Some(job))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    })
}

pub fn pop_next_pending_job(lane: &str, now_ns: u64) -> Result<Option<ScheduledJob>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM jobs WHERE status = 'Pending' AND lane = ?1 AND scheduled_for_ns <= ?2 ORDER BY priority ASC, created_at_ns ASC LIMIT 1",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map((lane, now_ns as i64), |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

pub fn delete_job(id: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM jobs WHERE id = ?1", [id])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_inbox_message(id: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM inbox WHERE id = ?1", [id])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_outbox_message(id: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM outbox WHERE id = ?1", [id])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_turn(id: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM turns WHERE id = ?1", [id])
            .map_err(|err| err.to_string())?;
        conn.execute("DELETE FROM tool_calls WHERE turn_id = ?1", [id])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

pub fn delete_transition(id: &str) -> Result<(), String> {
    backend::with_connection(|conn| {
        conn.execute("DELETE FROM transitions WHERE id = ?1", [id])
            .map_err(|err| err.to_string())?;
        Ok(())
    })
}

// -- Conversation listing --

pub fn list_conversation_summaries() -> Result<Vec<(String, u64, u32)>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT sender, MAX(timestamp_ns) as last_ns, COUNT(*) as cnt
                 FROM conversations
                 GROUP BY sender
                 ORDER BY last_ns DESC",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt.query([]).map_err(|err| err.to_string())?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(|err| err.to_string())? {
            let sender = row.get::<_, String>(0).map_err(|err| err.to_string())?;
            let last_ns = row.get::<_, i64>(1).map_err(|err| err.to_string())? as u64;
            let count = row.get::<_, i64>(2).map_err(|err| err.to_string())? as u32;
            results.push((sender, last_ns, count));
        }
        Ok(results)
    })
}

// -- Skill retrieval by name --

pub fn get_skill(name: &str) -> Result<Option<SkillRecord>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM skills WHERE name = ?1")
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query_map([name], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        match rows.next() {
            Some(row) => from_payload_json(row.map_err(|err| err.to_string())?).map(Some),
            None => Ok(None),
        }
    })
}

// -- Retention: bulk delete by age --

pub fn delete_jobs_older_than(cutoff_ns: u64, limit: usize) -> Result<Vec<String>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM jobs WHERE created_at_ns < ?1 AND status IN ('Succeeded', 'Failed', 'TimedOut', 'Skipped') ORDER BY created_at_ns ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((cutoff_ns as i64, limit as i64), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let ids: Vec<String> = rows
            .map(|r| r.map_err(|err| err.to_string()))
            .collect::<Result<_, _>>()?;
        if !ids.is_empty() {
            conn.execute(
                "DELETE FROM jobs WHERE id IN (SELECT id FROM jobs WHERE created_at_ns < ?1 AND status IN ('Succeeded', 'Failed', 'TimedOut', 'Skipped') ORDER BY created_at_ns ASC LIMIT ?2)",
                (cutoff_ns as i64, limit as i64),
            ).map_err(|err| err.to_string())?;
        }
        Ok(ids)
    })
}

pub fn delete_inbox_older_than(
    cutoff_ns: u64,
    limit: usize,
    protected_ids: &[String],
) -> Result<Vec<String>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM inbox WHERE posted_at_ns < ?1 AND status = 'Consumed' ORDER BY posted_at_ns ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((cutoff_ns as i64, limit as i64), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let mut ids = Vec::new();
        for row in rows {
            let id = row.map_err(|err| err.to_string())?;
            if !protected_ids.contains(&id) {
                ids.push(id);
            }
        }
        for id in &ids {
            conn.execute("DELETE FROM inbox WHERE id = ?1", [id.as_str()])
                .map_err(|err| err.to_string())?;
        }
        Ok(ids)
    })
}

pub fn delete_outbox_older_than(
    cutoff_ns: u64,
    limit: usize,
    protected_inbox_ids: &[String],
) -> Result<Vec<String>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, payload_json FROM outbox WHERE created_at_ns < ?1 ORDER BY created_at_ns ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query((cutoff_ns as i64, limit as i64))
            .map_err(|err| err.to_string())?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().map_err(|err| err.to_string())? {
            let id = row.get::<_, String>(0).map_err(|err| err.to_string())?;
            let payload_json = row.get::<_, String>(1).map_err(|err| err.to_string())?;
            let msg: OutboxMessage = from_payload_json(payload_json)?;
            let is_protected = msg
                .source_inbox_ids
                .iter()
                .any(|iid| protected_inbox_ids.contains(iid));
            if !is_protected {
                ids.push(id);
            }
        }
        for id in &ids {
            conn.execute("DELETE FROM outbox WHERE id = ?1", [id.as_str()])
                .map_err(|err| err.to_string())?;
        }
        Ok(ids)
    })
}

pub fn delete_turns_older_than(
    cutoff_ns: u64,
    limit: usize,
) -> Result<Vec<(String, TurnRecord)>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, payload_json FROM turns WHERE created_at_ns < ?1 ORDER BY created_at_ns ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query((cutoff_ns as i64, limit as i64))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().map_err(|err| err.to_string())? {
            let id = row.get::<_, String>(0).map_err(|err| err.to_string())?;
            let payload_json = row.get::<_, String>(1).map_err(|err| err.to_string())?;
            let turn: TurnRecord = from_payload_json(payload_json)?;
            records.push((id, turn));
        }
        if !records.is_empty() {
            conn.execute(
                "DELETE FROM tool_calls WHERE turn_id IN (SELECT id FROM turns WHERE created_at_ns < ?1 ORDER BY created_at_ns ASC LIMIT ?2)",
                (cutoff_ns as i64, limit as i64),
            ).map_err(|err| err.to_string())?;
            conn.execute(
                "DELETE FROM turns WHERE id IN (SELECT id FROM turns WHERE created_at_ns < ?1 ORDER BY created_at_ns ASC LIMIT ?2)",
                (cutoff_ns as i64, limit as i64),
            ).map_err(|err| err.to_string())?;
        }
        Ok(records)
    })
}

pub fn delete_transitions_older_than(
    cutoff_ns: u64,
    limit: usize,
) -> Result<Vec<(String, TransitionLogRecord)>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, payload_json FROM transitions WHERE occurred_at_ns < ?1 ORDER BY occurred_at_ns ASC LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let mut rows = stmt
            .query((cutoff_ns as i64, limit as i64))
            .map_err(|err| err.to_string())?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().map_err(|err| err.to_string())? {
            let id = row.get::<_, String>(0).map_err(|err| err.to_string())?;
            let payload_json = row.get::<_, String>(1).map_err(|err| err.to_string())?;
            let transition: TransitionLogRecord = from_payload_json(payload_json)?;
            records.push((id, transition));
        }
        if !records.is_empty() {
            conn.execute(
                "DELETE FROM transitions WHERE id IN (SELECT id FROM transitions WHERE occurred_at_ns < ?1 ORDER BY occurred_at_ns ASC LIMIT ?2)",
                (cutoff_ns as i64, limit as i64),
            ).map_err(|err| err.to_string())?;
        }
        Ok(records)
    })
}

// -- Extend table_count for new tables --

pub fn table_count_extended(table: &str) -> Result<u64, String> {
    let table_name = match table {
        "transitions"
        | "turns"
        | "journal"
        | "tool_calls"
        | "inbox"
        | "outbox"
        | "conversations"
        | "jobs"
        | "memory_facts"
        | "skills"
        | "strategy_templates"
        | "abi_artifacts"
        | "hot_runtime_snapshot"
        | "hot_scheduler_runtime"
        | "hot_task_configs"
        | "hot_task_runtimes"
        | "hot_topup_state"
        | "hot_survival_operation_runtime"
        | "http_domain_allowlist"
        | "prompt_layers"
        | "retention_runtime"
        | "session_summaries"
        | "turn_window_summaries"
        | "memory_rollups"
        | "strategy_activations"
        | "strategy_revocations"
        | "strategy_kill_switches"
        | "strategy_outcome_stats"
        | "strategy_budgets"
        | "autonomy_tool_failures"
        | "reflection_memory"
        | "autonomy_policy"
        | "active_exposures"
        | "strategy_quarantines"
        | "decision_records"
        | "exposure_reconciliation_status"
        | "runtime_scalars" => table,
        _ => return Err(format!("unsupported table: {table}")),
    };
    backend::with_connection(|conn| {
        let query = format!("SELECT COUNT(1) FROM {table_name}");
        let count: i64 = conn
            .query_row(&query, [], |row| row.get(0))
            .map_err(|err| err.to_string())?;
        Ok(u64::try_from(count).unwrap_or(0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{
        ActionSpec, ActiveExposure, AgentEvent, AgentState, AutonomyPolicy, ContractRoleBinding,
        DecisionOutcome, DecisionRecord, DecisionTrigger, ExposureReconciliationStatus,
        InboxMessageSource, InboxMessageStatus, JobStatus, ReflectionMemoryRecord,
        ReflectionOrigin, RuntimeSnapshot, SchedulerRuntime, StrategyQuarantine,
        SurvivalOperationClass, TaskKind, TaskLane, TaskScheduleConfig, TaskScheduleRuntime,
        TemplateStatus, ToolCallRecord,
    };
    use crate::features::cycle_topup::TopUpStage;

    fn sample_turn(id: &str, created_at_ns: u64) -> TurnRecord {
        TurnRecord {
            id: id.to_string(),
            created_at_ns,
            finished_at_ns: Some(created_at_ns.saturating_add(1)),
            duration_ms: Some(1),
            state_from: AgentState::Idle,
            state_to: AgentState::Persisting,
            source_events: 1,
            tool_call_count: 1,
            input_summary: format!("summary-{id}"),
            inner_dialogue: Some("ok".to_string()),
            inference_round_count: 1,
            continuation_stop_reason: crate::domain::types::ContinuationStopReason::None,
            error: None,
        }
    }

    #[test]
    fn connection_lifecycle() {
        init_storage().expect("init sqlite");
        assert!(schema_version().expect("schema version") >= 1);
        close_storage().expect("close sqlite");
        init_storage().expect("reopen sqlite");
    }

    #[test]
    fn migrations_reach_version_one() {
        close_storage().expect("close before migration test");
        init_storage().expect("init sqlite");
        assert_eq!(
            schema_version().expect("schema version"),
            CURRENT_SCHEMA_VERSION as u64
        );
    }

    #[test]
    fn schema_contains_required_historical_tables() {
        init_storage().expect("init sqlite");
        for table in [
            "transitions",
            "turns",
            "tool_calls",
            "inbox",
            "outbox",
            "conversations",
            "jobs",
            "memory_facts",
            "skills",
            "strategy_templates",
            "abi_artifacts",
            "hot_runtime_snapshot",
            "hot_scheduler_runtime",
            "hot_task_configs",
            "hot_task_runtimes",
            "hot_topup_state",
            "hot_survival_operation_runtime",
        ] {
            assert_eq!(table_count(table).expect("table count"), 0, "table {table}");
        }
    }

    #[test]
    fn reflection_memory_schema_is_present() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");
        assert_eq!(
            table_count_extended("reflection_memory").expect("reflection_memory count"),
            0
        );

        let has_index = backend::with_connection(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    ["idx_reflection_memory_updated_at"],
                    |row| row.get(0),
                )
                .map_err(|err| err.to_string())?;
            Ok(count)
        })
        .expect("index lookup should succeed");
        assert_eq!(has_index, 1);
    }

    #[test]
    fn autonomy_runtime_schema_is_present() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        for table in [
            "autonomy_policy",
            "active_exposures",
            "strategy_quarantines",
            "decision_records",
            "exposure_reconciliation_status",
        ] {
            assert_eq!(
                table_count_extended(table).expect("autonomy runtime table count"),
                0,
                "table {table}"
            );
        }
    }

    #[test]
    fn reflection_memory_payload_round_trip_uses_typed_record() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        let record = ReflectionMemoryRecord {
            key: "evm_read:eth_call:missing_calldata".to_string(),
            tool: "evm_read".to_string(),
            subject: "eth_call".to_string(),
            error_class: "missing_calldata".to_string(),
            what_failed: "evm_read[eth_call] failed: calldata missing; successful calls require address + calldata".to_string(),
            what_worked: Some("worked recently with address + calldata".to_string()),
            degraded_turn_count: 2,
            repeat_count: 2,
            last_failed_at_ns: 1_000,
            last_failed_turn_id: "turn-1".to_string(),
            last_worked_at_ns: Some(2_000),
            last_worked_turn_id: Some("turn-2".to_string()),
            last_origin: ReflectionOrigin::Autonomy,
            updated_at_ns: 2_000,
        };
        let payload_json = row_payload(&record).expect("serialize reflection memory");

        backend::with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO reflection_memory(key, updated_at_ns, payload_json) VALUES(?1, ?2, ?3)",
                (&record.key, record.updated_at_ns as i64, payload_json),
            )
            .map_err(|err| err.to_string())?;
            Ok(())
        })
        .expect("insert reflection memory row");

        let stored: ReflectionMemoryRecord = backend::with_connection(|conn| {
            let payload_json = conn
                .query_row(
                    "SELECT payload_json FROM reflection_memory WHERE key = ?1",
                    [record.key.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|err| err.to_string())?;
            from_payload_json(payload_json)
        })
        .expect("load reflection memory row");

        assert_eq!(stored, record);
        assert_eq!(
            table_count_extended("reflection_memory").expect("reflection_memory count"),
            1
        );
    }

    #[test]
    fn reflection_memory_crud_helpers_round_trip_and_sort_newest_first() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        let older = ReflectionMemoryRecord {
            key: "market_fetch:dexscreener:missing_param".to_string(),
            tool: "market_fetch".to_string(),
            subject: "dexscreener".to_string(),
            error_class: "missing_param".to_string(),
            what_failed: "market_fetch[dexscreener] failed".to_string(),
            what_worked: None,
            degraded_turn_count: 1,
            repeat_count: 1,
            last_failed_at_ns: 100,
            last_failed_turn_id: "turn-1".to_string(),
            last_worked_at_ns: None,
            last_worked_turn_id: None,
            last_origin: ReflectionOrigin::Autonomy,
            updated_at_ns: 100,
        };
        let newer = ReflectionMemoryRecord {
            key: "evm_read:eth_call:missing_calldata".to_string(),
            tool: "evm_read".to_string(),
            subject: "eth_call".to_string(),
            error_class: "missing_calldata".to_string(),
            what_failed: "evm_read[eth_call] failed".to_string(),
            what_worked: Some("worked recently with address + calldata".to_string()),
            degraded_turn_count: 2,
            repeat_count: 3,
            last_failed_at_ns: 200,
            last_failed_turn_id: "turn-2".to_string(),
            last_worked_at_ns: Some(210),
            last_worked_turn_id: Some("turn-3".to_string()),
            last_origin: ReflectionOrigin::Autonomy,
            updated_at_ns: 210,
        };

        upsert_reflection_memory(&older).expect("older reflection should persist");
        upsert_reflection_memory(&newer).expect("newer reflection should persist");

        assert_eq!(count_reflection_memory().expect("reflection count"), 2);
        assert_eq!(
            get_reflection_memory(&older.key).expect("load older reflection"),
            Some(older.clone())
        );

        let listed = list_reflection_memory(10).expect("list reflections");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0], newer);
        assert_eq!(listed[1], older);

        delete_reflection_memory("market_fetch:dexscreener:missing_param")
            .expect("older reflection should delete");
        assert_eq!(count_reflection_memory().expect("reflection count"), 1);
    }

    #[test]
    fn reflection_memory_prune_and_keep_helpers_remove_oldest_records() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        for index in 0..3u64 {
            let record = ReflectionMemoryRecord {
                key: format!("tool:{index}:error"),
                tool: "tool".to_string(),
                subject: format!("subject-{index}"),
                error_class: "error".to_string(),
                what_failed: format!("failed-{index}"),
                what_worked: None,
                degraded_turn_count: 1,
                repeat_count: 1,
                last_failed_at_ns: index,
                last_failed_turn_id: format!("turn-{index}"),
                last_worked_at_ns: None,
                last_worked_turn_id: None,
                last_origin: ReflectionOrigin::Autonomy,
                updated_at_ns: index,
            };
            upsert_reflection_memory(&record).expect("reflection should persist");
        }

        let pruned = prune_reflection_memory(2, 10).expect("stale reflections should prune");
        assert_eq!(
            pruned,
            vec!["tool:0:error".to_string(), "tool:1:error".to_string()]
        );
        assert_eq!(count_reflection_memory().expect("reflection count"), 1);

        for index in 3..6u64 {
            let record = ReflectionMemoryRecord {
                key: format!("tool:{index}:error"),
                tool: "tool".to_string(),
                subject: format!("subject-{index}"),
                error_class: "error".to_string(),
                what_failed: format!("failed-{index}"),
                what_worked: None,
                degraded_turn_count: 1,
                repeat_count: 1,
                last_failed_at_ns: index,
                last_failed_turn_id: format!("turn-{index}"),
                last_worked_at_ns: None,
                last_worked_turn_id: None,
                last_origin: ReflectionOrigin::Autonomy,
                updated_at_ns: index,
            };
            upsert_reflection_memory(&record).expect("reflection should persist");
        }

        let deleted = delete_oldest_reflection_memory(2).expect("cap pruning should succeed");
        assert_eq!(deleted, 2);
        let listed = list_reflection_memory(10).expect("list reflections");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].key, "tool:5:error");
        assert_eq!(listed[1].key, "tool:4:error");
    }

    #[test]
    fn autonomy_runtime_helpers_round_trip_and_bound_decision_fifo() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        let policy = AutonomyPolicy::conservative_default(42);
        write_autonomy_policy(&policy).expect("policy should persist");
        assert_eq!(
            read_autonomy_policy().expect("policy should load"),
            Some(policy.clone())
        );

        let exposure = ActiveExposure {
            strategy_id: "aave-enter".to_string(),
            protocol: "aave".to_string(),
            chain_id: 8453,
            asset_symbol: "ETH".to_string(),
            notional_wei: Some(123_000_000_000_000_000),
            asset_address: None,
            decimals: Some(18),
            amount_raw: Some("123000000000000000".to_string()),
            updated_at_ns: 100,
        };
        upsert_active_exposure(&exposure).expect("exposure should persist");
        assert_eq!(
            get_active_exposure(&exposure.strategy_id).expect("exposure should load"),
            Some(exposure.clone())
        );
        assert_eq!(count_active_exposures().expect("exposure count"), 1);

        let quarantine = StrategyQuarantine {
            strategy_id: exposure.strategy_id.clone(),
            reason: "repeated_failure".to_string(),
            failure_count: 3,
            quarantined_at_ns: 120,
            release_after_ns: Some(200),
        };
        upsert_strategy_quarantine(&quarantine).expect("quarantine should persist");
        assert_eq!(
            get_strategy_quarantine(&quarantine.strategy_id).expect("quarantine should load"),
            Some(quarantine.clone())
        );
        assert_eq!(count_strategy_quarantines().expect("quarantine count"), 1);

        for index in 0..205u64 {
            let record = DecisionRecord {
                turn_id: format!("turn-{index:03}"),
                timestamp_ns: 1_000 + index,
                trigger: DecisionTrigger::ScheduledReview,
                outcome: DecisionOutcome::NoOp {
                    reason: format!("reason-{index}"),
                },
                policy_version: 1,
                candidates_summary: format!("candidate-{index}"),
                explanation: format!("explanation-{index}"),
            };
            append_decision_record(&record, 200).expect("decision record should persist");
        }

        assert_eq!(count_decision_records().expect("decision count"), 200);
        assert!(
            get_decision_record("turn-000")
                .expect("decision lookup should succeed")
                .is_none(),
            "oldest decision should be evicted"
        );
        assert!(
            get_decision_record("turn-004")
                .expect("decision lookup should succeed")
                .is_none(),
            "fifo cap should evict five oldest records"
        );
        assert_eq!(
            get_decision_record("turn-005").expect("decision lookup should succeed"),
            Some(DecisionRecord {
                turn_id: "turn-005".to_string(),
                timestamp_ns: 1_005,
                trigger: DecisionTrigger::ScheduledReview,
                outcome: DecisionOutcome::NoOp {
                    reason: "reason-5".to_string(),
                },
                policy_version: 1,
                candidates_summary: "candidate-5".to_string(),
                explanation: "explanation-5".to_string(),
            })
        );
        let listed = list_recent_decision_records(3).expect("recent decisions should list");
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].turn_id, "turn-204");
        assert_eq!(listed[1].turn_id, "turn-203");
        assert_eq!(listed[2].turn_id, "turn-202");

        let reconciliation = ExposureReconciliationStatus {
            last_attempted_at_ns: Some(500),
            last_succeeded_at_ns: Some(510),
            repaired_exposures: 1,
            recreated_exposures: 2,
            closed_exposures: 3,
            drift_reason: Some("execution_repair".to_string()),
            last_error: None,
        };
        write_exposure_reconciliation_status(&reconciliation)
            .expect("reconciliation status should persist");
        assert_eq!(
            read_exposure_reconciliation_status().expect("reconciliation status should load"),
            Some(reconciliation)
        );
    }

    #[test]
    fn hot_state_round_trip_uses_cache() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        let snapshot = RuntimeSnapshot {
            loop_enabled: false,
            turn_counter: 42,
            ..RuntimeSnapshot::default()
        };
        write_runtime_snapshot(&snapshot).expect("runtime snapshot write");

        let scheduler_runtime = SchedulerRuntime {
            enabled: false,
            paused_reason: Some("test".to_string()),
            ..SchedulerRuntime::default()
        };
        write_scheduler_runtime(&scheduler_runtime).expect("scheduler runtime write");

        let mut task_config = TaskScheduleConfig::default_for(&TaskKind::CheckCycles);
        task_config.interval_secs = 77;
        write_task_config(&task_config).expect("task config write");

        let task_runtime = TaskScheduleRuntime {
            kind: TaskKind::CheckCycles,
            next_due_ns: 999,
            backoff_until_ns: Some(1_111),
            consecutive_failures: 3,
            pending_job_id: Some("job-1".to_string()),
            last_started_ns: Some(1),
            last_finished_ns: Some(2),
            last_error: Some("boom".to_string()),
        };
        write_task_runtime(&TaskKind::CheckCycles, &task_runtime).expect("task runtime write");

        let topup_state = TopUpStage::Preflight;
        write_topup_state(&topup_state).expect("topup state write");

        write_survival_operation_runtime(
            &SurvivalOperationClass::Inference,
            &SurvivalOperationRuntimeRecord {
                consecutive_failures: 2,
                backoff_until_ns: Some(500),
            },
        )
        .expect("survival runtime write");

        let loaded_snapshot = read_runtime_snapshot()
            .expect("runtime snapshot read")
            .expect("runtime snapshot should exist");
        assert_eq!(loaded_snapshot.turn_counter, 42);
        assert!(!loaded_snapshot.loop_enabled);

        let loaded_scheduler = read_scheduler_runtime()
            .expect("scheduler runtime read")
            .expect("scheduler runtime should exist");
        assert!(!loaded_scheduler.enabled);
        assert_eq!(loaded_scheduler.paused_reason.as_deref(), Some("test"));

        let loaded_task_config = read_task_config(&TaskKind::CheckCycles)
            .expect("task config read")
            .expect("task config should exist");
        assert_eq!(loaded_task_config.interval_secs, 77);

        let loaded_task_runtime = read_task_runtime(&TaskKind::CheckCycles)
            .expect("task runtime read")
            .expect("task runtime should exist");
        assert_eq!(loaded_task_runtime.pending_job_id.as_deref(), Some("job-1"));

        let loaded_topup_state = read_topup_state()
            .expect("topup state read")
            .expect("topup state should exist");
        assert!(matches!(loaded_topup_state, TopUpStage::Preflight));

        let loaded_survival = read_survival_operation_runtime(&SurvivalOperationClass::Inference)
            .expect("survival runtime read");
        assert_eq!(loaded_survival.consecutive_failures, 2);
        assert_eq!(loaded_survival.backoff_until_ns, Some(500));

        clear_topup_state().expect("topup clear");
        assert!(read_topup_state()
            .expect("topup read after clear")
            .is_none());
    }

    #[test]
    fn writes_insert_rows() {
        init_storage().expect("init sqlite");
        upsert_transition(&TransitionLogRecord {
            id: "t1".to_string(),
            turn_id: "turn-1".to_string(),
            from_state: AgentState::Bootstrapping,
            to_state: AgentState::Idle,
            event: format!("{:?}", AgentEvent::TimerTick),
            error: None,
            occurred_at_ns: 10,
        })
        .expect("transition");
        upsert_turn(&TurnRecord {
            id: "turn-1".to_string(),
            created_at_ns: 10,
            finished_at_ns: Some(11),
            duration_ms: Some(1),
            state_from: AgentState::Idle,
            state_to: AgentState::Persisting,
            source_events: 1,
            tool_call_count: 1,
            input_summary: "summary".to_string(),
            inner_dialogue: Some("ok".to_string()),
            inference_round_count: 1,
            continuation_stop_reason: crate::domain::types::ContinuationStopReason::None,
            error: None,
        })
        .expect("turn");
        replace_tool_calls(
            "turn-1",
            &[ToolCallRecord {
                turn_id: "turn-1".to_string(),
                tool: "recall".to_string(),
                args_json: "{\"q\":\"x\"}".to_string(),
                output: "[]".to_string(),
                success: true,
                outcome: crate::domain::types::ToolCallOutcome::Executed,
                error: None,
                failure_kind: None,
            }],
        )
        .expect("tools");
        upsert_job(&ScheduledJob {
            id: "job-1".to_string(),
            kind: TaskKind::PollInbox,
            lane: TaskLane::Mutating,
            dedupe_key: "k".to_string(),
            priority: 1,
            created_at_ns: 20,
            scheduled_for_ns: 20,
            started_at_ns: None,
            finished_at_ns: None,
            status: crate::domain::types::JobStatus::Pending,
            attempts: 0,
            max_attempts: 3,
            last_error: None,
        })
        .expect("job");
        upsert_strategy_template(&StrategyTemplate {
            key: crate::domain::types::StrategyTemplateKey {
                protocol: "p".to_string(),
                primitive: "q".to_string(),
                chain_id: 8453,
                template_id: "id".to_string(),
            },
            status: TemplateStatus::Draft,
            contract_roles: vec![ContractRoleBinding {
                role: "pool".to_string(),
                address: "0x1111111111111111111111111111111111111111".to_string(),
                source_ref: "test".to_string(),
                codehash: None,
            }],
            actions: vec![ActionSpec {
                action_id: "a1".to_string(),
                call_sequence: vec![],
                preconditions: vec![],
                postconditions: vec![],
                risk_checks: vec![],
            }],
            constraints_json: "{}".to_string(),
            created_at_ns: 1,
            updated_at_ns: 2,
        })
        .expect("template");
        assert_eq!(table_count("transitions").expect("count"), 1);
        assert_eq!(table_count("turns").expect("count"), 1);
        assert_eq!(table_count("tool_calls").expect("count"), 1);
        assert_eq!(table_count("jobs").expect("count"), 1);
        assert_eq!(table_count("strategy_templates").expect("count"), 1);
    }

    #[test]
    fn read_paths_return_expected_records() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        upsert_turn(&sample_turn("turn-1", 10)).expect("turn 1");
        upsert_turn(&sample_turn("turn-2", 20)).expect("turn 2");
        replace_tool_calls(
            "turn-2",
            &[ToolCallRecord {
                turn_id: "turn-2".to_string(),
                tool: "memory_stats".to_string(),
                args_json: "{}".to_string(),
                output: "{\"total_facts\":0}".to_string(),
                success: true,
                outcome: crate::domain::types::ToolCallOutcome::Executed,
                error: None,
                failure_kind: None,
            }],
        )
        .expect("tool calls");
        upsert_inbox(&InboxMessage {
            id: "inbox:00000000000000000001".to_string(),
            seq: 1,
            body: "hello".to_string(),
            posted_at_ns: 11,
            posted_by: "alice".to_string(),
            source: InboxMessageSource::EvmInbox,
            status: InboxMessageStatus::Pending,
            staged_at_ns: None,
            consumed_at_ns: None,
        })
        .expect("inbox one");
        upsert_inbox(&InboxMessage {
            id: "inbox:00000000000000000002".to_string(),
            seq: 2,
            body: "hello again".to_string(),
            posted_at_ns: 12,
            posted_by: "alice".to_string(),
            source: InboxMessageSource::EvmInbox,
            status: InboxMessageStatus::Consumed,
            staged_at_ns: Some(13),
            consumed_at_ns: Some(14),
        })
        .expect("inbox two");
        upsert_outbox(&OutboxMessage {
            id: "outbox:00000000000000000001".to_string(),
            seq: 1,
            turn_id: "turn-1".to_string(),
            body: "reply".to_string(),
            created_at_ns: 21,
            source_inbox_ids: vec!["inbox:00000000000000000001".to_string()],
        })
        .expect("outbox");
        append_conversation(
            "alice",
            &ConversationEntry {
                inbox_message_id: "inbox:00000000000000000001".to_string(),
                outbox_message_id: Some("outbox:00000000000000000001".to_string()),
                sender_body: "hello".to_string(),
                agent_reply: "reply".to_string(),
                turn_id: "turn-1".to_string(),
                timestamp_ns: 30,
            },
        )
        .expect("conversation");
        upsert_job(&ScheduledJob {
            id: "job:00000000000000000001:00000000000000000001".to_string(),
            kind: TaskKind::PollInbox,
            lane: TaskLane::Mutating,
            dedupe_key: "poll:1".to_string(),
            priority: 1,
            created_at_ns: 50,
            scheduled_for_ns: 50,
            started_at_ns: None,
            finished_at_ns: None,
            status: JobStatus::Pending,
            attempts: 0,
            max_attempts: 3,
            last_error: None,
        })
        .expect("job");

        let turns = list_turns(1).expect("list turns");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-2");

        let tools = get_tools_for_turn("turn-2").expect("tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool, "memory_stats");

        let inbox = list_inbox_messages(2).expect("inbox list");
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[0].seq, 2);

        let outbox = list_outbox_messages(1).expect("outbox list");
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].id, "outbox:00000000000000000001");

        let conversation = get_conversation_log("alice")
            .expect("conversation lookup")
            .expect("conversation exists");
        assert_eq!(conversation.entries.len(), 1);
        assert_eq!(conversation.last_activity_ns, 30);

        let jobs = list_recent_jobs(1).expect("jobs list");
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].id.starts_with("job:"));
    }

    #[test]
    fn strategy_and_abi_read_paths_use_sqlite_storage() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        let key = crate::domain::types::StrategyTemplateKey {
            protocol: "p".to_string(),
            primitive: "q".to_string(),
            chain_id: 8453,
            template_id: "id".to_string(),
        };
        let template = StrategyTemplate {
            key: key.clone(),
            status: TemplateStatus::Draft,
            contract_roles: vec![ContractRoleBinding {
                role: "pool".to_string(),
                address: "0x1111111111111111111111111111111111111111".to_string(),
                source_ref: "test".to_string(),
                codehash: None,
            }],
            actions: vec![ActionSpec {
                action_id: "a1".to_string(),
                call_sequence: vec![],
                preconditions: vec![],
                postconditions: vec![],
                risk_checks: vec![],
            }],
            constraints_json: "{}".to_string(),
            created_at_ns: 1,
            updated_at_ns: 2,
        };
        upsert_strategy_template(&template).expect("template");

        let loaded = strategy_template(&key)
            .expect("query strategy")
            .expect("exists");
        assert_eq!(loaded.updated_at_ns, 2);

        let listed = list_strategy_templates(&key, 10).expect("templates list");
        assert_eq!(listed.len(), 1);

        let artifact = AbiArtifact {
            key: AbiArtifactKey {
                protocol: key.protocol.clone(),
                chain_id: key.chain_id,
                role: "pool".to_string(),
            },
            source_ref: "src".to_string(),
            codehash: None,
            abi_json: "[]".to_string(),
            functions: Vec::new(),
            created_at_ns: 5,
            updated_at_ns: 5,
        };
        upsert_abi_artifact(&artifact).expect("artifact");
        assert!(abi_artifact(&artifact.key).expect("abi query").is_some());
    }

    #[test]
    fn sql_query_tooling_enforces_read_only_and_limits_rows() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        upsert_turn(&sample_turn("turn-1", 10)).expect("turn 1");
        upsert_turn(&sample_turn("turn-2", 20)).expect("turn 2");
        upsert_turn(&sample_turn("turn-3", 30)).expect("turn 3");

        let output = sql_query_read_only("SELECT id FROM turns ORDER BY created_at_ns DESC", 2)
            .expect("sql query");
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&output).expect("query output should be valid json");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], "turn-3");

        let denied = sql_query_read_only("UPDATE turns SET id = 'nope'", 10)
            .expect_err("non-SELECT must be rejected");
        assert!(denied.contains("only SELECT"));

        let denied_multi = sql_query_read_only("SELECT 1; SELECT 2", 10)
            .expect_err("multi statement must be rejected");
        assert!(denied_multi.contains("single SELECT"));
    }

    #[test]
    fn strategy_execution_storage_round_trip_due_order_and_duplicate_idempotency() {
        use crate::domain::types::{
            PendingStrategyExecutionCall, PendingStrategyExecutionState, StrategyExecutionCall,
            StrategyExecutionCallState, StrategyTemplateKey,
        };
        close_storage().unwrap();
        init_storage().unwrap();
        let make = |id: &str, created_at_ns: u64, next_check_at_ns: u64| PendingStrategyExecution {
            execution_id: id.to_string(),
            turn_id: format!("turn-{id}"),
            key: StrategyTemplateKey {
                protocol: "p".into(),
                primitive: "lend".into(),
                chain_id: 8453,
                template_id: "t".into(),
            },
            action_id: "enter_supply".into(),
            plan_digest: format!("digest-{id}"),
            asset_effects: vec![],
            calls: vec![PendingStrategyExecutionCall {
                index: 0,
                call: StrategyExecutionCall {
                    role: "pool".into(),
                    to: "0x1111111111111111111111111111111111111111".into(),
                    value_wei: "0".into(),
                    data: "0x".into(),
                },
                tx_hash: Some("0xabc".into()),
                state: StrategyExecutionCallState::Submitted,
                receipt_block_number: None,
                receipt_block_hash: None,
                submitted_at_ns: Some(created_at_ns),
                last_checked_at_ns: None,
                error: None,
            }],
            state: PendingStrategyExecutionState::Pending,
            created_at_ns,
            updated_at_ns: created_at_ns,
            next_check_at_ns,
            consecutive_rpc_failures: 0,
            bookkeeping_applied: false,
            terminal_bookkeeping_applied: false,
        };
        let later = make("later", 20, 20);
        let earlier = make("earlier", 10, 10);
        assert_eq!(insert_pending_strategy_execution(&later).unwrap(), later);
        assert_eq!(
            insert_pending_strategy_execution(&earlier).unwrap(),
            earlier
        );
        assert_eq!(
            insert_pending_strategy_execution(&earlier).unwrap(),
            earlier
        );
        assert_eq!(
            get_pending_strategy_execution("earlier").unwrap(),
            Some(earlier.clone())
        );
        let due = list_due_pending_strategy_executions(20, 10).unwrap();
        assert_eq!(
            due.iter()
                .map(|item| item.execution_id.as_str())
                .collect::<Vec<_>>(),
            vec!["earlier", "later"]
        );
        let mut updated = earlier.clone();
        updated.bookkeeping_applied = true;
        update_pending_strategy_execution(&updated).unwrap();
        // Re-running initialization models the post-upgrade migration path on
        // the same stable SQLite database and must preserve the JSON payload.
        init_storage().unwrap();
        assert!(
            get_pending_strategy_execution("earlier")
                .unwrap()
                .unwrap()
                .bookkeeping_applied
        );
    }
}
