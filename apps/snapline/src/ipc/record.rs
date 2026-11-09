use snapline_services::dto::record::{RecordReq, RecordResp};
use snapline_services::Services;

use super::IpcError;

#[tauri::command]
pub async fn ipc_record(
    services: tauri::State<'_, Services>,
    req: RecordReq,
) -> Result<RecordResp, IpcError> {
    let services = services.inner().clone();
    tauri::async_runtime::spawn_blocking(move || services.record().handle(req))
        .await
        .map_err(IpcError::join)?
        .map_err(IpcError::from_service)
}
