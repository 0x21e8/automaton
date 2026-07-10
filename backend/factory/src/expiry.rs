use crate::session_transitions::{apply_session_event_in_state, SpawnSessionEvent};
use crate::state::{delete_spawn_provider_secrets, write_state, FactoryState};
use crate::types::{FactoryError, SessionAuditActor, SpawnSession};

pub(crate) fn expire_session_in_state(
    state: &mut FactoryState,
    session_id: &str,
    actor: SessionAuditActor,
    now_ms: u64,
    reason: &str,
) -> Result<SpawnSession, FactoryError> {
    apply_session_event_in_state(
        state,
        session_id,
        actor,
        now_ms,
        SpawnSessionEvent::SessionExpired,
        reason,
    )
}

pub fn expire_spawn_session(session_id: &str, now_ms: u64) -> Result<SpawnSession, FactoryError> {
    let result = write_state(|state| {
        expire_session_in_state(
            state,
            session_id,
            SessionAuditActor::System,
            now_ms,
            "session expired",
        )
    });
    if result.is_ok() {
        delete_spawn_provider_secrets(session_id);
    }
    result
}
