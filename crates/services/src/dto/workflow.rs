use serde::{Deserialize, Serialize};
use snapline_core::chord::Chord;
use snapline_core::ids::{DestId, WorkflowId};
use snapline_core::link_format::LinkFormat;
use snapline_core::workflow::{CaptureKind, RecordKind, Step, Violation, Workflow};

use crate::dto::settings::LinkFormatDto;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum WorkflowReq {
    Ping,
    List,
    Get {
        id: String,
    },
    Create {
        workflow: WorkflowInput,
    },
    Update {
        id: String,
        workflow: WorkflowInput,
    },
    Delete {
        id: String,
    },
    Duplicate {
        id: String,
    },
    Validate {
        id: Option<String>,
        workflow: WorkflowInput,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum WorkflowResp {
    Pong,
    List {
        workflows: Vec<WorkflowInfo>,
        destinations: Vec<DestinationOption>,
    },
    Workflow {
        workflow: WorkflowInfo,
    },
    Deleted {
        id: String,
    },
    Validation {
        violations: Vec<ViolationDto>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInput {
    pub name: String,
    pub hotkey: Option<String>,
    pub enabled: bool,
    pub steps: Vec<StepDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInfo {
    pub id: String,
    pub name: String,
    pub hotkey: Option<String>,
    pub enabled: bool,
    pub steps: Vec<StepDto>,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViolationDto {
    pub step: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStepKindDto {
    Region,
    Window,
    Screen,
    LastRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStepKindDto {
    Video,
    Gif,
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum StepDto {
    Capture {
        kind: CaptureStepKindDto,
    },
    Record {
        kind: RecordStepKindDto,
    },
    CopyImage,
    Annotate,
    Save,
    Upload {
        #[serde(default)]
        dest: Option<String>,
    },
    CopyLink {
        format: LinkFormatDto,
    },
    ExtractText,
    CopyText,
    ConvertToGif,
    Notify,
    PlaySound,
}

impl From<CaptureKind> for CaptureStepKindDto {
    fn from(value: CaptureKind) -> Self {
        match value {
            CaptureKind::Region => Self::Region,
            CaptureKind::Window => Self::Window,
            CaptureKind::Screen => Self::Screen,
            CaptureKind::LastRegion => Self::LastRegion,
        }
    }
}

impl From<CaptureStepKindDto> for CaptureKind {
    fn from(value: CaptureStepKindDto) -> Self {
        match value {
            CaptureStepKindDto::Region => Self::Region,
            CaptureStepKindDto::Window => Self::Window,
            CaptureStepKindDto::Screen => Self::Screen,
            CaptureStepKindDto::LastRegion => Self::LastRegion,
        }
    }
}

impl From<RecordKind> for RecordStepKindDto {
    fn from(value: RecordKind) -> Self {
        match value {
            RecordKind::Video => Self::Video,
            RecordKind::Gif => Self::Gif,
            RecordKind::Window => Self::Window,
        }
    }
}

impl From<RecordStepKindDto> for RecordKind {
    fn from(value: RecordStepKindDto) -> Self {
        match value {
            RecordStepKindDto::Video => Self::Video,
            RecordStepKindDto::Gif => Self::Gif,
            RecordStepKindDto::Window => Self::Window,
        }
    }
}

impl From<LinkFormat> for LinkFormatDto {
    fn from(value: LinkFormat) -> Self {
        match value {
            LinkFormat::Direct => Self::Direct,
            LinkFormat::Markdown => Self::Markdown,
            LinkFormat::Html => Self::Html,
            LinkFormat::Bbcode => Self::Bbcode,
            LinkFormat::Page => Self::Page,
        }
    }
}

impl From<LinkFormatDto> for LinkFormat {
    fn from(value: LinkFormatDto) -> Self {
        match value {
            LinkFormatDto::Direct => Self::Direct,
            LinkFormatDto::Markdown => Self::Markdown,
            LinkFormatDto::Html => Self::Html,
            LinkFormatDto::Bbcode => Self::Bbcode,
            LinkFormatDto::Page => Self::Page,
        }
    }
}

impl From<Step> for StepDto {
    fn from(value: Step) -> Self {
        match value {
            Step::Capture { kind } => Self::Capture { kind: kind.into() },
            Step::Record { kind } => Self::Record { kind: kind.into() },
            Step::CopyImage => Self::CopyImage,
            Step::Annotate => Self::Annotate,
            Step::Save => Self::Save,
            Step::Upload { dest } => Self::Upload {
                dest: dest.map(|dest| dest.as_str().to_owned()),
            },
            Step::CopyLink { format } => Self::CopyLink {
                format: format.into(),
            },
            Step::ExtractText => Self::ExtractText,
            Step::CopyText => Self::CopyText,
            Step::ConvertToGif => Self::ConvertToGif,
            Step::Notify => Self::Notify,
            Step::PlaySound => Self::PlaySound,
        }
    }
}

impl From<StepDto> for Step {
    fn from(value: StepDto) -> Self {
        match value {
            StepDto::Capture { kind } => Self::Capture { kind: kind.into() },
            StepDto::Record { kind } => Self::Record { kind: kind.into() },
            StepDto::CopyImage => Self::CopyImage,
            StepDto::Annotate => Self::Annotate,
            StepDto::Save => Self::Save,
            StepDto::Upload { dest } => Self::Upload {
                dest: dest.map(DestId::new),
            },
            StepDto::CopyLink { format } => Self::CopyLink {
                format: format.into(),
            },
            StepDto::ExtractText => Self::ExtractText,
            StepDto::CopyText => Self::CopyText,
            StepDto::ConvertToGif => Self::ConvertToGif,
            StepDto::Notify => Self::Notify,
            StepDto::PlaySound => Self::PlaySound,
        }
    }
}

impl From<&Workflow> for WorkflowInfo {
    fn from(value: &Workflow) -> Self {
        Self {
            id: value.id.as_str().to_owned(),
            name: value.name.clone(),
            hotkey: value.hotkey.map(|chord| chord.to_string()),
            enabled: value.enabled,
            steps: value.steps.iter().cloned().map(StepDto::from).collect(),
            is_default: value.is_default(),
        }
    }
}

impl From<Violation> for ViolationDto {
    fn from(value: Violation) -> Self {
        Self {
            step: value.step,
            message: value.error.to_string(),
        }
    }
}

impl WorkflowInput {
    pub fn into_workflow(self, id: &str, name: String) -> Result<Workflow, String> {
        let hotkey = match self.hotkey.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => {
                Some(Chord::parse(text).map_err(|error| error.to_string())?)
            }
            _ => None,
        };
        Ok(Workflow {
            id: WorkflowId::new(id),
            name,
            hotkey,
            enabled: self.enabled,
            steps: self.steps.into_iter().map(Step::from).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn steps_use_the_internally_tagged_step_field() {
        assert_eq!(
            serde_json::to_value(StepDto::Capture {
                kind: CaptureStepKindDto::Region
            })
            .expect("json"),
            json!({ "step": "capture", "kind": "region" })
        );
        assert_eq!(
            serde_json::to_value(StepDto::Record {
                kind: RecordStepKindDto::Gif
            })
            .expect("json"),
            json!({ "step": "record", "kind": "gif" })
        );
        assert_eq!(
            serde_json::to_value(StepDto::CopyLink {
                format: LinkFormatDto::Markdown
            })
            .expect("json"),
            json!({ "step": "copy_link", "format": "markdown" })
        );
        assert_eq!(
            serde_json::to_value(StepDto::CopyImage).expect("json"),
            json!({ "step": "copy_image" })
        );
    }

    #[test]
    fn steps_round_trip_through_the_core_model() {
        let steps = vec![
            Step::Capture {
                kind: CaptureKind::LastRegion,
            },
            Step::Record {
                kind: RecordKind::Window,
            },
            Step::Upload {
                dest: Some(DestId::new("imgur")),
            },
            Step::CopyLink {
                format: LinkFormat::Page,
            },
            Step::CopyImage,
            Step::Annotate,
            Step::Save,
            Step::ExtractText,
            Step::CopyText,
            Step::ConvertToGif,
            Step::Notify,
            Step::PlaySound,
        ];
        for step in steps {
            let dto = StepDto::from(step.clone());
            let value = serde_json::to_value(&dto).expect("json");
            let parsed: StepDto = serde_json::from_value(value).expect("dto");
            assert_eq!(Step::from(parsed.clone()), step);
            assert_eq!(Step::from(dto), step);
        }
    }

    #[test]
    fn a_request_uses_the_area_tag_and_content_shape() {
        assert_eq!(
            serde_json::to_value(WorkflowReq::Delete {
                id: "annotate".to_owned()
            })
            .expect("json"),
            json!({ "kind": "delete", "data": { "id": "annotate" } })
        );
        assert_eq!(
            serde_json::to_value(WorkflowReq::List).expect("json"),
            json!({ "kind": "list" })
        );
    }

    #[test]
    fn an_input_parses_and_normalises_the_chord() {
        let input = WorkflowInput {
            name: "Annotate and share".to_owned(),
            hotkey: Some("ctrl+shift+a".to_owned()),
            enabled: true,
            steps: vec![StepDto::Capture {
                kind: CaptureStepKindDto::Region,
            }],
        };
        let workflow = input.into_workflow("annotate", "Annotate and share".to_owned());
        let workflow = workflow.expect("workflow");
        assert_eq!(
            workflow.hotkey.map(|chord| chord.to_string()),
            Some("Ctrl+Shift+A".to_owned())
        );
    }

    #[test]
    fn an_empty_chord_is_unbound() {
        let input = WorkflowInput {
            name: "Save quietly".to_owned(),
            hotkey: Some("   ".to_owned()),
            enabled: true,
            steps: Vec::new(),
        };
        let workflow = input
            .into_workflow("save-quietly", "Save quietly".to_owned())
            .expect("workflow");
        assert_eq!(workflow.hotkey, None);
    }

    #[test]
    fn the_info_mirrors_the_default_flag_and_chord() {
        let mut workflow = Workflow::new("default", "Screenshot to link");
        workflow.steps.push(Step::CopyImage);
        let info = WorkflowInfo::from(&workflow);
        assert!(info.is_default);
        assert_eq!(info.id, "default");
        assert_eq!(info.hotkey, None);
        assert_eq!(info.steps, vec![StepDto::CopyImage]);
    }
}
