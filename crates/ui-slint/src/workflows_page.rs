use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{bail, Result};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};
use snapline_services::dto::hotkeys::{BlockedReasonDto, HotkeysResp, PreviewDto};
use snapline_services::dto::settings::LinkFormatDto;
use snapline_services::dto::workflow::{
    CaptureStepKindDto, DestinationOption, RecordStepKindDto, StepDto, WorkflowInfo, WorkflowInput,
    WorkflowReq, WorkflowResp,
};

use crate::backend::Backend;
use crate::{EditorStepData, SnaplineWindow, WorkflowItemData, WorkflowsState};

pub const NAME_ERROR: &str = "The name must be 1 to 60 characters.";

pub const STEP_CHOICES: &[(&str, &str)] = &[
    ("capture", "Capture"),
    ("record", "Record"),
    ("copy_image", "Copy image"),
    ("annotate", "Annotate"),
    ("save", "Save"),
    ("upload", "Upload"),
    ("copy_link", "Copy link"),
    ("extract_text", "Extract text"),
    ("copy_text", "Copy text"),
    ("convert_to_gif", "Convert to GIF"),
    ("notify", "Notify"),
    ("play_sound", "Play sound"),
];

const CAPTURE_KINDS: &[(CaptureStepKindDto, &str)] = &[
    (CaptureStepKindDto::Region, "Capture region"),
    (CaptureStepKindDto::Window, "Capture window"),
    (CaptureStepKindDto::Screen, "Capture screen"),
    (CaptureStepKindDto::LastRegion, "Capture last region"),
];

const RECORD_KINDS: &[(RecordStepKindDto, &str)] = &[
    (RecordStepKindDto::Video, "Record video"),
    (RecordStepKindDto::Gif, "Record GIF"),
    (RecordStepKindDto::Window, "Record window"),
];

const LINK_FORMATS: &[(LinkFormatDto, &str)] = &[
    (LinkFormatDto::Direct, "Direct link"),
    (LinkFormatDto::Markdown, "Markdown"),
    (LinkFormatDto::Html, "HTML <img>"),
    (LinkFormatDto::Bbcode, "BBCode"),
    (LinkFormatDto::Page, "Page link"),
];

#[derive(Default, Clone)]
pub struct Editor {
    pub id: Option<String>,
    pub is_default: bool,
    pub name: String,
    pub hotkey: String,
    pub enabled: bool,
    pub steps: Vec<StepDto>,
    pub destinations: Vec<DestinationOption>,
}

pub type SharedEditor = Rc<RefCell<Editor>>;

pub fn name_map(backend: &Backend) -> HashMap<String, String> {
    let Ok(WorkflowResp::List { workflows, .. }) = backend.workflow_request(WorkflowReq::List)
    else {
        return HashMap::new();
    };
    workflows.into_iter().map(|w| (w.id, w.name)).collect()
}

pub fn display_name(workflow: &WorkflowInfo) -> String {
    if workflow.is_default {
        format!("{} (default)", workflow.name)
    } else {
        workflow.name.clone()
    }
}

pub fn step_label(step: &StepDto) -> &'static str {
    match step {
        StepDto::Capture { kind } => match kind {
            CaptureStepKindDto::Region => "Capture region",
            CaptureStepKindDto::Window => "Capture window",
            CaptureStepKindDto::Screen => "Capture screen",
            CaptureStepKindDto::LastRegion => "Capture last region",
        },
        StepDto::Record { kind } => match kind {
            RecordStepKindDto::Video => "Record video",
            RecordStepKindDto::Gif => "Record GIF",
            RecordStepKindDto::Window => "Record window",
        },
        StepDto::CopyImage => "Copy image",
        StepDto::Annotate => "Annotate",
        StepDto::Save => "Save",
        StepDto::Upload { .. } => "Upload",
        StepDto::CopyLink { .. } => "Copy link",
        StepDto::ExtractText => "Extract text",
        StepDto::CopyText => "Copy text",
        StepDto::ConvertToGif => "Convert to GIF",
        StepDto::Notify => "Notify",
        StepDto::PlaySound => "Play sound",
    }
}

