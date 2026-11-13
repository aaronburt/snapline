use serde::{Deserialize, Serialize};

use crate::dto::history::CaptureKindDto;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecentStatusDto {
    Local,
    Uploading,
    Uploaded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentCaptureDto {
    pub id: i64,
    pub name: String,
    pub kind: CaptureKindDto,
    pub status: RecentStatusDto,
    pub created_at: i64,
    pub url: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AppReq {
    Ping,
    Ready,
    Info,
    PanelResize { height: f64 },
    PanelClose,
    PanelAction { action: String },
    PanelCopyLink,
    OpenMain { route: String },
    OpenCapture { id: u32 },
    Recent,
    Stats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AppResp {
    Pong,
    Ready,
    Info { has_mica: bool },
    PanelResize,
    PanelClose,
    PanelAction,
    PanelCopyLink,
    OpenMain,
    OpenCapture,
    Recent { captures: Vec<RecentCaptureDto> },
    Stats { count: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_requests_use_camel_case_tags_and_fields() {
        let resize = serde_json::to_value(AppReq::PanelResize { height: 321.0 }).unwrap();
        assert_eq!(
            resize,
            serde_json::json!({ "kind": "panelResize", "data": { "height": 321.0 } })
        );
        let main = serde_json::to_value(AppReq::OpenMain {
            route: "/history".to_string(),
        })
        .unwrap();
        assert_eq!(
            main,
            serde_json::json!({ "kind": "openMain", "data": { "route": "/history" } })
        );
    }

    #[test]
    fn recent_and_stats_use_the_contract_names() {
        let recent = serde_json::to_value(AppReq::Recent).unwrap();
        assert_eq!(recent, serde_json::json!({ "kind": "recent" }));
        let stats = serde_json::to_value(AppReq::Stats).unwrap();
        assert_eq!(stats, serde_json::json!({ "kind": "stats" }));
        let response = serde_json::to_value(AppResp::Recent {
            captures: vec![RecentCaptureDto {
                id: 7,
                name: "one.png".to_string(),
                kind: CaptureKindDto::Image,
                status: RecentStatusDto::Uploaded,
                created_at: 1_760_000_000_000,
                url: Some("https://example.com/one.png".to_string()),
                duration_ms: None,
            }],
        })
        .unwrap();
        assert_eq!(
            response,
            serde_json::json!({
                "kind": "recent",
                "data": {
                    "captures": [{
                        "id": 7,
                        "name": "one.png",
                        "kind": "image",
                        "status": "uploaded",
                        "createdAt": 1_760_000_000_000_i64,
                        "url": "https://example.com/one.png",
                        "durationMs": null
                    }]
                }
            })
        );
        let stats = serde_json::to_value(AppResp::Stats { count: 4 }).unwrap();
        assert_eq!(
            stats,
            serde_json::json!({ "kind": "stats", "data": { "count": 4 } })
        );
    }
}
