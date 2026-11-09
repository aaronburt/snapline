use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub launch_at_signin: bool,
    pub start_minimised: bool,
    pub notifications: bool,
    pub save_folder: String,
    pub subfolders: Subfolders,
    pub image_format: ImageFormat,
    pub lossy_quality: u8,
    pub name_pattern: String,
    pub default_workflow: Vec<WorkflowStep>,
    pub copied_link_format: LinkFormat,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for General {
    fn default() -> Self {
        Self {
            launch_at_signin: true,
            start_minimised: true,
            notifications: true,
            save_folder: "%USERPROFILE%\\Pictures\\Snapline".to_owned(),
            subfolders: Subfolders::ByMonth,
            image_format: ImageFormat::Png,
            lossy_quality: 90,
            name_pattern: "{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}".to_owned(),
            default_workflow: default_workflow(),
            copied_link_format: LinkFormat::Direct,
            extra: toml::Table::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subfolders {
    ByMonth,
    ByDay,
    ByApp,
    None,
}

impl Default for Subfolders {
    fn default() -> Self {
        Self::ByMonth
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl Default for ImageFormat {
    fn default() -> Self {
        Self::Png
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkFormat {
    Direct,
    Markdown,
    Html,
    Bbcode,
    Page,
}

impl Default for LinkFormat {
    fn default() -> Self {
        Self::Direct
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowStep {
    pub kind: WorkflowStepKind,
    pub enabled: bool,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for WorkflowStep {
    fn default() -> Self {
        Self {
            kind: WorkflowStepKind::CopyImage,
            enabled: false,
            extra: toml::Table::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepKind {
    CopyImage,
    Annotate,
    Save,
    Upload,
    CopyLink,
    PlaySound,
}

impl Default for WorkflowStepKind {
    fn default() -> Self {
        Self::CopyImage
    }
}

pub fn default_workflow() -> Vec<WorkflowStep> {
    [
        (WorkflowStepKind::CopyImage, true),
        (WorkflowStepKind::Annotate, false),
        (WorkflowStepKind::Save, true),
        (WorkflowStepKind::Upload, true),
        (WorkflowStepKind::CopyLink, true),
        (WorkflowStepKind::PlaySound, true),
    ]
    .into_iter()
    .map(|(kind, enabled)| WorkflowStep {
        kind,
        enabled,
        extra: toml::Table::new(),
    })
    .collect()
}
