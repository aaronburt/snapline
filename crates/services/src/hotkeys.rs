use std::collections::BTreeMap;
use std::sync::{Arc, MutexGuard};

use serde::{Deserialize, Serialize};
use snapline_core::actions::{self, ActionId, CATALOGUE};
use snapline_core::chord::Chord;
use snapline_core::hotkey_plan::{self, BlockReason, Blocked, Owner, Preview as CorePreview};
use snapline_core::ports::{Notice, Notifier};
use snapline_store::settings::workflows::Workflow;
use snapline_store::settings::{self as store_settings, Workflows};
use ts_rs::TS;

use crate::dto::hotkeys::{
    BindingDto, BlockedDto, BlockedReasonDto, HotkeysReq, HotkeysResp, PreviewDto,
};
use crate::{ServiceError, Services};

pub trait HotkeyInput: Send + Sync {
    fn rebind(&self, bindings: Vec<(Owner, Chord)>) -> Result<Vec<Blocked>, ServiceError>;
    fn pause(&self) -> Result<(), ServiceError>;
    fn resume(&self) -> Result<(), ServiceError>;
}

pub trait PrtscRegistry: Send + Sync {
    fn current(&self) -> Result<Option<u32>, ServiceError>;
    fn enable(&self) -> Result<Option<u32>, ServiceError>;
    fn restore(&self, previous: Option<u32>) -> Result<(), ServiceError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "HotkeysAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct HotkeysRuntime {
    pub(crate) input: Option<Arc<dyn HotkeyInput>>,
    pub(crate) prtsc: Option<Arc<dyn PrtscRegistry>>,
    pub(crate) notifier: Option<Arc<dyn Notifier + Send + Sync>>,
    entries: Vec<(Owner, Chord)>,
    bindings: Vec<(Owner, Chord)>,
    blocked: Vec<Blocked>,
    external: Vec<Chord>,
    labels: BTreeMap<String, String>,
    paused: bool,
    applied: bool,
    prtsc_applied: bool,
}

impl Default for HotkeysRuntime {
    fn default() -> Self {
        Self {
            input: None,
            prtsc: None,
            notifier: None,
            entries: Vec::new(),
            bindings: Vec::new(),
            blocked: Vec::new(),
            external: Vec::new(),
            labels: BTreeMap::new(),
            paused: false,
            applied: false,
            prtsc_applied: false,
        }
    }
}

pub struct HotkeysService {
    services: Services,
}

