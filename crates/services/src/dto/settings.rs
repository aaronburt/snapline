use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use snapline_store::settings as store;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum SettingsReq {
    Ping,
    Get,
    Update {
        #[ts(type = "Record<string, unknown>")]
        patch: serde_json::Value,
    },
    PreviewName {
        pattern: String,
        format: String,
    },
    BrowseFolder {
        current: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum SettingsResp {
    Pong,
    Settings { settings: SettingsDto },
    Updated { settings: SettingsDto },
    PreviewName { name: String },
    BrowseFolder { path: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub version: u32,
    pub general: GeneralDto,
    pub capture: CaptureDto,
    pub hotkeys: HotkeysDto,
    pub uploads: UploadsDto,
    pub workflows: WorkflowsDto,
    pub window: WindowDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GeneralDto {
    pub launch_at_signin: bool,
    pub start_minimised: bool,
    pub tray_click_capture: bool,
    pub notifications: bool,
    pub save_folder: String,
    pub subfolders: SubfoldersDto,
    pub image_format: ImageFormatDto,
    pub lossy_quality: u8,
    pub name_pattern: String,
    pub default_workflow: Vec<WorkflowStepDto>,
    pub copied_link_format: LinkFormatDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepDto {
    pub kind: WorkflowStepKindDto,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SubfoldersDto {
    ByMonth,
    ByDay,
    ByApp,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormatDto {
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LinkFormatDto {
    Direct,
    Markdown,
    Html,
    Bbcode,
    Page,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepKindDto {
    CopyImage,
    Annotate,
    Save,
    Upload,
    CopyLink,
    PlaySound,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDto {
    pub include_cursor: bool,
    pub capture_delay: CaptureDelayDto,
    pub shutter_sound: bool,
    pub keep_window_shadow: bool,
    pub freeze_screen: bool,
    pub magnifier: bool,
    pub magnifier_zoom: MagnifierZoomDto,
    pub snap_to_windows: bool,
    pub show_size_and_position: bool,
    pub fullscreen_crosshair: bool,
    pub fullscreen_capture_uses: FullscreenCaptureDto,
    pub video_format: VideoFormatDto,
    pub frame_rate: FrameRateDto,
    pub video_quality: VideoQualityDto,
    pub record_system_audio: bool,
    pub record_microphone: bool,
    pub microphone_device: String,
    pub recording_border: bool,
    pub ffmpeg_path: Option<String>,
    pub gif_frame_rate: GifFrameRateDto,
    pub gif_max_width: GifMaxWidthDto,
    pub gif_optimise_palette: bool,
    pub scroll_delay_ms: u32,
    pub scroll_limit: ScrollLimitDto,
    pub remove_sticky_headers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum CaptureDelayDto {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "1s")]
    OneSecond,
    #[serde(rename = "3s")]
    ThreeSeconds,
    #[serde(rename = "5s")]
    FiveSeconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum MagnifierZoomDto {
    #[serde(rename = "2x")]
    X2,
    #[serde(rename = "4x")]
    X4,
    #[serde(rename = "8x")]
    X8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum FullscreenCaptureDto {
    AllMonitors,
    MonitorUnderCursor,
    PrimaryMonitor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum VideoFormatDto {
    Mp4H264,
    Mp4H265Hevc,
    WebmVp9,
    MkvH264,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum FrameRateDto {
    Fps24,
    Fps30,
    Fps60,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum VideoQualityDto {
    High,
    Balanced,
    SmallFile,
    Lossless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GifFrameRateDto {
    Fps10,
    Fps15,
    Fps20,
    Fps30,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GifMaxWidthDto {
    Original,
    Px1280,
    Px800,
    Px480,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ScrollLimitDto {
    Ten,
    Twenty,
    Thirty,
    Fifty,
    Unlimited,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HotkeysDto {
    pub bindings: BTreeMap<String, String>,
    pub take_over_prtsc: bool,
    pub warn_about_clashes: bool,
    pub restore: RestoreDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RestoreDto {
    pub print_screen_key_for_snipping_enabled: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UploadsDto {
    pub images: String,
    pub videos_and_gifs: String,
    pub text_and_code: String,
    pub other_files: String,
    pub link_shortener: String,
    pub destinations: Vec<DestinationDto>,
    pub strip_exif: bool,
    pub ask_before_clipboard_upload: bool,
    pub retry: RetryDto,
    pub timeout: TimeoutDto,
    pub keep_history: KeepHistoryDto,
    pub proxy: ProxyDto,
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct DestinationDto {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RetryDto {
    Never,
    Once,
    ThreeTimes,
    FiveTimes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum TimeoutDto {
    #[serde(rename = "30s")]
    Secs30,
    #[serde(rename = "60s")]
    Secs60,
    #[serde(rename = "120s")]
    Secs120,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KeepHistoryDto {
    Forever,
    OneYear,
    Days90,
    Days30,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProxyDto {
    System,
    None,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowsDto {
    pub list: Vec<WorkflowDto>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct WorkflowDto {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WindowDto {
    pub main: MainWindowDto,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MainWindowDto {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl From<&store::Settings> for SettingsDto {
    fn from(value: &store::Settings) -> Self {
        Self {
            version: value.version,
            general: GeneralDto::from(&value.general),
            capture: CaptureDto::from(&value.capture),
            hotkeys: HotkeysDto::from(&value.hotkeys),
            uploads: UploadsDto::from(&value.uploads),
            workflows: WorkflowsDto::from(&value.workflows),
            window: WindowDto::from(&value.window),
        }
    }
}

impl From<&store::General> for GeneralDto {
    fn from(value: &store::General) -> Self {
        Self {
            launch_at_signin: value.launch_at_signin,
            start_minimised: value.start_minimised,
            tray_click_capture: value.tray_click_capture,
            notifications: value.notifications,
            save_folder: value.save_folder.clone(),
            subfolders: SubfoldersDto::from(value.subfolders),
            image_format: ImageFormatDto::from(value.image_format),
            lossy_quality: value.lossy_quality,
            name_pattern: value.name_pattern.clone(),
            default_workflow: value
                .default_workflow
                .iter()
                .map(WorkflowStepDto::from)
                .collect(),
            copied_link_format: LinkFormatDto::from(value.copied_link_format),
        }
    }
}

impl From<&store::general::WorkflowStep> for WorkflowStepDto {
    fn from(value: &store::general::WorkflowStep) -> Self {
        Self {
            kind: WorkflowStepKindDto::from(value.kind),
            enabled: value.enabled,
        }
    }
}

impl From<store::general::Subfolders> for SubfoldersDto {
    fn from(value: store::general::Subfolders) -> Self {
        match value {
            store::general::Subfolders::ByMonth => Self::ByMonth,
            store::general::Subfolders::ByDay => Self::ByDay,
            store::general::Subfolders::ByApp => Self::ByApp,
            store::general::Subfolders::None => Self::None,
        }
    }
}

impl From<store::general::ImageFormat> for ImageFormatDto {
    fn from(value: store::general::ImageFormat) -> Self {
        match value {
            store::general::ImageFormat::Png => Self::Png,
            store::general::ImageFormat::Jpeg => Self::Jpeg,
            store::general::ImageFormat::Webp => Self::Webp,
        }
    }
}

impl From<store::general::LinkFormat> for LinkFormatDto {
    fn from(value: store::general::LinkFormat) -> Self {
        match value {
            store::general::LinkFormat::Direct => Self::Direct,
            store::general::LinkFormat::Markdown => Self::Markdown,
            store::general::LinkFormat::Html => Self::Html,
            store::general::LinkFormat::Bbcode => Self::Bbcode,
            store::general::LinkFormat::Page => Self::Page,
        }
    }
}

impl From<store::general::WorkflowStepKind> for WorkflowStepKindDto {
    fn from(value: store::general::WorkflowStepKind) -> Self {
        match value {
            store::general::WorkflowStepKind::CopyImage => Self::CopyImage,
            store::general::WorkflowStepKind::Annotate => Self::Annotate,
            store::general::WorkflowStepKind::Save => Self::Save,
            store::general::WorkflowStepKind::Upload => Self::Upload,
            store::general::WorkflowStepKind::CopyLink => Self::CopyLink,
            store::general::WorkflowStepKind::PlaySound => Self::PlaySound,
        }
    }
}

impl From<&store::Capture> for CaptureDto {
    fn from(value: &store::Capture) -> Self {
        Self {
            include_cursor: value.include_cursor,
            capture_delay: CaptureDelayDto::from(value.capture_delay),
            shutter_sound: value.shutter_sound,
            keep_window_shadow: value.keep_window_shadow,
            freeze_screen: value.freeze_screen,
            magnifier: value.magnifier,
            magnifier_zoom: MagnifierZoomDto::from(value.magnifier_zoom),
            snap_to_windows: value.snap_to_windows,
            show_size_and_position: value.show_size_and_position,
            fullscreen_crosshair: value.fullscreen_crosshair,
            fullscreen_capture_uses: FullscreenCaptureDto::from(value.fullscreen_capture_uses),
            video_format: VideoFormatDto::from(value.video_format),
            frame_rate: FrameRateDto::from(value.frame_rate),
            video_quality: VideoQualityDto::from(value.video_quality),
            record_system_audio: value.record_system_audio,
            record_microphone: value.record_microphone,
            microphone_device: value.microphone_device.clone(),
            recording_border: value.recording_border,
            ffmpeg_path: value
                .ffmpeg_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            gif_frame_rate: GifFrameRateDto::from(value.gif_frame_rate),
            gif_max_width: GifMaxWidthDto::from(value.gif_max_width),
            gif_optimise_palette: value.gif_optimise_palette,
            scroll_delay_ms: value.scroll_delay_ms,
            scroll_limit: ScrollLimitDto::from(value.scroll_limit),
            remove_sticky_headers: value.remove_sticky_headers,
        }
    }
}

impl From<store::capture::CaptureDelay> for CaptureDelayDto {
    fn from(value: store::capture::CaptureDelay) -> Self {
        match value {
            store::capture::CaptureDelay::Off => Self::Off,
            store::capture::CaptureDelay::OneSecond => Self::OneSecond,
            store::capture::CaptureDelay::ThreeSeconds => Self::ThreeSeconds,
            store::capture::CaptureDelay::FiveSeconds => Self::FiveSeconds,
        }
    }
}

impl From<store::capture::MagnifierZoom> for MagnifierZoomDto {
    fn from(value: store::capture::MagnifierZoom) -> Self {
        match value {
            store::capture::MagnifierZoom::X2 => Self::X2,
            store::capture::MagnifierZoom::X4 => Self::X4,
            store::capture::MagnifierZoom::X8 => Self::X8,
        }
    }
}

impl From<store::capture::FullscreenCapture> for FullscreenCaptureDto {
    fn from(value: store::capture::FullscreenCapture) -> Self {
        match value {
            store::capture::FullscreenCapture::AllMonitors => Self::AllMonitors,
            store::capture::FullscreenCapture::MonitorUnderCursor => Self::MonitorUnderCursor,
            store::capture::FullscreenCapture::PrimaryMonitor => Self::PrimaryMonitor,
        }
    }
}

impl From<store::capture::VideoFormat> for VideoFormatDto {
    fn from(value: store::capture::VideoFormat) -> Self {
        match value {
            store::capture::VideoFormat::Mp4H264 => Self::Mp4H264,
            store::capture::VideoFormat::Mp4H265Hevc => Self::Mp4H265Hevc,
            store::capture::VideoFormat::WebmVp9 => Self::WebmVp9,
            store::capture::VideoFormat::MkvH264 => Self::MkvH264,
        }
    }
}

impl From<store::capture::FrameRate> for FrameRateDto {
    fn from(value: store::capture::FrameRate) -> Self {
        match value {
            store::capture::FrameRate::Fps24 => Self::Fps24,
            store::capture::FrameRate::Fps30 => Self::Fps30,
            store::capture::FrameRate::Fps60 => Self::Fps60,
        }
    }
}

impl From<store::capture::VideoQuality> for VideoQualityDto {
    fn from(value: store::capture::VideoQuality) -> Self {
        match value {
            store::capture::VideoQuality::High => Self::High,
            store::capture::VideoQuality::Balanced => Self::Balanced,
            store::capture::VideoQuality::SmallFile => Self::SmallFile,
            store::capture::VideoQuality::Lossless => Self::Lossless,
        }
    }
}

impl From<store::capture::GifFrameRate> for GifFrameRateDto {
    fn from(value: store::capture::GifFrameRate) -> Self {
        match value {
            store::capture::GifFrameRate::Fps10 => Self::Fps10,
            store::capture::GifFrameRate::Fps15 => Self::Fps15,
            store::capture::GifFrameRate::Fps20 => Self::Fps20,
            store::capture::GifFrameRate::Fps30 => Self::Fps30,
        }
    }
}

impl From<store::capture::GifMaxWidth> for GifMaxWidthDto {
    fn from(value: store::capture::GifMaxWidth) -> Self {
        match value {
            store::capture::GifMaxWidth::Original => Self::Original,
            store::capture::GifMaxWidth::Px1280 => Self::Px1280,
            store::capture::GifMaxWidth::Px800 => Self::Px800,
            store::capture::GifMaxWidth::Px480 => Self::Px480,
        }
    }
}

impl From<store::capture::ScrollLimit> for ScrollLimitDto {
    fn from(value: store::capture::ScrollLimit) -> Self {
        match value {
            store::capture::ScrollLimit::Ten => Self::Ten,
            store::capture::ScrollLimit::Twenty => Self::Twenty,
            store::capture::ScrollLimit::Thirty => Self::Thirty,
            store::capture::ScrollLimit::Fifty => Self::Fifty,
            store::capture::ScrollLimit::Unlimited => Self::Unlimited,
        }
    }
}

impl From<&store::Hotkeys> for HotkeysDto {
    fn from(value: &store::Hotkeys) -> Self {
        Self {
            bindings: value.bindings.clone(),
            take_over_prtsc: value.take_over_prtsc,
            warn_about_clashes: value.warn_about_clashes,
            restore: RestoreDto {
                print_screen_key_for_snipping_enabled: value
                    .restore
                    .print_screen_key_for_snipping_enabled,
            },
        }
    }
}

impl From<&store::Uploads> for UploadsDto {
    fn from(value: &store::Uploads) -> Self {
        Self {
            images: value.images.as_str().to_owned(),
            videos_and_gifs: value.videos_and_gifs.as_str().to_owned(),
            text_and_code: value.text_and_code.as_str().to_owned(),
            other_files: value.other_files.as_str().to_owned(),
            link_shortener: value.link_shortener.as_str().to_owned(),
            destinations: value
                .destinations
                .iter()
                .map(DestinationDto::from)
                .collect(),
            strip_exif: value.strip_exif,
            ask_before_clipboard_upload: value.ask_before_clipboard_upload,
            retry: RetryDto::from(value.retry),
            timeout: TimeoutDto::from(value.timeout),
            keep_history: KeepHistoryDto::from(value.keep_history),
            proxy: ProxyDto::from(value.proxy),
            proxy_url: value.proxy_url.clone(),
        }
    }
}

impl From<&store::uploads::Destination> for DestinationDto {
    fn from(_: &store::uploads::Destination) -> Self {
        Self {}
    }
}

impl From<store::uploads::Retry> for RetryDto {
    fn from(value: store::uploads::Retry) -> Self {
        match value {
            store::uploads::Retry::Never => Self::Never,
            store::uploads::Retry::Once => Self::Once,
            store::uploads::Retry::ThreeTimes => Self::ThreeTimes,
            store::uploads::Retry::FiveTimes => Self::FiveTimes,
        }
    }
}

impl From<store::uploads::Timeout> for TimeoutDto {
    fn from(value: store::uploads::Timeout) -> Self {
        match value {
            store::uploads::Timeout::Secs30 => Self::Secs30,
            store::uploads::Timeout::Secs60 => Self::Secs60,
            store::uploads::Timeout::Secs120 => Self::Secs120,
        }
    }
}

impl From<store::uploads::KeepHistory> for KeepHistoryDto {
    fn from(value: store::uploads::KeepHistory) -> Self {
        match value {
            store::uploads::KeepHistory::Forever => Self::Forever,
            store::uploads::KeepHistory::OneYear => Self::OneYear,
            store::uploads::KeepHistory::Days90 => Self::Days90,
            store::uploads::KeepHistory::Days30 => Self::Days30,
        }
    }
}

impl From<store::uploads::Proxy> for ProxyDto {
    fn from(value: store::uploads::Proxy) -> Self {
        match value {
            store::uploads::Proxy::System => Self::System,
            store::uploads::Proxy::None => Self::None,
            store::uploads::Proxy::Custom => Self::Custom,
        }
    }
}

impl From<&store::Workflows> for WorkflowsDto {
    fn from(value: &store::Workflows) -> Self {
        Self {
            list: value.list.iter().map(WorkflowDto::from).collect(),
        }
    }
}

impl From<&store::workflows::Workflow> for WorkflowDto {
    fn from(_: &store::workflows::Workflow) -> Self {
        Self {}
    }
}

impl From<&store::Window> for WindowDto {
    fn from(value: &store::Window) -> Self {
        Self {
            main: MainWindowDto {
                x: value.main.x,
                y: value.main.y,
                width: value.main.width,
                height: value.main.height,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_dto_mirrors_the_store_defaults() {
        let dto = SettingsDto::from(&store::Settings::default());
        assert_eq!(dto.version, 2);
        assert!(dto.general.launch_at_signin);
        assert_eq!(dto.general.image_format, ImageFormatDto::Png);
        assert_eq!(dto.capture.magnifier_zoom, MagnifierZoomDto::X4);
        assert_eq!(dto.capture.ffmpeg_path, None);
        assert_eq!(dto.hotkeys.bindings.len(), 11);
        assert_eq!(dto.uploads.images, "save_locally_only");
        assert_eq!(dto.uploads.retry, RetryDto::ThreeTimes);
        assert!(dto.workflows.list.is_empty());
        assert_eq!(dto.window.main.width, None);
    }

    #[test]
    fn dto_serialises_with_camel_case_fields_and_snake_case_values() {
        let dto = SettingsDto::from(&store::Settings::default());
        let value = serde_json::to_value(&dto).expect("json");
        let general = value.get("general").expect("general");
        assert!(general.get("launchAtSignin").is_some());
        assert!(general.get("launch_at_signin").is_none());
        assert_eq!(
            general.get("imageFormat").and_then(|value| value.as_str()),
            Some("png")
        );
    }
}
