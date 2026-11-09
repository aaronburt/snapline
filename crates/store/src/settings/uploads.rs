use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Uploads {
    pub images: DestinationRef,
    pub videos_and_gifs: DestinationRef,
    pub text_and_code: DestinationRef,
    pub other_files: DestinationRef,
    pub link_shortener: DestinationRef,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub destinations: Vec<Destination>,
    pub strip_exif: bool,
    pub ask_before_clipboard_upload: bool,
    pub retry: Retry,
    pub timeout: Timeout,
    pub keep_history: KeepHistory,
    pub proxy: Proxy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Uploads {
    fn default() -> Self {
        Self {
            images: DestinationRef::save_locally_only(),
            videos_and_gifs: DestinationRef::save_locally_only(),
            text_and_code: DestinationRef::save_locally_only(),
            other_files: DestinationRef::save_locally_only(),
            link_shortener: DestinationRef::none(),
            destinations: Vec::new(),
            strip_exif: true,
            ask_before_clipboard_upload: true,
            retry: Retry::ThreeTimes,
            timeout: Timeout::Secs60,
            keep_history: KeepHistory::Forever,
            proxy: Proxy::System,
            proxy_url: None,
            extra: toml::Table::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DestinationRef(String);

impl DestinationRef {
    pub const SAVE_LOCALLY_ONLY: &'static str = "save_locally_only";
    pub const NONE: &'static str = "none";

    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn save_locally_only() -> Self {
        Self(Self::SAVE_LOCALLY_ONLY.to_owned())
    }

    pub fn none() -> Self {
        Self(Self::NONE.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for DestinationRef {
    fn default() -> Self {
        Self::save_locally_only()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Destination {
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    Never,
    Once,
    ThreeTimes,
    FiveTimes,
}

impl Default for Retry {
    fn default() -> Self {
        Self::ThreeTimes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeout {
    #[serde(rename = "30s")]
    Secs30,
    #[serde(rename = "60s")]
    Secs60,
    #[serde(rename = "120s")]
    Secs120,
}

impl Default for Timeout {
    fn default() -> Self {
        Self::Secs60
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepHistory {
    Forever,
    OneYear,
    Days90,
    Days30,
}

impl Default for KeepHistory {
    fn default() -> Self {
        Self::Forever
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Proxy {
    System,
    None,
    Custom,
}

impl Default for Proxy {
    fn default() -> Self {
        Self::System
    }
}
