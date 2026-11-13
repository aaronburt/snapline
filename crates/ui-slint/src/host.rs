use serde_json::Value;
use snapline_core::image::Image;
use snapline_services::dto::settings::{SettingsDto, SettingsReq, SettingsResp};
use snapline_services::recording::DEFAULT_MICROPHONE;
use snapline_services::Services;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrScanResult {
    pub text: String,
    pub link: Option<String>,
    pub is_url: bool,
}

pub trait Host: Send + Sync {
    fn browse_folder(&self, _current: Option<&str>) -> Option<String> {
        None
    }

    fn copy_text(&self, _text: &str) -> Result<(), String> {
        Err("Copying is not available here.".to_owned())
    }

    fn copy_image_bytes(&self, _bytes: &[u8]) -> Result<(), String> {
        Err("Copying images is not available here.".to_owned())
    }

    fn open_url(&self, _url: &str) -> Result<(), String> {
        Err("Opening links is not available here.".to_owned())
    }

    fn save_png(&self, _suggested_name: &str, _bytes: &[u8]) -> Result<Option<String>, String> {
        Err("Saving images is not available here.".to_owned())
    }

    fn select_region_image(&self) -> Result<Option<Image>, String> {
        Err("Screen capture is not available here.".to_owned())
    }

    fn run_action(&self, _action: &str) -> Result<(), String> {
        Err("This tool is not available here.".to_owned())
    }

    fn is_light(&self) -> bool {
        false
    }

    fn configure_window(&self, _hwnd: isize, _light: bool) {}

    fn monitors(&self) -> Vec<MonitorInfo> {
        Vec::new()
    }

    fn microphones(&self) -> Vec<String> {
        vec![DEFAULT_MICROPHONE.to_owned()]
    }

    fn update_settings(&self, services: &Services, patch: Value) -> Result<SettingsDto, String> {
        match services.settings().handle(SettingsReq::Update { patch }) {
            Ok(SettingsResp::Updated { settings }) => Ok(settings),
            Ok(other) => Err(format!("unexpected settings response: {other:?}")),
            Err(error) => Err(error.to_string()),
        }
    }
}

pub struct PlainHost;

impl Host for PlainHost {}
