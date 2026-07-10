//! Candid and Serde types shared across the factory/automaton spawn boundary.
//!
//! This crate deliberately contains only values that cross the canister
//! installation or bootstrap boundary. Factory stable state and automaton
//! runtime state remain owned by their respective canisters.

use candid::{CandidType, Principal};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, CandidType, Serialize, Deserialize,
)]
pub enum InferenceTransport {
    #[default]
    OpenrouterDirect,
    OpenrouterProxyWorker,
}

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, CandidType, Serialize, Deserialize,
)]
pub enum OpenRouterReasoningLevel {
    #[default]
    Default,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnProviderBootstrapArgs {
    pub open_router_api_key: Option<String>,
    pub model: Option<String>,
    pub brave_search_api_key: Option<String>,
    #[serde(default)]
    pub inference_transport: InferenceTransport,
    #[serde(default)]
    pub open_router_reasoning_level: OpenRouterReasoningLevel,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnBootstrapArgs {
    pub steward_address: String,
    pub session_id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub factory_principal: Principal,
    pub risk: u8,
    #[serde(default)]
    pub strategies: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    pub provider: SpawnProviderBootstrapArgs,
    pub version_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct InitArgs {
    pub ecdsa_key_name: String,
    #[serde(default)]
    pub inbox_contract_address: Option<String>,
    #[serde(default)]
    pub evm_chain_id: Option<u64>,
    #[serde(default)]
    pub evm_rpc_url: Option<String>,
    #[serde(default)]
    pub evm_confirmation_depth: Option<u64>,
    #[serde(default)]
    pub evm_bootstrap_lookback_blocks: Option<u64>,
    #[serde(default)]
    pub http_allowed_domains: Option<Vec<String>>,
    #[serde(default)]
    pub llm_canister_id: Option<Principal>,
    #[serde(default)]
    pub search_api_key: Option<String>,
    #[serde(default)]
    pub inference_proxy_worker_base_url: Option<String>,
    #[serde(default)]
    pub inference_proxy_trusted_callback_principal: Option<Principal>,
    #[serde(default)]
    pub cycle_topup_enabled: Option<bool>,
    #[serde(default)]
    pub auto_topup_cycle_threshold: Option<u64>,
    #[serde(default)]
    pub spawn_bootstrap: Option<SpawnBootstrapArgs>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct SpawnBootstrapView {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub factory_principal: Option<Principal>,
    #[serde(default)]
    pub risk: Option<u8>,
    #[serde(default)]
    pub strategies: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub version_commit: Option<String>,
}
