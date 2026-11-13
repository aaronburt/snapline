use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_core::ids::WorkflowId;
use snapline_core::workflow::{
    CaptureKind, Step, ValidationError, Violation, Workflow, DEFAULT_WORKFLOW_ID, NAME_MAX_CHARS,
};
use snapline_store::settings::Settings;

use crate::dto::workflow::{
    DestinationOption, ViolationDto, WorkflowInfo, WorkflowReq, WorkflowResp,
};
use crate::{ServiceError, Services};

pub use crate::workflow_runner::{
    install_runner, installed_runner, CaptureServicePort, HotkeyGate, QueueUploadHandOff,
    QueueUploadPort, RunOutcome, RunnerPorts, ServiceTextPort, ServiceWorkflowOptions,
    WorkerSavePort, WorkflowGate, WorkflowRunner, WorkflowUploadHandOff,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct WorkflowService {
    services: Services,
}

impl WorkflowService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }

    pub fn runner(&self) -> Result<WorkflowRunner, ServiceError> {
        installed_runner()
    }

    pub fn run(&self, id: &str) -> Result<RunOutcome, ServiceError> {
        installed_runner()?.run_workflow(id)
    }

    pub fn handle(&self, req: WorkflowReq) -> Result<WorkflowResp, ServiceError> {
        match req {
            WorkflowReq::Ping => Ok(WorkflowResp::Pong),
            WorkflowReq::List => {
                let settings = self.services.settings_snapshot()?;
                let workflows = workflows_of(&settings)
                    .iter()
                    .map(WorkflowInfo::from)
                    .collect();
                Ok(WorkflowResp::List {
                    workflows,
                    destinations: destinations_of(&settings),
                })
            }
            WorkflowReq::Get { id } => {
                let workflow = self.find(&id)?;
                Ok(WorkflowResp::Workflow {
                    workflow: WorkflowInfo::from(&workflow),
                })
            }
            WorkflowReq::Create { workflow } => {
                let mut list = self.load()?;
                let name = workflow.name.trim().to_owned();
                let id = unique_id(&name, &list);
                let created = workflow
                    .into_workflow(&id, name)
                    .map_err(ServiceError::Invalid)?;
                validate(&created, false)?;
                list.push(created.clone());
                self.save(&list)?;
                self.replan()?;
                Ok(WorkflowResp::Workflow {
                    workflow: WorkflowInfo::from(&created),
                })
            }
            WorkflowReq::Update { id, workflow } => {
                let mut list = self.load()?;
                let position = position_of(&list, &id).ok_or(ServiceError::NotFound)?;
                let existing = list[position].clone();
                let is_default = existing.is_default();
                let name = if is_default {
                    existing.name.clone()
                } else {
                    workflow.name.trim().to_owned()
                };
                let mut updated = workflow
                    .into_workflow(&id, name)
                    .map_err(ServiceError::Invalid)?;
                if is_default {
                    updated.name = existing.name.clone();
                    updated.hotkey = None;
                    updated.enabled = true;
                }
                validate(&updated, is_default)?;
                list[position] = updated.clone();
                self.save(&list)?;
                self.replan()?;
                Ok(WorkflowResp::Workflow {
                    workflow: WorkflowInfo::from(&updated),
                })
            }
            WorkflowReq::Delete { id } => {
                if id == DEFAULT_WORKFLOW_ID {
                    return Err(ServiceError::Invalid(
                        "The default workflow cannot be deleted.".to_owned(),
                    ));
                }
                let mut list = self.load()?;
                let position = position_of(&list, &id).ok_or(ServiceError::NotFound)?;
                list.remove(position);
                self.save(&list)?;
                self.replan()?;
                Ok(WorkflowResp::Deleted { id })
            }
            WorkflowReq::Duplicate { id } => {
                let mut list = self.load()?;
                let source = list
                    .iter()
                    .find(|workflow| workflow.id.as_str() == id)
                    .cloned()
                    .ok_or(ServiceError::NotFound)?;
                let name = duplicate_name(&source.name);
                let new_id = unique_id(&name, &list);
                let from_default = source.is_default();
                let mut copy = source.clone();
                copy.id = WorkflowId::new(new_id);
                copy.name = name;
                copy.hotkey = None;
                if from_default {
                    copy.steps.insert(
                        0,
                        Step::Capture {
                            kind: CaptureKind::Region,
                        },
                    );
                }
                validate(&copy, false)?;
                list.push(copy.clone());
                self.save(&list)?;
                self.replan()?;
                Ok(WorkflowResp::Workflow {
                    workflow: WorkflowInfo::from(&copy),
                })
            }
            WorkflowReq::Validate { id, workflow } => {
                let is_default = id.as_deref() == Some(DEFAULT_WORKFLOW_ID);
                let name = workflow.name.trim().to_owned();
                let candidate = workflow
                    .into_workflow("preview", name)
                    .map_err(ServiceError::Invalid)?;
                Ok(WorkflowResp::Validation {
                    violations: violations(&candidate, is_default)
                        .into_iter()
                        .map(ViolationDto::from)
                        .collect(),
                })
            }
        }
    }

    fn load(&self) -> Result<Vec<Workflow>, ServiceError> {
        let settings = self.services.settings_snapshot()?;
        Ok(workflows_of(&settings))
    }

    fn find(&self, id: &str) -> Result<Workflow, ServiceError> {
        self.load()?
            .into_iter()
            .find(|workflow| workflow.id.as_str() == id)
            .ok_or(ServiceError::NotFound)
    }

    fn save(&self, list: &[Workflow]) -> Result<(), ServiceError> {
        let list = serde_json::to_value(list).map_err(|error| {
            ServiceError::Internal(format!("The workflows could not be saved: {error}"))
        })?;
        self.services
            .update_settings(&serde_json::json!({ "workflows": { "list": list } }))?;
        Ok(())
    }

    fn replan(&self) -> Result<(), ServiceError> {
        self.services.hotkeys().on_settings_changed()
    }
}