impl HotkeysService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }

    pub fn handle(&self, req: HotkeysReq) -> Result<HotkeysResp, ServiceError> {
        match req {
            HotkeysReq::Ping => Ok(HotkeysResp::Pong),
            HotkeysReq::Get => {
                self.ensure_plan()?;
                let runtime = self.runtime()?;
                Ok(state_response(&runtime))
            }
            HotkeysReq::Preview { owner, chord } => {
                self.ensure_plan()?;
                let chord = Chord::parse(&chord)
                    .map_err(|error| ServiceError::Invalid(error.to_string()))?;
                let (entries, external) = {
                    let runtime = self.runtime()?;
                    (runtime.entries.clone(), runtime.external.clone())
                };
                let preview =
                    hotkey_plan::preview(&entries, &(Owner::new(owner), chord), &external);
                Ok(HotkeysResp::Preview {
                    preview: preview_dto(preview),
                })
            }
            HotkeysReq::Pause => self
                .set_paused(true)
                .map(|paused| HotkeysResp::Paused { paused }),
            HotkeysReq::Resume => self
                .set_paused(false)
                .map(|paused| HotkeysResp::Paused { paused }),
        }
    }

    pub fn on_settings_changed(&self) -> Result<(), ServiceError> {
        if let Err(error) = self.sync_prtsc() {
            tracing::warn!(%error, "the PrtSc key setting could not be changed");
        }
        self.compute(true)
    }

    pub fn on_resume(&self) -> Result<(), ServiceError> {
        self.compute(true)
    }

    pub fn set_paused(&self, paused: bool) -> Result<bool, ServiceError> {
        let (input, current) = {
            let runtime = self.runtime()?;
            (runtime.input.clone(), runtime.paused)
        };
        if paused == current {
            return Ok(paused);
        }
        self.runtime()?.paused = paused;
        if let Some(input) = input {
            if paused {
                input.pause()?;
            } else {
                input.resume()?;
            }
        }
        self.compute(!paused)?;
        Ok(paused)
    }

    fn ensure_plan(&self) -> Result<(), ServiceError> {
        if self.runtime()?.applied {
            return Ok(());
        }
        self.compute(false)
    }

    fn compute(&self, notify: bool) -> Result<(), ServiceError> {
        let settings = self.services.settings_snapshot()?;
        let entries = collect_entries(&settings);
        let labels = workflow_labels(&settings.workflows);
        let plan = hotkey_plan::plan(&entries);
        let (input, paused) = {
            let runtime = self.runtime()?;
            (runtime.input.clone(), runtime.paused)
        };
        let external = match input {
            Some(input) if paused => {
                input.pause()?;
                Vec::new()
            }
            Some(input) => input.rebind(plan.bind.clone())?,
            None => Vec::new(),
        };
        let failures: Vec<Chord> = external.iter().map(|blocked| blocked.chord).collect();
        let full = plan.with_external(&failures);
        {
            let mut runtime = self.runtime()?;
            runtime.entries = entries;
            runtime.bindings = if runtime.paused {
                Vec::new()
            } else {
                full.bind.clone()
            };
            runtime.blocked = full.blocked.clone();
            runtime.external = failures;
            runtime.labels = labels;
            runtime.applied = true;
        }
        if notify && settings.hotkeys.warn_about_clashes && !full.blocked.is_empty() {
            self.notify_clash(full.blocked.len());
        }
        Ok(())
    }

    fn sync_prtsc(&self) -> Result<(), ServiceError> {
        let settings = self.services.settings_snapshot()?;
        let (registry, applied) = {
            let runtime = self.runtime()?;
            (runtime.prtsc.clone(), runtime.prtsc_applied)
        };
        let Some(registry) = registry else {
            return Ok(());
        };
        let take_over = settings.hotkeys.take_over_prtsc;
        let stored = settings
            .hotkeys
            .restore
            .print_screen_key_for_snipping_enabled;
        if take_over {
            if applied {
                return Ok(());
            }
            if stored.is_some() {
                let _ = registry.enable()?;
            } else if registry.current()? == Some(0) {
                self.runtime()?.prtsc_applied = true;
                return Ok(());
            } else {
                let previous = registry.enable()?;
                self.store_restore(previous)?;
            }
            self.runtime()?.prtsc_applied = true;
        } else if applied || stored.is_some() {
            registry.restore(stored)?;
            self.store_restore(None)?;
            self.runtime()?.prtsc_applied = false;
        }
        Ok(())
    }

    fn store_restore(&self, previous: Option<u32>) -> Result<(), ServiceError> {
        let value = previous.map_or(serde_json::Value::Null, |value| serde_json::json!(value));
        let patch = serde_json::json!({
            "hotkeys": {
                "restore": { "print_screen_key_for_snipping_enabled": value }
            }
        });
        self.services.update_settings(&patch)?;
        Ok(())
    }

    fn notify_clash(&self, count: usize) {
        let text = clash_text(count);
        let notifier = self
            .runtime()
            .ok()
            .and_then(|runtime| runtime.notifier.clone());
        if let Some(notifier) = notifier {
            notifier.toast(Notice::new("Hotkeys", text));
        } else {
            tracing::info!(message = %text, "hotkeys are off because of a clash");
        }
    }

    fn runtime(&self) -> Result<MutexGuard<'_, HotkeysRuntime>, ServiceError> {
        self.services
            .hotkeys_state()
            .map_err(|_| ServiceError::Internal("Hotkeys are unavailable.".to_owned()))
    }
}

