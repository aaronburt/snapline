use snapline_services::dto::tool::{ToolReq, ToolResp};
use snapline_services::Services;

use super::IpcError;

#[tauri::command]
pub async fn ipc_tool(
    services: tauri::State<'_, Services>,
    req: ToolReq,
) -> Result<ToolResp, IpcError> {
    let services = services.inner().clone();
    tauri::async_runtime::spawn_blocking(move || services.tool().handle(req))
        .await
        .map_err(IpcError::join)?
        .map_err(IpcError::from_service)
}
