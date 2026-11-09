use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RecordKindDto {
    Video,
    Gif,
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RecordStateDto {
    #[default]
    Idle,
    Arming,
    Recording,
    Stopping,
    Encoding,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecordStatusDto {
    pub state: RecordStateDto,
    pub kind: Option<RecordKindDto>,
    #[ts(as = "i32")]
    pub elapsed_ms: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[ts(as = "Option<i32>")]
    pub duration_ms: Option<i64>,
    #[ts(as = "Option<i32>")]
    pub capture_id: Option<i64>,
    pub path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum RecordReq {
    Ping,
    Start {
        kind: RecordKindDto,
    },
    Stop,
    State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum RecordResp {
    Pong,
    Status {
        status: RecordStatusDto,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> RecordStatusDto {
        RecordStatusDto {
            state: RecordStateDto::Recording,
            kind: Some(RecordKindDto::Video),
            elapsed_ms: 42_000,
            width: Some(1920),
            height: Some(1080),
            duration_ms: Some(42_000),
            capture_id: Some(7),
            path: Some(r"C:\Videos\clip.mp4".to_owned()),
            error: None,
        }
    }

    #[test]
    fn start_requests_round_trip_with_their_kind() {
        for (kind, text) in [
            (RecordKindDto::Video, "video"),
            (RecordKindDto::Gif, "gif"),
            (RecordKindDto::Window, "window"),
        ] {
            let req = RecordReq::Start { kind };
            let json = serde_json::to_string(&req).expect("serialise");
            assert_eq!(json, format!(r#"{{"kind":"start","data":{{"kind":"{text}"}}}}"#));
            assert_eq!(serde_json::from_str::<RecordReq>(&json).expect("parse"), req);
        }
    }

    #[test]
    fn stop_and_state_requests_round_trip() {
        for req in [RecordReq::Stop, RecordReq::State, RecordReq::Ping] {
            let json = serde_json::to_string(&req).expect("serialise");
            assert_eq!(serde_json::from_str::<RecordReq>(&json).expect("parse"), req);
        }
        assert_eq!(
            serde_json::to_string(&RecordReq::Stop).expect("serialise"),
            r#"{"kind":"stop"}"#
        );
    }

    #[test]
    fn a_status_response_round_trips_every_field() {
        let resp = RecordResp::Status { status: status() };
        let json = serde_json::to_string(&resp).expect("serialise");
        assert!(json.contains(r#""state":"recording""#));
        assert!(json.contains(r#""kind":"video""#));
        assert!(json.contains(r#""elapsedMs":42000"#));
        assert_eq!(serde_json::from_str::<RecordResp>(&json).expect("parse"), resp);
    }

    #[test]
    fn every_state_has_the_documented_wire_name() {
        for (state, text) in [
            (RecordStateDto::Idle, "idle"),
            (RecordStateDto::Arming, "arming"),
            (RecordStateDto::Recording, "recording"),
            (RecordStateDto::Stopping, "stopping"),
            (RecordStateDto::Encoding, "encoding"),
            (RecordStateDto::Done, "done"),
            (RecordStateDto::Failed, "failed"),
        ] {
            let json = serde_json::to_string(&state).expect("serialise");
            assert_eq!(json, format!("\"{text}\""));
        }
        assert_eq!(RecordStateDto::default(), RecordStateDto::Idle);
    }
}