pub fn step_icon(step: &StepDto) -> &'static str {
    match step {
        StepDto::Capture { kind } => match kind {
            CaptureStepKindDto::LastRegion => "stop",
            CaptureStepKindDto::Window => "window",
            CaptureStepKindDto::Screen => "screen",
            CaptureStepKindDto::Region => "region",
        },
        StepDto::Record { kind } => match kind {
            RecordStepKindDto::Gif => "gif-mark",
            RecordStepKindDto::Window => "window",
            RecordStepKindDto::Video => "record",
        },
        StepDto::CopyImage => "copy",
        StepDto::Annotate => "edit",
        StepDto::Save => "save",
        StepDto::Upload { .. } => "upload",
        StepDto::CopyLink { .. } => "link",
        StepDto::ExtractText => "ocr",
        StepDto::CopyText => "text",
        StepDto::ConvertToGif => "gif",
        StepDto::Notify => "bell",
        StepDto::PlaySound => "speaker",
    }
}

fn new_step(choice: &str) -> Option<StepDto> {
    Some(match choice {
        "capture" => StepDto::Capture {
            kind: CaptureStepKindDto::Region,
        },
        "record" => StepDto::Record {
            kind: RecordStepKindDto::Video,
        },
        "copy_image" => StepDto::CopyImage,
        "annotate" => StepDto::Annotate,
        "save" => StepDto::Save,
        "upload" => StepDto::Upload { dest: None },
        "copy_link" => StepDto::CopyLink {
            format: LinkFormatDto::Direct,
        },
        "extract_text" => StepDto::ExtractText,
        "copy_text" => StepDto::CopyText,
        "convert_to_gif" => StepDto::ConvertToGif,
        "notify" => StepDto::Notify,
        "play_sound" => StepDto::PlaySound,
        _ => return None,
    })
}

fn owner_of(editor: &Editor) -> String {
    if editor.is_default {
        "capture_region".into()
    } else {
        format!(
            "workflow:{}",
            editor.id.clone().unwrap_or_else(|| "new".into())
        )
    }
}

fn workflow_input(editor: &Editor) -> WorkflowInput {
    WorkflowInput {
        name: editor.name.clone(),
        hotkey: if editor.hotkey.trim().is_empty() {
            None
        } else {
            Some(editor.hotkey.clone())
        },
        enabled: editor.enabled,
        steps: editor.steps.clone(),
    }
}

pub fn list(backend: &Backend) -> Result<(Vec<WorkflowInfo>, Vec<DestinationOption>)> {
    match backend.workflow_request(WorkflowReq::List)? {
        WorkflowResp::List {
            workflows,
            destinations,
        } => Ok((workflows, destinations)),
        other => bail!("unexpected workflow list response: {other:?}"),
    }
}

pub fn get(backend: &Backend, id: &str) -> Result<WorkflowInfo> {
    match backend.workflow_request(WorkflowReq::Get { id: id.to_string() })? {
        WorkflowResp::Workflow { workflow } => Ok(workflow),
        other => bail!("unexpected workflow response: {other:?}"),
    }
}

fn violations(
    backend: &Backend,
    id: Option<&str>,
    editor: &Editor,
) -> Result<Vec<(Option<usize>, String)>> {
    match backend.workflow_request(WorkflowReq::Validate {
        id: id.map(str::to_string),
        workflow: workflow_input(editor),
    })? {
        WorkflowResp::Validation { violations } => Ok(violations
            .into_iter()
            .map(|v| (v.step, v.message))
            .collect()),
        other => bail!("unexpected validation response: {other:?}"),
    }
}

pub fn matches_query(workflow: &WorkflowInfo, chord: &str, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    if display_name(workflow).to_lowercase().contains(&needle)
        || chord.to_lowercase().contains(&needle)
    {
        return true;
    }
    workflow
        .steps
        .iter()
        .any(|step| step_label(step).to_lowercase().contains(&needle))
}

pub fn workflow_owner(workflow: &WorkflowInfo) -> String {
    if workflow.is_default {
        "capture_region".to_string()
    } else {
        format!("workflow:{}", workflow.id)
    }
}

pub fn chord_for(owner: &str, workflow: &WorkflowInfo, hotkeys: &HotkeysResp) -> String {
    if let HotkeysResp::State {
        bindings, blocked, ..
    } = hotkeys
    {
        if let Some(binding) = bindings.iter().find(|b| b.owner == owner) {
            return binding.chord.clone();
        }
        if let Some(blocked) = blocked.iter().find(|b| b.owner == owner) {
            return blocked.chord.clone();
        }
    }
    workflow.hotkey.clone().unwrap_or_else(|| "—".to_string())
}

pub fn chord_blocked(owner: &str, hotkeys: &HotkeysResp) -> bool {
    let HotkeysResp::State { blocked, .. } = hotkeys else {
        return false;
    };
    blocked.iter().any(|b| b.owner == owner)
}

