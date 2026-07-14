//! Typed client for the factory-hosted shared room APIs.
//!
//! This module only transports room payloads. Message bodies remain untrusted
//! opaque data and are not parsed, rendered, or elevated into privileged
//! context here.

use crate::domain::types::{PostRoomMessageRequest, RoomMessage, RoomMessagePage};
use crate::storage::stable;
use candid::{CandidType, Principal};
use serde::Deserialize;

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FactoryPeer {
    pub name: Option<String>,
    pub constitution_hash: Option<String>,
    pub canister_id: String,
    pub steward_address: String,
    pub evm_address: String,
    pub chain: FactoryPeerChain,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    pub created_at: u64,
    pub version_commit: String,
    pub controllers: Option<Vec<String>>,
    pub control_status: Option<String>,
    pub control_verified_at: Option<u64>,
    pub death_cause: Option<String>,
    pub died_at: Option<u64>,
    pub estate_disposition: Option<String>,
    pub death_recorded_by: Option<String>,
    pub death_incident_reference: Option<String>,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum FactoryPeerChain {
    Base,
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FactoryPeerPage {
    pub items: Vec<FactoryPeer>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct FactoryRoomClient {
    factory_principal: Principal,
}

impl FactoryRoomClient {
    pub const fn new(factory_principal: Principal) -> Self {
        Self { factory_principal }
    }

    pub fn from_runtime() -> Result<Self, String> {
        let factory_principal = stable::factory_principal()
            .ok_or_else(|| "factory principal is not configured".to_string())?;
        Ok(Self::new(factory_principal))
    }

    pub const fn factory_principal(&self) -> Principal {
        self.factory_principal
    }

    pub async fn post_room_message(
        &self,
        request: PostRoomMessageRequest,
    ) -> Result<RoomMessage, String> {
        let encoded_args = candid::encode_one(request)
            .map_err(|error| format!("failed to encode post_room_message args: {error}"))?;
        let response_bytes =
            do_factory_room_call(self.factory_principal, "post_room_message", encoded_args).await?;
        decode_factory_room_response("post_room_message", &response_bytes)
    }

    pub async fn list_room_messages(
        &self,
        after_seq: Option<u64>,
        limit: Option<u64>,
    ) -> Result<RoomMessagePage, String> {
        let encoded_args = candid::encode_args((after_seq, limit))
            .map_err(|error| format!("failed to encode list_room_messages args: {error}"))?;
        let response_bytes =
            do_factory_room_call(self.factory_principal, "list_room_messages", encoded_args)
                .await?;
        decode_factory_room_response("list_room_messages", &response_bytes)
    }

    pub async fn list_my_room_messages(
        &self,
        after_seq: Option<u64>,
        limit: Option<u64>,
    ) -> Result<RoomMessagePage, String> {
        let encoded_args = candid::encode_args((after_seq, limit))
            .map_err(|error| format!("failed to encode list_my_room_messages args: {error}"))?;
        let response_bytes = do_factory_room_call(
            self.factory_principal,
            "list_my_room_messages",
            encoded_args,
        )
        .await?;
        decode_factory_room_response("list_my_room_messages", &response_bytes)
    }

    pub async fn list_peers(
        &self,
        cursor: Option<String>,
        limit: u64,
    ) -> Result<FactoryPeerPage, String> {
        let encoded_args = candid::encode_args((cursor, limit))
            .map_err(|error| format!("failed to encode list_spawned_automatons args: {error}"))?;
        let response_bytes = do_factory_room_call(
            self.factory_principal,
            "list_spawned_automatons",
            encoded_args,
        )
        .await?;
        decode_factory_room_response("list_spawned_automatons", &response_bytes)
    }

    pub async fn get_peer(&self, canister_id: &str) -> Result<FactoryPeer, String> {
        let encoded_args = candid::encode_one(canister_id.to_string())
            .map_err(|error| format!("failed to encode get_spawned_automaton args: {error}"))?;
        let response_bytes = do_factory_room_call(
            self.factory_principal,
            "get_spawned_automaton",
            encoded_args,
        )
        .await?;
        decode_factory_room_response("get_spawned_automaton", &response_bytes)
    }
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) enum FactoryRoomCallResult<T> {
    Ok(T),
    Err(FactoryRoomError),
}

#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) enum FactoryRoomError {
    ArtifactHashMismatch {
        expected: String,
        actual: String,
    },
    ArtifactUploadIncomplete {
        expected: u64,
        received: u64,
    },
    ArtifactUploadTooLarge {
        expected: u64,
        attempted: u64,
    },
    NoPendingArtifactUpload,
    QuoteTermsHashMismatch {
        expected: String,
        received: String,
    },
    RegistryRecordNotFound {
        canister_id: String,
    },
    InvalidAmount {
        value: String,
    },
    InvalidSha256 {
        value: String,
    },
    InvalidVersionCommit {
        value: String,
    },
    UnauthorizedAdmin {
        caller: String,
    },
    SessionNotRetryable {
        session_id: String,
        state: FactoryRoomSpawnSessionState,
    },
    ManagementCallFailed {
        method: String,
        message: String,
    },
    RpcRequestFailed {
        operation: String,
        endpoint: String,
        category: FactoryRoomRpcFailureCategory,
        code: Option<i64>,
        message: String,
    },
    InsufficientCyclesPool {
        available: candid::Nat,
        required: candid::Nat,
    },
    InsufficientCyclesForOperation {
        operation: String,
        available: candid::Nat,
        required: candid::Nat,
    },
    SessionNotFound {
        session_id: String,
    },
    ControllerInvariantViolation {
        canister_id: String,
    },
    FactoryPaused {
        pause: bool,
    },
    UnauthorizedSteward {
        session_id: String,
        caller: String,
    },
    UnauthorizedRoomPoster {
        caller: String,
    },
    EmptyRoomMessageBody,
    RoomMessageBodyTooLarge {
        provided_bytes: u64,
        max_bytes: u64,
    },
    TooManyRoomMentions {
        provided: u64,
        max_mentions: u64,
    },
    InvalidRoomContentType {
        value: String,
    },
    InvalidRoomMessageJson {
        message: String,
    },
    PaymentNotSettled {
        status: FactoryRoomPaymentStatus,
        session_id: String,
    },
    SessionNotRefundable {
        session_id: String,
        payment_status: FactoryRoomPaymentStatus,
        state: FactoryRoomSpawnSessionState,
    },
    GrossBelowRequiredMinimum {
        provided: String,
        required: String,
    },
    AutomatonRuntimeNotFound {
        canister_id: String,
    },
    MissingChildRuntimeConfig {
        field: String,
    },
    InvalidPaginationLimit {
        limit: u64,
    },
    SessionNotReadyForSpawn {
        session_id: String,
        state: FactoryRoomSpawnSessionState,
    },
    SessionExpired {
        session_id: String,
        expires_at: u64,
    },
    EscrowClaimNotFound {
        session_id: String,
    },
}

#[derive(CandidType, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactoryRoomRpcFailureCategory {
    Transport,
    MalformedResponse,
    ResponseTooLarge,
    RateLimited,
    Unavailable,
    Upstream,
}

#[derive(CandidType, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactoryRoomPaymentStatus {
    Refunded,
    Paid,
    Unpaid,
    Partial,
}

