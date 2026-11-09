use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
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
}
