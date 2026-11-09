use std::path::{Path, PathBuf};

use snapline_services::uploads::FileSource;
use snapline_services::ServiceError;
use snapline_store::history::HistoryRepo;
use snapline_upload::queue::UploadFile;

pub struct HistoryFileSource {
    history: PathBuf,
}

impl HistoryFileSource {
    pub fn new(history: PathBuf) -> Self {
        Self { history }
    }
}

impl FileSource for HistoryFileSource {
    fn file(&self, capture_id: i64) -> Result<Option<UploadFile>, ServiceError> {
        let repo = HistoryRepo::open_read(&self.history)
            .map_err(|error| ServiceError::Io(error.to_string()))?;
        let Some(capture) = repo
            .get(capture_id)
            .map_err(|error| ServiceError::Io(error.to_string()))?
        else {
            return Ok(None);
        };
        let path = PathBuf::from(&capture.path);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ServiceError::Io(error.to_string())),
        };
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("capture")
            .to_string();
        let content_type = content_type(&path).to_string();
        Ok(Some(UploadFile::new(path, file_name, content_type, bytes)))
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        _ => "application/octet-stream",
    }
}
