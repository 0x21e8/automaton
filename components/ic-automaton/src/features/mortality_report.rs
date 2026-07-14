//! Best-effort child-to-factory mortality attestation.

use candid::{CandidType, Principal};
use serde::Serialize;

#[cfg(test)]
thread_local! {
    static TEST_REPORT_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static TEST_OBSERVED_PHASE: std::cell::RefCell<Option<crate::domain::types::MortalityPhase>> = const { std::cell::RefCell::new(None) };
}

#[derive(CandidType, Serialize)]
struct ReportDeathRequest {
    cause: String,
    estate_disposition: String,
    terminal_turn_id: String,
}

pub async fn report_starvation(
    factory_principal: Principal,
    terminal_turn_id: &str,
    estate_disposition: &str,
) -> Result<(), String> {
    let args = candid::encode_one(ReportDeathRequest {
        cause: "starved".to_string(),
        estate_disposition: estate_disposition.to_string(),
        terminal_turn_id: terminal_turn_id.to_string(),
    })
    .map_err(|error| format!("failed to encode death report: {error}"))?;
    do_call(factory_principal, args).await
}

#[cfg(target_arch = "wasm32")]
async fn do_call(factory_principal: Principal, args: Vec<u8>) -> Result<(), String> {
    use ic_cdk::call::Call;
    Call::bounded_wait(factory_principal, "report_death")
        .take_raw_args(args)
        .await
        .map(|_| ())
        .map_err(|error| format!("factory death report failed: {error:?}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn do_call(_factory_principal: Principal, _args: Vec<u8>) -> Result<(), String> {
    #[cfg(test)]
    {
        TEST_OBSERVED_PHASE.with(|phase| {
            *phase.borrow_mut() = Some(crate::storage::stable::mortality_runtime().phase);
        });
        TEST_REPORT_ERROR.with(|error| match error.borrow().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        })
    }
    #[cfg(not(test))]
    Ok(())
}

#[cfg(test)]
pub(crate) fn set_test_report_error(error: Option<&str>) {
    TEST_REPORT_ERROR.with(|value| *value.borrow_mut() = error.map(str::to_string));
    TEST_OBSERVED_PHASE.with(|value| *value.borrow_mut() = None);
}

#[cfg(test)]
pub(crate) fn test_observed_phase() -> Option<crate::domain::types::MortalityPhase> {
    TEST_OBSERVED_PHASE.with(|value| *value.borrow())
}
