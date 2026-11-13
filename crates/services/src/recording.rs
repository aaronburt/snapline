use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_core::ffmpeg_args::{
    AudioPipe, FfmpegCommand, GifFrameRate, GifMaxWidth, GifSettings, VideoEncoder, VideoFormat,
    VideoFrameRate, VideoQuality,
};
use snapline_core::geometry::Rect;
use snapline_core::ids::WindowId;
use snapline_core::image::Image;
use snapline_core::ports::{MagnifierZoom, OverlayTheme};
use snapline_core::record_place::OutputBounds;
use snapline_core::recording::{RecordingState, RecordingTarget};
use snapline_core::workflow::RecordKind;
use snapline_store::io::Paths;
use snapline_store::settings::capture::{
    Capture, FrameRate, GifFrameRate as StoreGifFrameRate, GifMaxWidth as StoreGifMaxWidth,
    VideoFormat as StoreVideoFormat, VideoQuality as StoreVideoQuality,
};
use snapline_store::settings::Settings;
use thiserror::Error;

use crate::dto::record::{RecordKindDto, RecordReq, RecordResp, RecordStateDto, RecordStatusDto};
use crate::{AppEvent, ServiceError, Services};

mod notices;
mod session;
mod storage;

pub use notices::{
    recording_saved_notice, recording_upload_failed_notice, was_reported, RECORDING_SAVED_TITLE,
};
pub use storage::{store_recording, ServicesRecordingStore, WorkerRecordingStore};

use session::{convert_with, Session, SessionControl, SessionMode, SessionPlan, SessionSettings};

pub const TICK: Duration = Duration::from_secs(1);
pub const TEMP_FOLDER: &str = "snapline";
pub const DEFAULT_MICROPHONE: &str = "Default microphone";
pub const RECORDING_TITLE: &str = "Recording";

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PORTS: OnceLock<Mutex<Option<RecordingPorts>>> = OnceLock::new();
static CURRENT: OnceLock<Mutex<Option<SessionControl>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
    State(RecordStatusDto),
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordedKind {
    Video,
    Gif,
}

