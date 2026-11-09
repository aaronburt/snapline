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
    Destination(DestinationReq),
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
    Destination(DestinationResp),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DestinationKindDto {
    Imgur,
    S3,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DestinationStatusDto {
    Configured,
    NotSetUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DestinationActionDto {
    Configure,
    SetUp,
    Edit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DestinationHeaderDto {
    pub name: String,
    pub saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DestinationViewDto {
    pub id: String,
    pub kind: DestinationKindDto,
    pub name: String,
    pub detail: String,
    pub status: DestinationStatusDto,
    pub action: DestinationActionDto,
    pub configured: bool,
    pub client_id_saved: bool,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub path_prefix: Option<String>,
    pub public_url_pattern: Option<String>,
    pub acl_public_read: bool,
    pub access_key_id_saved: bool,
    pub secret_access_key_saved: bool,
    pub method: Option<String>,
    pub request_url: Option<String>,
    pub body: Option<String>,
    pub file_form_field: Option<String>,
    pub link: Option<String>,
    pub thumbnail: Option<String>,
    pub deletion: Option<String>,
    pub error_message: Option<String>,
    pub headers: Vec<DestinationHeaderDto>,
    pub imported_from_sxcu: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DestinationHeaderInput {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DestinationInput {
    pub id: Option<String>,
    pub kind: DestinationKindDto,
    pub name: Option<String>,
    pub client_id: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub path_prefix: Option<String>,
    pub public_url_pattern: Option<String>,
    pub acl_public_read: Option<bool>,
    pub method: Option<String>,
    pub request_url: Option<String>,
    pub body: Option<String>,
    pub file_form_field: Option<String>,
    pub link: Option<String>,
    pub thumbnail: Option<String>,
    pub deletion: Option<String>,
    pub error_message: Option<String>,
    pub headers: Vec<DestinationHeaderInput>,
    pub imported_from_sxcu: Option<bool>,
}

impl std::fmt::Debug for DestinationInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DestinationInput")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("client_id", &self.client_id.as_ref().map(|_| "***"))
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field(
                "access_key_id",
                &self.access_key_id.as_ref().map(|_| "***"),
            )
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| "***"),
            )
            .field("path_prefix", &self.path_prefix)
            .field("public_url_pattern", &self.public_url_pattern)
            .field("acl_public_read", &self.acl_public_read)
            .field("method", &self.method)
            .field("request_url", &self.request_url)
            .field("body", &self.body)
            .field("file_form_field", &self.file_form_field)
            .field("link", &self.link)
            .field("thumbnail", &self.thumbnail)
            .field("deletion", &self.deletion)
            .field("error_message", &self.error_message)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|header| DestinationHeaderInput {
                        name: header.name.clone(),
                        value: header.value.as_ref().map(|_| "***".to_owned()),
                    })
                    .collect::<Vec<_>>(),
            )
            .field("imported_from_sxcu", &self.imported_from_sxcu)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DestinationTestStatusDto {
    NotTested,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DestinationTestOutcomeDto {
    pub status: DestinationTestStatusDto,
    pub link: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum DestinationReq {
    List,
    Add {
        kind: DestinationKindDto,
    },
    Save {
        input: DestinationInput,
    },
    Remove {
        id: String,
    },
    Test {
        id: String,
    },
    ImportSxcu,
    ExportSxcu {
        id: String,
        include_header_values: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum DestinationResp {
    Destinations(Vec<DestinationViewDto>),
    Destination(DestinationViewDto),
    Removed { id: String },
    Test {
        outcome: DestinationTestOutcomeDto,
    },
    Imported {
        destination: Option<DestinationViewDto>,
        warnings: Vec<String>,
    },
    Exported {
        path: Option<String>,
    },
}
