use snapline_services::dto::history::{HistoryReq, HistoryResp};
use snapline_services::Services;

use super::IpcError;

#[tauri::command]
pub async fn ipc_history(
    app: tauri::AppHandle,
    services: tauri::State<'_, Services>,
    req: HistoryReq,
) -> Result<HistoryResp, IpcError> {
    let services = services.inner().clone();
    let response = tauri::async_runtime::spawn_blocking(move || services.history().handle(req))
        .await
        .map_err(IpcError::join)?
        .map_err(IpcError::from_service)?;
    if let HistoryResp::Delete { id } = &response {
        crate::events::emit(
            &app,
            &snapline_services::AppEvent::History(snapline_services::history::Event::Removed {
                id: *id,
            }),
        );
    }
    Ok(response)
}