impl RecordedKind {
    const fn dto(self) -> RecordKindDto {
        match self {
            Self::Video => RecordKindDto::Video,
            Self::Gif => RecordKindDto::Gif,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordStatus {
    pub state: RecordingState,
    pub kind: Option<RecordedKind>,
    pub elapsed_ms: u64,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub capture_id: Option<i64>,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}

impl Default for RecordStatus {
    fn default() -> Self {
        Self {
            state: RecordingState::Idle,
            kind: None,
            elapsed_ms: 0,
            width: 0,
            height: 0,
            duration_ms: 0,
            capture_id: None,
            path: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedWindow {
    pub handle: isize,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedTarget {
    pub rect: Rect,
    pub monitor: u32,
    pub window: Option<PickedWindow>,
}

impl PickedTarget {
    pub fn recording_target(&self) -> RecordingTarget {
        self.recording_target_at(self.rect)
    }

    pub fn recording_target_at(&self, rect: Rect) -> RecordingTarget {
        match &self.window {
            Some(window) => RecordingTarget::window(
                WindowId::new(u64::try_from(window.handle).unwrap_or(0)),
                rect,
            ),
            None => RecordingTarget::region(rect),
        }
    }

    pub fn title(&self) -> String {
        match &self.window {
            Some(window) if !window.title.trim().is_empty() => window.title.clone(),
            _ => RECORDING_TITLE.to_owned(),
        }
    }

    fn width(&self) -> u32 {
        u32::try_from(self.rect.width.get()).unwrap_or(0)
    }

    fn height(&self) -> u32 {
        u32::try_from(self.rect.height.get()).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickOptions {
    pub magnifier: bool,
    pub magnifier_zoom: MagnifierZoom,
    pub full_screen_crosshair: bool,
    pub theme: OverlayTheme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorderRequest {
    pub rect: Rect,
    pub monitor: u32,
    pub theme: OverlayTheme,
}

#[derive(Clone)]
pub struct ControlsRequest {
    pub rect: Rect,
    pub monitor: u32,
    pub theme: OverlayTheme,
    pub stop: Arc<dyn Fn() + Send + Sync>,
    pub cancel: Arc<dyn Fn() + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOptions {
    pub system: bool,
    pub microphone: bool,
    pub microphone_device: String,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            system: false,
            microphone: false,
            microphone_device: DEFAULT_MICROPHONE.to_owned(),
        }
    }
}

impl AudioOptions {
    pub fn enabled(&self) -> bool {
        self.system || self.microphone
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfmpegProgress {
    pub out_time_us: Option<u64>,
    pub frame: Option<u64>,
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegExit {
    pub success: bool,
    pub timed_out: bool,
    pub message: String,
}

impl FfmpegExit {
    pub fn success() -> Self {
        Self {
            success: true,
            timed_out: false,
            message: String::new(),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            timed_out: false,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedFile {
    pub media: PathBuf,
    pub first_frame: Option<Image>,
    pub kind: RecordedKind,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub source_app: Option<String>,
    pub window_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedRecording {
    pub capture_id: Option<i64>,
    pub path: PathBuf,
    pub file_name: String,
    pub bytes: i64,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub kind: RecordedKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOutcome {
    details: snapline_core::workflow::RecordingDetails,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecordError {
    #[error("There is no recording area to record.")]
    NoTarget,
    #[error("The recording was cancelled.")]
    Cancelled,
    #[error("Another recording is already in progress.")]
    Busy,
    #[error("The recording border could not be shown. {0}")]
    Border(String),
    #[error("Audio capture could not start. {0}")]
    Audio(String),
    #[error("FFmpeg could not start. {0}")]
    FfmpegStart(String),
    #[error("FFmpeg could not be stopped. {0}")]
    Stop(String),
    #[error("{0}")]
    Ffmpeg(String),
    #[error("{0}")]
    FfmpegStopped(String),
    #[error("The GIF could not be made. {0}")]
    Convert(String),
    #[error("The recording could not be saved. {0}")]
    Save(String),
    #[error("{0}")]
    Invalid(String),
    #[error("Recording is not available yet.")]
    NotYet,
}

impl From<RecordError> for ServiceError {
    fn from(error: RecordError) -> Self {
        let message = error.to_string();
        match error {
            RecordError::NotYet => Self::NotYet,
            RecordError::Busy => Self::Busy,
            RecordError::NoTarget
            | RecordError::Cancelled
            | RecordError::Border(_)
            | RecordError::Audio(_)
            | RecordError::FfmpegStart(_)
            | RecordError::Stop(_)
            | RecordError::Ffmpeg(_)
            | RecordError::FfmpegStopped(_)
            | RecordError::Convert(_)
            | RecordError::Invalid(_) => Self::Invalid(message),
            RecordError::Save(_) => Self::Io(message),
        }
    }
}

pub trait TargetPicker: Send + Sync {
    fn pick(&self, options: PickOptions) -> Result<PickedTarget, RecordError>;
    fn active_window(&self) -> Result<PickedTarget, RecordError>;
}

pub trait BorderHandle: Send {
    fn close(&mut self);
}

pub trait BorderHost: Send + Sync {
    fn show(&self, request: &BorderRequest) -> Result<Box<dyn BorderHandle>, RecordError>;
}

pub trait ControlsHandle: Send {
    fn close(&mut self);
}

pub trait ControlsHost: Send + Sync {
    fn show(&self, request: &ControlsRequest) -> Result<Box<dyn ControlsHandle>, RecordError>;
}

pub trait FfmpegProcess: Send {
    fn progress(&mut self) -> Option<FfmpegProgress>;
    fn is_running(&mut self) -> bool;
    fn stop(&mut self) -> Result<FfmpegExit, RecordError>;
    fn kill(&mut self) -> Result<FfmpegExit, RecordError>;
    fn wait(&mut self) -> Result<FfmpegExit, RecordError>;
}

pub trait FfmpegRunner: Send + Sync {
    fn start(&self, command: FfmpegCommand) -> Result<Box<dyn FfmpegProcess>, RecordError>;
    fn best_encoder(&self, ffmpeg: &Path, format: VideoFormat) -> VideoEncoder;

    fn outputs(&self) -> Vec<OutputBounds> {
        Vec::new()
    }
}

pub trait AudioSession: Send {
    fn pipes(&self) -> Vec<AudioPipe>;
    fn stop(&mut self);
}

pub trait AudioPort: Send + Sync {
    fn start(&self, options: &AudioOptions) -> Result<Box<dyn AudioSession>, RecordError>;
}

pub trait MediaPort: Send + Sync {
    fn first_frame(&self, video: &Path, ffmpeg: &Path) -> Result<Image, RecordError>;
}

pub trait RecordingStore: Send + Sync {
    fn save(&self, file: RecordedFile) -> Result<SavedRecording, RecordError>;
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: AppEvent);
}

pub trait RecordingPublisher: Send + Sync {
    fn published(&self, saved: &SavedRecording);
    fn cancelled(&self);
    fn failed(&self, _message: &str) {}
}

pub struct RunnerPublisher;

impl RecordingPublisher for RunnerPublisher {
    fn published(&self, saved: &SavedRecording) {
        match crate::workflows::installed_runner() {
            Ok(runner) => {
                if let Err(error) = runner.publish_recording(saved) {
                    tracing::warn!(%error, "the finished recording could not be published");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the finished recording could not be published");
            }
        }
    }

    fn cancelled(&self) {
        if let Ok(runner) = crate::workflows::installed_runner() {
            runner.recording_cancelled();
        }
    }

    fn failed(&self, message: &str) {
        if let Ok(runner) = crate::workflows::installed_runner() {
            runner.recording_failed(message);
        }
    }
}

#[derive(Clone)]
pub struct RecordingPorts {
    pub picker: Arc<dyn TargetPicker>,
    pub border: Arc<dyn BorderHost>,
    pub controls: Arc<dyn ControlsHost>,
    pub ffmpeg: Arc<dyn FfmpegRunner>,
    pub audio: Arc<dyn AudioPort>,
    pub media: Arc<dyn MediaPort>,
    pub store: Arc<dyn RecordingStore>,
    pub events: Arc<dyn EventSink>,
    pub publisher: Arc<dyn RecordingPublisher>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickMode {
    Region,
    ActiveWindow,
}

pub fn status_dto(status: &RecordStatus) -> RecordStatusDto {
    RecordStatusDto {
        state: match status.state {
            RecordingState::Idle => RecordStateDto::Idle,
            RecordingState::Arming => RecordStateDto::Arming,
            RecordingState::Recording => RecordStateDto::Recording,
            RecordingState::Stopping => RecordStateDto::Stopping,
            RecordingState::Encoding => RecordStateDto::Encoding,
            RecordingState::Done => RecordStateDto::Done,
            RecordingState::Failed => RecordStateDto::Failed,
        },
        kind: status.kind.map(RecordedKind::dto),
        elapsed_ms: status.elapsed_ms,
        width: (status.width > 0).then_some(status.width),
        height: (status.height > 0).then_some(status.height),
        duration_ms: (status.duration_ms > 0)
            .then(|| i64::try_from(status.duration_ms).unwrap_or(i64::MAX)),
        capture_id: status.capture_id,
        path: status
            .path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        error: status.error.clone(),
    }
}

fn remove_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

pub fn temp_folder() -> PathBuf {
    std::env::temp_dir().join(TEMP_FOLDER)
}

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

fn ports_slot() -> MutexGuard<'static, Option<RecordingPorts>> {
    PORTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn current_slot() -> MutexGuard<'static, Option<SessionControl>> {
    CURRENT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub fn install_ports(ports: RecordingPorts) -> Result<(), ServiceError> {
    let mut slot = ports_slot();
    if slot.is_some() {
        return Err(ServiceError::Internal(
            "The recorder is already installed.".to_owned(),
        ));
    }
    *slot = Some(ports);
    Ok(())
}

fn installed_ports() -> Result<RecordingPorts, RecordError> {
    ports_slot().clone().ok_or(RecordError::NotYet)
}

pub fn current_status() -> RecordStatus {
    match current_slot().as_ref() {
        Some(control) => control.status(),
        None => RecordStatus::default(),
    }
}

fn current_control() -> Option<SessionControl> {
    current_slot().clone()
}

fn install_control(control: SessionControl) {
    *current_slot() = Some(control);
}

pub fn cancel_recording() -> RecordStatus {
    match current_control() {
        Some(control) => {
            if control.is_active() {
                control.cancel();
            }
            control.status()
        }
        None => RecordStatus::default(),
    }
}

pub fn stop_recording() -> RecordStatus {
    match current_control() {
        Some(control) => {
            if control.is_active() {
                control.stop();
            }
            control.status()
        }
        None => RecordStatus::default(),
    }
}

pub fn start_recording(
    services: &Services,
    kind: RecordKindDto,
) -> Result<RecordStatus, RecordError> {
    if let Some(control) = current_control() {
        if control.is_active() {
            control.stop();
            return Ok(control.status());
        }
    }
    let Some(session) = begin(services, kind, SessionMode::Save)? else {
        return Ok(current_status());
    };
    let control = session.control();
    install_control(control.clone());
    let status = control.status();
    drop(session);
    Ok(status)
}

pub fn start_recording_region(
    services: &Services,
    kind: RecordKindDto,
    rect: Rect,
    monitor: u32,
) -> Result<RecordStatus, RecordError> {
    if let Some(control) = current_control() {
        if control.is_active() {
            control.stop();
            return Ok(control.status());
        }
    }
    let target = PickedTarget {
        rect,
        monitor,
        window: None,
    };
    let Some(session) = begin_with_target(
        services,
        installed_ports()?,
        kind,
        SessionMode::Save,
        Some(target),
    )?
    else {
        return Ok(current_status());
    };
    let control = session.control();
    install_control(control.clone());
    let status = control.status();
    drop(session);
    Ok(status)
}

pub fn record_blocking(
    services: &Services,
    kind: RecordKind,
) -> Result<snapline_core::workflow::Recorded, RecordError> {
    if current_control().is_some_and(|control| control.is_active()) {
        return Err(RecordError::Busy);
    }
    let kind = match kind {
        RecordKind::Video => RecordKindDto::Video,
        RecordKind::Gif => RecordKindDto::Gif,
        RecordKind::Window => RecordKindDto::Window,
    };
    let Some(session) = begin(services, kind, SessionMode::Temporary)? else {
        return Err(RecordError::Cancelled);
    };
    install_control(session.control());
    let outcome = session.wait()?;
    Ok(snapline_core::workflow::Recorded {
        path: outcome.path,
        details: Some(outcome.details),
    })
}

fn begin(
    services: &Services,
    kind: RecordKindDto,
    mode: SessionMode,
) -> Result<Option<Session>, RecordError> {
    begin_with(services, installed_ports()?, kind, mode)
}

fn begin_with(
    services: &Services,
    ports: RecordingPorts,
    kind: RecordKindDto,
    mode: SessionMode,
) -> Result<Option<Session>, RecordError> {
    begin_with_target(services, ports, kind, mode, None)
}

fn begin_with_target(
    services: &Services,
    ports: RecordingPorts,
    kind: RecordKindDto,
    mode: SessionMode,
    preset: Option<PickedTarget>,
) -> Result<Option<Session>, RecordError> {
    let settings = services
        .settings_snapshot()
        .map_err(|error| RecordError::Invalid(error.to_string()))?;
    let paths = services
        .paths()
        .map_err(|error| RecordError::Invalid(error.to_string()))?;
    let mut session_settings = session_settings(&settings, &paths);
    let pick_mode = match kind {
        RecordKindDto::Window => PickMode::ActiveWindow,
        RecordKindDto::Video | RecordKindDto::Gif => PickMode::Region,
    };
    let recorded = match kind {
        RecordKindDto::Gif => RecordedKind::Gif,
        RecordKindDto::Video | RecordKindDto::Window => RecordedKind::Video,
    };
    if mode == SessionMode::Temporary && recorded == RecordedKind::Video {
        session_settings.quality = VideoQuality::Lossless;
        session_settings.format = VideoFormat::Mp4H264;
    }
    let options = pick_options(&settings.capture);
    let target = match (preset, pick_mode) {
        (Some(target), _) => Ok(target),
        (None, PickMode::Region) => ports.picker.pick(options),
        (None, PickMode::ActiveWindow) => ports.picker.active_window(),
    };
    let target = match target {
        Ok(target) => target,
        Err(RecordError::Cancelled) => return Ok(None),
        Err(error) => return Err(error),
    };
    if target.rect.is_empty() {
        return Err(RecordError::NoTarget);
    }
    let folder = temp_folder();
    std::fs::create_dir_all(&folder).map_err(|error| RecordError::Save(error.to_string()))?;
    let id = next_id();
    let extension = match recorded {
        RecordedKind::Video => session_settings.format.extension(),
        RecordedKind::Gif => "mp4",
    };
    let output = folder.join(format!("record-{id}.{extension}"));
    let converted =
        (recorded == RecordedKind::Gif).then(|| folder.join(format!("record-{id}.gif")));
    Session::start(
        SessionPlan {
            kind: recorded,
            mode,
            target,
            output,
            converted,
            settings: session_settings,
        },
        ports,
    )
    .map(Some)
}

fn pick_options(capture: &Capture) -> PickOptions {
    PickOptions {
        magnifier: capture.magnifier,
        magnifier_zoom: magnifier_zoom(capture.magnifier_zoom),
        full_screen_crosshair: capture.fullscreen_crosshair,
        theme: OverlayTheme::Dark,
    }
}

fn magnifier_zoom(zoom: snapline_store::settings::capture::MagnifierZoom) -> MagnifierZoom {
    use snapline_store::settings::capture::MagnifierZoom as StoreZoom;
    match zoom {
        StoreZoom::X2 => MagnifierZoom::X2,
        StoreZoom::X4 => MagnifierZoom::X4,
        StoreZoom::X8 => MagnifierZoom::X8,
    }
}

fn session_settings(settings: &Settings, paths: &Paths) -> SessionSettings {
    SessionSettings {
        ffmpeg: settings.capture.resolved_ffmpeg_path(&paths.exe_dir),
        frame_rate: video_frame_rate(settings.capture.frame_rate),
        quality: video_quality(settings.capture.video_quality),
        format: video_format(settings.capture.video_format),
        gif: gif_settings(settings),
        border: settings.capture.recording_border,
        audio: AudioOptions {
            system: settings.capture.record_system_audio,
            microphone: settings.capture.record_microphone,
            microphone_device: settings.capture.microphone_device.clone(),
        },
        theme: OverlayTheme::Dark,
    }
}

fn video_frame_rate(value: FrameRate) -> VideoFrameRate {
    match value {
        FrameRate::Fps24 => VideoFrameRate::Fps24,
        FrameRate::Fps30 => VideoFrameRate::Fps30,
        FrameRate::Fps60 => VideoFrameRate::Fps60,
    }
}

fn video_quality(value: StoreVideoQuality) -> VideoQuality {
    match value {
        StoreVideoQuality::High => VideoQuality::High,
        StoreVideoQuality::Balanced => VideoQuality::Balanced,
        StoreVideoQuality::SmallFile => VideoQuality::SmallFile,
        StoreVideoQuality::Lossless => VideoQuality::Lossless,
    }
}

fn video_format(value: StoreVideoFormat) -> VideoFormat {
    match value {
        StoreVideoFormat::Mp4H264 => VideoFormat::Mp4H264,
        StoreVideoFormat::Mp4H265Hevc => VideoFormat::Mp4H265,
        StoreVideoFormat::WebmVp9 => VideoFormat::WebmVp9,
        StoreVideoFormat::MkvH264 => VideoFormat::MkvH264,
    }
}

fn gif_settings(settings: &Settings) -> GifSettings {
    GifSettings {
        frame_rate: match settings.capture.gif_frame_rate {
            StoreGifFrameRate::Fps10 => GifFrameRate::Fps10,
            StoreGifFrameRate::Fps15 => GifFrameRate::Fps15,
            StoreGifFrameRate::Fps20 => GifFrameRate::Fps20,
            StoreGifFrameRate::Fps30 => GifFrameRate::Fps30,
        },
        max_width: match settings.capture.gif_max_width {
            StoreGifMaxWidth::Original => GifMaxWidth::Original,
            StoreGifMaxWidth::Px1280 => GifMaxWidth::Px1280,
            StoreGifMaxWidth::Px800 => GifMaxWidth::Px800,
            StoreGifMaxWidth::Px480 => GifMaxWidth::Px480,
        },
        optimise_palette: settings.capture.gif_optimise_palette,
    }
}

pub fn convert_to_gif(services: &Services, input: &Path) -> Result<PathBuf, RecordError> {
    let ports = installed_ports()?;
    let settings = services
        .settings_snapshot()
        .map_err(|error| RecordError::Invalid(error.to_string()))?;
    let paths = services
        .paths()
        .map_err(|error| RecordError::Invalid(error.to_string()))?;
    let ffmpeg = settings.capture.resolved_ffmpeg_path(&paths.exe_dir);
    let folder = temp_folder();
    std::fs::create_dir_all(&folder).map_err(|error| RecordError::Convert(error.to_string()))?;
    let output = folder.join(format!("convert-{}.gif", next_id()));
    convert_with(
        ports.ffmpeg.as_ref(),
        &ffmpeg,
        input,
        &output,
        gif_settings(&settings),
    )
}

pub struct RecordService {
    services: Services,
}

impl RecordService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }

    pub fn handle(&self, req: RecordReq) -> Result<RecordResp, ServiceError> {
        match req {
            RecordReq::Ping => Ok(RecordResp::Pong),
            RecordReq::State => Ok(RecordResp::Status {
                status: status_dto(&current_status()),
            }),
            RecordReq::Start { kind } => {
                let status = start_recording(&self.services, kind)?;
                Ok(RecordResp::Status {
                    status: status_dto(&status),
                })
            }
            RecordReq::Stop => Ok(RecordResp::Status {
                status: status_dto(&stop_recording()),
            }),
        }
    }
}

pub fn run(action: &ActionId, services: &Services) -> Result<(), ServiceError> {
    let kind = match action.as_str() {
        "record_video" => Some(RecordKindDto::Video),
        "record_gif" => Some(RecordKindDto::Gif),
        "record_window" => Some(RecordKindDto::Window),
        "record_stop" => None,
        "record_cancel" => {
            cancel_recording();
            return Ok(());
        }
        _ => return Err(ServiceError::NotYet),
    };
    match kind {
        Some(kind) => {
            start_recording(services, kind)?;
            Ok(())
        }
        None => {
            stop_recording();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
