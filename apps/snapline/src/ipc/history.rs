use snapline_services::dto::history::{HistoryReq, HistoryResp};
use snapline_services::Services;

use super::IpcError;

fn event_for(response: &HistoryResp) -> Option<snapline_services::history::Event> {
    match response {
        HistoryResp::Delete { id } => Some(snapline_services::history::Event::Removed { id: *id }),
        HistoryResp::DeleteMany { removed, .. } if !removed.is_empty() => {
            Some(snapline_services::history::Event::RemovedMany {
                ids: removed.clone(),
            })
        }
        _ => None,
    }
}

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
    if let Some(event) = event_for(&response) {
        crate::events::emit(&app, &snapline_services::AppEvent::History(event));
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use snapline_services::history::Event;

    use super::*;

    #[test]
    fn deleting_one_or_many_emits_a_removal_event() {
        assert!(matches!(
            event_for(&HistoryResp::Delete { id: 4 }),
            Some(Event::Removed { id: 4 })
        ));
        match event_for(&HistoryResp::DeleteMany {
            removed: vec![1, 2, 3],
            failed: 0,
        }) {
            Some(Event::RemovedMany { ids }) => assert_eq!(ids, vec![1, 2, 3]),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn a_delete_that_removed_nothing_emits_no_event() {
        assert!(event_for(&HistoryResp::DeleteMany {
            removed: vec![],
            failed: 2,
        })
        .is_none());
        assert!(event_for(&HistoryResp::Pong).is_none());
    }
}
