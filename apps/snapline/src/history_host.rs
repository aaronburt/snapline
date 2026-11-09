use std::path::Path;
use std::sync::Arc;

use snapline_core::ports::Clipboard as ClipboardPort;
use snapline_services::history::HistoryHost;
use snapline_services::ServiceError;
use snapline_win::shell;
use snapline_win::wic::{WicCodec, WicError};

pub struct WindowsHistoryHost {
    clipboard: Arc<dyn ClipboardPort + Send + Sync>,
}

impl WindowsHistoryHost {
    pub fn new(clipboard: Arc<dyn ClipboardPort + Send + Sync>) -> Self {
        Self { clipboard }
    }
}

impl HistoryHost for WindowsHistoryHost {
    fn confirm_delete(&self, message: &str) -> Result<bool, ServiceError> {
        shell::confirm_delete(message).map_err(shell_error)
    }

    fn set_clipboard_image(&self, bytes: &[u8]) -> Result<(), ServiceError> {
        let codec = WicCodec::new().map_err(codec_error)?;
        let image = codec.decode(bytes).map_err(codec_error)?;
        self.clipboard
            .set_image(&image)
            .map_err(|error| ServiceError::Io(error.to_string()))
    }

    fn show_in_folder(&self, path: &Path) -> Result<(), ServiceError> {
        shell::show_in_folder(path).map_err(shell_error)
    }

    fn open_file(&self, path: &Path) -> Result<(), ServiceError> {
        shell::open_file(path).map_err(shell_error)
    }

    fn open_url(&self, url: &str) -> Result<(), ServiceError> {
        shell::open_url(url).map_err(shell_error)
    }

    fn recycle_file(&self, path: &Path) -> Result<(), ServiceError> {
        shell::recycle(path).map_err(shell_error)
    }
}

fn shell_error(error: shell::ShellError) -> ServiceError {
    ServiceError::Io(error.to_string())
}

fn codec_error(error: WicError) -> ServiceError {
    ServiceError::Io(error.to_string())
}
