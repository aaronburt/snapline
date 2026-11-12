use snapline_core::actions::{ActionId, WORKFLOW_PREFIX};
use snapline_core::ports::Notice;

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
        Some(Target::Capture) if routes_through_workflow(action.as_str()) => {
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

fn routes_through_workflow(id: &str) -> bool {
    matches!(
        id,
        "capture_region" | "capture_window" | "capture_screen" | "capture_last_region"
    )
}

pub fn dispatch_notifying(action: &ActionId, services: &Services) -> Result<(), ServiceError> {
    let result = dispatch(action, services);
    if let Err(error) = &result {
        notify_failure(error, services);
    }
    result
}

pub fn notify_failure(error: &ServiceError, services: &Services) {
    if !should_notify(error) || crate::recording::was_reported(&error.to_string()) {
        return;
    }
    if let Some(notifier) = services.notifier() {
        notifier.toast(Notice::new(
            "Snapline could not complete the action",
            error.to_string(),
        ));
    }
}

pub fn notify_unavailable(action: &str, services: &Services) {
    if let Some(notifier) = services.notifier() {
        notifier.toast(Notice::new(
            "Action not available",
            format!("{action} is not available yet."),
        ));
    }
}

fn should_notify(error: &ServiceError) -> bool {
    match error {
        ServiceError::NotYet => false,
        ServiceError::Invalid(message) => !message.ends_with("was cancelled"),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapline_core::actions::CATALOGUE;
    use snapline_core::ports::Notifier;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingNotifier {
        notices: Mutex<Vec<Notice>>,
    }

    impl Notifier for RecordingNotifier {
        fn toast(&self, notice: Notice) {
            self.notices.lock().expect("notices").push(notice);
        }
    }

    #[test]
    fn a_failed_action_produces_a_user_facing_notice() {
        let notifier = Arc::new(RecordingNotifier::default());
        let services = Services::new().with_notifier(notifier.clone());
        let result = dispatch_notifying(&ActionId::new("nonsense_action"), &services);
        assert!(result.is_err());
        let notices = notifier.notices.lock().expect("notices");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].body, "Unknown action: nonsense_action");
    }

    #[test]
    fn cancelled_and_unbuilt_actions_stay_quiet() {
        assert!(!should_notify(&ServiceError::NotYet));
        assert!(!should_notify(&ServiceError::Invalid(
            "Capture was cancelled".to_owned()
        )));
        assert!(should_notify(&ServiceError::Io("disk".to_owned())));
        assert!(should_notify(&ServiceError::Invalid("bad".to_owned())));
    }

    #[test]
    fn every_catalogue_action_routes_to_a_module() {
        for definition in CATALOGUE {
            let action = ActionId::new(definition.id);
            assert!(target(&action).is_some(), "{}", definition.id);
        }
    }

    #[test]
    fn every_quick_panel_tile_action_is_in_the_catalogue() {
        for id in ["capture_region", "capture_window", "capture_screen"] {
            assert!(
                CATALOGUE.iter().any(|definition| definition.id == id),
                "{id}"
            );
            assert!(routes_through_workflow(id), "{id}");
        }
    }

    #[test]
    fn an_unknown_panel_action_is_reported() {
        let notifier = Arc::new(RecordingNotifier::default());
        let services = Services::new().with_notifier(notifier.clone());
        let result = dispatch_notifying(&ActionId::new("panel_nothing"), &services);
        assert!(result.is_err());
        assert_eq!(notifier.notices.lock().expect("notices").len(), 1);
    }

    #[test]
    fn an_unavailable_action_shows_a_notice() {
        let notifier = Arc::new(RecordingNotifier::default());
        let services = Services::new().with_notifier(notifier.clone());
        notify_unavailable("tool_x", &services);
        let notices = notifier.notices.lock().expect("notices");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].body, "tool_x is not available yet.");
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
