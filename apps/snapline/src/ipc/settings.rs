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
            let affects_hotkeys = touches_hotkeys(&patch);
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
                        // Put the saved setting back so Settings does not claim something untrue.
                        let revert = serde_json::json!({
                            "general": { "launch_at_signin": was_launch }
                        });
                        if let Err(error) = services.update_settings(&revert) {
                            tracing::error!(%error, "could not revert start at sign-in");
                        }
                        if affects_hotkeys {
                            let _ = services.hotkeys().on_settings_changed();
                        }
                        return Err(IpcError {
                            code: "io",
                            message: "Snapline could not update the start-at-sign-in setting."
                                .to_string(),
                        });
                    }
                }
            }
            if affects_hotkeys {
                services
                    .hotkeys()
                    .on_settings_changed()
                    .map_err(IpcError::from_service)?;
            }
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

/// Hotkeys only need re-registering when the change can alter a binding or a workflow's hotkey.
fn touches_hotkeys(patch: &serde_json::Value) -> bool {
    patch
        .as_object()
        .is_some_and(|object| object.contains_key("hotkeys") || object.contains_key("workflows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hotkey_and_workflow_changes_reregister_hotkeys() {
        assert!(touches_hotkeys(
            &serde_json::json!({ "hotkeys": { "bindings": {} } })
        ));
        assert!(touches_hotkeys(
            &serde_json::json!({ "workflows": { "list": [] } })
        ));
        assert!(!touches_hotkeys(
            &serde_json::json!({ "general": { "save_folder": "C:\\x" } })
        ));
        assert!(!touches_hotkeys(&serde_json::json!(5)));
    }
}
