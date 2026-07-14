use crate::types::FactoryError;

#[cfg(target_arch = "wasm32")]
pub(crate) fn rejection_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(target_arch = "wasm32")]
fn canister_principal(canister_id: &str) -> Result<candid::Principal, FactoryError> {
    use candid::Principal;

    Principal::from_text(canister_id).map_err(|error| FactoryError::ManagementCallFailed {
        method: "parse_canister_id".to_string(),
        message: error.to_string(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedFactoryControllers(Vec<String>);

impl VerifiedFactoryControllers {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn first(&self) -> &str {
        self.0
            .first()
            .expect("verified factory controller list is non-empty")
    }

    pub(crate) fn into_vec(self) -> Vec<String> {
        self.0
    }
}

pub(crate) fn verify_factory_only_controller_ids(
    canister_id: &str,
    factory_controller: &str,
    controllers: Vec<String>,
) -> Result<VerifiedFactoryControllers, FactoryError> {
    if controllers != vec![factory_controller.to_string()] {
        return Err(FactoryError::ControllerInvariantViolation {
            canister_id: canister_id.to_string(),
        });
    }
    Ok(VerifiedFactoryControllers(controllers))
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn complete_controller_handoff_live(
    canister_id: &str,
) -> Result<VerifiedFactoryControllers, FactoryError> {
    use ic_cdk::management_canister::{
        canister_status, update_settings, CanisterSettings, CanisterStatusArgs, UpdateSettingsArgs,
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
            controllers: Some(vec![factory_controller]),
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
        Err(error) => return Err(error),
    };

    verify_factory_only_controller_ids(
        canister_id,
        &factory_controller.to_text(),
        status
            .settings
            .controllers
            .iter()
            .map(candid::Principal::to_text)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::verify_factory_only_controller_ids;

    #[test]
    fn unknown_or_child_control_cannot_be_attested_for_registry_completion() {
        let canister_id = "ryjl3-tyaaa-aaaaa-aaaba-cai";
        let factory = "rrkah-fqaaa-aaaaa-aaaaq-cai";

        for controllers in [
            vec![],
            vec![canister_id.to_string()],
            vec!["2vxsx-fae".to_string()],
        ] {
            assert!(verify_factory_only_controller_ids(canister_id, factory, controllers).is_err());
        }
        assert_eq!(
            verify_factory_only_controller_ids(canister_id, factory, vec![factory.to_string()])
                .expect("factory-only control should be attestable"),
            super::VerifiedFactoryControllers(vec![factory.to_string()])
        );
    }
}
