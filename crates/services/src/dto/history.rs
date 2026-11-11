use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HistoryFilter {
    All,
    Images,
    Videos,
    Gifs,
    Files,
    Uploaded,
    LocalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CaptureKindDto {
    Image,
    Video,
    Gif,
    Text,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum UploadStateDto {
    Queued,
    Uploading,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UploadDto {
    pub dest_id: String,
    pub state: UploadStateDto,
    #[ts(as = "i32")]
    pub sent: i64,
    #[ts(as = "i32")]
    pub total: i64,
    pub url: Option<String>,
    pub deletion_url: Option<String>,
    pub error: Option<String>,
    #[ts(as = "i32")]
    pub attempts: i64,
    #[ts(as = "i32")]
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(rename = "HistoryCaptureDto")]
#[serde(rename_all = "camelCase")]
pub struct CaptureDto {
    #[ts(as = "i32")]
    pub id: i64,
    pub kind: CaptureKindDto,
    pub owned: bool,
    pub missing: bool,
    pub name: String,
    #[ts(as = "i32")]
    pub bytes: i64,
    #[ts(as = "Option<i32>")]
    pub width: Option<i64>,
    #[ts(as = "Option<i32>")]
    pub height: Option<i64>,
    #[ts(as = "Option<i32>")]
    pub duration_ms: Option<i64>,
    pub source_app: Option<String>,
    #[ts(as = "i32")]
    pub created_at: i64,
    pub folder: String,
    pub upload: Option<UploadDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    #[ts(as = "i32")]
    pub created_at: i64,
    #[ts(as = "i32")]
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<CaptureDto>,
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    #[ts(as = "i32")]
    pub count: i64,
    #[ts(as = "i32")]
    pub bytes: i64,
    #[ts(as = "i32")]
    pub uploaded_this_week: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HistoryReq {
    Ping,
    Page {
        filter: HistoryFilter,
        cursor: Option<Cursor>,
    },
    Search {
        text: String,
        filter: HistoryFilter,
        cursor: Option<Cursor>,
    },
    Get {
        #[ts(as = "i32")]
        id: i64,
    },
    Stats,
    ConfirmDelete {
        #[ts(as = "i32")]
        id: i64,
    },
    Delete {
        #[ts(as = "i32")]
        id: i64,
    },
    DeleteMany {
        #[ts(as = "Vec<i32>")]
        ids: Vec<i64>,
    },
    CopyImage {
        #[ts(as = "i32")]
        id: i64,
    },
    ShowInFolder {
        #[ts(as = "i32")]
        id: i64,
    },
    OpenFile {
        #[ts(as = "i32")]
        id: i64,
    },
    OpenUrl {
        #[ts(as = "i32")]
        id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HistoryResp {
    Pong,
    Page {
        page: HistoryPage,
    },
    Search {
        page: HistoryPage,
    },
    Get {
        capture: CaptureDto,
    },
    Stats {
        stats: HistoryStats,
    },
    ConfirmDelete {
        confirmed: bool,
    },
    Delete {
        #[ts(as = "i32")]
        id: i64,
    },
    DeleteMany {
        #[ts(as = "Vec<i32>")]
        removed: Vec<i64>,
        failed: usize,
    },
    CopyImage,
    ShowInFolder,
    OpenFile,
    OpenUrl,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_responses_use_camel_case_tags_and_fields() {
        let page = serde_json::to_value(HistoryReq::Page {
            filter: HistoryFilter::LocalOnly,
            cursor: Some(Cursor {
                created_at: 1_760_000_000_000,
                id: 7,
            }),
        })
        .unwrap();
        assert_eq!(
            page,
            serde_json::json!({
                "kind": "page",
                "data": {
                    "filter": "localOnly",
                    "cursor": { "createdAt": 1_760_000_000_000_i64, "id": 7 }
                }
            })
        );
        let stats = serde_json::to_value(HistoryResp::Stats {
            stats: HistoryStats {
                count: 3,
                bytes: 4096,
                uploaded_this_week: 1,
            },
        })
        .unwrap();
        assert_eq!(
            stats,
            serde_json::json!({
                "kind": "stats",
                "data": { "stats": { "count": 3, "bytes": 4096, "uploadedThisWeek": 1 } }
            })
        );
        let get = serde_json::to_value(HistoryReq::Get { id: 42 }).unwrap();
        assert_eq!(
            get,
            serde_json::json!({ "kind": "get", "data": { "id": 42 } })
        );
    }

    #[test]
    fn filters_serialise_to_the_contract_names() {
        assert_eq!(serde_json::to_value(HistoryFilter::All).unwrap(), "all");
        assert_eq!(
            serde_json::to_value(HistoryFilter::LocalOnly).unwrap(),
            "localOnly"
        );
        assert_eq!(serde_json::to_value(UploadStateDto::Done).unwrap(), "done");
        assert_eq!(serde_json::to_value(CaptureKindDto::Gif).unwrap(), "gif");
    }
}
