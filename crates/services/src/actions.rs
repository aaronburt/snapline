use snapline_core::actions::{ActionId, WORKFLOW_PREFIX};

use crate::capture;
use crate::{app, recording, tools, uploads, workflows, ServiceError, Services};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Capture,
    Recording,
    Tools,
    Uploads,
    Workflows,
    App,
}

pub fn target(action: &ActionId) -> Option<Target> {
    let id = action.as_str();
    if id.starts_with(WORKFLOW_PREFIX) {
        return Some(Target::Workflows);
    }
    if id.starts_with("capture_") {
        return Some(Target::Capture);
    }
    if id.starts_with("record_") {
        return Some(Target::Recording);
    }
    if id.starts_with("tool_") {
        return Some(Target::Tools);
    }
    if id.starts_with("upload_") {
        return Some(Target::Uploads);
    }
    if id.starts_with("app_") {
        return Some(Target::App);
    }
    None
}

pub fn dispatch(action: &ActionId, services: &Services) -> Result<(), ServiceError> {
    match target(action) {
        Some(Target::Capture) if action.as_str() == "capture_region" => {
            match workflows::installed_runner() {
                Ok(runner) => runner.run_action(action).map(|_| ()),
                Err(ServiceError::NotYet) => capture::run(action, services),
                Err(error) => Err(error),
            }
        }
        Some(Target::Capture) => capture::run(action, services),
        Some(Target::Recording) => recording::run(action, services),
        Some(Target::Tools) => tools::run(action, services),
        Some(Target::Uploads) => uploads::run(action, services),
        Some(Target::Workflows) => workflows::run(action, services),
        Some(Target::App) => app::run(action, services),
        None => Err(ServiceError::Invalid(format!("Unknown action: {action}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapline_core::actions::CATALOGUE;

    #[test]
    fn every_catalogue_action_routes_to_a_module() {
        for definition in CATALOGUE {
            let action = ActionId::new(definition.id);
            assert!(target(&action).is_some(), "{}", definition.id);
        }
    }

    #[test]
    fn workflow_actions_route_to_workflows() {
        assert_eq!(
            target(&ActionId::new("workflow:default")),
            Some(Target::Workflows)
        );
    }

    #[test]
    fn unknown_actions_do_not_route() {
        assert_eq!(target(&ActionId::new("nonsense")), None);
        assert!(dispatch(&ActionId::new("nonsense"), &Services::new()).is_err());
    }
}