pub fn clash_for(owner: &str, hotkeys: &HotkeysResp) -> String {
    let HotkeysResp::State { blocked, .. } = hotkeys else {
        return String::new();
    };
    let Some(entry) = blocked.iter().find(|b| b.owner == owner) else {
        return String::new();
    };
    if entry.reason != BlockedReasonDto::Internal {
        return String::new();
    }
    match blocked.iter().find(|b| {
        b.owner != owner && b.chord == entry.chord && b.reason == BlockedReasonDto::Internal
    }) {
        Some(other) => format!(
            "Off because {} is also used by {}.",
            entry.chord, other.label
        ),
        None => String::new(),
    }
}

pub fn refresh_list(app: &Weak<SnaplineWindow>, backend: &Backend) -> Result<()> {
    let Some(app) = app.upgrade() else {
        return Ok(());
    };
    let (workflows, _) = list(backend)?;
    let hotkeys = backend.hotkeys()?;
    let state = app.global::<WorkflowsState>();
    let query = state.get_query().to_string();
    state.set_has_workflows(!workflows.is_empty());

    let items: Vec<WorkflowItemData> = workflows
        .iter()
        .filter_map(|w| {
            let owner = workflow_owner(w);
            let chord = chord_for(&owner, w, &hotkeys);
            if !matches_query(w, &chord, &query) {
                return None;
            }
            let step_labels: Vec<&'static str> = w.steps.iter().map(step_label).collect();
            Some(WorkflowItemData {
                id: w.id.clone().into(),
                title: display_name(w).into(),
                chord: chord.into(),
                chord_blocked: chord_blocked(&owner, &hotkeys),
                steps: step_labels.join("  ›  ").into(),
                step_list: ModelRc::new(VecModel::from(
                    step_labels
                        .into_iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )),
                clash: clash_for(&owner, &hotkeys).into(),
                enabled: w.enabled,
                is_default: w.is_default,
            })
        })
        .collect();
    state.set_items(ModelRc::new(VecModel::from(items)));
    Ok(())
}

pub fn push_editor(app: &Weak<SnaplineWindow>, backend: &Backend, editor: &Editor) -> Result<()> {
    let Some(app) = app.upgrade() else {
        return Ok(());
    };
    let issues = violations(backend, editor.id.as_deref(), editor).unwrap_or_default();
    let name_error = issues
        .iter()
        .find(|(_, m)| m == NAME_ERROR)
        .map(|(_, m)| m.clone())
        .unwrap_or_default();
    let list_error = issues
        .iter()
        .find(|(step, _)| step.is_none())
        .filter(|(_, m)| m != NAME_ERROR)
        .map(|(_, m)| m.clone())
        .unwrap_or_default();
    let steps: Vec<EditorStepData> = editor
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let step_errors: Vec<String> = issues
                .iter()
                .filter(|(s, _)| *s == Some(index))
                .map(|(_, m)| m.clone())
                .collect();
            let (choices, choice) = step_choices(step, &editor.destinations);
            EditorStepData {
                label: step_label(step).into(),
                icon: step_icon(step).into(),
                index: index as i32,
                error: step_errors.join("\n").into(),
                choices: ModelRc::new(VecModel::from(
                    choices
                        .into_iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )),
                choice: choice.into(),
            }
        })
        .collect();
    let clash = if editor.is_default {
        String::new()
    } else {
        hotkey_clash(backend, editor, &editor.hotkey)
    };
    let s = app.global::<WorkflowsState>();
    s.set_editing(true);
    s.set_editor_is_default(editor.is_default);
    if s.get_editor_name().as_str() != editor.name {
        s.set_editor_name(editor.name.clone().into());
    }
    s.set_editor_hotkey(editor.hotkey.clone().into());
    s.set_editor_clash(clash.into());
    s.set_editor_enabled(editor.enabled);
    s.set_name_error(name_error.into());
    s.set_list_error(list_error.into());
    s.set_editor_steps(ModelRc::new(VecModel::from(steps)));
    s.set_add_choices(ModelRc::new(VecModel::from(
        STEP_CHOICES
            .iter()
            .map(|(_, label)| SharedString::from(*label))
            .collect::<Vec<_>>(),
    )));
    s.set_is_new(editor.id.is_none());
    s.set_editor_title(if editor.id.is_none() || editor.name.is_empty() {
        "New workflow".into()
    } else {
        editor.name.clone().into()
    });
    Ok(())
}

