use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum WorkflowReq {
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum WorkflowResp {
    Pong,
}
