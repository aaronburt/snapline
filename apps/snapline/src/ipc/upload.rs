use snapline_services::dto::upload::{UploadReq, UploadResp};
use snapline_services::Services;

use super::IpcError;

#[tauri::command]
pub async fn ipc_upload(
    services: tauri::State<'_, Services>,
    req: UploadReq,
) -> Result<UploadResp, IpcError> {
    let services = services.inner().clone();
    tauri::async_runtime::spawn_blocking(move || services.upload().handle(req))
        .await
        .map_err(IpcError::join)?
        .map_err(IpcError::from_service)
}
