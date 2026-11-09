use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum UploadStatus {
    Queued,
    Uploading,
    BackingOff,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UploadState {
    #[ts(as = "u32")]
    pub id: u64,
    pub dest: String,
    pub run_id: Option<String>,
    pub status: UploadStatus,
    pub attempts: u32,
    pub link: Option<String>,
    pub page_link: Option<String>,
    pub deletion_link: Option<String>,
    pub error: Option<String>,
    #[ts(as = "Option<u32>")]
    pub retry_in_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum UploadReq {
    Ping,
    State {
        #[ts(as = "u32")]
        id: u64,
    },
    Cancel {
        #[ts(as = "u32")]
        id: u64,
    },
    Retry {
        #[ts(as = "u32")]
        id: u64,
    },
    List,
    UploadElsewhere {
        #[ts(as = "i32")]
        capture_id: i64,
        dest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum UploadResp {
    Pong,
    State(UploadState),
    States(Vec<UploadState>),
    Accepted {
        #[ts(as = "u32")]
        id: u64,
    },
    Cancelled {
        #[ts(as = "u32")]
        id: u64,
    },
    Retried {
        #[ts(as = "u32")]
        id: u64,
    },
}