fn find_label<T: Copy + Eq>(pairs: &[(T, &str)], value: T) -> String {
    pairs
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_default()
}

fn find_value<T: Copy>(pairs: &[(T, &str)], label: &str) -> Option<T> {
    pairs.iter().find(|(_, l)| *l == label).map(|(v, _)| *v)
}

fn step_choices(step: &StepDto, destinations: &[DestinationOption]) -> (Vec<String>, String) {
    match step {
        StepDto::Capture { kind } => (
            CAPTURE_KINDS
                .iter()
                .map(|(_, l)| (*l).to_string())
                .collect(),
            find_label(CAPTURE_KINDS, *kind),
        ),
        StepDto::Record { kind } => (
            RECORD_KINDS.iter().map(|(_, l)| (*l).to_string()).collect(),
            find_label(RECORD_KINDS, *kind),
        ),
        StepDto::CopyLink { format } => (
            LINK_FORMATS.iter().map(|(_, l)| (*l).to_string()).collect(),
            find_label(LINK_FORMATS, *format),
        ),
        StepDto::Upload { dest } => {
            let mut labels = vec!["Default destination".to_string()];
            labels.extend(destinations.iter().map(|d| d.name.clone()));
            let current = match dest.as_deref() {
                Some(dest_id) => destinations
                    .iter()
                    .find(|d| d.id == dest_id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| dest_id.to_string()),
                None => "Default destination".to_string(),
            };
            (labels, current)
        }
        _ => (Vec::new(), String::new()),
    }
}

fn set_choice(editor: &mut Editor, index: usize, label: &str) {
    let Some(step) = editor.steps.get_mut(index) else {
        return;
    };
    match step {
        StepDto::Capture { kind } => {
            if let Some(next) = find_value(CAPTURE_KINDS, label) {
                *kind = next;
            }
        }
        StepDto::Record { kind } => {
            if let Some(next) = find_value(RECORD_KINDS, label) {
                *kind = next;
            }
        }
        StepDto::CopyLink { format } => {
            if let Some(next) = find_value(LINK_FORMATS, label) {
                *format = next;
            }
        }
        StepDto::Upload { dest } => {
            *dest = if label == "Default destination" {
                None
            } else {
                editor
                    .destinations
                    .iter()
                    .find(|d| d.name == label)
                    .map(|d| d.id.clone())
            };
        }
        _ => {}
    }
}

pub fn open_new(backend: &Backend) -> Editor {
    let destinations = list(backend).map(|(_, d)| d).unwrap_or_default();
    Editor {
        enabled: true,
        destinations,
        ..Editor::default()
    }
}

pub fn open_existing(backend: &Backend, id: &str) -> Result<Editor> {
    let workflow = get(backend, id)?;
    let (_, destinations) = list(backend)?;
    Ok(Editor {
        id: Some(id.to_string()),
        is_default: workflow.is_default,
        name: workflow.name,
        hotkey: workflow.hotkey.unwrap_or_default(),
        enabled: workflow.enabled,
        steps: workflow.steps,
        destinations,
    })
}

pub fn hotkey_clash(backend: &Backend, editor: &Editor, chord: &str) -> String {
    if chord.trim().is_empty() {
        return String::new();
    }
    let Ok(preview) = backend.hotkey_preview(&owner_of(editor), chord) else {
        return String::new();
    };
    let mut workflow_names = name_map(backend);
    if let Some(id) = &editor.id {
        workflow_names.insert(id.clone(), editor.name.clone());
    }
    format_preview_clash(&preview, &owner_of(editor), &workflow_names)
}

pub fn format_preview_clash(
    preview: &PreviewDto,
    owner: &str,
    workflow_names: &HashMap<String, String>,
) -> String {
    match preview {
        PreviewDto::Free => String::new(),
        PreviewDto::External => "Off: another app is using this shortcut.".into(),
        PreviewDto::Internal { owners } => {
            let other_labels: Vec<String> = owners
                .iter()
                .filter(|o| o.as_str() != owner)
                .map(|o| crate::owner_label(o, workflow_names))
                .collect();
            if other_labels.is_empty() {
                String::new()
            } else {
                format!(
                    "Off: also used by {}.",
                    crate::format_owners(&other_labels, "")
                )
            }
        }
    }
}

pub fn save(backend: &Backend, editor: &Editor) -> Result<()> {
    match &editor.id {
        None => {
            backend.workflow_request(WorkflowReq::Create {
                workflow: workflow_input(editor),
            })?;
        }
        Some(id) => {
            backend.workflow_request(WorkflowReq::Update {
                id: id.clone(),
                workflow: workflow_input(editor),
            })?;
        }
    }
    Ok(())
}

