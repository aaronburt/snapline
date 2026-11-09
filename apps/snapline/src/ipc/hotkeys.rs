use snapline_services::dto::hotkeys::{HotkeysReq, HotkeysResp};
use snapline_services::Services;

use super::IpcError;

#[tauri::command]
pub async fn ipc_hotkeys(
    services: tauri::State<'_, Services>,
    req: HotkeysReq,
) -> Result<HotkeysResp, IpcError> {
    let services = services.inner().clone();
    tauri::async_runtime::spawn_blocking(move || services.hotkeys().handle(req))
        .await
        .map_err(IpcError::join)?
        .map_err(IpcError::from_service)
}
