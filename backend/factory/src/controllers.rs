#[cfg(target_arch = "wasm32")]
use crate::types::FactoryError;

#[cfg(target_arch = "wasm32")]
pub(crate) fn rejection_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(target_arch = "wasm32")]
fn already_self_controlled(message: &str) -> bool {
    message.contains("Only the controllers of the canister")
}

#[cfg(target_arch = "wasm32")]
fn is_self_controlled_error(error: &FactoryError) -> bool {
    matches!(
        error,
        FactoryError::ManagementCallFailed { message, .. } if already_self_controlled(message)
    )
}

#[cfg(target_arch = "wasm32")]
fn canister_principal(canister_id: &str) -> Result<candid::Principal, FactoryError> {
    use candid::Principal;

    Principal::from_text(canister_id).map_err(|error| FactoryError::ManagementCallFailed {
        method: "parse_canister_id".to_string(),
        message: error.to_string(),
    })
}

#[cfg(target_arch = "wasm32")]
pub async fn complete_controller_handoff_live(canister_id: &str) -> Result<(), FactoryError> {
    use ic_cdk::management_canister::{
        canister_status, update_settings, CanisterSettings, CanisterStatusArgs,
        UpdateSettingsArgs,
    };

    let canister = canister_principal(canister_id)?;
    let factory_controller = ic_cdk::api::canister_self();

    let first_update = update_settings(&UpdateSettingsArgs {
        canister_id: canister,
        settings: CanisterSettings {
            controllers: Some(vec![factory_controller, canister]),
            ..Default::default()
        },
    })
    .await
    .map_err(|error| FactoryError::ManagementCallFailed {
        method: "update_settings".to_string(),
        message: rejection_message(error),
    });
    match first_update {
        Ok(()) => {}
        Err(error) if is_self_controlled_error(&error) => return Ok(()),
        Err(error) => return Err(error),
    }

    let first_status = canister_status(&CanisterStatusArgs {
        canister_id: canister,
    })
    .await
    .map_err(|error| FactoryError::ManagementCallFailed {
        method: "canister_status".to_string(),
        message: rejection_message(error),
    });
    let status = match first_status {
        Ok(status) => status,
        Err(error) if is_self_controlled_error(&error) => return Ok(()),
        Err(error) => return Err(error),
    };

    if status.settings.controllers.len() != 2
        || !status
            .settings
            .controllers
            .iter()
            .any(|controller| controller == &factory_controller)
        || !status
            .settings
            .controllers
            .iter()
            .any(|controller| controller == &canister)
    {
        return Err(FactoryError::ControllerInvariantViolation {
            canister_id: canister_id.to_string(),
        });
    }

    let second_update = update_settings(&UpdateSettingsArgs {
        canister_id: canister,
        settings: CanisterSettings {
            controllers: Some(vec![canister]),
            ..Default::default()
        },
    })
    .await
    .map_err(|error| FactoryError::ManagementCallFailed {
        method: "update_settings".to_string(),
        message: rejection_message(error),
    });
    match second_update {
        Ok(()) => {}
        Err(error) if is_self_controlled_error(&error) => return Ok(()),
        Err(error) => return Err(error),
    }

    let second_status = canister_status(&CanisterStatusArgs {
        canister_id: canister,
    })
    .await
    .map_err(|error| FactoryError::ManagementCallFailed {
        method: "canister_status".to_string(),
        message: rejection_message(error),
    });
    let status = match second_status {
        Ok(status) => status,
        Err(error) if is_self_controlled_error(&error) => return Ok(()),
        Err(error) => return Err(error),
    };

    if status.settings.controllers != vec![canister] {
        return Err(FactoryError::ControllerInvariantViolation {
            canister_id: canister_id.to_string(),
        });
    }

    Ok(())
}
