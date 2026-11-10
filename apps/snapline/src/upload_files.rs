use std::path::PathBuf;

use snapline_core::media::content_type;
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
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ServiceError::Io(error.to_string())),
        };
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("capture")
            .to_string();
        let modified_ms = match capture.source_modified_ms {
            Some(value) => value,
            None => {
                let modified = metadata
                    .modified()
                    .map_err(|error| ServiceError::Io(error.to_string()))?;
                let elapsed = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| ServiceError::Io(error.to_string()))?;
                i64::try_from(elapsed.as_millis())
                    .map_err(|error| ServiceError::Io(error.to_string()))?
            }
        };
        let len = if capture.owned {
            metadata.len()
        } else {
            u64::try_from(capture.bytes).unwrap_or_default()
        };
        let content_type = content_type(&file_name).to_owned();
        Ok(Some(UploadFile::from_path(
            path,
            file_name,
            content_type,
            len,
            modified_ms,
        )))
    }
}
