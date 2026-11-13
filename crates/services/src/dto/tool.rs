use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolReq {
    Ping,
    QrMake {
        text: String,
    },
    QrScan {
        capture_id: Option<i64>,
    },
    QrCopyText {
        text: String,
    },
    QrCopyImage {
        text: String,
    },
    QrSavePng {
        text: String,
    },
    QrOpen {
        link: String,
    },
    ConvertPickFile,
    ConvertStart {
        source: ConvertSource,
        format: ConvertFormatDto,
    },
    ConvertProgress {
        job_id: u64,
    },
    ConvertCancel {
        job_id: u64,
    },
    ConvertResult {
        job_id: u64,
    },
    CombinePickFiles,
    CombinePreview {
        request: CombineRequestDto,
    },
    CombineSave {
        request: CombineRequestDto,
    },
    EditorOpenHistory {
        capture_id: i64,
    },
    EditorOpenFile,
    EditorOpenClipboard,
    PinHistory {
        capture_id: i64,
    },
    PinFile,
    PinClipboard,
    PinMenu,
    Available {
        tool: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorActionDto {
    Copy,
    Save,
    Pin,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinSourceDto {
    File,
    Clipboard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorOutcomeDto {
    pub action: Option<EditorActionDto>,
    pub capture_id: Option<i64>,
    pub path: Option<String>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CombineSourceDto {
    History { id: i64 },
    Picked { id: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CombineLayoutDto {
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CombineAlignDto {
    Start,
    Centre,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CombineBackgroundDto {
    Transparent,
    White,
    Black,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombineOptionsDto {
    pub layout: CombineLayoutDto,
    pub spacing: u32,
    pub align: CombineAlignDto,
    pub background: CombineBackgroundDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombineRequestDto {
    pub sources: Vec<CombineSourceDto>,
    pub options: CombineOptionsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombinePickedDto {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConvertSource {
    Capture { capture_id: i64 },
    File { path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConvertFormatDto {
    Gif,
    WebmVp9,
    Mp4Small,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConvertPhaseDto {
    Converting,
    Saving,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertProgressDto {
    pub phase: ConvertPhaseDto,
    pub percent: Option<f64>,
    pub out_time_ms: Option<u64>,
    pub speed: Option<f64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertedDto {
    pub capture_id: i64,
    pub path: String,
    pub name: String,
    pub format: ConvertFormatDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolResp {
    Pong,
    QrMade {
        svg: String,
        link: Option<String>,
    },
    QrScanned {
        text: String,
        link: Option<String>,
        is_url: bool,
    },
    QrCopied,
    QrOpened,
    QrSaved {
        path: String,
    },
    ConvertFilePicked {
        path: Option<String>,
    },
    ConvertStarted {
        job_id: u64,
    },
    ConvertProgressed {
        job_id: u64,
        progress: ConvertProgressDto,
    },
    ConvertCancelled {
        job_id: u64,
    },
    Converted {
        converted: ConvertedDto,
    },
    CombinePickedFiles {
        files: Vec<CombinePickedDto>,
    },
    CombinePreviewed {
        job: u32,
    },
    CombineSaved {
        capture_id: i64,
    },
    EditorOpened {
        outcome: EditorOutcomeDto,
    },
    Pinned {
        source: PinSourceDto,
    },
    PinCancelled,
    Available {
        available: bool,
    },
}