pub fn set_enabled(backend: &Backend, id: &str, enabled: bool) -> Result<()> {
    let mut editor = open_existing(backend, id)?;
    editor.enabled = enabled;
    save(backend, &editor)
}

pub fn step_choice_apply(editor: &SharedEditor, index: usize, label: &str) {
    set_choice(&mut editor.borrow_mut(), index, label);
}

pub fn add_step(editor: &SharedEditor, label: &str) {
    let choice = STEP_CHOICES
        .iter()
        .find(|(_, l)| *l == label)
        .map(|(v, _)| *v)
        .unwrap_or_default();
    if let Some(step) = new_step(choice) {
        editor.borrow_mut().steps.push(step);
    }
}

pub fn duplicate(backend: &Backend, id: &str) -> Result<String> {
    match backend.workflow_request(WorkflowReq::Duplicate { id: id.to_string() })? {
        WorkflowResp::Workflow { workflow } => Ok(workflow.id),
        other => bail!("unexpected duplicate response: {other:?}"),
    }
}

pub fn delete(backend: &Backend, id: &str) -> Result<()> {
    backend.workflow_request(WorkflowReq::Delete { id: id.to_string() })?;
    Ok(())
}

pub fn violations_for(backend: &Backend, editor: &Editor) -> Vec<String> {
    violations(backend, editor.id.as_deref(), editor)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, message)| message)
        .collect()
}

#[cfg(test)]
mod tests {
    use snapline_services::dto::hotkeys::BlockedDto;

    use super::*;

    #[test]
    fn matches_query_checks_title_chord_and_steps() {
        let wf = WorkflowInfo {
            id: "default".to_owned(),
            name: "Region Capture".to_owned(),
            hotkey: Some("PrtSc".to_owned()),
            enabled: true,
            steps: vec![StepDto::CopyImage, StepDto::PlaySound],
            is_default: true,
        };
        assert!(matches_query(&wf, "PrtSc", ""));
        assert!(matches_query(&wf, "PrtSc", "default"));
        assert!(matches_query(&wf, "Ctrl+Alt+R", "alt+r"));
        assert!(matches_query(&wf, "PrtSc", "play sound"));
        assert!(!matches_query(&wf, "PrtSc", "upload"));
    }

    #[test]
    fn clash_for_and_format_preview_clash_distinguish_external_and_internal() {
        let hotkeys = HotkeysResp::State {
            bindings: Vec::new(),
            blocked: vec![
                BlockedDto {
                    owner: "workflow:a".to_owned(),
                    label: "Alpha".to_owned(),
                    chord: "Ctrl+1".to_owned(),
                    reason: BlockedReasonDto::Internal,
                },
                BlockedDto {
                    owner: "workflow:b".to_owned(),
                    label: "Beta".to_owned(),
                    chord: "Ctrl+1".to_owned(),
                    reason: BlockedReasonDto::Internal,
                },
                BlockedDto {
                    owner: "workflow:c".to_owned(),
                    label: "Gamma".to_owned(),
                    chord: "Ctrl+2".to_owned(),
                    reason: BlockedReasonDto::External,
                },
            ],
            paused: false,
        };
        assert_eq!(
            clash_for("workflow:a", &hotkeys),
            "Off because Ctrl+1 is also used by Beta."
        );
        assert_eq!(clash_for("workflow:c", &hotkeys), "");
        assert_eq!(clash_for("workflow:missing", &hotkeys), "");

        let names = HashMap::from([("b".to_owned(), "Beta".to_owned())]);
        assert_eq!(
            format_preview_clash(&PreviewDto::Free, "workflow:a", &names),
            ""
        );
        assert_eq!(
            format_preview_clash(&PreviewDto::External, "workflow:a", &names),
            "Off: another app is using this shortcut."
        );
        assert_eq!(
            format_preview_clash(
                &PreviewDto::Internal {
                    owners: vec!["workflow:a".to_owned()],
                },
                "workflow:a",
                &names,
            ),
            ""
        );
        assert_eq!(
            format_preview_clash(
                &PreviewDto::Internal {
                    owners: vec![
                        "workflow:a".to_owned(),
                        "capture_window".to_owned(),
                        "workflow:b".to_owned(),
                    ],
                },
                "workflow:a",
                &names,
            ),
            "Off: also used by Capture window and Beta."
        );
    }
}