#[derive(CandidType, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactoryRoomSpawnSessionState {
    Failed,
    BroadcastingRelease,
    Spawning,
    Complete,
    AwaitingPayment,
    PaymentDetected,
    Expired,
}

impl std::fmt::Display for FactoryRoomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn decode_factory_room_response<T>(method: &str, response_bytes: &[u8]) -> Result<T, String>
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let result: FactoryRoomCallResult<T> = candid::decode_one(response_bytes)
        .map_err(|error| format!("failed to decode {method} response: {error}"))?;
    match result {
        FactoryRoomCallResult::Ok(value) => Ok(value),
        FactoryRoomCallResult::Err(error) => {
            Err(format!("factory room {method} returned error: {error}"))
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn do_factory_room_call(
    canister_id: Principal,
    method: &str,
    encoded_args: Vec<u8>,
) -> Result<Vec<u8>, String> {
    use ic_cdk::call::{Call, CallFailed};

    Call::bounded_wait(canister_id, method)
        .take_raw_args(encoded_args)
        .await
        .map(|response| response.into_bytes())
        .map_err(|err| match &err {
            CallFailed::InsufficientLiquidCycleBalance(e) => format!(
                "factory room call insufficient cycles: available={} required={}",
                e.available, e.required
            ),
            CallFailed::CallPerformFailed(e) => {
                format!("factory room call_perform failed (system error): {e:?}")
            }
            CallFailed::CallRejected(e) => format!(
                "factory room call rejected: code={} msg={}",
                e.raw_reject_code(),
                e.reject_message()
            ),
        })
}

#[cfg(not(target_arch = "wasm32"))]
async fn do_factory_room_call(
    canister_id: Principal,
    method: &str,
    encoded_args: Vec<u8>,
) -> Result<Vec<u8>, String> {
    #[cfg(test)]
    {
        MOCK_FACTORY_ROOM_CALL.with(|mock| {
            mock.borrow()
                .as_ref()
                .map(|f| f(canister_id, method, &encoded_args))
                .unwrap_or_else(|| Err(format!("no mock registered for {canister_id}.{method}")))
        })
    }
    #[cfg(not(test))]
    {
        let _ = (canister_id, method, encoded_args);
        Err("factory room client unavailable on non-wasm32 targets".to_string())
    }
}

#[cfg(test)]
type MockFactoryRoomCallFn = dyn Fn(Principal, &str, &[u8]) -> Result<Vec<u8>, String>;

#[cfg(test)]
thread_local! {
    static MOCK_FACTORY_ROOM_CALL: std::cell::RefCell<Option<Box<MockFactoryRoomCallFn>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub fn set_mock_factory_room_call(
    f: impl Fn(Principal, &str, &[u8]) -> Result<Vec<u8>, String> + 'static,
) {
    MOCK_FACTORY_ROOM_CALL.with(|mock| *mock.borrow_mut() = Some(Box::new(f)));
}

#[cfg(test)]
pub fn clear_mock_factory_room_call() {
    MOCK_FACTORY_ROOM_CALL.with(|mock| *mock.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{RoomContentType, SpawnBootstrapView};
    use crate::util::block_on_with_spin;

    fn test_factory_principal() -> Principal {
        Principal::from_text("rrkah-fqaaa-aaaaa-aaaaq-cai").expect("test principal should parse")
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

    #[test]
    fn from_runtime_requires_factory_principal() {
        clear_mock_factory_room_call();
        crate::storage::stable::init_storage();

        let error = FactoryRoomClient::from_runtime().expect_err("missing config must fail");
        assert_eq!(error, "factory principal is not configured");
    }

    #[test]
    fn from_runtime_uses_bootstrap_factory_principal() {
        clear_mock_factory_room_call();
        crate::storage::stable::init_storage();
        crate::storage::stable::set_spawn_bootstrap_metadata(SpawnBootstrapView {
            contract_version: None,
            name: None,
            constitution: None,
            session_id: None,
            parent_id: None,
            factory_principal: Some(test_factory_principal()),
            risk: None,
            strategies: Vec::new(),
            skills: Vec::new(),
            version_commit: None,
        });

        let client = FactoryRoomClient::from_runtime().expect("factory principal should load");
        assert_eq!(client.factory_principal(), test_factory_principal());
    }

    #[test]
    fn post_room_message_round_trips_through_mock_call() {
        clear_mock_factory_room_call();
        let principal = test_factory_principal();
        let expected = sample_room_message(7);
        let expected_for_mock = expected.clone();

        set_mock_factory_room_call(move |canister_id, method, encoded_args| {
            assert_eq!(canister_id, principal);
            assert_eq!(method, "post_room_message");

            let request: PostRoomMessageRequest =
                candid::decode_one(encoded_args).expect("request should decode");
            assert_eq!(request.body, "fleet update");
            assert_eq!(
                request.mentions,
                Some(vec!["um5iw-rqaaa-aaaaq-qaaba-cai".to_string()])
            );
            assert_eq!(request.content_type, Some(RoomContentType::TextPlain));

            candid::encode_one(FactoryRoomCallResult::Ok(expected_for_mock.clone()))
                .map_err(|error| error.to_string())
        });

        let client = FactoryRoomClient::new(principal);
        let response = block_on_with_spin(client.post_room_message(PostRoomMessageRequest {
            body: "fleet update".to_string(),
            mentions: Some(vec!["um5iw-rqaaa-aaaaq-qaaba-cai".to_string()]),
            content_type: Some(RoomContentType::TextPlain),
        }))
        .expect("room post should succeed");

        assert_eq!(response, expected);
        clear_mock_factory_room_call();
    }

    #[test]
    fn list_my_room_messages_uses_caller_bound_factory_method() {
        clear_mock_factory_room_call();
        let principal = test_factory_principal();
        let expected = RoomMessagePage {
            messages: vec![sample_room_message(9)],
            next_after_seq: Some(9),
            latest_seq: Some(12),
        };
        let expected_for_mock = expected.clone();

        set_mock_factory_room_call(move |canister_id, method, encoded_args| {
            assert_eq!(canister_id, principal);
            assert_eq!(method, "list_my_room_messages");

            let (after_seq, limit): (Option<u64>, Option<u64>) =
                candid::decode_args(encoded_args).expect("query args should decode");
            assert_eq!(after_seq, Some(8));
            assert_eq!(limit, Some(25));

            candid::encode_one(FactoryRoomCallResult::Ok(expected_for_mock.clone()))
                .map_err(|error| error.to_string())
        });

        let client = FactoryRoomClient::new(principal);
        let response = block_on_with_spin(client.list_my_room_messages(Some(8), Some(25)))
            .expect("filtered room read should succeed");

        assert_eq!(response, expected);
        clear_mock_factory_room_call();
    }

    #[test]
    fn list_my_room_messages_surfaces_factory_err_variants() {
        clear_mock_factory_room_call();
        let principal = test_factory_principal();

        set_mock_factory_room_call(move |canister_id, method, _encoded_args| {
            assert_eq!(canister_id, principal);
            assert_eq!(method, "list_my_room_messages");
            candid::encode_one::<FactoryRoomCallResult<RoomMessagePage>>(
                FactoryRoomCallResult::Err(FactoryRoomError::InvalidPaginationLimit { limit: 0 }),
            )
            .map_err(|error| error.to_string())
        });

        let client = FactoryRoomClient::new(principal);
        let error = block_on_with_spin(client.list_my_room_messages(Some(8), Some(0)))
            .expect_err("factory room error should surface");

        assert!(error.contains("list_my_room_messages"));
        assert!(error.contains("InvalidPaginationLimit"));
        clear_mock_factory_room_call();
    }
}