pub fn clash_text(count: usize) -> String {
    if count == 1 {
        "1 hotkey is off because of a clash".to_owned()
    } else {
        format!("{count} hotkeys are off because of a clash")
    }
}

pub fn collect_entries(settings: &store_settings::Settings) -> Vec<(Owner, Chord)> {
    let workflows = workflow_entries(&settings.workflows);
    plan_entries(&settings.hotkeys.bindings, &workflows)
}

pub fn plan_entries(
    bindings: &BTreeMap<String, String>,
    workflows: &[(String, bool, String)],
) -> Vec<(Owner, Chord)> {
    let mut entries = Vec::new();
    for definition in CATALOGUE {
        let Some(text) = bindings.get(definition.id) else {
            continue;
        };
        if let Ok(chord) = Chord::parse(text) {
            entries.push((Owner::from(definition.id), chord));
        }
    }
    for (id, enabled, text) in workflows {
        if !enabled {
            continue;
        }
        if let Ok(chord) = Chord::parse(text) {
            entries.push((
                Owner::new(format!("{}{}", actions::WORKFLOW_PREFIX, id)),
                chord,
            ));
        }
    }
    entries
}

fn workflow_entries(workflows: &Workflows) -> Vec<(String, bool, String)> {
    workflows.list.iter().filter_map(workflow_entry).collect()
}

fn workflow_entry(workflow: &Workflow) -> Option<(String, bool, String)> {
    let chord = workflow.hotkey?;
    Some((
        workflow.id.as_str().to_owned(),
        workflow.enabled,
        chord.to_string(),
    ))
}

fn workflow_labels(workflows: &Workflows) -> BTreeMap<String, String> {
    workflows
        .list
        .iter()
        .map(|workflow| {
            (
                format!("{}{}", actions::WORKFLOW_PREFIX, workflow.id.as_str()),
                workflow.name.clone(),
            )
        })
        .collect()
}

pub fn label_for(owner: &Owner) -> String {
    let id = owner.as_str();
    if let Some(rest) = id.strip_prefix(actions::WORKFLOW_PREFIX) {
        return format!("Workflow {rest}");
    }
    actions::find(&ActionId::new(id))
        .map_or_else(|| id.to_owned(), |definition| definition.label.to_owned())
}

fn label_with(owner: &Owner, labels: &BTreeMap<String, String>) -> String {
    labels
        .get(owner.as_str())
        .cloned()
        .unwrap_or_else(|| label_for(owner))
}

fn state_response(runtime: &HotkeysRuntime) -> HotkeysResp {
    let bindings = runtime
        .bindings
        .iter()
        .map(|(owner, chord)| BindingDto {
            owner: owner.as_str().to_owned(),
            label: label_with(owner, &runtime.labels),
            chord: chord.to_string(),
        })
        .collect();
    let blocked = runtime
        .blocked
        .iter()
        .flat_map(|blocked| blocked_rows(blocked, &runtime.labels))
        .collect();
    HotkeysResp::State {
        bindings,
        blocked,
        paused: runtime.paused,
    }
}

fn blocked_rows(blocked: &Blocked, labels: &BTreeMap<String, String>) -> Vec<BlockedDto> {
    let reason = match blocked.reason {
        BlockReason::Internal => BlockedReasonDto::Internal,
        BlockReason::External => BlockedReasonDto::External,
    };
    blocked
        .owners
        .iter()
        .map(|owner| BlockedDto {
            owner: owner.as_str().to_owned(),
            label: label_with(owner, labels),
            chord: blocked.chord.to_string(),
            reason,
        })
        .collect()
}

fn preview_dto(preview: CorePreview) -> PreviewDto {
    match preview {
        CorePreview::Free => PreviewDto::Free,
        CorePreview::External => PreviewDto::External,
        CorePreview::Internal { owners } => PreviewDto::Internal {
            owners: owners
                .iter()
                .map(|owner| owner.as_str().to_owned())
                .collect(),
        },
    }
}

