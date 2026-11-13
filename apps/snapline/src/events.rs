use snapline_services::AppEvent;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT_NAME: &str = "snapline://event";

pub fn emit(app: &AppHandle, event: &AppEvent) {
    if let Err(error) = app.emit(EVENT_NAME, event) {
        tracing::warn!(%error, "could not forward an application event");
    }
    if let Some(manager) = app.try_state::<crate::windows::WindowManager>() {
        if let Some(ui) = manager.slint_if_open() {
            ui.emit_event(event);
        }
    }
}
