use thiserror::Error;
use time::OffsetDateTime;

use crate::geometry::{DpiScale, Rect};
use crate::ids::WindowId;
use crate::image::{Image, ImageFormat};
use crate::secret::{Secret, SecretKey};

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub id: u32,
    pub name: String,
    pub bounds: Rect,
    pub work_area: Rect,
    pub dpi: DpiScale,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrozenMonitor {
    pub monitor: Monitor,
    pub image: Image,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrozenDesktop {
    pub monitors: Vec<FrozenMonitor>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CaptureError {
    #[error("Screen capture is not available: {0}")]
    Unavailable(String),
    #[error("The capture failed: {0}")]
    Failed(String),
    #[error("The requested window no longer exists.")]
    WindowMissing,
    #[error("The display layout changed during the capture.")]
    DisplayChanged,
}

pub trait ScreenCapture {
    fn monitors(&self) -> Vec<Monitor>;
    fn freeze_all(&self) -> Result<FrozenDesktop, CaptureError>;
    fn window(&self, id: WindowId, keep_shadow: bool) -> Result<Image, CaptureError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ClipboardError {
    #[error("The clipboard is busy. Try again.")]
    Busy,
    #[error("The clipboard could not be read.")]
    Read,
    #[error("The clipboard could not be written.")]
    Write,
    #[error("The clipboard does not contain an image.")]
    NoImage,
}

pub trait Clipboard {
    fn set_image(&self, image: &Image) -> Result<(), ClipboardError>;
    fn set_text(&self, text: &str) -> Result<(), ClipboardError>;
    fn get_image(&self) -> Result<Option<Image>, ClipboardError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SecretError {
    #[error("The saved secret could not be read.")]
    Read,
    #[error("The secret could not be saved.")]
    Write,
    #[error("The secret could not be removed.")]
    Delete,
    #[error("The secret is too large.")]
    TooLarge,
}

pub trait SecretStore {
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError>;
    fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError>;
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrText {
    pub lines: Vec<String>,
}

impl OcrText {
    pub fn text(&self) -> String {
        self.lines.join("\r\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.trim().is_empty())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OcrError {
    #[error("Windows has no text recognition language installed. Add one in Settings > Time & language > Language & region.")]
    NoLanguage,
    #[error("Text recognition failed: {0}")]
    Failed(String),
}

pub trait Ocr {
    fn recognise(&self, image: &Image, language: Option<&str>) -> Result<OcrText, OcrError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CodecError {
    #[error("This image format is not supported on this PC.")]
    Unsupported,
    #[error("The image could not be encoded: {0}")]
    Encode(String),
    #[error("The image could not be decoded: {0}")]
    Decode(String),
}

pub trait ImageCodec {
    fn encode(
        &self,
        image: &Image,
        format: ImageFormat,
        quality: u8,
    ) -> Result<Vec<u8>, CodecError>;
    fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub capture_id: Option<i64>,
}

impl Notice {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            capture_id: None,
        }
    }

    pub fn for_capture(title: impl Into<String>, body: impl Into<String>, capture_id: i64) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            capture_id: Some(capture_id),
        }
    }
}

pub trait Notifier {
    fn toast(&self, notice: Notice);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundId {
    Shutter,
    Done,
}

pub trait Sound {
    fn play(&self, sound: SoundId);
}

pub trait Clock {
    fn now_local(&self) -> OffsetDateTime;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_text_joins_lines_with_carriage_return_newline() {
        let text = OcrText {
            lines: vec!["one".to_string(), "two".to_string()],
        };
        assert_eq!(text.text(), "one\r\ntwo");
        assert!(!text.is_empty());
    }

    #[test]
    fn blank_ocr_text_is_empty() {
        assert!(OcrText {
            lines: vec![String::new(), "  ".to_string()]
        }
        .is_empty());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTheme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    Copy,
    Save,
    Pin,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MagnifierZoom {
    X2,
    X4,
    X8,
}

impl MagnifierZoom {
    pub const fn factor(self) -> i32 {
        match self {
            Self::X2 => 2,
            Self::X4 => 4,
            Self::X8 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagnifierSettings {
    pub enabled: bool,
    pub zoom: MagnifierZoom,
}

impl Default for MagnifierSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            zoom: MagnifierZoom::X4,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayRequest {
    pub desktop: FrozenDesktop,
    pub annotate: bool,
    pub upload_on_finish: bool,
    pub snap: crate::overlay::SnapSet,
    pub magnifier: MagnifierSettings,
    pub full_screen_crosshair: bool,
    pub theme: OverlayTheme,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayResult {
    pub action: Option<OverlayAction>,
    pub selection: Rect,
    pub monitor: Option<u32>,
    pub tool: crate::overlay::Tool,
    pub colour: crate::overlay::Rgb,
    pub image: Option<Image>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OverlayError {
    #[error("The overlay was cancelled.")]
    Cancelled,
    #[error("The overlay could not be shown: {0}")]
    Failed(String),
    #[error("There is no monitor to draw the overlay on.")]
    NoMonitors,
}

pub trait OverlayHost {
    fn select_region(&self, request: OverlayRequest) -> Result<OverlayResult, OverlayError>;
}
