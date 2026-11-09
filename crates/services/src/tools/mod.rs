use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use ts_rs::TS;

use crate::dto::tool::{ToolReq, ToolResp};
use crate::{ServiceError, Services};

pub mod ocr;
pub mod qr;
pub mod scrolling;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "ToolAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct ToolService {
    _services: Services,
}

impl ToolService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            _services: services.clone(),
        }
    }

    pub fn handle(&self, req: ToolReq) -> Result<ToolResp, ServiceError> {
        match req {
            ToolReq::Ping => Ok(ToolResp::Pong),
            ToolReq::QrMake { text } => self.qr_make(&text),
            ToolReq::QrScan { .. } => Err(ServiceError::NotYet),
            ToolReq::QrCopyText { .. } => Err(ServiceError::NotYet),
            ToolReq::QrCopyImage { text } => self.qr_copy_image(&text),
            ToolReq::QrSavePng { .. } => Err(ServiceError::NotYet),
            ToolReq::QrOpen { link } => self.qr_open(&link),
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
        assert_eq!(services.tool().handle(ToolReq::Ping), Ok(ToolResp::Pong));
    }
}
