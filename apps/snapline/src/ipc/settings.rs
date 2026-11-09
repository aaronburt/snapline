use snapline_services::dto::settings::{SettingsReq, SettingsResp};
use snapline_services::Services;

use super::IpcError;

#[tauri::command]
pub async fn ipc_settings(
    services: tauri::State<'_, Services>,
    req: SettingsReq,
) -> Result<SettingsResp, IpcError> {
    match req {
        SettingsReq::BrowseFolder { current } => {
            let path = tauri::async_runtime::spawn_blocking(move || {
                snapline_win::dialogs::pick_folder(current.as_deref().map(std::path::Path::new))
            })
            .await
            .map_err(IpcError::join)?
            .map_err(|error| IpcError {
                code: "io",
                message: error.to_string(),
            })?;
            Ok(SettingsResp::BrowseFolder {
                path: path.map(|path| path.to_string_lossy().into_owned()),
            })
        }
        req => {
            let services = services.inner().clone();
            tauri::async_runtime::spawn_blocking(move || services.settings().handle(req))
                .await
                .map_err(IpcError::join)?
                .map_err(IpcError::from_service)
        }
    }
}
