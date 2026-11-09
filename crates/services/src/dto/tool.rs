use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolReq {
    Ping,
    QrMake {
        text: String,
    },
    QrScan {
        #[ts(as = "Option<i32>")]
        capture_id: Option<i64>,
    },
    QrCopyText {
        text: String,
    },
    QrCopyImage {
        text: String,
    },
    QrSavePng {
        text: String,
    },
    QrOpen {
        link: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolResp {
    Pong,
    QrMade {
        svg: String,
        link: Option<String>,
    },
    QrScanned {
        text: String,
        link: Option<String>,
        is_url: bool,
    },
    QrCopied,
    QrOpened,
    QrSaved {
        path: String,
    },
}
