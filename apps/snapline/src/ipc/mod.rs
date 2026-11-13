pub mod app;
pub mod history;
pub mod hotkeys;
pub mod record;
pub mod settings;
pub mod tool;
pub mod upload;
pub mod workflow;

use serde::Serialize;
use snapline_services::ServiceError;

#[derive(Debug, Serialize)]
pub struct IpcError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl IpcError {
    fn join(error: tauri::Error) -> Self {
        tracing::error!(%error, "ipc task failed");
        Self {
            code: "internal",
            message: "The request could not be completed.".to_string(),
        }
    }

    fn from_tauri(error: tauri::Error) -> Self {
        tracing::error!(%error, "ipc window operation failed");
        Self {
            code: "internal",
            message: "The window could not be shown.".to_string(),
        }
    }

    fn from_service(error: ServiceError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}
