use snapline_services::AppEvent;
use tauri::{AppHandle, Emitter};

pub const EVENT_NAME: &str = "snapline://event";

pub fn emit(app: &AppHandle, event: &AppEvent) {
    if let Err(error) = app.emit(EVENT_NAME, event) {
        tracing::warn!(%error, "could not forward an application event");
    }
}