fn workflows_of(settings: &Settings) -> Vec<Workflow> {
    let mut workflows = settings.workflows.list.clone();
    if !workflows.iter().any(Workflow::is_default) {
        workflows.insert(
            0,
            crate::workflow_runner::default_from_general(&settings.general.default_workflow),
        );
    }
    workflows
}

fn destinations_of(settings: &Settings) -> Vec<DestinationOption> {
    let value = match serde_json::to_value(&settings.uploads) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let items = match value.get("destinations").and_then(|value| value.as_array()) {
        Some(items) => items,
        None => return Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?;
            let name = item
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(id);
            Some(DestinationOption {
                id: id.to_owned(),
                name: name.to_owned(),
            })
        })
        .collect()
}

fn position_of(list: &[Workflow], id: &str) -> Option<usize> {
    list.iter().position(|workflow| workflow.id.as_str() == id)
}

fn violations(workflow: &Workflow, is_default: bool) -> Vec<Violation> {
    workflow
        .validate()
        .into_iter()
        .filter(|violation| !(is_default && violation.error == ValidationError::FirstStep))
        .collect()
}

fn validate(workflow: &Workflow, is_default: bool) -> Result<(), ServiceError> {
    match violations(workflow, is_default).first() {
        Some(violation) => Err(ServiceError::Invalid(violation.to_string())),
        None => Ok(()),
    }
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else if !slug.is_empty() && !separator {
            slug.push('-');
            separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "workflow".to_owned()
    } else {
        slug.to_owned()
    }
}

fn unique_id(name: &str, list: &[Workflow]) -> String {
    let slug = slugify(name);
    let base = if slug == DEFAULT_WORKFLOW_ID {
        format!("{slug}-2")
    } else {
        slug
    };
    if position_of(list, &base).is_none() {
        return base;
    }
    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if position_of(list, &candidate).is_none() {
            return candidate;
        }
        suffix += 1;
    }
}

fn duplicate_name(name: &str) -> String {
    let suffix = " copy";
    let keep = NAME_MAX_CHARS.saturating_sub(suffix.chars().count());
    let mut copy: String = name.chars().take(keep).collect();
    copy.push_str(suffix);
    copy
}

