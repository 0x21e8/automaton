#![cfg(feature = "pocketic_tests")]

use std::path::Path;
use std::time::Duration;

use candid::{decode_one, encode_args, CandidType, Principal};
use pocket_ic::PocketIc;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

const STEWARD_CHAIN_ID: u64 = 8453;
const PROOF_EXPIRY_NS: u64 = 4_102_444_800_000_000_000;
const WASM_PATHS: &[&str] = &[
    "target/wasm32-wasip1/release/backend_nowasi.wasm",
    "target/wasm32-wasip1/release/backend.wasm",
    "target/wasm32-wasip1/release/deps/backend.wasm",
    "target/wasm32-unknown-unknown/release/backend.wasm",
    "target/wasm32-unknown-unknown/release/deps/backend.wasm",
];

#[derive(CandidType, Clone, Debug, Deserialize, Serialize)]
struct InitArgs {
    ecdsa_key_name: String,
    inbox_contract_address: Option<String>,
    evm_chain_id: Option<u64>,
    evm_rpc_url: Option<String>,
    evm_confirmation_depth: Option<u64>,
    evm_bootstrap_lookback_blocks: Option<u64>,
    http_allowed_domains: Option<Vec<String>>,
    llm_canister_id: Option<Principal>,
    cycle_topup_enabled: Option<bool>,
    auto_topup_cycle_threshold: Option<u64>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StewardState {
    chain_id: u64,
    address: String,
    enabled: bool,
    last_used_at_ns: Option<u64>,
    principal: Option<Principal>,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StewardStatusView {
    active_steward: Option<StewardState>,
    next_nonce: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct EvmStewardProof {
    canister_id: String,
    chain_id: u64,
    address: String,
    command_hash: String,
    nonce: u64,
    expires_at_ns: u64,
    signature: String,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum StewardCommand {
    Noop,
    SetLoopEnabled {
        enabled: bool,
    },
    SetInferenceModel {
        model: String,
    },
    SetOpenrouterReasoningLevel {
        level: OpenRouterReasoningLevel,
    },
    SendStewardMessage {
        sender: String,
        message: String,
    },
    SetPrincipal {
        principal: Option<Principal>,
    },
    UpdateSteward {
        chain_id: u64,
        address: String,
        enabled: bool,
    },
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum InferenceProvider {
    IcLlm,
    OpenRouter,
    OpenRouterProxyWorker,
}

#[derive(CandidType, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum OpenRouterReasoningLevel {
    Default,
    Low,
    Medium,
    High,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct InferenceConfigView {
    provider: InferenceProvider,
    model: String,
    openrouter_base_url: String,
    openrouter_has_api_key: bool,
    openrouter_max_response_bytes: u64,
    openrouter_reasoning_level: OpenRouterReasoningLevel,
    openrouter_proxy_worker_base_url: Option<String>,
    openrouter_proxy_trusted_callback_principal: Option<String>,
}

#[derive(CandidType, Clone, Copy, Debug)]
enum TaskKind {
    AgentTurn,
    PollInbox,
    CheckCycles,
    TopUpCycles,
    Reconcile,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ConversationEntry {
    inbox_message_id: String,
    outbox_message_id: Option<String>,
    sender_body: String,
    agent_reply: String,
    turn_id: String,
    timestamp_ns: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ConversationLog {
    sender: String,
    entries: Vec<ConversationEntry>,
    last_activity_ns: u64,
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ConversationSummary {
    sender: String,
    last_activity_ns: u64,
    entry_count: u32,
}

fn assert_wasm_artifact_present() -> Vec<u8> {
    for path in WASM_PATHS {
        if Path::new(path).exists() {
            return std::fs::read(path).unwrap_or_else(|error| {
                panic!("cannot read PocketIC test artifact {path}: {error}");
            });
        }
    }
    panic!(
        "build artifact not found at any expected path ({:?}); run `icp build` before PocketIC tests",
        WASM_PATHS
    );
}

fn with_backend_canister() -> (PocketIc, Principal) {
    let pic = PocketIc::new();
    let canister_id = pic.create_canister();
    let wasm = assert_wasm_artifact_present();
    let init_args = encode_args((InitArgs {
        ecdsa_key_name: "dfx_test_key".to_string(),
        inbox_contract_address: None,
        evm_chain_id: Some(STEWARD_CHAIN_ID),
        evm_rpc_url: Some("https://mainnet.base.org".to_string()),
        evm_confirmation_depth: None,
        evm_bootstrap_lookback_blocks: None,
        http_allowed_domains: None,
        llm_canister_id: None,
        cycle_topup_enabled: None,
        auto_topup_cycle_threshold: None,
    },))
    .expect("failed to encode init args");

    pic.add_cycles(canister_id, 2_000_000_000_000);
    pic.install_canister(canister_id, wasm, init_args, None);

    (pic, canister_id)
}

fn call_query<T>(pic: &PocketIc, canister_id: Principal, method: &str, payload: Vec<u8>) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .query_call(canister_id, Principal::anonymous(), method, payload)
        .unwrap_or_else(|error| panic!("query call {method} failed: {error:?}"));
    decode_one(&response)
        .unwrap_or_else(|error| panic!("failed decoding {method} response: {error:?}"))
}

fn call_update<T>(pic: &PocketIc, canister_id: Principal, method: &str, payload: Vec<u8>) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    call_update_as(pic, canister_id, Principal::anonymous(), method, payload)
}

fn call_update_as<T>(
    pic: &PocketIc,
    canister_id: Principal,
    caller: Principal,
    method: &str,
    payload: Vec<u8>,
) -> T
where
    T: for<'de> Deserialize<'de> + CandidType,
{
    let response = pic
        .update_call(canister_id, caller, method, payload)
        .unwrap_or_else(|error| panic!("update call {method} failed: {error:?}"));
    decode_one(&response)
        .unwrap_or_else(|error| panic!("failed decoding {method} response: {error:?}"))
}

fn set_steward_admin(
    pic: &PocketIc,
    canister_id: Principal,
    chain_id: u64,
    address: String,
    enabled: bool,
) -> StewardState {
    let payload = encode_args((chain_id, address, enabled)).expect("failed to encode steward args");
    let result: Result<StewardState, String> =
        call_update(pic, canister_id, "set_steward_admin", payload);
    result.unwrap_or_else(|error| panic!("set_steward_admin failed: {error}"))
}

fn get_steward_status(pic: &PocketIc, canister_id: Principal) -> StewardStatusView {
    call_query(
        pic,
        canister_id,
        "get_steward_status",
        encode_args(()).expect("failed to encode get_steward_status"),
    )
}

fn get_inference_config(pic: &PocketIc, canister_id: Principal) -> InferenceConfigView {
    call_query(
        pic,
        canister_id,
        "get_inference_config",
        encode_args(()).expect("failed to encode get_inference_config"),
    )
}

fn steward_execute(
    pic: &PocketIc,
    canister_id: Principal,
    command: StewardCommand,
    proof: EvmStewardProof,
) -> Result<String, String> {
    let payload =
        encode_args((command, proof)).expect("failed to encode steward_execute payload args");
    call_update(pic, canister_id, "steward_execute", payload)
}

fn steward_execute_ingress(
    pic: &PocketIc,
    canister_id: Principal,
    caller: Principal,
    command: StewardCommand,
) -> Result<String, String> {
    let payload =
        encode_args((command,)).expect("failed to encode steward_execute_ingress payload args");
    call_update_as(pic, canister_id, caller, "steward_execute_ingress", payload)
}

fn set_inference_provider(pic: &PocketIc, canister_id: Principal, provider: InferenceProvider) {
    let payload = encode_args((provider,)).expect("failed to encode set_inference_provider");
    let _: String = call_update(pic, canister_id, "set_inference_provider", payload);
}

fn set_inference_model(pic: &PocketIc, canister_id: Principal, model: &str) {
    let payload = encode_args((model.to_string(),)).expect("failed to encode set_inference_model");
    let result: Result<String, String> =
        call_update(pic, canister_id, "set_inference_model", payload);
    assert!(result.is_ok(), "set_inference_model failed: {result:?}");
}

fn set_task_enabled(pic: &PocketIc, canister_id: Principal, kind: TaskKind, enabled: bool) {
    let payload = encode_args((kind, enabled)).expect("failed to encode set_task_enabled");
    let _: String = call_update(pic, canister_id, "set_task_enabled", payload);
}

fn set_task_interval_secs(pic: &PocketIc, canister_id: Principal, kind: TaskKind, interval: u64) {
    let payload = encode_args((kind, interval)).expect("failed to encode set_task_interval_secs");
    let result: Result<String, String> =
        call_update(pic, canister_id, "set_task_interval_secs", payload);
    assert!(result.is_ok(), "set_task_interval_secs failed: {result:?}");
}

fn set_scheduler_base_tick_secs(pic: &PocketIc, canister_id: Principal, interval_secs: u64) {
    let payload = encode_args((interval_secs,)).expect("failed to encode base tick payload");
    let result: Result<u64, String> =
        call_update(pic, canister_id, "set_scheduler_base_tick_secs", payload);
    assert!(
        result.is_ok(),
        "set_scheduler_base_tick_secs failed: {result:?}"
    );
}

fn list_conversations(pic: &PocketIc, canister_id: Principal) -> Vec<ConversationSummary> {
    call_query(
        pic,
        canister_id,
        "list_conversations",
        encode_args(()).expect("failed to encode list_conversations"),
    )
}

fn get_conversation(
    pic: &PocketIc,
    canister_id: Principal,
    sender: String,
) -> Option<ConversationLog> {
    call_query(
        pic,
        canister_id,
        "get_conversation",
        encode_args((sender,)).expect("failed to encode get_conversation"),
    )
}

fn signing_key_from_hex(hex_key: &str) -> k256::ecdsa::SigningKey {
    let mut secret_key = [0u8; 32];
    hex::decode_to_slice(hex_key, &mut secret_key).expect("hex private key should decode");
    k256::ecdsa::SigningKey::from_bytes((&secret_key).into())
        .expect("test private key should parse")
}

fn steward_test_signing_key() -> k256::ecdsa::SigningKey {
    signing_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318")
}

fn alternate_signing_key() -> k256::ecdsa::SigningKey {
    k256::ecdsa::SigningKey::from_slice(&[2u8; 32]).expect("alternate test key should parse")
}

fn steward_address_from_key(signing_key: &k256::ecdsa::SigningKey) -> String {
    let uncompressed = signing_key.verifying_key().to_encoded_point(false);
    let digest = Keccak256::digest(&uncompressed.as_bytes()[1..]);
    format!("0x{}", hex::encode(&digest[12..32]))
}

fn canonical_steward_signing_payload(
    canister_id: &str,
    chain_id: u64,
    address: &str,
    command_hash: &str,
    nonce: u64,
    expires_at_ns: u64,
) -> String {
    format!(
        "ic-automaton:steward-execute:v1\ncanister_id:{canister_id}\nchain_id:{chain_id}\naddress:{address}\ncommand_hash:{command_hash}\nnonce:{nonce}\nexpires_at_ns:{expires_at_ns}"
    )
}

fn ethereum_personal_message_hash(message: &str) -> [u8; 32] {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn sign_steward_payload(payload: &str, signing_key: &k256::ecdsa::SigningKey) -> String {
    let prehash = ethereum_personal_message_hash(payload);
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&prehash)
        .expect("test payload should sign");

    let mut bytes = [0u8; 65];
    bytes[..64].copy_from_slice(signature.to_bytes().as_slice());
    bytes[64] = recovery_id.to_byte() + 27;
    format!("0x{}", hex::encode(bytes))
}

fn parse_expected_command_hash(error: &str) -> Option<String> {
    let (_, tail) = error.split_once("expected=")?;
    let expected = tail.split_whitespace().next()?;
    if expected.starts_with("0x") && expected.len() == 66 {
        return Some(expected.to_string());
    }
    None
}

fn resolve_expected_command_hash(
    pic: &PocketIc,
    canister_id: Principal,
    command: &StewardCommand,
    signing_key: &k256::ecdsa::SigningKey,
    nonce: u64,
) -> String {
    let probe = EvmStewardProof {
        canister_id: canister_id.to_text(),
        chain_id: STEWARD_CHAIN_ID,
        address: steward_address_from_key(signing_key).to_ascii_uppercase(),
        command_hash: format!("0x{}", "00".repeat(32)),
        nonce,
        expires_at_ns: PROOF_EXPIRY_NS,
        signature: format!("0x{}", "00".repeat(65)),
    };
    let error = steward_execute(pic, canister_id, command.clone(), probe)
        .expect_err("probe proof should fail with command hash mismatch");
    parse_expected_command_hash(&error).unwrap_or_else(|| {
        panic!("failed to extract expected command hash from error: {error}");
    })
}

fn build_steward_proof(
    pic: &PocketIc,
    canister_id: Principal,
    command: &StewardCommand,
    signing_key: &k256::ecdsa::SigningKey,
    nonce: u64,
) -> EvmStewardProof {
    let canister_id_text = canister_id.to_text();
    let normalized_address = steward_address_from_key(signing_key);
    let command_hash = resolve_expected_command_hash(pic, canister_id, command, signing_key, nonce);
    let payload = canonical_steward_signing_payload(
        &canister_id_text,
        STEWARD_CHAIN_ID,
        &normalized_address,
        &command_hash,
        nonce,
        PROOF_EXPIRY_NS,
    );
    EvmStewardProof {
        canister_id: canister_id_text,
        chain_id: STEWARD_CHAIN_ID,
        address: normalized_address.to_ascii_uppercase(),
        command_hash,
        nonce,
        expires_at_ns: PROOF_EXPIRY_NS,
        signature: sign_steward_payload(&payload, signing_key),
    }
}

#[test]
fn steward_pocketic_enforces_valid_execution_replay_nonce_and_rotation() {
    let (pic, canister_id) = with_backend_canister();
    let steward_key = steward_test_signing_key();
    let non_steward_key = alternate_signing_key();
    let steward_address = steward_address_from_key(&steward_key);
    let non_steward_address = steward_address_from_key(&non_steward_key);

    let stored = set_steward_admin(
        &pic,
        canister_id,
        STEWARD_CHAIN_ID,
        steward_address.clone(),
        true,
    );
    assert_eq!(stored.address, steward_address);
    assert_eq!(get_steward_status(&pic, canister_id).next_nonce, 0);

    let valid_command = StewardCommand::SetLoopEnabled { enabled: true };
    let valid_proof = build_steward_proof(&pic, canister_id, &valid_command, &steward_key, 0);
    let valid_result = steward_execute(&pic, canister_id, valid_command, valid_proof)
        .expect("valid steward command should execute");
    assert_eq!(valid_result, "loop_enabled=true");
    assert_eq!(get_steward_status(&pic, canister_id).next_nonce, 1);

    let non_steward_command = StewardCommand::Noop;
    let non_steward_proof =
        build_steward_proof(&pic, canister_id, &non_steward_command, &non_steward_key, 1);
    let non_steward_error =
        steward_execute(&pic, canister_id, non_steward_command, non_steward_proof)
            .expect_err("non-steward proof must be rejected");
    assert!(
        non_steward_error.contains("proof address does not match active steward"),
        "unexpected non-steward rejection error: {non_steward_error}"
    );
    assert_eq!(get_steward_status(&pic, canister_id).next_nonce, 1);

    let replay_command = StewardCommand::Noop;
    let replay_proof = build_steward_proof(&pic, canister_id, &replay_command, &steward_key, 1);
    steward_execute(
        &pic,
        canister_id,
        replay_command.clone(),
        replay_proof.clone(),
    )
    .expect("first proof use should succeed");
    let replay_error = steward_execute(&pic, canister_id, replay_command, replay_proof)
        .expect_err("replayed proof must fail");
    assert!(
        replay_error.contains("proof nonce mismatch"),
        "unexpected replay error: {replay_error}"
    );
    assert_eq!(get_steward_status(&pic, canister_id).next_nonce, 2);

    let rotate_command = StewardCommand::UpdateSteward {
        chain_id: STEWARD_CHAIN_ID,
        address: non_steward_address.clone(),
        enabled: true,
    };
    let rotate_proof = build_steward_proof(&pic, canister_id, &rotate_command, &steward_key, 2);
    let rotate_result = steward_execute(&pic, canister_id, rotate_command, rotate_proof)
        .expect("steward rotation must execute");
    assert_eq!(rotate_result, "steward_update_steward_executed");
    let rotated_status = get_steward_status(&pic, canister_id);
    assert_eq!(rotated_status.next_nonce, 0);
    assert_eq!(
        rotated_status
            .active_steward
            .as_ref()
            .map(|steward| steward.address.clone()),
        Some(non_steward_address.clone())
    );

    let old_wallet_command = StewardCommand::Noop;
    let old_wallet_proof =
        build_steward_proof(&pic, canister_id, &old_wallet_command, &steward_key, 0);
    let old_wallet_error = steward_execute(&pic, canister_id, old_wallet_command, old_wallet_proof)
        .expect_err("rotated-out steward key must be rejected");
    assert!(
        old_wallet_error.contains("proof address does not match active steward"),
        "unexpected old wallet rejection error: {old_wallet_error}"
    );

    let new_wallet_command = StewardCommand::Noop;
    let new_wallet_proof =
        build_steward_proof(&pic, canister_id, &new_wallet_command, &non_steward_key, 0);
    let new_wallet_result =
        steward_execute(&pic, canister_id, new_wallet_command, new_wallet_proof)
            .expect("rotated-in steward key should execute");
    assert_eq!(new_wallet_result, "steward_noop_executed");
    assert_eq!(get_steward_status(&pic, canister_id).next_nonce, 1);
}

#[test]
fn steward_signed_model_and_reasoning_commands_apply_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();
    let steward_key = steward_test_signing_key();
    let steward_address = steward_address_from_key(&steward_key);
    set_steward_admin(&pic, canister_id, STEWARD_CHAIN_ID, steward_address, true);

    let model_command = StewardCommand::SetInferenceModel {
        model: "google/gemini-3-flash-preview".to_string(),
    };
    let model_proof = build_steward_proof(&pic, canister_id, &model_command, &steward_key, 0);
    let model_result = steward_execute(&pic, canister_id, model_command, model_proof)
        .expect("signed model command should execute");
    assert_eq!(
        model_result,
        "inference_model=google/gemini-3-flash-preview"
    );

    let reasoning_command = StewardCommand::SetOpenrouterReasoningLevel {
        level: OpenRouterReasoningLevel::High,
    };
    let reasoning_proof =
        build_steward_proof(&pic, canister_id, &reasoning_command, &steward_key, 1);
    let reasoning_result = steward_execute(&pic, canister_id, reasoning_command, reasoning_proof)
        .expect("signed reasoning command should execute");
    assert_eq!(reasoning_result, "openrouter_reasoning_level=High");

    let config = get_inference_config(&pic, canister_id);
    assert_eq!(config.model, "google/gemini-3-flash-preview");
    assert_eq!(
        config.openrouter_reasoning_level,
        OpenRouterReasoningLevel::High
    );
    assert_eq!(get_steward_status(&pic, canister_id).next_nonce, 2);
}

#[test]
fn steward_ingress_principal_can_execute_without_signature_and_unauthorized_is_rejected() {
    let (pic, canister_id) = with_backend_canister();
    let steward_key = steward_test_signing_key();
    let steward_address = steward_address_from_key(&steward_key);
    set_steward_admin(
        &pic,
        canister_id,
        STEWARD_CHAIN_ID,
        steward_address.clone(),
        true,
    );

    let principal = Principal::self_authenticating(b"ii-steward");
    let link_command = StewardCommand::SetPrincipal {
        principal: Some(principal),
    };
    let link_proof = build_steward_proof(&pic, canister_id, &link_command, &steward_key, 0);
    let link_result = steward_execute(&pic, canister_id, link_command, link_proof)
        .expect("signed principal link command should execute");
    assert_eq!(
        link_result,
        format!("steward_principal={}", principal.to_text())
    );
    assert_eq!(get_steward_status(&pic, canister_id).next_nonce, 1);

    let ingress_result = steward_execute_ingress(
        &pic,
        canister_id,
        principal,
        StewardCommand::SetLoopEnabled { enabled: true },
    )
    .expect("authorized ingress principal should execute");
    assert_eq!(ingress_result, "loop_enabled=true");
    assert_eq!(
        get_steward_status(&pic, canister_id)
            .active_steward
            .as_ref()
            .and_then(|steward| steward.principal),
        Some(principal)
    );
    assert_eq!(
        get_steward_status(&pic, canister_id).next_nonce,
        1,
        "ingress execution must not consume EVM nonce"
    );

    let attacker = Principal::self_authenticating(b"ii-attacker");
    let rejected = steward_execute_ingress(&pic, canister_id, attacker, StewardCommand::Noop)
        .expect_err("unauthorized ingress principal must be rejected");
    assert!(rejected.contains("caller principal does not match active steward principal"));
}

#[test]
fn steward_direct_message_reaches_conversation_flow_in_pocketic() {
    let (pic, canister_id) = with_backend_canister();
    set_scheduler_base_tick_secs(&pic, canister_id, 1);
    for kind in [
        TaskKind::AgentTurn,
        TaskKind::PollInbox,
        TaskKind::CheckCycles,
        TaskKind::TopUpCycles,
        TaskKind::Reconcile,
    ] {
        set_task_enabled(&pic, canister_id, kind, false);
        set_task_interval_secs(&pic, canister_id, kind, 1);
    }
    set_task_enabled(&pic, canister_id, TaskKind::AgentTurn, true);
    set_inference_provider(&pic, canister_id, InferenceProvider::IcLlm);
    set_inference_model(&pic, canister_id, "deterministic-local");

    let steward_key = steward_test_signing_key();
    let steward_address = steward_address_from_key(&steward_key);
    set_steward_admin(
        &pic,
        canister_id,
        STEWARD_CHAIN_ID,
        steward_address.clone(),
        true,
    );

    let sender = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let message = "steward direct message integration path".to_string();
    let message_command = StewardCommand::SendStewardMessage {
        sender: sender.clone(),
        message: message.clone(),
    };
    let message_proof = build_steward_proof(&pic, canister_id, &message_command, &steward_key, 0);
    let execute_result = steward_execute(&pic, canister_id, message_command, message_proof)
        .expect("steward direct message command should execute");
    assert!(
        execute_result.starts_with("steward_direct_message_ingested id=inbox:"),
        "unexpected dispatch result: {execute_result}"
    );

    for _ in 0..30 {
        pic.advance_time(Duration::from_secs(2));
        pic.tick();

        let summaries = list_conversations(&pic, canister_id);
        if summaries.is_empty() {
            continue;
        }
        let matching = summaries
            .into_iter()
            .find(|summary| summary.sender == sender);
        let Some(summary) = matching else {
            continue;
        };
        assert!(
            summary.entry_count >= 1,
            "conversation summary should contain at least one entry"
        );

        let conversation = get_conversation(&pic, canister_id, sender.clone())
            .expect("conversation should exist after agent turn");
        assert!(
            conversation
                .entries
                .iter()
                .any(|entry| entry.sender_body == message && !entry.agent_reply.is_empty()),
            "conversation should contain the steward message and an agent reply"
        );
        return;
    }

    panic!("steward direct message did not appear in conversation flow within expected ticks");
}