pub fn run(_action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    Err(ServiceError::NotYet)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use snapline_store::io::Paths;

    use super::*;

    #[derive(Default)]
    struct FakeInput {
        bindings: Mutex<Vec<(Owner, Chord)>>,
        external: Vec<Chord>,
        pauses: Mutex<u32>,
        resumes: Mutex<u32>,
        rebinds: Mutex<u32>,
    }

    impl HotkeyInput for FakeInput {
        fn rebind(&self, bindings: Vec<(Owner, Chord)>) -> Result<Vec<Blocked>, ServiceError> {
            *self.bindings.lock().expect("bindings") = bindings.clone();
            *self.rebinds.lock().expect("rebinds") += 1;
            Ok(bindings
                .into_iter()
                .filter(|(_, chord)| self.external.contains(chord))
                .map(|(owner, chord)| Blocked::external(chord, owner))
                .collect())
        }

        fn pause(&self) -> Result<(), ServiceError> {
            *self.pauses.lock().expect("pauses") += 1;
            Ok(())
        }

        fn resume(&self) -> Result<(), ServiceError> {
            *self.resumes.lock().expect("resumes") += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeRegistry {
        value: Mutex<Option<u32>>,
    }

    impl PrtscRegistry for FakeRegistry {
        fn current(&self) -> Result<Option<u32>, ServiceError> {
            Ok(*self.value.lock().expect("value"))
        }

        fn enable(&self) -> Result<Option<u32>, ServiceError> {
            let mut value = self.value.lock().expect("value");
            let previous = *value;
            *value = Some(0);
            Ok(previous)
        }

        fn restore(&self, previous: Option<u32>) -> Result<(), ServiceError> {
            *self.value.lock().expect("value") = previous;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeNotifier {
        notices: Mutex<Vec<String>>,
    }

    impl Notifier for FakeNotifier {
        fn toast(&self, notice: Notice) {
            self.notices.lock().expect("notices").push(notice.body);
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

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("chord")
    }

    fn state(resp: HotkeysResp) -> HotkeysResp {
        match resp {
            HotkeysResp::State { .. } => resp,
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn blocked_of(resp: &HotkeysResp) -> Vec<BlockedDto> {
        match resp {
            HotkeysResp::State { blocked, .. } => blocked.clone(),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn bindings_of(resp: &HotkeysResp) -> Vec<BindingDto> {
        match resp {
            HotkeysResp::State { bindings, .. } => bindings.clone(),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    fn clash_chord_patch() -> serde_json::Value {
        serde_json::json!({
            "hotkeys": {
                "bindings": {
                    "capture_region": "Ctrl+Shift+A",
                    "capture_window": "Ctrl+Shift+A"
                }
            },
            "workflows": { "list": [] }
        })
    }

    #[test]
    fn chord_collection_excludes_disabled_workflows() {
        let bindings = BTreeMap::from([("capture_region".to_owned(), "PrtSc".to_owned())]);
        let workflows = vec![
            ("grab_text".to_owned(), false, "Ctrl+Alt+T".to_owned()),
            ("annotate".to_owned(), true, "Ctrl+Alt+R".to_owned()),
        ];
        let entries = plan_entries(&bindings, &workflows);
        assert_eq!(
            entries,
            vec![
                (Owner::new("capture_region"), chord("PrtSc")),
                (Owner::new("workflow:annotate"), chord("Ctrl+Alt+R")),
            ]
        );
    }

    #[test]
    fn chord_collection_adds_every_enabled_workflow_chord() {
        let bindings = BTreeMap::from([("capture_region".to_owned(), "PrtSc".to_owned())]);
        let workflows = vec![
            (
                "annotate-and-share".to_owned(),
                true,
                "Ctrl+Shift+A".to_owned(),
            ),
            ("bug-report".to_owned(), true, "Ctrl+Alt+R".to_owned()),
        ];
        let entries = plan_entries(&bindings, &workflows);
        assert_eq!(
            entries,
            vec![
                (Owner::new("capture_region"), chord("PrtSc")),
                (
                    Owner::new("workflow:annotate-and-share"),
                    chord("Ctrl+Shift+A")
                ),
                (Owner::new("workflow:bug-report"), chord("Ctrl+Alt+R")),
            ]
        );
    }

    #[test]
    fn a_workflow_without_a_chord_contributes_nothing() {
        let workflow = snapline_store::settings::workflows::default_workflow();
        assert_eq!(workflow_entry(&workflow), None);
        let workflows: Vec<(String, bool, String)> = Vec::new();
        assert!(plan_entries(&BTreeMap::new(), &workflows).is_empty());
    }

    #[test]
    fn an_enabled_workflow_clashing_with_an_action_blocks_both() {
        let bindings = BTreeMap::from([
            ("capture_region".to_owned(), "Ctrl+Shift+A".to_owned()),
            ("capture_window".to_owned(), "Alt+PrtSc".to_owned()),
        ]);
        let workflows = vec![(
            "annotate-and-share".to_owned(),
            true,
            "Ctrl+Shift+A".to_owned(),
        )];
        let entries = plan_entries(&bindings, &workflows);
        let plan = hotkey_plan::plan(&entries);
        assert_eq!(
            plan.blocked,
            vec![Blocked::internal(
                chord("Ctrl+Shift+A"),
                vec![
                    Owner::new("capture_region"),
                    Owner::new("workflow:annotate-and-share"),
                ]
            )]
        );
        assert_eq!(
            plan.bind,
            vec![(Owner::new("capture_window"), chord("Alt+PrtSc"))]
        );
    }

    #[test]
    fn workflow_labels_use_the_saved_name() {
        let mut workflow =
            snapline_core::workflow::Workflow::new("annotate-and-share", "Annotate and share");
        workflow.hotkey = Some(chord("Ctrl+Shift+A"));
        let workflows = Workflows {
            list: vec![workflow],
            extra: toml::Table::new(),
        };
        let labels = workflow_labels(&workflows);
        assert_eq!(
            labels.get("workflow:annotate-and-share"),
            Some(&"Annotate and share".to_owned())
        );
    }

    #[test]
    fn an_internal_clash_blocks_both_owners_and_binds_neither() {
        let directory = tempfile::tempdir().expect("temp dir");
        let input = Arc::new(FakeInput::default());
        let services = services(directory.path()).with_hotkey_input(input.clone());
        services
            .update_settings(&clash_chord_patch())
            .expect("settings");
        services.hotkeys().on_settings_changed().expect("refresh");
        let resp = state(services.hotkeys().handle(HotkeysReq::Get).expect("get"));
        let blocked = blocked_of(&resp);
        assert_eq!(
            blocked,
            vec![
                BlockedDto {
                    owner: "capture_region".to_owned(),
                    label: "Capture region".to_owned(),
                    chord: "Ctrl+Shift+A".to_owned(),
                    reason: BlockedReasonDto::Internal,
                },
                BlockedDto {
                    owner: "capture_window".to_owned(),
                    label: "Capture window".to_owned(),
                    chord: "Ctrl+Shift+A".to_owned(),
                    reason: BlockedReasonDto::Internal,
                },
            ]
        );
        assert!(bindings_of(&resp)
            .iter()
            .all(|binding| binding.chord != "Ctrl+Shift+A"));
        assert!(input
            .bindings
            .lock()
            .expect("bindings")
            .iter()
            .all(|(_, value)| value != &chord("Ctrl+Shift+A")));
    }

    #[test]
    fn an_external_failure_is_reported_exactly() {
        let directory = tempfile::tempdir().expect("temp dir");
        let input = Arc::new(FakeInput {
            external: vec![chord("PrtSc")],
            ..FakeInput::default()
        });
        let services = services(directory.path()).with_hotkey_input(input);
        services.hotkeys().on_settings_changed().expect("refresh");
        let resp = state(services.hotkeys().handle(HotkeysReq::Get).expect("get"));
        assert_eq!(
            blocked_of(&resp),
            vec![BlockedDto {
                owner: "capture_region".to_owned(),
                label: "Capture region".to_owned(),
                chord: "PrtSc".to_owned(),
                reason: BlockedReasonDto::External,
            }]
        );
        assert!(bindings_of(&resp)
            .iter()
            .all(|binding| binding.owner != "capture_region"));
    }

    #[test]
    fn clash_text_is_exact_for_singular_and_plural() {
        assert_eq!(clash_text(1), "1 hotkey is off because of a clash");
        assert_eq!(clash_text(2), "2 hotkeys are off because of a clash");
        assert_eq!(clash_text(11), "11 hotkeys are off because of a clash");
    }

    #[test]
    fn one_notice_is_shown_for_a_clash() {
        let directory = tempfile::tempdir().expect("temp dir");
        let notifier = Arc::new(FakeNotifier::default());
        let services = services(directory.path()).with_notifier(notifier.clone());
        services
            .update_settings(&clash_chord_patch())
            .expect("settings");
        services.hotkeys().on_settings_changed().expect("refresh");
        assert_eq!(
            *notifier.notices.lock().expect("notices"),
            vec!["2 hotkeys are off because of a clash".to_owned()]
        );
    }

    #[test]
    fn a_single_external_failure_uses_the_singular_notice() {
        let directory = tempfile::tempdir().expect("temp dir");
        let notifier = Arc::new(FakeNotifier::default());
        let input = Arc::new(FakeInput {
            external: vec![chord("PrtSc")],
            ..FakeInput::default()
        });
        let services = services(directory.path())
            .with_hotkey_input(input)
            .with_notifier(notifier.clone());
        services.hotkeys().on_settings_changed().expect("refresh");
        assert_eq!(
            *notifier.notices.lock().expect("notices"),
            vec!["1 hotkey is off because of a clash".to_owned()]
        );
    }

    #[test]
    fn pause_unregisters_and_resume_replans() {
        let directory = tempfile::tempdir().expect("temp dir");
        let input = Arc::new(FakeInput::default());
        let services = services(directory.path()).with_hotkey_input(input.clone());
        services.hotkeys().on_settings_changed().expect("refresh");
        let before = *input.rebinds.lock().expect("rebinds");
        assert!(services.hotkeys().set_paused(true).expect("pause"));
        assert_eq!(*input.pauses.lock().expect("pauses"), 2);
        let paused = state(services.hotkeys().handle(HotkeysReq::Get).expect("get"));
        match paused {
            HotkeysResp::State {
                paused, bindings, ..
            } => {
                assert!(paused);
                assert!(bindings.is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
        }
        assert!(!services.hotkeys().set_paused(false).expect("resume"));
        assert_eq!(*input.resumes.lock().expect("resumes"), 1);
        assert!(*input.rebinds.lock().expect("rebinds") > before);
        match services.hotkeys().handle(HotkeysReq::Get).expect("get") {
            HotkeysResp::State {
                paused, bindings, ..
            } => {
                assert!(!paused);
                assert!(!bindings.is_empty());
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn preview_checks_bindings_and_external_failures_while_typing() {
        let directory = tempfile::tempdir().expect("temp dir");
        let input = Arc::new(FakeInput {
            external: vec![chord("Ctrl+PrtSc")],
            ..FakeInput::default()
        });
        let services = services(directory.path()).with_hotkey_input(input);
        services.hotkeys().on_settings_changed().expect("refresh");
        assert_eq!(
            services
                .hotkeys()
                .handle(HotkeysReq::Preview {
                    owner: "capture_screen".to_owned(),
                    chord: "Alt+PrtSc".to_owned(),
                })
                .expect("preview"),
            HotkeysResp::Preview {
                preview: PreviewDto::Internal {
                    owners: vec!["capture_window".to_owned()]
                }
            }
        );
        assert_eq!(
            services
                .hotkeys()
                .handle(HotkeysReq::Preview {
                    owner: "capture_screen".to_owned(),
                    chord: "Ctrl+PrtSc".to_owned(),
                })
                .expect("preview"),
            HotkeysResp::Preview {
                preview: PreviewDto::External
            }
        );
        assert_eq!(
            services
                .hotkeys()
                .handle(HotkeysReq::Preview {
                    owner: "capture_screen".to_owned(),
                    chord: "Win+F5".to_owned(),
                })
                .expect("preview"),
            HotkeysResp::Preview {
                preview: PreviewDto::Free
            }
        );
    }

    #[test]
    fn preview_rejects_an_invalid_chord_with_the_core_message() {
        let services = Services::new();
        let error = services
            .hotkeys()
            .handle(HotkeysReq::Preview {
                owner: "capture_region".to_owned(),
                chord: "Win+L".to_owned(),
            })
            .expect_err("reserved");
        assert_eq!(error.to_string(), "Windows uses this shortcut.");
    }

    #[test]
    fn prtsc_takeover_remembers_and_restores_the_previous_value() {
        let directory = tempfile::tempdir().expect("temp dir");
        let registry = Arc::new(FakeRegistry {
            value: Mutex::new(Some(1)),
        });
        let services = services(directory.path()).with_prtsc_registry(registry.clone());
        services.hotkeys().on_settings_changed().expect("enable");
        assert_eq!(*registry.value.lock().expect("value"), Some(0));
        assert_eq!(
            services
                .settings_snapshot()
                .expect("settings")
                .hotkeys
                .restore
                .print_screen_key_for_snipping_enabled,
            Some(1)
        );
        services
            .update_settings(&serde_json::json!({
                "hotkeys": { "take_over_prtsc": false }
            }))
            .expect("settings");
        services.hotkeys().on_settings_changed().expect("restore");
        assert_eq!(*registry.value.lock().expect("value"), Some(1));
        assert_eq!(
            services
                .settings_snapshot()
                .expect("settings")
                .hotkeys
                .restore
                .print_screen_key_for_snipping_enabled,
            None
        );
    }

    #[test]
    fn prtsc_takeover_remembers_an_absent_value_as_an_absent_value() {
        let directory = tempfile::tempdir().expect("temp dir");
        let registry = Arc::new(FakeRegistry::default());
        let services = services(directory.path()).with_prtsc_registry(registry.clone());
        services.hotkeys().on_settings_changed().expect("enable");
        assert_eq!(*registry.value.lock().expect("value"), Some(0));
        assert_eq!(
            services
                .settings_snapshot()
                .expect("settings")
                .hotkeys
                .restore
                .print_screen_key_for_snipping_enabled,
            None
        );
        services
            .update_settings(&serde_json::json!({
                "hotkeys": { "take_over_prtsc": false }
            }))
            .expect("settings");
        services.hotkeys().on_settings_changed().expect("restore");
        assert_eq!(*registry.value.lock().expect("value"), None);
    }

    #[test]
    fn ping_round_trips() {
        let services = Services::new();
        assert_eq!(
            services.hotkeys().handle(HotkeysReq::Ping),
            Ok(HotkeysResp::Pong)
        );
    }

    #[test]
    fn get_without_an_input_port_returns_the_planned_bindings() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let resp = state(services.hotkeys().handle(HotkeysReq::Get).expect("get"));
        assert!(bindings_of(&resp)
            .iter()
            .any(|binding| binding.owner == "capture_region" && binding.chord == "PrtSc"));
        assert!(blocked_of(&resp).is_empty());
    }

    #[test]
    fn label_for_reads_the_action_catalogue() {
        assert_eq!(label_for(&Owner::new("capture_region")), "Capture region");
        assert_eq!(
            label_for(&Owner::new("workflow:annotate")),
            "Workflow annotate"
        );
        assert_eq!(label_for(&Owner::new("missing")), "missing");
    }
}
