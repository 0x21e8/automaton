//! SQLite storage adapter used during the StableBTreeMap -> SQL migration.
//!
//! Phase 1 keeps stable maps as source of truth and dual-writes historical
//! collections into SQLite for parity checks and backfill validation.

use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, ConversationEntry, ConversationLog, InboxMessage, MemoryFact,
    OutboxMessage, ScheduledJob, SkillRecord, StrategyTemplate, StrategyTemplateKey,
    TemplateVersion, ToolCallRecord, TransitionLogRecord, TurnRecord,
};
#[cfg(target_arch = "wasm32")]
use ic_rusqlite::rusqlite::types::ValueRef as SqlValueRef;
#[cfg(not(target_arch = "wasm32"))]
use rusqlite::types::ValueRef as SqlValueRef;
use serde_json::Value;
use serde_json::{Map as JsonMap, Number as JsonNumber};

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
    payload_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_jobs_status_created ON jobs(status, created_at_ns);

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

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use super::MIGRATION_001_BASE_SCHEMA;
    use rusqlite::{params, Connection};
    use std::cell::RefCell;

    thread_local! {
        static SQLITE: RefCell<Option<Connection>> = const { RefCell::new(None) };
    }

    pub type SqlResult<T> = Result<T, String>;

    pub fn init() -> SqlResult<()> {
        with_connection(|conn| {
            apply_migrations(conn)?;
            Ok(())
        })
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
        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        if version < 1 {
            conn.execute(
                "INSERT INTO schema_migrations(version, applied_at_ns) VALUES(1, ?1)",
                params![crate::timing::current_time_ns() as i64],
            )
            .map_err(|err| err.to_string())?;
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod backend {
    pub type SqlResult<T> = Result<T, String>;

    pub fn init() -> SqlResult<()> {
        ic_rusqlite::with_connection(|conn| {
            conn.execute_batch(super::MIGRATION_001_BASE_SCHEMA)
                .map_err(|err| err.to_string())?;
            let version: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| row.get(0),
                )
                .map_err(|err| err.to_string())?;
            if version < 1 {
                conn.execute(
                    "INSERT INTO schema_migrations(version, applied_at_ns) VALUES(1, ?1)",
                    [crate::timing::current_time_ns() as i64],
                )
                .map_err(|err| err.to_string())?;
            }
            Ok::<(), String>(())
        })
        .map_err(|err| format!("{err:?}"))?
    }

    pub fn close() -> SqlResult<()> {
        Ok(())
    }

    pub fn with_connection<T, F>(f: F) -> SqlResult<T>
    where
        F: FnOnce(&ic_rusqlite::rusqlite::Connection) -> SqlResult<T>,
    {
        ic_rusqlite::with_connection(|conn| f(conn)).map_err(|err| format!("{err:?}"))?
    }
}

fn row_payload<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| err.to_string())
}

fn json_path_str(payload_json: &str, path: &str) -> String {
    serde_json::from_str::<Value>(payload_json)
        .ok()
        .and_then(|root| {
            root.pointer(path)
                .and_then(|v| v.as_str().map(ToString::to_string))
        })
        .unwrap_or_default()
}

fn json_path_bool(payload_json: &str, path: &str) -> i64 {
    serde_json::from_str::<Value>(payload_json)
        .ok()
        .and_then(|root| root.pointer(path).and_then(|v| v.as_bool()))
        .map(|v| if v { 1 } else { 0 })
        .unwrap_or(0)
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
        if format!(" {lowered} ").contains(forbidden) {
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

pub fn init_storage() -> Result<(), String> {
    backend::init()
}

pub fn close_storage() -> Result<(), String> {
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
            let tool_name = json_path_str(&payload_json, "/tool");
            let success = json_path_bool(&payload_json, "/success");
            conn.execute(
                "INSERT INTO tool_calls(turn_id, call_index, tool_name, success, payload_json) VALUES(?1, ?2, ?3, ?4, ?5)",
                (turn, i64::try_from(index).unwrap_or(i64::MAX), tool_name, success, payload_json),
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
            "INSERT OR REPLACE INTO jobs(id, kind, lane, status, priority, created_at_ns, payload_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                &job.id,
                format!("{:?}", job.kind),
                format!("{:?}", job.lane),
                format!("{:?}", job.status),
                i64::from(job.priority),
                job.created_at_ns as i64,
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
    let template_id = format!(
        "{}:{}:{}:{}@{}.{}.{}",
        template.key.protocol,
        template.key.primitive,
        template.key.chain_id,
        template.key.template_id,
        template.version.major,
        template.version.minor,
        template.version.patch
    );
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
    let artifact_id = format!(
        "{}:{}:{}@{}.{}.{}",
        artifact.key.protocol,
        artifact.key.chain_id,
        artifact.key.role,
        artifact.key.version.major,
        artifact.key.version.minor,
        artifact.key.version.patch
    );
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
                 ORDER BY name ASC",
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
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_templates
                 WHERE protocol = ?1 AND primitive = ?2 AND chain_id = ?3
                 ORDER BY updated_at_ns DESC",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map(
                (&key.protocol, &key.primitive, key.chain_id as i64),
                |row| row.get::<_, String>(0),
            )
            .map_err(|err| err.to_string())?;
        for row in rows {
            let template: StrategyTemplate =
                from_payload_json(row.map_err(|err| err.to_string())?)?;
            if template.key.template_id == key.template_id && &template.version == version {
                return Ok(Some(template));
            }
        }
        Ok(None)
    })
}

pub fn list_strategy_template_versions(
    key: &StrategyTemplateKey,
) -> Result<Vec<TemplateVersion>, String> {
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM strategy_templates
                 WHERE protocol = ?1 AND primitive = ?2 AND chain_id = ?3",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map(
                (&key.protocol, &key.primitive, key.chain_id as i64),
                |row| row.get::<_, String>(0),
            )
            .map_err(|err| err.to_string())?;
        let mut versions = Vec::new();
        for row in rows {
            let template: StrategyTemplate =
                from_payload_json(row.map_err(|err| err.to_string())?)?;
            if template.key.template_id == key.template_id {
                versions.push(template.version);
            }
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
    let versions = list_strategy_template_versions(key)?;
    let mut templates = Vec::new();
    for version in versions.into_iter().take(limit) {
        if let Some(template) = strategy_template(key, &version)? {
            templates.push(template);
        }
    }
    Ok(templates)
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
    backend::with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM abi_artifacts
                 WHERE protocol = ?1 AND role = ?2 AND chain_id = ?3
                 ORDER BY created_at_ns DESC",
            )
            .map_err(|err| err.to_string())?;
        let rows = stmt
            .query_map((&key.protocol, &key.role, key.chain_id as i64), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| err.to_string())?;
        for row in rows {
            let artifact: AbiArtifact = from_payload_json(row.map_err(|err| err.to_string())?)?;
            if artifact.key.version == key.version {
                return Ok(Some(artifact));
            }
        }
        Ok(None)
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
        | "memory_facts" | "skills" | "strategy_templates" | "abi_artifacts" => table,
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
        TaskKind, TaskLane, TemplateStatus, TemplateVersion, ToolCallRecord,
    };

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
        assert_eq!(schema_version().expect("schema version"), 1);
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
        ] {
            assert_eq!(table_count(table).expect("table count"), 0, "table {table}");
        }
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
