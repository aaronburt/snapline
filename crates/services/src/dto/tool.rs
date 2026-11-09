use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
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
        #[ts(as = "Option<i32>")]
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
        #[ts(as = "u32")]
        job_id: u64,
    },
    ConvertCancel {
        #[ts(as = "u32")]
        job_id: u64,
    },
    ConvertResult {
        #[ts(as = "u32")]
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
        #[ts(as = "i32")]
        capture_id: i64,
    },
    EditorOpenFile,
    EditorOpenClipboard,
    PinFile,
    PinClipboard,
    PinMenu,
    Available {
        tool: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EditorActionDto {
    Copy,
    Save,
    Pin,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PinSourceDto {
    File,
    Clipboard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EditorOutcomeDto {
    pub action: Option<EditorActionDto>,
    #[ts(as = "Option<i32>")]
    pub capture_id: Option<i64>,
    pub path: Option<String>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CombineSourceDto {
    History {
        #[ts(as = "i32")]
        id: i64,
    },
    Picked {
        #[ts(as = "u32")]
        id: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CombineLayoutDto {
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CombineAlignDto {
    Start,
    Centre,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CombineBackgroundDto {
    Transparent,
    White,
    Black,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CombineOptionsDto {
    pub layout: CombineLayoutDto,
    pub spacing: u32,
    pub align: CombineAlignDto,
    pub background: CombineBackgroundDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CombineRequestDto {
    pub sources: Vec<CombineSourceDto>,
    pub options: CombineOptionsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CombinePickedDto {
    #[ts(as = "u32")]
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConvertSource {
    Capture {
        #[ts(as = "i32")]
        capture_id: i64,
    },
    File {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConvertFormatDto {
    Gif,
    WebmVp9,
    Mp4Small,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConvertPhaseDto {
    Converting,
    Saving,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConvertProgressDto {
    pub phase: ConvertPhaseDto,
    pub percent: Option<f64>,
    #[ts(as = "Option<i32>")]
    pub out_time_ms: Option<u64>,
    pub speed: Option<f64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConvertedDto {
    #[ts(as = "i32")]
    pub capture_id: i64,
    pub path: String,
    pub name: String,
    pub format: ConvertFormatDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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
        #[ts(as = "u32")]
        job_id: u64,
    },
    ConvertProgressed {
        #[ts(as = "u32")]
        job_id: u64,
        progress: ConvertProgressDto,
    },
    ConvertCancelled {
        #[ts(as = "u32")]
        job_id: u64,
    },
    Converted {
        converted: ConvertedDto,
    },
    CombinePickedFiles {
        files: Vec<CombinePickedDto>,
    },
    CombinePreviewed {
        #[ts(as = "u32")]
        job: u32,
    },
    CombineSaved {
        #[ts(as = "i32")]
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
