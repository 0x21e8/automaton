//! SQLite storage adapter used during the StableBTreeMap -> SQL migration.
//!
//! Phase 1 keeps stable maps as source of truth and dual-writes historical
//! collections into SQLite for parity checks and backfill validation.

use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, ConversationEntry, ConversationLog, InboxMessage, MemoryFact,
    OutboxMessage, RuntimeSnapshot, ScheduledJob, SchedulerRuntime, SkillRecord,
    StrategyTemplate, StrategyTemplateKey, SurvivalOperationClass, TaskKind, TaskScheduleConfig,
    TaskScheduleRuntime, TemplateVersion, ToolCallRecord, TransitionLogRecord, TurnRecord,
};
use crate::features::cycle_topup::TopUpStage;
#[cfg(target_arch = "wasm32")]
use ic_rusqlite::rusqlite::types::ValueRef as SqlValueRef;
#[cfg(not(target_arch = "wasm32"))]
use rusqlite::types::ValueRef as SqlValueRef;
use serde_json::Value;
use serde_json::{Map as JsonMap, Number as JsonNumber};
use std::cell::RefCell;
use std::collections::BTreeMap;

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

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use super::{MIGRATION_001_BASE_SCHEMA, MIGRATION_002_HOT_STATE_SCHEMA, MIGRATION_003_REMAINING_SCHEMA};
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
        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let now = crate::timing::current_time_ns() as i64;
        for v in (version + 1)..=3 {
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
    use super::{MIGRATION_001_BASE_SCHEMA, MIGRATION_002_HOT_STATE_SCHEMA, MIGRATION_003_REMAINING_SCHEMA};

    pub type SqlResult<T> = Result<T, String>;

    pub fn init() -> SqlResult<()> {
        ic_rusqlite::with_connection(|conn| {
            conn.execute_batch(MIGRATION_001_BASE_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_002_HOT_STATE_SCHEMA)
                .map_err(|err| err.to_string())?;
            conn.execute_batch(MIGRATION_003_REMAINING_SCHEMA)
                .map_err(|err| err.to_string())?;
            let version: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| row.get(0),
                )
                .map_err(|err| err.to_string())?;
            let now = crate::timing::current_time_ns() as i64;
            for v in (version + 1)..=3 {
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

fn canonicalize_version_list(mut versions: Vec<TemplateVersion>) -> Vec<TemplateVersion> {
    versions.sort();
    versions.dedup();
    versions.reverse();
    versions
}

fn strategy_template_pk(key: &StrategyTemplateKey, version: &TemplateVersion) -> String {
    format!(
        "{}:{}:{}:{}@{}.{}.{}",
        key.protocol, key.primitive, key.chain_id, key.template_id,
        version.major, version.minor, version.patch
    )
}

fn abi_artifact_pk(key: &AbiArtifactKey) -> String {
    format!(
        "{}:{}:{}@{}.{}.{}",
        key.protocol, key.chain_id, key.role,
        key.version.major, key.version.minor, key.version.patch
    )
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
                .prepare(
                    "SELECT operation_key, payload_json FROM hot_survival_operation_runtime",
                )
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
                kind_key.parse().ok().map(|kind: TaskKind| (kind, config.clone()))
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

pub fn upsert_strategy_template(template: &StrategyTemplate) -> Result<(), String> {
    let payload_json = row_payload(template)?;
    let template_id = strategy_template_pk(&template.key, &template.version);
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

pub fn strategy_template(
    key: &StrategyTemplateKey,
    version: &TemplateVersion,
) -> Result<Option<StrategyTemplate>, String> {
    let pk = strategy_template_pk(key, version);
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

pub fn list_strategy_template_versions(
    key: &StrategyTemplateKey,
) -> Result<Vec<TemplateVersion>, String> {
    let pattern = format!(
        "{}:{}:{}:{}@%",
        key.protocol, key.primitive, key.chain_id, key.template_id
    );
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_templates
                 WHERE template_id LIKE ?1",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map([pattern.as_str()], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        let mut versions = Vec::new();
        for row in rows {
            let template: StrategyTemplate =
                from_payload_json(row.map_err(|err| err.to_string())?)?;
            versions.push(template.version);
        }
        Ok(canonicalize_version_list(versions))
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
    let pattern = format!(
        "{}:{}:{}:{}@%",
        key.protocol, key.primitive, key.chain_id, key.template_id
    );
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_templates
                 WHERE template_id LIKE ?1
                 ORDER BY updated_at_ns DESC
                 LIMIT ?2",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((pattern.as_str(), keep as i64), |row| {
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

pub fn list_abi_artifact_versions(
    protocol: &str,
    chain_id: u64,
    role: &str,
) -> Result<Vec<TemplateVersion>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM abi_artifacts
                 WHERE protocol = ?1 AND role = ?2 AND chain_id = ?3",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((protocol, role, chain_id as i64), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        let mut versions = Vec::new();
        for row in rows {
            let artifact: AbiArtifact = from_payload_json(row.map_err(|err| err.to_string())?)?;
            versions.push(artifact.key.version);
        }
        Ok(canonicalize_version_list(versions))
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
            if output.len() % 25 == 0
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
        "transitions" | "turns" | "tool_calls" | "inbox" | "outbox" | "conversations" | "jobs"
        | "memory_facts" | "skills" | "strategy_templates" | "abi_artifacts"
        | "hot_runtime_snapshot" | "hot_scheduler_runtime" | "hot_task_configs"
        | "hot_task_runtimes" | "hot_topup_state" | "hot_survival_operation_runtime" => table,
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
            .prepare(
                "SELECT payload_json FROM session_summaries ORDER BY date_key DESC LIMIT ?1",
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

pub fn list_memory_rollups<T: serde::de::DeserializeOwned>(
    limit: usize,
) -> Result<Vec<T>, String> {
    let keep = bounded_limit(limit, 25, 128);
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM memory_rollups ORDER BY rollup_key DESC LIMIT ?1",
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
            .prepare(
                "SELECT payload_json FROM memory_facts ORDER BY key ASC LIMIT ?1",
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

pub fn delete_inbox_older_than(cutoff_ns: u64, limit: usize, protected_ids: &[String]) -> Result<Vec<String>, String> {
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

pub fn delete_outbox_older_than(cutoff_ns: u64, limit: usize, protected_inbox_ids: &[String]) -> Result<Vec<String>, String> {
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
            let is_protected = msg.source_inbox_ids.iter().any(|iid| protected_inbox_ids.contains(iid));
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

pub fn delete_turns_older_than(cutoff_ns: u64, limit: usize) -> Result<Vec<(String, TurnRecord)>, String> {
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

pub fn delete_transitions_older_than(cutoff_ns: u64, limit: usize) -> Result<Vec<(String, TransitionLogRecord)>, String> {
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
        "transitions" | "turns" | "tool_calls" | "inbox" | "outbox" | "conversations" | "jobs"
        | "memory_facts" | "skills" | "strategy_templates" | "abi_artifacts"
        | "hot_runtime_snapshot" | "hot_scheduler_runtime" | "hot_task_configs"
        | "hot_task_runtimes" | "hot_topup_state" | "hot_survival_operation_runtime"
        | "http_domain_allowlist" | "prompt_layers" | "retention_runtime"
        | "session_summaries" | "turn_window_summaries" | "memory_rollups"
        | "strategy_activations" | "strategy_revocations" | "strategy_kill_switches"
        | "strategy_outcome_stats" | "strategy_budgets" | "autonomy_tool_failures"
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
        ActionSpec, AgentEvent, AgentState, ContractRoleBinding, InboxMessageStatus, JobStatus,
        RuntimeSnapshot, SchedulerRuntime, SurvivalOperationClass, TaskKind, TaskLane,
        TaskScheduleConfig, TaskScheduleRuntime, TemplateStatus, TemplateVersion, ToolCallRecord,
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
        assert_eq!(schema_version().expect("schema version"), 3);
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
    fn hot_state_round_trip_uses_cache() {
        close_storage().expect("reset sqlite");
        init_storage().expect("init sqlite");

        let mut snapshot = RuntimeSnapshot::default();
        snapshot.loop_enabled = false;
        snapshot.turn_counter = 42;
        write_runtime_snapshot(&snapshot).expect("runtime snapshot write");

        let mut scheduler_runtime = SchedulerRuntime::default();
        scheduler_runtime.enabled = false;
        scheduler_runtime.paused_reason = Some("test".to_string());
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
        assert!(read_topup_state().expect("topup read after clear").is_none());
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
                error: None,
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
            version: TemplateVersion {
                major: 1,
                minor: 0,
                patch: 0,
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
                error: None,
            }],
        )
        .expect("tool calls");
        upsert_inbox(&InboxMessage {
            id: "inbox:00000000000000000001".to_string(),
            seq: 1,
            body: "hello".to_string(),
            posted_at_ns: 11,
            posted_by: "alice".to_string(),
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
        let v1 = TemplateVersion {
            major: 1,
            minor: 0,
            patch: 0,
        };
        let v2 = TemplateVersion {
            major: 1,
            minor: 1,
            patch: 0,
        };

        let mut template = StrategyTemplate {
            key: key.clone(),
            version: v1.clone(),
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
        upsert_strategy_template(&template).expect("template v1");
        template.version = v2.clone();
        template.updated_at_ns = 3;
        upsert_strategy_template(&template).expect("template v2");

        let loaded = strategy_template(&key, &v2)
            .expect("query strategy")
            .expect("exists");
        assert_eq!(loaded.version, v2);

        let versions = list_strategy_template_versions(&key).expect("versions");
        assert_eq!(versions, vec![v2.clone(), v1.clone()]);
        let listed = list_strategy_templates(&key, 10).expect("templates list");
        assert_eq!(listed.len(), 2);

        let artifact = AbiArtifact {
            key: AbiArtifactKey {
                protocol: key.protocol.clone(),
                chain_id: key.chain_id,
                role: "pool".to_string(),
                version: v2.clone(),
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
        let abi_versions = list_abi_artifact_versions("p", 8453, "pool").expect("abi versions");
        assert_eq!(abi_versions, vec![v2]);
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
}
