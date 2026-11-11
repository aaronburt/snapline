use serde::{Deserialize, Serialize};

use crate::ids::DestId;
use crate::link_format::LinkFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    #[serde(alias = "scrolling")]
    Region,
    Window,
    Screen,
    LastRegion,
}

impl CaptureKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Region => "Region",
            Self::Window => "Window",
            Self::Screen => "Screen",
            Self::LastRegion => "Last region",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Video,
    Gif,
    Window,
}

impl RecordKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Gif => "GIF",
            Self::Window => "Window",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    Capture {
        kind: CaptureKind,
    },
    Record {
        kind: RecordKind,
    },
    CopyImage,
    Annotate,
    Save,
    Upload {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dest: Option<DestId>,
    },
    CopyLink {
        #[serde(with = "crate::workflow::serde_link_format")]
        format: LinkFormat,
    },
    ExtractText,
    CopyText,
    ConvertToGif,
    Notify,
    PlaySound,
}

impl Step {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Capture { .. } => "Capture",
            Self::Record { .. } => "Record",
            Self::CopyImage => "Copy image",
            Self::Annotate => "Annotate",
            Self::Save => "Save",
            Self::Upload { .. } => "Upload",
            Self::CopyLink { .. } => "Copy link",
            Self::ExtractText => "Extract text",
            Self::CopyText => "Copy text",
            Self::ConvertToGif => "Convert to GIF",
            Self::Notify => "Notify",
            Self::PlaySound => "Play sound",
        }
    }

    pub const fn unavailable_label(&self) -> &'static str {
        self.label()
    }

    pub const fn is_capture_or_record(&self) -> bool {
        matches!(self, Self::Capture { .. } | Self::Record { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_kinds_use_the_editor_names() {
        assert_eq!(CaptureKind::Region.label(), "Region");
        assert_eq!(CaptureKind::Window.label(), "Window");
        assert_eq!(CaptureKind::Screen.label(), "Screen");
        assert_eq!(CaptureKind::LastRegion.label(), "Last region");
    }

    #[test]
    fn record_kinds_use_the_editor_names() {
        assert_eq!(RecordKind::Video.label(), "Video");
        assert_eq!(RecordKind::Gif.label(), "GIF");
        assert_eq!(RecordKind::Window.label(), "Window");
    }

    #[test]
    fn every_step_has_the_editor_label() {
        let steps = [
            (
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                "Capture",
            ),
            (
                Step::Record {
                    kind: RecordKind::Video,
                },
                "Record",
            ),
            (Step::CopyImage, "Copy image"),
            (Step::Annotate, "Annotate"),
            (Step::Save, "Save"),
            (Step::Upload { dest: None }, "Upload"),
            (
                Step::CopyLink {
                    format: LinkFormat::Direct,
                },
                "Copy link",
            ),
            (Step::ExtractText, "Extract text"),
            (Step::CopyText, "Copy text"),
            (Step::ConvertToGif, "Convert to GIF"),
            (Step::Notify, "Notify"),
            (Step::PlaySound, "Play sound"),
        ];
        for (step, label) in steps {
            assert_eq!(step.label(), label);
        }
    }

    #[test]
    fn unavailable_steps_use_their_normal_names() {
        assert_eq!(
            Step::Record {
                kind: RecordKind::Video
            }
            .unavailable_label(),
            "Record"
        );
        assert_eq!(Step::ConvertToGif.unavailable_label(), "Convert to GIF");
        assert_eq!(Step::ExtractText.unavailable_label(), "Extract text");
        assert_eq!(
            Step::Capture {
                kind: CaptureKind::Region
            }
            .unavailable_label(),
            "Capture"
        );
    }

    #[test]
    fn capture_and_record_count_as_the_first_step() {
        assert!(Step::Capture {
            kind: CaptureKind::Screen
        }
        .is_capture_or_record());
        assert!(Step::Record {
            kind: RecordKind::Gif
        }
        .is_capture_or_record());
        assert!(!Step::CopyImage.is_capture_or_record());
    }
}
