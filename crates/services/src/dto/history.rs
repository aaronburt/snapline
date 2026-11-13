use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureKindDto {
    Image,
    Video,
    Gif,
    Text,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UploadStateDto {
    Queued,
    Uploading,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadDto {
    pub dest_id: String,
    pub state: UploadStateDto,
    pub sent: i64,
    pub total: i64,
    pub url: Option<String>,
    pub deletion_url: Option<String>,
    pub error: Option<String>,
    pub attempts: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDto {
    pub id: i64,
    pub kind: CaptureKindDto,
    pub owned: bool,
    pub missing: bool,
    pub name: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub source_app: Option<String>,
    pub created_at: i64,
    pub folder: String,
    pub upload: Option<UploadDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub created_at: i64,
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<CaptureDto>,
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    pub count: i64,
    pub bytes: i64,
    pub uploaded_this_week: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        id: i64,
    },
    Stats,
    ConfirmDelete {
        id: i64,
    },
    Delete {
        id: i64,
    },
    DeleteMany {
        ids: Vec<i64>,
    },
    CopyImage {
        id: i64,
    },
    ShowInFolder {
        id: i64,
    },
    OpenFile {
        id: i64,
    },
    OpenUrl {
        id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HistoryResp {
    Pong,
    Page { page: HistoryPage },
    Search { page: HistoryPage },
    Get { capture: CaptureDto },
    Stats { stats: HistoryStats },
    ConfirmDelete { confirmed: bool },
    Delete { id: i64 },
    DeleteMany { removed: Vec<i64>, failed: usize },
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
