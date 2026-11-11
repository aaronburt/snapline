use snapline_core::actions::ActionId;
use snapline_services::actions::dispatch;
use snapline_services::dto::app::{AppReq, AppResp};
use snapline_services::{ServiceError, Services};
use tauri::Manager;

use crate::panel;
use crate::windows::WindowManager;

use super::IpcError;

#[tauri::command]
pub async fn ipc_app(
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, WindowManager>,
    services: tauri::State<'_, Services>,
    req: AppReq,
) -> Result<AppResp, IpcError> {
    let from_panel = window.label() == panel::PANEL_LABEL;
    match req {
        AppReq::Ready => {
            if from_panel {
                panel::ready(&window).map_err(IpcError::from_tauri)?;
            } else {
                manager.ready(&window).map_err(IpcError::from_tauri)?;
            }
            return Ok(AppResp::Ready);
        }
        AppReq::Info => {
            return Ok(AppResp::Info {
                has_mica: crate::theme::has_mica(),
            });
        }
        AppReq::PanelResize { height } => {
            panel::resize(&window, height).map_err(IpcError::from_tauri)?;
            return Ok(AppResp::PanelResize);
        }
        AppReq::PanelClose => {
            manager.close_panel(window.app_handle());
            return Ok(AppResp::PanelClose);
        }
        AppReq::PanelCopyLink => {
            let services = services.inner().clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let response = services
                    .upload()
                    .handle(snapline_services::dto::upload::UploadReq::LastLink)?;
                if let snapline_services::dto::upload::UploadResp::LastLink { link: Some(link) } =
                    response
                {
                    snapline_win::clipboard::Clipboard::new()
                        .set_text(&link)
                        .map_err(|error| snapline_services::ServiceError::Io(error.to_string()))?;
                }
                Ok::<(), snapline_services::ServiceError>(())
            })
            .await
            .map_err(IpcError::join)?;
            result.map_err(IpcError::from_service)?;
            return Ok(AppResp::PanelCopyLink);
        }
        AppReq::OpenMain { route } => {
            let route = if route.starts_with('/') {
                route
            } else {
                "/history".to_string()
            };
            manager
                .open_main(window.app_handle(), &route)
                .map_err(IpcError::from_tauri)?;
            if from_panel {
                manager.close_panel(window.app_handle());
            }
            return Ok(AppResp::OpenMain);
        }
        AppReq::OpenCapture { id } => {
            tracing::info!(
                capture = id,
                "opening a capture from the panel is not available yet"
            );
            manager
                .open_main(window.app_handle(), "/history")
                .map_err(IpcError::from_tauri)?;
            if from_panel {
                manager.close_panel(window.app_handle());
            }
            return Ok(AppResp::OpenCapture);
        }
        AppReq::PanelAction { action } => {
            let services = services.inner().clone();
            let notify_services = services.clone();
            let action_name = action.clone();
            let action = ActionId::new(action);
            let hide_first = matches!(
                action.as_str(),
                "capture_window" | "capture_screen" | "capture_last_region"
            );
            if hide_first && from_panel {
                manager.close_panel(window.app_handle());
            }
            let result = tauri::async_runtime::spawn_blocking(move || {
                if hide_first {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                if action.as_str() == "capture_region" {
                    snapline_services::workflows::installed_runner()?
                        .run_default_from_capture()
                        .map(|_| ())
                } else {
                    dispatch(&action, &services)
                }
            })
            .await
            .map_err(IpcError::join)?;
            if let Err(error) = &result {
                snapline_services::actions::notify_failure(error, &notify_services);
            }
            if from_panel {
                manager.close_panel(window.app_handle());
            }
            match result {
                Ok(()) => {}
                Err(ServiceError::NotYet) => {
                    tracing::warn!(%action_name, "the panel action is not available yet");
                    snapline_services::actions::notify_unavailable(&action_name, &notify_services);
                }
                Err(error) => {
                    return Err(IpcError::from_service(error));
                }
            }
            return Ok(AppResp::PanelAction);
        }
        req => {
            let services = services.inner().clone();
            return tauri::async_runtime::spawn_blocking(move || services.app().handle(req))
                .await
                .map_err(IpcError::join)?
                .map_err(IpcError::from_service);
        }
    }
}
