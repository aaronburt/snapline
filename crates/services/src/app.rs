use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use ts_rs::TS;

use crate::dto::app::{AppReq, AppResp};
use crate::{ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "AppAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
    Navigate { route: String },
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct AppService {
    _services: Services,
}

impl AppService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            _services: services.clone(),
        }
    }

    pub fn handle(&self, req: AppReq) -> Result<AppResp, ServiceError> {
        match req {
            AppReq::Ping => Ok(AppResp::Pong),
            AppReq::Ready => Ok(AppResp::Ready),
            AppReq::Info => Ok(AppResp::Info { has_mica: false }),
            AppReq::PanelResize { .. } => Ok(AppResp::PanelResize),
            AppReq::PanelClose => Ok(AppResp::PanelClose),
            AppReq::PanelAction { .. } => Ok(AppResp::PanelAction),
            AppReq::PanelCopyLink => Ok(AppResp::PanelCopyLink),
            AppReq::OpenMain { .. } => Ok(AppResp::OpenMain),
            AppReq::OpenCapture { .. } => Ok(AppResp::OpenCapture),
        }
    }
}

pub fn run(_action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    Err(ServiceError::NotYet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_round_trips() {
        let services = Services::new();
        assert_eq!(services.app().handle(AppReq::Ping), Ok(AppResp::Pong));
    }

    #[test]
    fn panel_requests_return_typed_responses() {
        let services = Services::new();
        assert_eq!(
            services.app().handle(AppReq::PanelResize { height: 321.0 }),
            Ok(AppResp::PanelResize)
        );
        assert_eq!(
            services.app().handle(AppReq::PanelClose),
            Ok(AppResp::PanelClose)
        );
        assert_eq!(
            services.app().handle(AppReq::PanelAction {
                action: "capture_region".to_string()
            }),
            Ok(AppResp::PanelAction)
        );
        assert_eq!(
            services.app().handle(AppReq::PanelCopyLink),
            Ok(AppResp::PanelCopyLink)
        );
        assert_eq!(
            services.app().handle(AppReq::OpenMain {
                route: "/history".to_string()
            }),
            Ok(AppResp::OpenMain)
        );
        assert_eq!(
            services.app().handle(AppReq::OpenCapture { id: 1 }),
            Ok(AppResp::OpenCapture)
        );
    }
}
