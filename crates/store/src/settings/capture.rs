use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::io;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capture {
    pub include_cursor: bool,
    pub capture_delay: CaptureDelay,
    pub shutter_sound: bool,
    pub keep_window_shadow: bool,
    pub freeze_screen: bool,
    pub magnifier: bool,
    pub magnifier_zoom: MagnifierZoom,
    pub snap_to_windows: bool,
    pub show_size_and_position: bool,
    pub fullscreen_crosshair: bool,
    pub fullscreen_capture_uses: FullscreenCapture,
    pub video_format: VideoFormat,
    pub frame_rate: FrameRate,
    pub video_quality: VideoQuality,
    pub record_system_audio: bool,
    pub record_microphone: bool,
    pub microphone_device: String,
    pub recording_border: bool,
    pub ffmpeg_path: Option<PathBuf>,
    pub gif_frame_rate: GifFrameRate,
    pub gif_max_width: GifMaxWidth,
    pub gif_optimise_palette: bool,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            include_cursor: false,
            capture_delay: CaptureDelay::Off,
            shutter_sound: true,
            keep_window_shadow: true,
            freeze_screen: true,
            magnifier: true,
            magnifier_zoom: MagnifierZoom::X4,
            snap_to_windows: true,
            show_size_and_position: true,
            fullscreen_crosshair: false,
            fullscreen_capture_uses: FullscreenCapture::MonitorUnderCursor,
            video_format: VideoFormat::Mp4H264,
            frame_rate: FrameRate::Fps30,
            video_quality: VideoQuality::High,
            record_system_audio: true,
            record_microphone: false,
            microphone_device: "Default microphone".to_owned(),
            recording_border: true,
            ffmpeg_path: None,
            gif_frame_rate: GifFrameRate::Fps15,
            gif_max_width: GifMaxWidth::Px800,
            gif_optimise_palette: true,
            extra: toml::Table::new(),
        }
    }
}

impl Capture {
    pub fn resolved_ffmpeg_path(&self, exe_dir: &Path) -> PathBuf {
        self.ffmpeg_path
            .clone()
            .unwrap_or_else(|| io::bundled_ffmpeg(exe_dir))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureDelay {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "1s")]
    OneSecond,
    #[serde(rename = "3s")]
    ThreeSeconds,
    #[serde(rename = "5s")]
    FiveSeconds,
}

impl Default for CaptureDelay {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MagnifierZoom {
    #[serde(rename = "2x")]
    X2,
    #[serde(rename = "4x")]
    X4,
    #[serde(rename = "8x")]
    X8,
}

impl Default for MagnifierZoom {
    fn default() -> Self {
        Self::X4
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullscreenCapture {
    AllMonitors,
    MonitorUnderCursor,
    PrimaryMonitor,
}

impl Default for FullscreenCapture {
    fn default() -> Self {
        Self::MonitorUnderCursor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoFormat {
    Mp4H264,
    Mp4H265Hevc,
    WebmVp9,
    MkvH264,
}

impl Default for VideoFormat {
    fn default() -> Self {
        Self::Mp4H264
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameRate {
    Fps24,
    Fps30,
    Fps60,
}

impl Default for FrameRate {
    fn default() -> Self {
        Self::Fps30
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    High,
    Balanced,
    SmallFile,
    Lossless,
}

impl Default for VideoQuality {
    fn default() -> Self {
        Self::High
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GifFrameRate {
    Fps10,
    Fps15,
    Fps20,
    Fps30,
}

impl Default for GifFrameRate {
    fn default() -> Self {
        Self::Fps15
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GifMaxWidth {
    Original,
    Px1280,
    Px800,
    Px480,
}

impl Default for GifMaxWidth {
    fn default() -> Self {
        Self::Px800
    }
}
