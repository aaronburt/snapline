use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use ts_rs::TS;

use crate::dto::workflow::{WorkflowReq, WorkflowResp};
use crate::{ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "WorkflowAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct WorkflowService {
    _services: Services,
}

impl WorkflowService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            _services: services.clone(),
        }
    }

    pub fn handle(&self, req: WorkflowReq) -> Result<WorkflowResp, ServiceError> {
        match req {
            WorkflowReq::Ping => Ok(WorkflowResp::Pong),
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
            services.workflow().handle(WorkflowReq::Ping),
            Ok(WorkflowResp::Pong)
        );
    }
}
