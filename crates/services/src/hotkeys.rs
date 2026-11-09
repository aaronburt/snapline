use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use ts_rs::TS;

use crate::dto::hotkeys::{HotkeysReq, HotkeysResp};
use crate::{ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "HotkeysAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct HotkeysService {
    _services: Services,
}

impl HotkeysService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            _services: services.clone(),
        }
    }

    pub fn handle(&self, req: HotkeysReq) -> Result<HotkeysResp, ServiceError> {
        match req {
            HotkeysReq::Ping => Ok(HotkeysResp::Pong),
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
            services.hotkeys().handle(HotkeysReq::Ping),
            Ok(HotkeysResp::Pong)
        );
    }
}
