//! Feature modules — each sub-module implements a distinct capability of the automaton.
//!
//! | Module               | Responsibility                                                |
//! |----------------------|---------------------------------------------------------------|
//! | `inference`          | LLM inference (IC LLM canister + OpenRouter)                 |
//! | `evm`                | EVM RPC, event polling, EIP-1559 signing/broadcast           |
//! | `http_fetch`         | Allowlisted HTTPS GET with cycle-affordability guard         |
//! | `threshold_signer`   | IC threshold ECDSA signing and EVM address derivation        |
//! | `cycle_topup`        | Multi-stage USDC → cycles top-up state machine               |
//! | `cycle_topup_host`   | Host-side orchestration and scheduler integration for top-up |
//! | `canister_call`      | Generic inter-canister call tool with skill-defined allowlists |
//! | `signer`             | Mock signer for unit tests                                   |
//! | `skills`             | Skill loader abstraction                                     |

pub mod canister_call;
#[allow(dead_code)]
pub mod cycle_topup;
pub mod cycle_topup_host;
pub mod evm;
pub mod factory_room;
pub mod http_fetch;
pub mod inference;
pub mod signer;
pub mod skills;
pub mod threshold_signer;
pub mod web_search;

pub use evm::{EvmPoller, HttpEvmPoller};
pub use inference::{
    infer_with_provider, infer_with_provider_transcript, is_inference_proxy_deferred_output,
    InferenceDeferredReason, InferenceTranscriptMessage,
};
pub use signer::MockSignerAdapter;
pub use skills::DefaultSkillLoader;
#[cfg(target_arch = "wasm32")]
pub use threshold_signer::ThresholdSignerAdapter;
