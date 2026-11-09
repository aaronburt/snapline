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
        SettingsReq::Update { patch } => {
            let services = services.inner().clone();
            let before = services
                .settings_snapshot()
                .map_err(IpcError::from_service)?;
            let was_launch = before.general.launch_at_signin;
            let worker = services.clone();
            let response = tauri::async_runtime::spawn_blocking(move || {
                worker.settings().handle(SettingsReq::Update { patch })
            })
            .await
            .map_err(IpcError::join)?
            .map_err(IpcError::from_service)?;
            if let SettingsResp::Updated { settings } = &response {
                if settings.general.launch_at_signin != was_launch {
                    let result = if settings.general.launch_at_signin {
                        snapline_win::autostart::enable_current()
                    } else {
                        snapline_win::autostart::disable()
                    };
                    if let Err(error) = result {
                        tracing::error!(%error, "could not update start at sign-in");
                        return Err(IpcError {
                            code: "io",
                            message: "Snapline could not update the start-at-sign-in setting."
                                .to_string(),
                        });
                    }
                }
            }
            services
                .hotkeys()
                .on_settings_changed()
                .map_err(IpcError::from_service)?;
            Ok(response)
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
