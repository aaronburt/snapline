use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::chord::Chord;
use crate::ids::{DestId, WorkflowId};
use crate::image::Image;
use crate::link_format::LinkFormat;
use crate::ports::{Clipboard, Clock, Notifier, Sound};

mod run;
mod step;

pub use run::{
    resume_after_upload, run, Recorded, RecordingDetails, RunContext, RunError, RunErrorKind,
    RunReport, SaveRequest, StepOutcome, StepStatus, UploadRequest, Uploaded,
};
pub use step::{CaptureKind, RecordKind, Step};

pub const DEFAULT_WORKFLOW_ID: &str = "default";
pub const DEFAULT_WORKFLOW_NAME: &str = "Screenshot to link";
pub const NAME_MIN_CHARS: usize = 1;
pub const NAME_MAX_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_chord")]
    pub hotkey: Option<Chord>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub steps: Vec<Step>,
}

fn default_enabled() -> bool {
    true
}

impl Workflow {
    pub fn new(id: impl Into<WorkflowId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            hotkey: None,
            enabled: true,
            steps: Vec::new(),
        }
    }

    pub fn is_default(&self) -> bool {
        self.id.as_str() == DEFAULT_WORKFLOW_ID
    }

    pub fn annotates_inline(&self, capture_index: usize) -> bool {
        let next_is_annotate = matches!(self.steps.get(capture_index + 1), Some(Step::Annotate));
        if next_is_annotate {
            return true;
        }
        self.is_default() && !self.steps.iter().any(|step| matches!(step, Step::Annotate))
    }

    pub fn validate(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        let characters = self.name.chars().count();
        if self.name.trim().is_empty() || characters < NAME_MIN_CHARS || characters > NAME_MAX_CHARS
        {
            violations.push(Violation {
                step: None,
                error: ValidationError::Name,
            });
        }
        match self.steps.first() {
            Some(first) if first.is_capture_or_record() => {}
            Some(_) => violations.push(Violation {
                step: Some(0),
                error: ValidationError::FirstStep,
            }),
            None => violations.push(Violation {
                step: None,
                error: ValidationError::FirstStep,
            }),
        }
        let mut uploaded = false;
        let mut extracted = false;
        let mut recorded = false;
        for (index, step) in self.steps.iter().enumerate() {
            match step {
                Step::Upload { .. } => uploaded = true,
                Step::ExtractText => extracted = true,
                Step::Record { .. } => recorded = true,
                Step::CopyLink { .. } if !uploaded => violations.push(Violation {
                    step: Some(index),
                    error: ValidationError::CopyLinkWithoutUpload,
                }),
                Step::CopyText if !extracted => violations.push(Violation {
                    step: Some(index),
                    error: ValidationError::CopyTextWithoutExtractText,
                }),
                Step::ConvertToGif if !recorded => violations.push(Violation {
                    step: Some(index),
                    error: ValidationError::ConvertToGifWithoutRecord,
                }),
                _ => {}
            }
        }
        violations
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("The name must be 1 to 60 characters.")]
    Name,
    #[error("The first step must be a Capture or Record step.")]
    FirstStep,
    #[error("Copy link requires an earlier Upload step.")]
    CopyLinkWithoutUpload,
    #[error("Copy text requires an earlier Extract text step.")]
    CopyTextWithoutExtractText,
    #[error("Convert to GIF requires an earlier Record step.")]
    ConvertToGifWithoutRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Violation {
    pub step: Option<usize>,
    pub error: ValidationError,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowError {
    #[error("Not available yet")]
    NotAvailableYet,
    #[error("{0}")]
    Failed(String),
    #[error("The step was cancelled.")]
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UploadId(u64);

impl UploadId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

pub trait CapturePort {
    fn capture(&self, kind: CaptureKind) -> Result<Image, WorkflowError>;

    fn capture_for_workflow(
        &self,
        kind: CaptureKind,
        _annotate: bool,
    ) -> Result<Captured, WorkflowError> {
        Ok(Captured {
            image: self.capture(kind)?,
            annotated: self.includes_annotation(kind),
            action: None,
            source_app: None,
        })
    }

    fn includes_annotation(&self, _kind: CaptureKind) -> bool {
        false
    }
}

pub struct Captured {
    pub image: Image,
    pub annotated: bool,
    pub action: Option<crate::ports::OverlayAction>,
    pub source_app: Option<String>,
}

pub trait RecordPort {
    fn record(&self, kind: RecordKind) -> Result<Recorded, WorkflowError>;
}

pub trait AnnotatePort {
    fn annotate(&self, image: &Image) -> Result<Image, WorkflowError>;
}

pub trait SavePort {
    fn save(&self, request: SaveRequest<'_>) -> Result<PathBuf, WorkflowError>;
}

pub trait UploadPort {
    fn hand_off(&self, request: UploadRequest<'_>) -> Result<UploadId, WorkflowError>;
}

pub trait TextPort {
    fn extract(&self, image: &Image) -> Result<String, WorkflowError>;
}

pub trait GifPort {
    fn convert(&self, recording: &Path) -> Result<PathBuf, WorkflowError>;
}

pub struct WorkflowPorts<'a> {
    pub capture: &'a dyn CapturePort,
    pub record: &'a dyn RecordPort,
    pub annotate: &'a dyn AnnotatePort,
    pub save: &'a dyn SavePort,
    pub upload: &'a dyn UploadPort,
    pub clipboard: &'a dyn Clipboard,
    pub text: &'a dyn TextPort,
    pub gif: &'a dyn GifPort,
    pub notify: &'a dyn Notifier,
    pub sound: &'a dyn Sound,
    pub clock: &'a dyn Clock,
}

pub(crate) mod serde_chord {
    use super::*;

    pub fn serialize<S>(value: &Option<Chord>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(chord) => serializer.serialize_str(&chord.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Chord>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<String>::deserialize(deserializer)?;
        match value {
            Some(text) => Chord::from_str(&text)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

pub(crate) mod serde_link_format {
    use super::*;

    pub fn serialize<S>(value: &LinkFormat, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(name(*value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<LinkFormat, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("Unknown link format: {value}")))
    }

    const fn name(value: LinkFormat) -> &'static str {
        match value {
            LinkFormat::Direct => "direct",
            LinkFormat::Markdown => "markdown",
            LinkFormat::Html => "html",
            LinkFormat::Bbcode => "bbcode",
            LinkFormat::Page => "page",
        }
    }

    fn parse(value: &str) -> Option<LinkFormat> {
        match value {
            "direct" => Some(LinkFormat::Direct),
            "markdown" => Some(LinkFormat::Markdown),
            "html" => Some(LinkFormat::Html),
            "bbcode" => Some(LinkFormat::Bbcode),
            "page" => Some(LinkFormat::Page),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow(name: &str, steps: Vec<Step>) -> Workflow {
        let mut workflow = Workflow::new("test", name);
        workflow.steps = steps;
        workflow
    }

    fn violations(workflow: &Workflow) -> Vec<ValidationError> {
        workflow
            .validate()
            .into_iter()
            .map(|violation| violation.error)
            .collect()
    }

    #[test]
    fn a_capture_workflow_is_valid() {
        let flow = workflow(
            "Screenshot to link",
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::Save,
            ],
        );
        assert!(flow.validate().is_empty());
        assert!(flow.is_valid());
    }

    #[test]
    fn a_record_workflow_is_valid() {
        let flow = workflow(
            "Bug report clip",
            vec![
                Step::Record {
                    kind: RecordKind::Video,
                },
                Step::ConvertToGif,
            ],
        );
        assert!(flow.is_valid());
    }

    #[test]
    fn the_name_must_not_be_empty() {
        let flow = workflow(
            "",
            vec![Step::Capture {
                kind: CaptureKind::Region,
            }],
        );
        assert_eq!(violations(&flow), vec![ValidationError::Name]);
        let flow = workflow(
            "   ",
            vec![Step::Capture {
                kind: CaptureKind::Region,
            }],
        );
        assert_eq!(violations(&flow), vec![ValidationError::Name]);
    }

    #[test]
    fn the_name_accepts_one_to_sixty_characters() {
        let steps = || {
            vec![Step::Capture {
                kind: CaptureKind::Region,
            }]
        };
        assert!(workflow("a", steps()).is_valid());
        assert!(workflow(&"a".repeat(60), steps()).is_valid());
        assert_eq!(
            violations(&workflow(&"a".repeat(61), steps())),
            vec![ValidationError::Name]
        );
    }

    #[test]
    fn the_name_is_counted_in_characters() {
        let steps = || {
            vec![Step::Capture {
                kind: CaptureKind::Region,
            }]
        };
        assert!(workflow(&"é".repeat(60), steps()).is_valid());
        assert_eq!(
            violations(&workflow(&"é".repeat(61), steps())),
            vec![ValidationError::Name]
        );
    }

    #[test]
    fn the_first_step_must_be_a_capture_or_record() {
        let flow = workflow("Screenshot to link", vec![Step::CopyImage, Step::Save]);
        assert_eq!(violations(&flow), vec![ValidationError::FirstStep]);
        assert_eq!(flow.validate()[0].step, Some(0));
    }

    #[test]
    fn an_empty_workflow_fails_the_first_step_rule() {
        let flow = workflow("Screenshot to link", Vec::new());
        assert_eq!(violations(&flow), vec![ValidationError::FirstStep]);
        assert_eq!(flow.validate()[0].step, None);
    }

    #[test]
    fn copy_link_needs_an_earlier_upload() {
        let flow = workflow(
            "Screenshot to link",
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::CopyLink {
                    format: LinkFormat::Direct,
                },
                Step::Upload { dest: None },
            ],
        );
        let issues = flow.validate();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].error, ValidationError::CopyLinkWithoutUpload);
        assert_eq!(issues[0].step, Some(1));
        assert!(workflow(
            "Screenshot to link",
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::Upload { dest: None },
                Step::CopyLink {
                    format: LinkFormat::Direct,
                },
            ],
        )
        .is_valid());
    }

    #[test]
    fn copy_text_needs_an_earlier_extract_text() {
        let flow = workflow(
            "Grab text",
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::CopyText,
                Step::ExtractText,
            ],
        );
        let issues = flow.validate();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].error, ValidationError::CopyTextWithoutExtractText);
        assert_eq!(issues[0].step, Some(1));
        assert!(workflow(
            "Grab text",
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::ExtractText,
                Step::CopyText,
            ],
        )
        .is_valid());
    }

    #[test]
    fn convert_to_gif_needs_an_earlier_record() {
        let flow = workflow(
            "Bug report clip",
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::ConvertToGif,
            ],
        );
        assert_eq!(
            violations(&flow),
            vec![ValidationError::ConvertToGifWithoutRecord]
        );
        assert!(workflow(
            "Bug report clip",
            vec![
                Step::Record {
                    kind: RecordKind::Video,
                },
                Step::ConvertToGif,
            ],
        )
        .is_valid());
    }

    #[test]
    fn every_rule_is_reported_in_order() {
        let flow = workflow(
            "",
            vec![
                Step::CopyLink {
                    format: LinkFormat::Direct,
                },
                Step::ConvertToGif,
                Step::CopyText,
            ],
        );
        assert_eq!(
            violations(&flow),
            vec![
                ValidationError::Name,
                ValidationError::FirstStep,
                ValidationError::CopyLinkWithoutUpload,
                ValidationError::ConvertToGifWithoutRecord,
                ValidationError::CopyTextWithoutExtractText,
            ]
        );
    }

    #[test]
    fn the_default_workflow_is_identified_by_its_id() {
        let flow = Workflow::new(DEFAULT_WORKFLOW_ID, DEFAULT_WORKFLOW_NAME);
        assert!(flow.is_default());
        assert!(flow.hotkey.is_none());
        assert!(!Workflow::new("other", "Other").is_default());
    }

    #[test]
    fn inline_annotation_follows_the_step_after_capture() {
        let mut custom = Workflow::new("custom", "Custom");
        custom.steps = vec![
            Step::Capture {
                kind: CaptureKind::Region,
            },
            Step::Annotate,
            Step::Save,
        ];
        assert!(custom.annotates_inline(0));
        custom.steps = vec![
            Step::Capture {
                kind: CaptureKind::Region,
            },
            Step::Save,
            Step::Annotate,
        ];
        assert!(!custom.annotates_inline(0));
        let mut default = Workflow::new(DEFAULT_WORKFLOW_ID, DEFAULT_WORKFLOW_NAME);
        default.steps = vec![
            Step::Capture {
                kind: CaptureKind::Region,
            },
            Step::Save,
        ];
        assert!(default.annotates_inline(0));
        default.steps.push(Step::Annotate);
        assert!(!default.annotates_inline(0));
    }

    #[test]
    fn a_new_workflow_is_enabled_without_a_chord() {
        let flow = Workflow::new("other", "Other");
        assert!(flow.enabled);
        assert_eq!(flow.hotkey, None);
        assert!(flow.steps.is_empty());
    }

    #[test]
    fn menus_use_the_editor_labels() {
        assert_eq!(CaptureKind::LastRegion.label(), "Last region");
        assert_eq!(RecordKind::Gif.label(), "GIF");
        assert_eq!(Step::ConvertToGif.label(), "Convert to GIF");
    }
}
