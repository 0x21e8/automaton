use crate::domain::types::{
    AbiArtifact, AbiArtifactKey, ConversationEntry, ConversationLog, InboxMessage, MemoryFact,
    OutboxMessage, ScheduledJob, SkillRecord, StrategyTemplate, StrategyTemplateKey,
    TemplateVersion, ToolCallRecord, TransitionLogRecord, TurnRecord,
};

fn unsupported() -> String {
    "sqlite backend unavailable on wasm32-unknown-unknown target".to_string()
}

pub fn init_storage() -> Result<(), String> {
    Ok(())
}

pub fn close_storage() -> Result<(), String> {
    Ok(())
}

pub fn reopen_storage() -> Result<(), String> {
    Ok(())
}

pub fn schema_version() -> Result<u64, String> {
    Err(unsupported())
}

pub fn is_backfill_done(_marker: &str) -> Result<bool, String> {
    Err(unsupported())
}

pub fn mark_backfill_done(_marker: &str) -> Result<(), String> {
    Ok(())
}

pub fn upsert_transition(_record: &TransitionLogRecord) -> Result<(), String> {
    Ok(())
}

pub fn upsert_turn(_record: &TurnRecord) -> Result<(), String> {
    Ok(())
}

pub fn replace_tool_calls(_turn_id: &str, _tool_calls: &[ToolCallRecord]) -> Result<(), String> {
    Ok(())
}

pub fn upsert_inbox(_message: &InboxMessage) -> Result<(), String> {
    Ok(())
}

pub fn upsert_outbox(_message: &OutboxMessage) -> Result<(), String> {
    Ok(())
}

pub fn append_conversation(_sender: &str, _entry: &ConversationEntry) -> Result<(), String> {
    Ok(())
}

pub fn upsert_job(_job: &ScheduledJob) -> Result<(), String> {
    Ok(())
}

pub fn upsert_memory_fact(_fact: &MemoryFact) -> Result<(), String> {
    Ok(())
}

pub fn delete_memory_fact(_key: &str) -> Result<(), String> {
    Ok(())
}

pub fn upsert_skill(_skill: &SkillRecord) -> Result<(), String> {
    Ok(())
}

pub fn delete_skill(_name: &str) -> Result<(), String> {
    Ok(())
}

pub fn upsert_strategy_template(_template: &StrategyTemplate) -> Result<(), String> {
    Ok(())
}

pub fn upsert_abi_artifact(_artifact: &AbiArtifact) -> Result<(), String> {
    Ok(())
}

pub fn list_recent_transitions(_limit: usize) -> Result<Vec<TransitionLogRecord>, String> {
    Err(unsupported())
}

pub fn list_turns(_limit: usize) -> Result<Vec<TurnRecord>, String> {
    Err(unsupported())
}

pub fn get_tools_for_turn(_turn_id: &str) -> Result<Vec<ToolCallRecord>, String> {
    Err(unsupported())
}

pub fn list_inbox_messages(_limit: usize) -> Result<Vec<InboxMessage>, String> {
    Err(unsupported())
}

pub fn list_outbox_messages(_limit: usize) -> Result<Vec<OutboxMessage>, String> {
    Err(unsupported())
}

pub fn get_conversation_log(_sender: &str) -> Result<Option<ConversationLog>, String> {
    Err(unsupported())
}

pub fn list_recent_jobs(_limit: usize) -> Result<Vec<ScheduledJob>, String> {
    Err(unsupported())
}

pub fn list_skills() -> Result<Vec<SkillRecord>, String> {
    Err(unsupported())
}

pub fn strategy_template(
    _key: &StrategyTemplateKey,
    _version: &TemplateVersion,
) -> Result<Option<StrategyTemplate>, String> {
    Err(unsupported())
}

pub fn list_strategy_template_versions(
    _key: &StrategyTemplateKey,
) -> Result<Vec<TemplateVersion>, String> {
    Err(unsupported())
}

pub fn list_strategy_templates(
    _key: &StrategyTemplateKey,
    _limit: usize,
) -> Result<Vec<StrategyTemplate>, String> {
    Err(unsupported())
}

pub fn list_all_strategy_templates(_limit: usize) -> Result<Vec<StrategyTemplate>, String> {
    Err(unsupported())
}

pub fn abi_artifact(_key: &AbiArtifactKey) -> Result<Option<AbiArtifact>, String> {
    Err(unsupported())
}

pub fn list_abi_artifact_versions(
    _protocol: &str,
    _chain_id: u64,
    _role: &str,
) -> Result<Vec<TemplateVersion>, String> {
    Err(unsupported())
}

pub fn sql_query_read_only(_query: &str, _row_limit: usize) -> Result<String, String> {
    Err(unsupported())
}

pub fn table_count(_table: &str) -> Result<u64, String> {
    Err(unsupported())
}
