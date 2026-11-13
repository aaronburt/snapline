use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum HotkeysReq {
    Ping,
    Get,
    Preview { owner: String, chord: String },
    Pause,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum HotkeysResp {
    Pong,
    State {
        bindings: Vec<BindingDto>,
        blocked: Vec<BlockedDto>,
        paused: bool,
    },
    Preview {
        preview: PreviewDto,
    },
    Paused {
        paused: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingDto {
    pub owner: String,
    pub label: String,
    pub chord: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedDto {
    pub owner: String,
    pub label: String,
    pub chord: String,
    pub reason: BlockedReasonDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReasonDto {
    Internal,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewDto {
    Free,
    Internal { owners: Vec<String> },
    External,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_use_the_area_tag_and_content_shape() {
        let value = serde_json::to_value(HotkeysReq::Preview {
            owner: "capture_region".to_owned(),
            chord: "Ctrl+Shift+A".to_owned(),
        })
        .expect("json");
        assert_eq!(
            value,
            serde_json::json!({
                "kind": "preview",
                "data": { "owner": "capture_region", "chord": "Ctrl+Shift+A" }
            })
        );
        assert_eq!(
            serde_json::to_value(HotkeysReq::Get).expect("json"),
            serde_json::json!({ "kind": "get" })
        );
    }

    #[test]
    fn responses_use_camel_case_fields_and_snake_case_reasons() {
        let value = serde_json::to_value(HotkeysResp::State {
            bindings: vec![BindingDto {
                owner: "capture_region".to_owned(),
                label: "Capture region".to_owned(),
                chord: "PrtSc".to_owned(),
            }],
            blocked: vec![BlockedDto {
                owner: "capture_window".to_owned(),
                label: "Capture window".to_owned(),
                chord: "PrtSc".to_owned(),
                reason: BlockedReasonDto::External,
            }],
            paused: false,
        })
        .expect("json");
        let data = value.get("data").expect("data");
        assert_eq!(
            data.get("paused").and_then(|value| value.as_bool()),
            Some(false)
        );
        let blocked = data
            .get("blocked")
            .and_then(|value| value.as_array())
            .and_then(|value| value.first())
            .expect("blocked row");
        assert_eq!(
            blocked.get("label").and_then(|value| value.as_str()),
            Some("Capture window")
        );
        assert_eq!(
            blocked.get("reason").and_then(|value| value.as_str()),
            Some("external")
        );
    }

    #[test]
    fn preview_reports_owner_ids() {
        let value = serde_json::to_value(PreviewDto::Internal {
            owners: vec!["capture_window".to_owned()],
        })
        .expect("json");
        assert_eq!(
            value,
            serde_json::json!({ "internal": { "owners": ["capture_window"] } })
        );
    }
}
