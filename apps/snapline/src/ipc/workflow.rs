use snapline_services::dto::workflow::{WorkflowReq, WorkflowResp};
use snapline_services::Services;

use super::IpcError;

#[tauri::command]
pub async fn ipc_workflow(
    services: tauri::State<'_, Services>,
    req: WorkflowReq,
) -> Result<WorkflowResp, IpcError> {
    let services = services.inner().clone();
    tauri::async_runtime::spawn_blocking(move || services.workflow().handle(req))
        .await
        .map_err(IpcError::join)?
        .map_err(IpcError::from_service)
}