pub fn run(action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    installed_runner()?.run_action(action).map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use snapline_core::chord::Chord;
    use snapline_core::hotkey_plan::{Blocked, Owner};
    use snapline_core::workflow::{CaptureKind, RecordKind};
    use snapline_store::io::Paths;

    use super::*;
    use crate::dto::settings::LinkFormatDto;
    use crate::dto::workflow::{StepDto, WorkflowInput};
    use crate::hotkeys::HotkeyInput;

    #[derive(Default)]
    struct FakeInput {
        rebinds: Mutex<u32>,
    }

    impl HotkeyInput for FakeInput {
        fn rebind(&self, _bindings: Vec<(Owner, Chord)>) -> Result<Vec<Blocked>, ServiceError> {
            *self.rebinds.lock().expect("rebinds") += 1;
            Ok(Vec::new())
        }

        fn pause(&self) -> Result<(), ServiceError> {
            Ok(())
        }

        fn resume(&self) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    fn paths(directory: &Path) -> Paths {
        Paths::resolve(
            directory,
            &directory.join("Roaming"),
            &directory.join("Local"),
            &directory.join("Profile"),
            false,
        )
    }

    fn services(directory: &Path) -> Services {
        Services::with_paths(paths(directory)).expect("services")
    }

    fn capture(kind: CaptureKind) -> StepDto {
        StepDto::Capture { kind: kind.into() }
    }

    fn record(kind: RecordKind) -> StepDto {
        StepDto::Record { kind: kind.into() }
    }

    fn input(name: &str, steps: Vec<StepDto>) -> WorkflowInput {
        WorkflowInput {
            name: name.to_owned(),
            hotkey: None,
            enabled: true,
            steps,
        }
    }

    fn info_of(resp: WorkflowResp) -> WorkflowInfo {
        match resp {
            WorkflowResp::Workflow { workflow } => workflow,
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn listed(resp: WorkflowResp) -> Vec<WorkflowInfo> {
        match resp {
            WorkflowResp::List { workflows, .. } => workflows,
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn seed_default(services: &Services) {
        services
            .update_settings(&serde_json::json!({
                "workflows": {
                    "list": [{
                        "id": "default",
                        "name": "Screenshot to link",
                        "hotkey": null,
                        "enabled": true,
                        "steps": [{ "step": "copy_image" }]
                    }]
                }
            }))
            .expect("seed");
    }

    #[test]
    fn create_list_get_update_and_delete_round_trip() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let mut annotate = input(
            "Annotate and share",
            vec![
                capture(CaptureKind::Region),
                StepDto::Annotate,
                StepDto::Upload { dest: None },
                StepDto::CopyLink {
                    format: LinkFormatDto::Markdown,
                },
            ],
        );
        annotate.hotkey = Some("Ctrl+Shift+A".to_owned());
        let created = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Create {
                    workflow: annotate.clone(),
                })
                .expect("create"),
        );
        assert_eq!(created.id, "annotate-and-share");
        assert_eq!(created.hotkey, Some("Ctrl+Shift+A".to_owned()));
        assert!(!created.is_default);
        assert_eq!(created.steps.len(), 4);

        let list = listed(services.workflow().handle(WorkflowReq::List).expect("list"));
        assert!(list[0].is_default);
        assert_eq!(&list[1..], std::slice::from_ref(&created));

        let fetched = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Get {
                    id: created.id.clone(),
                })
                .expect("get"),
        );
        assert_eq!(fetched, created);

        let mut changed = annotate;
        changed.name = "Annotate and share later".to_owned();
        changed.enabled = false;
        let updated = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Update {
                    id: created.id.clone(),
                    workflow: changed,
                })
                .expect("update"),
        );
        assert_eq!(updated.name, "Annotate and share later");
        assert!(!updated.enabled);
        assert_eq!(updated.id, created.id);

        services
            .workflow()
            .handle(WorkflowReq::Delete {
                id: created.id.clone(),
            })
            .expect("delete");
        assert!(
            listed(services.workflow().handle(WorkflowReq::List).expect("list"))
                .iter()
                .all(|item| item.is_default)
        );
        assert_eq!(
            services
                .workflow()
                .handle(WorkflowReq::Get { id: created.id })
                .expect_err("gone"),
            ServiceError::NotFound
        );
    }

    #[test]
    fn duplicate_copies_the_steps_and_clears_the_chord() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let mut source = input(
            "Save quietly",
            vec![capture(CaptureKind::Screen), StepDto::Save],
        );
        source.hotkey = Some("Ctrl+Alt+PrtSc".to_owned());
        source.enabled = false;
        let source = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Create { workflow: source })
                .expect("create"),
        );
        let copy = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Duplicate {
                    id: source.id.clone(),
                })
                .expect("duplicate"),
        );
        assert_eq!(copy.id, "save-quietly-copy");
        assert_eq!(copy.name, "Save quietly copy");
        assert_eq!(copy.hotkey, None);
        assert!(!copy.enabled);
        assert_eq!(copy.steps, source.steps);
        assert_eq!(
            listed(services.workflow().handle(WorkflowReq::List).expect("list")).len(),
            3
        );
    }

    #[test]
    fn duplicating_the_default_prepends_a_capture_step() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        seed_default(&services);
        let copy = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Duplicate {
                    id: "default".to_owned(),
                })
                .expect("duplicate"),
        );
        assert_eq!(copy.id, "screenshot-to-link-copy");
        assert_eq!(copy.name, "Screenshot to link copy");
        assert!(!copy.is_default);
        assert_eq!(copy.hotkey, None);
        assert_eq!(
            copy.steps.first(),
            Some(&StepDto::Capture {
                kind: snapline_core::workflow::CaptureKind::Region.into()
            })
        );
        assert_eq!(copy.steps.len(), 2);
    }

    #[test]
    fn an_invalid_chord_uses_the_core_message() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let mut workflow = input("Save quietly", vec![capture(CaptureKind::Screen)]);
        workflow.hotkey = Some("Win+L".to_owned());
        let error = services
            .workflow()
            .handle(WorkflowReq::Create { workflow })
            .expect_err("reserved");
        assert_eq!(
            error,
            ServiceError::Invalid("Windows uses this shortcut.".to_owned())
        );
    }

    #[test]
    fn validation_uses_the_exact_strings() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let cases = [
            (
                input("", vec![capture(CaptureKind::Region)]),
                "The name must be 1 to 60 characters.",
            ),
            (
                input(&"a".repeat(61), vec![capture(CaptureKind::Region)]),
                "The name must be 1 to 60 characters.",
            ),
            (
                input("Annotate", vec![StepDto::CopyImage]),
                "The first step must be a Capture or Record step.",
            ),
            (
                input("Annotate", Vec::new()),
                "The first step must be a Capture or Record step.",
            ),
            (
                input(
                    "Annotate",
                    vec![
                        capture(CaptureKind::Region),
                        StepDto::CopyLink {
                            format: LinkFormatDto::Direct,
                        },
                        StepDto::Upload { dest: None },
                    ],
                ),
                "Copy link requires an earlier Upload step.",
            ),
            (
                input(
                    "Grab text",
                    vec![
                        capture(CaptureKind::Region),
                        StepDto::CopyText,
                        StepDto::ExtractText,
                    ],
                ),
                "Copy text requires an earlier Extract text step.",
            ),
            (
                input(
                    "Bug report clip",
                    vec![capture(CaptureKind::Region), StepDto::ConvertToGif],
                ),
                "Convert to GIF requires an earlier Record step.",
            ),
        ];
        for (workflow, message) in cases {
            let error = services
                .workflow()
                .handle(WorkflowReq::Create { workflow })
                .expect_err("invalid");
            assert_eq!(error, ServiceError::Invalid(message.to_owned()));
        }
        assert!(
            listed(services.workflow().handle(WorkflowReq::List).expect("list"))
                .iter()
                .all(|item| item.is_default)
        );
    }

    #[test]
    fn valid_combinations_are_accepted() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let flows = [
            input(
                "Screenshot to link",
                vec![
                    capture(CaptureKind::Region),
                    StepDto::CopyImage,
                    StepDto::Save,
                    StepDto::Upload { dest: None },
                    StepDto::CopyLink {
                        format: LinkFormatDto::Direct,
                    },
                ],
            ),
            input(
                "Bug report clip",
                vec![
                    record(RecordKind::Video),
                    StepDto::ConvertToGif,
                    StepDto::Upload {
                        dest: Some("s3".to_owned()),
                    },
                    StepDto::CopyLink {
                        format: LinkFormatDto::Direct,
                    },
                ],
            ),
            input(
                "Grab text",
                vec![
                    capture(CaptureKind::Region),
                    StepDto::ExtractText,
                    StepDto::CopyText,
                ],
            ),
        ];
        for workflow in flows {
            let expected = workflow.steps.len();
            let created = info_of(
                services
                    .workflow()
                    .handle(WorkflowReq::Create { workflow })
                    .expect("create"),
            );
            assert_eq!(created.steps.len(), expected);
        }
        assert_eq!(
            listed(services.workflow().handle(WorkflowReq::List).expect("list")).len(),
            4
        );
    }

    #[test]
    fn the_default_workflow_cannot_be_deleted_or_disabled() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        seed_default(&services);
        let error = services
            .workflow()
            .handle(WorkflowReq::Delete {
                id: "default".to_owned(),
            })
            .expect_err("guarded");
        assert_eq!(
            error,
            ServiceError::Invalid("The default workflow cannot be deleted.".to_owned())
        );
        let update = WorkflowInput {
            name: "Renamed".to_owned(),
            hotkey: Some("Ctrl+Alt+1".to_owned()),
            enabled: false,
            steps: vec![StepDto::Upload { dest: None }],
        };
        let updated = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Update {
                    id: "default".to_owned(),
                    workflow: update,
                })
                .expect("update"),
        );
        assert_eq!(updated.name, "Screenshot to link");
        assert!(updated.enabled);
        assert_eq!(updated.hotkey, None);
        assert!(updated.is_default);
        assert_eq!(updated.steps, vec![StepDto::Upload { dest: None }]);
    }

    #[test]
    fn the_default_workflow_may_start_with_a_step_other_than_capture() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        seed_default(&services);
        let list = listed(services.workflow().handle(WorkflowReq::List).expect("list"));
        assert_eq!(list.len(), 1);
        assert!(list[0].is_default);
        assert_eq!(list[0].steps, vec![StepDto::CopyImage]);
        assert_eq!(list[0].hotkey, None);
    }

    #[test]
    fn validate_reports_every_rule_with_the_step_index() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let resp = services
            .workflow()
            .handle(WorkflowReq::Validate {
                id: None,
                workflow: input(
                    "",
                    vec![
                        StepDto::CopyLink {
                            format: LinkFormatDto::Direct,
                        },
                        StepDto::ConvertToGif,
                    ],
                ),
            })
            .expect("validate");
        let WorkflowResp::Validation { violations } = resp else {
            panic!("unexpected response");
        };
        assert_eq!(
            violations,
            vec![
                ViolationDto {
                    step: None,
                    message: "The name must be 1 to 60 characters.".to_owned(),
                },
                ViolationDto {
                    step: Some(0),
                    message: "The first step must be a Capture or Record step.".to_owned(),
                },
                ViolationDto {
                    step: Some(0),
                    message: "Copy link requires an earlier Upload step.".to_owned(),
                },
                ViolationDto {
                    step: Some(1),
                    message: "Convert to GIF requires an earlier Record step.".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn validate_skips_the_first_step_rule_for_the_default_workflow() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let resp = services
            .workflow()
            .handle(WorkflowReq::Validate {
                id: Some("default".to_owned()),
                workflow: input("Screenshot to link", vec![StepDto::CopyImage]),
            })
            .expect("validate");
        assert_eq!(
            resp,
            WorkflowResp::Validation {
                violations: Vec::new()
            }
        );
    }

    #[test]
    fn creating_a_workflow_named_default_never_takes_the_built_in_id() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let created = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Create {
                    workflow: input("Default", vec![capture(CaptureKind::Region)]),
                })
                .expect("create"),
        );
        assert_eq!(created.id, "default-2");
        assert!(!created.is_default);
    }

    #[test]
    fn ids_are_unique_for_workflows_with_the_same_name() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        for _ in 0..2 {
            services
                .workflow()
                .handle(WorkflowReq::Create {
                    workflow: input("Save quietly", vec![capture(CaptureKind::Screen)]),
                })
                .expect("create");
        }
        let ids: Vec<String> = listed(services.workflow().handle(WorkflowReq::List).expect("list"))
            .into_iter()
            .map(|workflow| workflow.id)
            .collect();
        assert_eq!(ids, vec!["default", "save-quietly", "save-quietly-2"]);
    }

    #[test]
    fn duplicate_uses_an_id_that_is_still_unique() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let source = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Create {
                    workflow: input("Save quietly", vec![capture(CaptureKind::Screen)]),
                })
                .expect("create"),
        );
        let first = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Duplicate {
                    id: source.id.clone(),
                })
                .expect("duplicate"),
        );
        let second = info_of(
            services
                .workflow()
                .handle(WorkflowReq::Duplicate { id: source.id })
                .expect("duplicate"),
        );
        assert_eq!(first.id, "save-quietly-copy");
        assert_eq!(second.id, "save-quietly-copy-2");
    }

    #[test]
    fn saving_triggers_hotkey_replanning() {
        let directory = tempfile::tempdir().expect("temp dir");
        let port = Arc::new(FakeInput::default());
        let services = services(directory.path()).with_hotkey_input(port.clone());
        services
            .workflow()
            .handle(WorkflowReq::Create {
                workflow: input("Save quietly", vec![capture(CaptureKind::Screen)]),
            })
            .expect("create");
        assert_eq!(*port.rebinds.lock().expect("rebinds"), 1);
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        assert_eq!(
            services
                .workflow()
                .handle(WorkflowReq::Get {
                    id: "missing".to_owned(),
                })
                .expect_err("missing"),
            ServiceError::NotFound
        );
        assert_eq!(
            services
                .workflow()
                .handle(WorkflowReq::Delete {
                    id: "missing".to_owned(),
                })
                .expect_err("missing"),
            ServiceError::NotFound
        );
        assert_eq!(
            services
                .workflow()
                .handle(WorkflowReq::Duplicate {
                    id: "missing".to_owned(),
                })
                .expect_err("missing"),
            ServiceError::NotFound
        );
    }

    #[test]
    fn ping_round_trips() {
        let services = Services::new();
        assert_eq!(
            services.workflow().handle(WorkflowReq::Ping),
            Ok(WorkflowResp::Pong)
        );
    }
}
