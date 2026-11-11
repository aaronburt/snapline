use snapline_core::actions::ActionId;
use snapline_services::actions::dispatch;
use snapline_services::dto::app::{AppReq, AppResp};
use snapline_services::{ServiceError, Services};
use tauri::Manager;

use crate::panel;
use crate::windows::WindowManager;

use super::IpcError;

const PANEL_HIDE_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

fn hides_panel_first(action: &str) -> bool {
    action.starts_with("capture_") || action.starts_with("record_")
}

fn run_after_hide<T>(
    hide_first: bool,
    delay: std::time::Duration,
    wait: impl FnOnce(std::time::Duration),
    run: impl FnOnce() -> T,
) -> T {
    if hide_first {
        wait(delay);
    }
    run()
}

pub fn run_panel_action(
    services: &Services,
    action: &ActionId,
    hide_first: bool,
) -> Result<(), ServiceError> {
    run_after_hide(hide_first, PANEL_HIDE_DELAY, std::thread::sleep, || {
        if action.as_str() == "capture_region" {
            snapline_services::workflows::installed_runner()?
                .run_default_from_capture()
                .map(|_| ())
        } else {
            dispatch(action, services)
        }
    })
}

pub fn run_native_panel_action(services: &Services, name: &str) {
    let action = ActionId::new(name);
    let result = run_panel_action(services, &action, hides_panel_first(name));
    match result {
        Ok(()) => {}
        Err(ServiceError::NotYet) => {
            tracing::warn!(action = %name, "the panel action is not available yet");
            snapline_services::actions::notify_unavailable(name, services);
        }
        Err(error) => {
            snapline_services::actions::notify_failure(&error, services);
        }
    }
}

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
                snapline_services::shell_actions::run(
                    snapline_services::shell_actions::COPY_LAST_LINK,
                    &services,
                )
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
            let hide_first = hides_panel_first(action.as_str());
            if hide_first && from_panel {
                manager.close_panel(window.app_handle());
            }
            let result = tauri::async_runtime::spawn_blocking(move || {
                run_panel_action(&services, &action, hide_first)
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use super::*;

    #[test]
    fn every_capture_and_record_tile_hides_the_panel_first() {
        for action in [
            "capture_region",
            "capture_window",
            "capture_screen",
            "capture_last_region",
            "record_video",
            "record_gif",
        ] {
            assert!(hides_panel_first(action), "{action}");
        }
        assert!(!hides_panel_first("upload_copy_last_link"));
        assert!(!hides_panel_first("app_open"));
    }

    #[test]
    fn the_wait_happens_before_the_capture_starts() {
        let log = RefCell::new(Vec::new());
        let result = run_after_hide(
            true,
            Duration::from_millis(250),
            |delay| log.borrow_mut().push(format!("wait {}", delay.as_millis())),
            || {
                log.borrow_mut().push("capture".to_owned());
                7
            },
        );
        assert_eq!(result, 7);
        assert_eq!(*log.borrow(), ["wait 250", "capture"]);
    }

    #[test]
    fn actions_that_keep_the_panel_do_not_wait() {
        let log = RefCell::new(Vec::new());
        run_after_hide(
            false,
            Duration::from_millis(250),
            |_| log.borrow_mut().push("wait".to_owned()),
            || log.borrow_mut().push("run".to_owned()),
        );
        assert_eq!(*log.borrow(), ["run"]);
    }
}
