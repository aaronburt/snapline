use thiserror::Error;

mod migrate;
mod repo;

pub use migrate::{open_read, open_write, schema_version, CURRENT_VERSION};
pub use repo::{week_start_millis, HistoryRepo};

pub type CaptureId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    Image,
    Video,
    Gif,
}

impl CaptureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Gif => "gif",
        }
    }

    pub fn parse(value: &str) -> Result<Self, HistoryError> {
        match value {
            "image" => Ok(Self::Image),
            "video" => Ok(Self::Video),
            "gif" => Ok(Self::Gif),
            _ => Err(HistoryError::UnknownValue {
                field: "capture.kind",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadState {
    Queued,
    Uploading,
    Done,
    Failed,
    Cancelled,
}

impl UploadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Uploading => "uploading",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Result<Self, HistoryError> {
        match value {
            "queued" => Ok(Self::Queued),
            "uploading" => Ok(Self::Uploading),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(HistoryError::UnknownValue {
                field: "upload.state",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCapture {
    pub kind: CaptureKind,
    pub path: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub source_app: Option<String>,
    pub created_at: i64,
    pub ocr_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub id: CaptureId,
    pub kind: CaptureKind,
    pub path: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub source_app: Option<String>,
    pub created_at: i64,
    pub ocr_text: Option<String>,
    pub upload: Option<UploadRecord>,
}

impl Capture {
    pub fn name(&self) -> &str {
        capture_name(&self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRecord {
    pub id: i64,
    pub capture_id: CaptureId,
    pub dest_id: String,
    pub state: UploadState,
    pub url: Option<String>,
    pub deletion_url: Option<String>,
    pub error: Option<String>,
    pub attempts: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadUpdate {
    pub capture_id: CaptureId,
    pub dest_id: String,
    pub state: UploadState,
    pub url: Option<String>,
    pub deletion_url: Option<String>,
    pub error: Option<String>,
    pub attempts: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub created_at: i64,
    pub id: CaptureId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFilter {
    All,
    Images,
    Videos,
    Gifs,
    Uploaded,
    LocalOnly,
}

impl HistoryFilter {
    fn code(self) -> i64 {
        match self {
            Self::All => 0,
            Self::Images => 1,
            Self::Videos => 2,
            Self::Gifs => 3,
            Self::Uploaded => 4,
            Self::LocalOnly => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    pub items: Vec<Capture>,
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryStats {
    pub count: i64,
    pub bytes: i64,
    pub uploaded_this_week: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedCapture {
    pub id: CaptureId,
    pub kind: CaptureKind,
    pub path: String,
    pub created_at: i64,
}

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("could not create the history folder: {0}")]
    Folder(#[source] std::io::Error),
    #[error("history database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("history schema version {found} is newer than this build supports ({supported})")]
    FutureSchema { found: u32, supported: u32 },
    #[error("history schema version {found} is older than this build requires ({required})")]
    StaleSchema { found: u32, required: u32 },
    #[error("history schema version {0} is not a valid version")]
    InvalidSchemaVersion(i64),
    #[error("no history migration exists for version {0}")]
    MissingMigration(u32),
    #[error("the history database did not enable WAL mode (reported {0})")]
    JournalMode(String),
    #[error("local time is not available on this system")]
    LocalTimeUnavailable,
    #[error("the history database contains an unknown {field}: {value}")]
    UnknownValue { field: &'static str, value: String },
}

pub(crate) fn capture_name(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(path)
}
