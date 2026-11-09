use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use ts_rs::TS;

use crate::dto::history::{HistoryReq, HistoryResp};
use crate::{ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "HistoryAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct HistoryService {
    _services: Services,
}

impl HistoryService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            _services: services.clone(),
        }
    }

    pub fn handle(&self, req: HistoryReq) -> Result<HistoryResp, ServiceError> {
        match req {
            HistoryReq::Ping => Ok(HistoryResp::Pong),
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
        assert_eq!(
            services.history().handle(HistoryReq::Ping),
            Ok(HistoryResp::Pong)
        );
    }
}
