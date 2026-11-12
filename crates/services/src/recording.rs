use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, RecvTimeoutError, Sender};
use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_core::ffmpeg_args::{
    self, AudioPipe, FfmpegCommand, GifCaptureRequest, GifConvertRequest, GifFrameRate,
    GifMaxWidth, GifSettings, VideoEncoder, VideoFormat, VideoFrameRate, VideoQuality,
    VideoRequest,
};
use snapline_core::geometry::Rect;
use snapline_core::ids::WindowId;
use snapline_core::image::{Image, ImageFormat};
use snapline_core::naming::{self, NameContext};
use snapline_core::ports::{MagnifierZoom, OverlayTheme};
use snapline_core::record_place::{self, OutputBounds};
use snapline_core::recording::{RecordingEvent, RecordingMachine, RecordingState, RecordingTarget};
use snapline_core::workflow::RecordKind;
use snapline_store::history::{CaptureKind, HistoryRepo, NewCapture};
use snapline_store::io::{self, Paths};
use snapline_store::settings::capture::{
    Capture, FrameRate, GifFrameRate as StoreGifFrameRate, GifMaxWidth as StoreGifMaxWidth,
    VideoFormat as StoreVideoFormat, VideoQuality as StoreVideoQuality,
};
use snapline_store::settings::general::{General, Subfolders};
use snapline_store::settings::Settings;
use thiserror::Error;
use time::OffsetDateTime;
use ts_rs::TS;

use crate::dto::record::{RecordKindDto, RecordReq, RecordResp, RecordStateDto, RecordStatusDto};
use crate::saver::{self, THUMBNAIL_WIDTH};
use crate::{AppEvent, ServiceError, Services};

pub const TICK: Duration = Duration::from_secs(1);
pub const TEMP_FOLDER: &str = "snapline";
pub const DEFAULT_MICROPHONE: &str = "Default microphone";
pub const RECORDING_TITLE: &str = "Recording";

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PORTS: OnceLock<Mutex<Option<RecordingPorts>>> = OnceLock::new();
static CURRENT: OnceLock<Mutex<Option<SessionControl>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "RecordAreaEvent")]
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

pub const RECORDING_SAVED_TITLE: &str = "Recording saved";

pub fn recording_saved_notice(
    saved: &SavedRecording,
    thumbnail: Option<PathBuf>,
) -> snapline_core::ports::RichNotice {
    use crate::saver::{capture_tag, file_url};
    use snapline_core::ports::{NoticeButton, RichNotice};

    let video = file_url(&saved.path);
    let mut buttons = vec![NoticeButton::open("Open video", video.clone())];
    if let Some(folder) = saved.path.parent() {
        buttons.push(NoticeButton::open("Open folder", file_url(folder)));
    }
    RichNotice {
        title: RECORDING_SAVED_TITLE.to_owned(),
        lines: vec![
            saved.file_name.clone(),
            format!(
                "{}, {}",
                format_bytes(saved.bytes),
                format_duration(saved.duration_ms)
            ),
        ],
        image: thumbnail,
        open: Some(video),
        buttons,
        tag: saved.capture_id.map(capture_tag),
    }
}

pub fn recording_upload_failed_notice(
    saved: &SavedRecording,
    reason: &str,
    thumbnail: Option<PathBuf>,
) -> snapline_core::ports::RichNotice {
    let mut notice = recording_saved_notice(saved, thumbnail);
    notice.title = crate::uploads::UPLOAD_FAILED_TITLE.to_owned();
    notice.lines = vec![saved.file_name.clone(), reason.to_owned()];
    notice
}

fn format_bytes(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes >= 1_048_576.0 {
        format!("{:.1} MB", bytes / 1_048_576.0)
    } else {
        format!("{:.0} KB", (bytes / 1024.0).max(1.0))
    }
}

fn format_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
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

#[derive(Clone)]
pub struct ServicesRecordingStore {
    services: Services,
}

impl ServicesRecordingStore {
    pub fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }
}

impl RecordingStore for ServicesRecordingStore {
    fn save(&self, file: RecordedFile) -> Result<SavedRecording, RecordError> {
        store_recording(&self.services, file)
    }
}

#[derive(Clone)]
pub struct WorkerRecordingStore {
    worker: crate::worker::Handle,
}

impl WorkerRecordingStore {
    pub fn new(worker: crate::worker::Handle) -> Self {
        Self { worker }
    }
}

impl RecordingStore for WorkerRecordingStore {
    fn save(&self, file: RecordedFile) -> Result<SavedRecording, RecordError> {
        self.worker
            .save_recording(file)
            .map_err(|error| RecordError::Save(error.to_string()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionMode {
    Save,
    Temporary,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickMode {
    Region,
    ActiveWindow,
}

#[derive(Debug, Clone)]
struct SessionSettings {
    ffmpeg: PathBuf,
    frame_rate: VideoFrameRate,
    quality: VideoQuality,
    format: VideoFormat,
    gif: GifSettings,
    border: bool,
    audio: AudioOptions,
    theme: OverlayTheme,
}

struct SessionPlan {
    kind: RecordedKind,
    mode: SessionMode,
    target: PickedTarget,
    output: PathBuf,
    converted: Option<PathBuf>,
    settings: SessionSettings,
}

enum Command {
    Stop,
    Cancel,
}

enum Ended {
    Stopped(u64),
    Cancelled,
}

#[derive(Clone)]
struct SessionControl {
    stop: Sender<Command>,
    status: Arc<Mutex<RecordStatus>>,
}

impl SessionControl {
    fn stop(&self) {
        let _ = self.stop.try_send(Command::Stop);
    }

    fn cancel(&self) {
        let _ = self.stop.try_send(Command::Cancel);
    }

    fn status(&self) -> RecordStatus {
        status_lock(&self.status).clone()
    }

    fn is_active(&self) -> bool {
        self.status().state.is_active()
    }
}

struct Session {
    control: SessionControl,
    results: Receiver<Result<SessionOutcome, RecordError>>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    fn start(plan: SessionPlan, ports: RecordingPorts) -> Result<Self, RecordError> {
        if plan.target.rect.is_empty() {
            return Err(RecordError::NoTarget);
        }
        let status = Arc::new(Mutex::new(RecordStatus {
            state: RecordingState::Idle,
            kind: Some(plan.kind),
            elapsed_ms: 0,
            width: plan.target.width(),
            height: plan.target.height(),
            duration_ms: 0,
            capture_id: None,
            path: None,
            error: None,
        }));
        let mut machine = RecordingMachine::new();
        machine
            .apply(RecordingEvent::Start {
                target: plan.target.recording_target(),
                output: plan.output.clone(),
            })
            .map_err(|error| RecordError::Invalid(error.to_string()))?;
        publish(&status, &machine, ports.events.as_ref());
        let (commands, inbox) = unbounded();
        let (results, receiver) = bounded(1);
        let thread_status = Arc::clone(&status);
        let thread_ports = ports.clone();
        let stop_commands = commands.clone();
        let stop: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = stop_commands.try_send(Command::Stop);
        });
        let cancel_commands = commands.clone();
        let cancel: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = cancel_commands.try_send(Command::Cancel);
        });
        let thread = thread::Builder::new()
            .name("snapline-recorder".to_owned())
            .spawn(move || {
                let outcome = run_session(
                    &plan,
                    &thread_ports,
                    &mut machine,
                    &thread_status,
                    &inbox,
                    stop,
                    cancel,
                );
                if let Err(error) = &outcome {
                    remove_file(&plan.output);
                    if let Some(converted) = &plan.converted {
                        remove_file(converted);
                    }
                    if matches!(error, RecordError::Cancelled) {
                        thread_ports.publisher.cancelled();
                    } else {
                        thread_ports.publisher.failed(&error.to_string());
                    }
                    if !matches!(error, RecordError::Cancelled) && !machine.state().is_terminal() {
                        let _ = machine.apply(RecordingEvent::Failed(
                            outcome
                                .as_ref()
                                .err()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                        ));
                        publish(&thread_status, &machine, thread_ports.events.as_ref());
                    }
                }
                let _ = results.send(outcome);
            })
            .map_err(|error| RecordError::Invalid(error.to_string()))?;
        Ok(Self {
            control: SessionControl {
                stop: commands,
                status,
            },
            results: receiver,
            thread: Some(thread),
        })
    }

    fn control(&self) -> SessionControl {
        self.control.clone()
    }

    fn wait(mut self) -> Result<SessionOutcome, RecordError> {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        match self.results.recv() {
            Ok(result) => result,
            Err(_) => Err(RecordError::Invalid(
                "The recorder stopped before it finished.".to_owned(),
            )),
        }
    }
}

#[derive(Default)]
struct SessionResources {
    border: Option<Box<dyn BorderHandle>>,
    controls: Option<Box<dyn ControlsHandle>>,
    audio: Option<Box<dyn AudioSession>>,
    ffmpeg: Option<Box<dyn FfmpegProcess>>,
}

impl SessionResources {
    fn close_border(&mut self) {
        if let Some(mut border) = self.border.take() {
            border.close();
        }
    }

    fn close_controls(&mut self) {
        if let Some(mut controls) = self.controls.take() {
            controls.close();
        }
    }

    fn stop_audio(&mut self) {
        if let Some(mut audio) = self.audio.take() {
            audio.stop();
        }
    }

    fn stop_ffmpeg(&mut self) -> Result<FfmpegExit, RecordError> {
        match self.ffmpeg.take() {
            Some(mut ffmpeg) => ffmpeg
                .stop()
                .map_err(|error| RecordError::Stop(error.to_string())),
            None => Ok(FfmpegExit::success()),
        }
    }
}

impl Drop for SessionResources {
    fn drop(&mut self) {
        self.close_controls();
        self.close_border();
        self.stop_audio();
        if let Some(mut ffmpeg) = self.ffmpeg.take() {
            let _ = ffmpeg.kill();
        }
    }
}

fn run_session(
    plan: &SessionPlan,
    ports: &RecordingPorts,
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    inbox: &Receiver<Command>,
    stop: Arc<dyn Fn() + Send + Sync>,
    cancel: Arc<dyn Fn() + Send + Sync>,
) -> Result<SessionOutcome, RecordError> {
    let mut resources = SessionResources::default();
    let outputs = ports.ffmpeg.outputs();
    let (output_index, target) = if outputs.is_empty() {
        (0, plan.target.recording_target())
    } else {
        let placed = record_place::place(plan.target.rect, &outputs)
            .map_err(|error| RecordError::Invalid(error.to_string()))?;
        (
            placed.output_index,
            plan.target.recording_target_at(placed.rect),
        )
    };
    if plan.settings.border {
        let border = ports
            .border
            .show(&BorderRequest {
                rect: plan.target.rect,
                monitor: plan.target.monitor,
                theme: plan.settings.theme,
            })
            .map_err(|error| RecordError::Border(error.to_string()))?;
        resources.border = Some(border);
    }
    match ports.controls.show(&ControlsRequest {
        rect: plan.target.rect,
        monitor: plan.target.monitor,
        theme: plan.settings.theme,
        stop,
        cancel,
    }) {
        Ok(controls) => resources.controls = Some(controls),
        Err(error) => tracing::warn!(%error, "could not show the recording controls"),
    }
    let pipes = if plan.settings.audio.enabled() {
        let session = ports
            .audio
            .start(&plan.settings.audio)
            .map_err(|error| RecordError::Audio(error.to_string()))?;
        let pipes = session.pipes();
        resources.audio = Some(session);
        pipes
    } else {
        Vec::new()
    };
    let command = match plan.kind {
        RecordedKind::Video => ffmpeg_args::video_command(&VideoRequest {
            ffmpeg: plan.settings.ffmpeg.clone(),
            target,
            output: plan.output.clone(),
            output_index,
            format: plan.settings.format,
            encoder: ports
                .ffmpeg
                .best_encoder(&plan.settings.ffmpeg, plan.settings.format),
            frame_rate: plan.settings.frame_rate,
            quality: plan.settings.quality,
            audio: pipes,
        })
        .map_err(|error| RecordError::Invalid(error.to_string()))?,
        RecordedKind::Gif => ffmpeg_args::gif_capture_command(&GifCaptureRequest {
            ffmpeg: plan.settings.ffmpeg.clone(),
            target,
            output: plan.output.clone(),
            output_index,
            frame_rate: plan.settings.gif.frame_rate,
        })
        .map_err(|error| RecordError::Invalid(error.to_string()))?,
    };
    let process = ports
        .ffmpeg
        .start(command)
        .map_err(|error| RecordError::FfmpegStart(error.to_string()))?;
    resources.ffmpeg = Some(process);
    transition(
        machine,
        status,
        ports.events.as_ref(),
        RecordingEvent::Started,
    )?;

    let duration_ms = match record_until_stop(
        machine,
        status,
        ports.events.as_ref(),
        &mut resources,
        inbox,
    )? {
        Ended::Stopped(duration_ms) => duration_ms,
        Ended::Cancelled => return discard(plan, ports, machine, status, &mut resources),
    };

    transition(machine, status, ports.events.as_ref(), RecordingEvent::Stop)?;
    resources.close_controls();
    resources.close_border();
    resources.stop_audio();
    let exit = resources.stop_ffmpeg()?;
    transition(
        machine,
        status,
        ports.events.as_ref(),
        RecordingEvent::Stopped,
    )?;
    if !exit.success {
        return Err(RecordError::Ffmpeg(exit.message));
    }
    if cancel_requested(inbox) {
        return discard(plan, ports, machine, status, &mut resources);
    }
    update_status(status, |current| {
        current.duration_ms = duration_ms;
    });

    let media = if plan.kind == RecordedKind::Gif {
        convert_capture(plan, ports)?
    } else {
        plan.output.clone()
    };
    if cancel_requested(inbox) {
        return discard(plan, ports, machine, status, &mut resources);
    }

    let details = snapline_core::workflow::RecordingDetails {
        width: plan.target.width(),
        height: plan.target.height(),
        duration_ms,
    };
    match plan.mode {
        SessionMode::Temporary => {
            update_status(status, |current| {
                current.path = Some(media.clone());
            });
            transition(
                machine,
                status,
                ports.events.as_ref(),
                RecordingEvent::Encoded,
            )?;
            Ok(SessionOutcome {
                path: media,
                details,
            })
        }
        SessionMode::Save => {
            let first_frame = ports
                .media
                .first_frame(&media, &plan.settings.ffmpeg)
                .map_err(|error| {
                    tracing::warn!(%error, "the first frame of the recording is unavailable");
                    error
                })
                .ok();
            let saved = ports
                .store
                .save(RecordedFile {
                    media,
                    first_frame,
                    kind: plan.kind,
                    width: plan.target.width(),
                    height: plan.target.height(),
                    duration_ms,
                    source_app: None,
                    window_title: plan.target.title(),
                })
                .map_err(|error| RecordError::Save(error.to_string()))?;
            update_status(status, |current| {
                current.capture_id = saved.capture_id;
                current.path = Some(saved.path.clone());
                current.duration_ms = saved.duration_ms;
                current.kind = Some(saved.kind);
            });
            transition(
                machine,
                status,
                ports.events.as_ref(),
                RecordingEvent::Encoded,
            )?;
            if let Some(id) = saved.capture_id {
                ports
                    .events
                    .emit(AppEvent::History(crate::history::Event::Added { id }));
            }
            ports.publisher.published(&saved);
            Ok(SessionOutcome {
                path: saved.path,
                details,
            })
        }
    }
}

fn cancel_requested(inbox: &Receiver<Command>) -> bool {
    inbox
        .try_iter()
        .any(|command| matches!(command, Command::Cancel))
}

fn discard(
    plan: &SessionPlan,
    ports: &RecordingPorts,
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    resources: &mut SessionResources,
) -> Result<SessionOutcome, RecordError> {
    resources.close_controls();
    resources.close_border();
    resources.stop_audio();
    if let Some(mut ffmpeg) = resources.ffmpeg.take() {
        let _ = ffmpeg.kill();
    }
    remove_file(&plan.output);
    if let Some(converted) = &plan.converted {
        remove_file(converted);
    }
    machine.reset();
    update_status(status, |current| {
        current.capture_id = None;
        current.path = None;
        current.duration_ms = 0;
    });
    publish(status, machine, ports.events.as_ref());
    Err(RecordError::Cancelled)
}

fn record_until_stop(
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    events: &dyn EventSink,
    resources: &mut SessionResources,
    inbox: &Receiver<Command>,
) -> Result<Ended, RecordError> {
    let started = Instant::now();
    loop {
        let elapsed = elapsed_millis(started);
        match inbox.recv_timeout(TICK) {
            Ok(Command::Cancel) => return Ok(Ended::Cancelled),
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => {
                if machine.state() == RecordingState::Recording {
                    let _ = machine.apply(RecordingEvent::Tick(elapsed));
                }
                publish(status, machine, events);
                return Ok(Ended::Stopped(elapsed));
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(process) = resources.ffmpeg.as_mut() {
                    let _ = process.progress();
                    if !process.is_running() {
                        let detail = process
                            .wait()
                            .ok()
                            .map(|exit| ffmpeg_reason(&exit.message))
                            .unwrap_or_default();
                        return Err(RecordError::Ffmpeg(format!(
                            "FFmpeg stopped before the recording was stopped. {detail}"
                        )));
                    }
                }
                if machine.state() == RecordingState::Recording {
                    let _ = machine.apply(RecordingEvent::Tick(elapsed));
                }
                publish(status, machine, events);
            }
        }
    }
}

fn ffmpeg_reason(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(3);
    lines[start..].join(" ").trim().to_owned()
}

fn transition(
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    events: &dyn EventSink,
    event: RecordingEvent,
) -> Result<(), RecordError> {
    machine
        .apply(event)
        .map_err(|error| RecordError::Invalid(error.to_string()))?;
    publish(status, machine, events);
    Ok(())
}

fn publish(status: &Arc<Mutex<RecordStatus>>, machine: &RecordingMachine, events: &dyn EventSink) {
    let dto = {
        let mut guard = status_lock(status);
        guard.state = machine.state();
        guard.elapsed_ms = machine.elapsed_ms();
        guard.error = machine.error().map(str::to_owned);
        status_dto(&guard)
    };
    events.emit(AppEvent::Record(Event::State(dto)));
}

fn update_status(status: &Arc<Mutex<RecordStatus>>, update: impl FnOnce(&mut RecordStatus)) {
    update(&mut status_lock(status));
}

fn status_lock(status: &Mutex<RecordStatus>) -> MutexGuard<'_, RecordStatus> {
    status.lock().unwrap_or_else(PoisonError::into_inner)
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

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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

fn convert_capture(plan: &SessionPlan, ports: &RecordingPorts) -> Result<PathBuf, RecordError> {
    let output = plan
        .converted
        .as_ref()
        .ok_or_else(|| RecordError::Convert("The GIF output path is missing.".to_owned()))?;
    let media = convert_with(
        ports.ffmpeg.as_ref(),
        &plan.settings.ffmpeg,
        &plan.output,
        output,
        plan.settings.gif,
    )?;
    remove_file(&plan.output);
    Ok(media)
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

fn convert_with(
    runner: &dyn FfmpegRunner,
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    settings: GifSettings,
) -> Result<PathBuf, RecordError> {
    if !input.is_file() {
        return Err(RecordError::Convert(
            "The recording could not be found.".to_owned(),
        ));
    }
    let command = ffmpeg_args::gif_convert_command(&GifConvertRequest {
        ffmpeg: ffmpeg.to_path_buf(),
        input: input.to_path_buf(),
        output: output.to_path_buf(),
        settings,
    })
    .map_err(|error| RecordError::Invalid(error.to_string()))?;
    let mut process = runner
        .start(command)
        .map_err(|error| RecordError::Convert(error.to_string()))?;
    let exit = process
        .wait()
        .map_err(|error| RecordError::Convert(error.to_string()))?;
    if !exit.success {
        remove_file(output);
        return Err(RecordError::Convert(exit.message));
    }
    Ok(output.to_path_buf())
}

pub fn store_recording(
    services: &Services,
    file: RecordedFile,
) -> Result<SavedRecording, RecordError> {
    let settings = services
        .settings_snapshot()
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let paths = services
        .paths()
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let general = &settings.general;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let folder = capture_folder(general, &paths, now);
    std::fs::create_dir_all(&folder).map_err(|error| RecordError::Save(error.to_string()))?;
    let counter = saver::counter_from(general);
    let context = NameContext {
        app: file.source_app.clone(),
        window: file.window_title.clone(),
        timestamp: now,
        counter,
        width: file.width,
        height: file.height,
    };
    let extension = file
        .media
        .extension()
        .and_then(OsStr::to_str)
        .filter(|extension| !extension.is_empty())
        .unwrap_or("mp4");
    let stem = naming::expand(&general.name_pattern, &context, &naming::ThreadRandom);
    let name = naming::sanitise_file_name(&format!("{stem}.{extension}"));
    let file_name = naming::resolve_collision(&name, |candidate| folder.join(candidate).exists());
    let path = folder.join(&file_name);
    move_file(&file.media, &path)?;
    let bytes = std::fs::metadata(&path)
        .map_err(|error| RecordError::Save(error.to_string()))?
        .len();
    let bytes =
        i64::try_from(bytes).map_err(|_| RecordError::Save("The file is too large.".to_owned()))?;
    let created_at = now.unix_timestamp().saturating_mul(1000) + i64::from(now.millisecond());
    let kind = match file.kind {
        RecordedKind::Video => CaptureKind::Video,
        RecordedKind::Gif => CaptureKind::Gif,
    };
    let mut repo = HistoryRepo::open_write(&paths.history_file)
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let id = repo
        .record(NewCapture {
            kind,
            path: path.to_string_lossy().into_owned(),
            bytes,
            width: Some(i64::from(file.width)),
            height: Some(i64::from(file.height)),
            duration_ms: Some(i64::try_from(file.duration_ms).unwrap_or(i64::MAX)),
            source_app: file.source_app.clone(),
            created_at,
            ocr_text: None,
        })
        .map_err(|error| RecordError::Save(error.to_string()))?;
    if let Some(frame) = &file.first_frame {
        if let Err(error) = write_thumbnail(
            services,
            &paths.thumbs_dir,
            id,
            frame,
            general.lossy_quality,
        ) {
            tracing::warn!(%error, "the recording thumbnail could not be written");
        }
    }
    let next_counter = naming::next_counter(counter);
    if let Err(error) = services.update_settings(&serde_json::json!({
        "general": { "counter": i64::from(next_counter) }
    })) {
        tracing::warn!(%error, "the capture counter could not be saved");
    }
    Ok(SavedRecording {
        capture_id: Some(id),
        path,
        file_name,
        bytes,
        width: file.width,
        height: file.height,
        duration_ms: file.duration_ms,
        kind: file.kind,
    })
}

fn capture_folder(general: &General, paths: &Paths, now: OffsetDateTime) -> PathBuf {
    let profile = paths
        .captures_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""));
    let mut folder = PathBuf::from(saver::expand_profile(&general.save_folder, profile));
    match general.subfolders {
        Subfolders::ByMonth => {
            folder.push(format!("{:04}-{:02}", now.year(), now.month() as u8));
        }
        Subfolders::ByDay => folder.push(format!(
            "{:04}-{:02}-{:02}",
            now.year(),
            now.month() as u8,
            now.day()
        )),
        Subfolders::ByApp => folder.push(naming::sanitise_file_name(&naming::app_label(None))),
        Subfolders::None => {}
    }
    folder
}

fn move_file(from: &Path, to: &Path) -> Result<(), RecordError> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|error| RecordError::Save(error.to_string()))?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|error| RecordError::Save(error.to_string()))?;
    remove_file(from);
    Ok(())
}

fn write_thumbnail(
    services: &Services,
    directory: &Path,
    id: i64,
    image: &Image,
    quality: u8,
) -> Result<(), RecordError> {
    let codec = services
        .codec()
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let thumbnail = saver::scale_to_width(image, THUMBNAIL_WIDTH)
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let quality = quality.clamp(1, 100);
    let (bytes, extension) = match codec.encode(&thumbnail, ImageFormat::WebP, quality) {
        Ok(bytes) => (bytes, "webp"),
        Err(_) => (
            codec
                .encode(&thumbnail, ImageFormat::Jpeg, quality)
                .map_err(|error| RecordError::Save(error.to_string()))?,
            "jpg",
        ),
    };
    std::fs::create_dir_all(directory).map_err(|error| RecordError::Save(error.to_string()))?;
    io::write_atomic(&directory.join(format!("{id}.{extension}")), &bytes)
        .map_err(|error| RecordError::Save(error.to_string()))
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
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::Mutex;

    use snapline_core::ffmpeg_args::{AudioSource, FfmpegCommand, PcmSampleFormat};
    use snapline_core::geometry::Px;
    use snapline_core::ports::{CodecError, ImageCodec};
    use tempfile::{tempdir, TempDir};

    use super::*;

    struct TestCodec;

    impl ImageCodec for TestCodec {
        fn encode(
            &self,
            _image: &Image,
            _format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            Ok(vec![9_u8; 32])
        }

        fn decode(&self, _bytes: &[u8]) -> Result<Image, CodecError> {
            Err(CodecError::Decode("not used".to_owned()))
        }
    }

    struct FakePicker {
        result: Result<PickedTarget, RecordError>,
        picks: AtomicUsize,
    }

    impl TargetPicker for FakePicker {
        fn pick(&self, _options: PickOptions) -> Result<PickedTarget, RecordError> {
            self.picks.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }

        fn active_window(&self) -> Result<PickedTarget, RecordError> {
            self.picks.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }
    }

    struct FakeBorderHost {
        shows: AtomicUsize,
        closes: Arc<AtomicUsize>,
        fail: bool,
    }

    struct FakeBorderHandle {
        closes: Arc<AtomicUsize>,
    }

    impl BorderHandle for FakeBorderHandle {
        fn close(&mut self) {
            self.closes.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl BorderHost for FakeBorderHost {
        fn show(&self, _request: &BorderRequest) -> Result<Box<dyn BorderHandle>, RecordError> {
            self.shows.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(RecordError::Border("window".to_owned()));
            }
            Ok(Box::new(FakeBorderHandle {
                closes: Arc::clone(&self.closes),
            }))
        }
    }

    struct FakeControlsHost {
        shows: AtomicUsize,
        closes: Arc<AtomicUsize>,
        stops: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
        cancels: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
    }

    struct FakeControlsHandle {
        closes: Arc<AtomicUsize>,
    }

    impl ControlsHandle for FakeControlsHandle {
        fn close(&mut self) {
            self.closes.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl ControlsHost for FakeControlsHost {
        fn show(&self, request: &ControlsRequest) -> Result<Box<dyn ControlsHandle>, RecordError> {
            self.shows.fetch_add(1, Ordering::SeqCst);
            self.stops
                .lock()
                .expect("stops")
                .push(Arc::clone(&request.stop));
            self.cancels
                .lock()
                .expect("cancels")
                .push(Arc::clone(&request.cancel));
            Ok(Box::new(FakeControlsHandle {
                closes: Arc::clone(&self.closes),
            }))
        }
    }

    struct FakeAudio {
        starts: Mutex<Vec<AudioOptions>>,
        stops: Arc<AtomicUsize>,
    }

    struct FakeAudioSession {
        pipes: Vec<AudioPipe>,
        stops: Arc<AtomicUsize>,
    }

    impl AudioSession for FakeAudioSession {
        fn pipes(&self) -> Vec<AudioPipe> {
            self.pipes.clone()
        }

        fn stop(&mut self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl AudioPort for FakeAudio {
        fn start(&self, options: &AudioOptions) -> Result<Box<dyn AudioSession>, RecordError> {
            self.starts
                .lock()
                .expect("audio starts")
                .push(options.clone());
            Ok(Box::new(FakeAudioSession {
                pipes: vec![
                    pipe(0, AudioSource::System),
                    pipe(1, AudioSource::Microphone),
                ],
                stops: Arc::clone(&self.stops),
            }))
        }
    }

    fn pipe(index: u32, source: AudioSource) -> AudioPipe {
        AudioPipe {
            source,
            index,
            sample_format: PcmSampleFormat::S16Le,
            sample_rate: 48_000,
            channels: 2,
        }
    }

    struct FakeRunner {
        commands: Mutex<Vec<Vec<String>>>,
        fail: AtomicBool,
        stops: Arc<AtomicUsize>,
        kills: Arc<AtomicUsize>,
        encoder: VideoEncoder,
        outputs: Mutex<Vec<OutputBounds>>,
    }

    struct FakeProcess {
        running: bool,
        stops: Arc<AtomicUsize>,
        kills: Arc<AtomicUsize>,
    }

    impl FfmpegProcess for FakeProcess {
        fn progress(&mut self) -> Option<FfmpegProgress> {
            None
        }

        fn is_running(&mut self) -> bool {
            self.running
        }

        fn stop(&mut self) -> Result<FfmpegExit, RecordError> {
            self.running = false;
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(FfmpegExit::success())
        }

        fn kill(&mut self) -> Result<FfmpegExit, RecordError> {
            self.running = false;
            self.kills.fetch_add(1, Ordering::SeqCst);
            Ok(FfmpegExit::success())
        }

        fn wait(&mut self) -> Result<FfmpegExit, RecordError> {
            self.running = false;
            Ok(FfmpegExit::success())
        }
    }

    impl FfmpegRunner for FakeRunner {
        fn start(&self, command: FfmpegCommand) -> Result<Box<dyn FfmpegProcess>, RecordError> {
            let args = command
                .args()
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            self.commands.lock().expect("commands").push(args);
            if self.fail.load(Ordering::SeqCst) {
                return Err(RecordError::FfmpegStart("spawn".to_owned()));
            }
            Ok(Box::new(FakeProcess {
                running: true,
                stops: Arc::clone(&self.stops),
                kills: Arc::clone(&self.kills),
            }))
        }

        fn best_encoder(&self, _ffmpeg: &Path, _format: VideoFormat) -> VideoEncoder {
            self.encoder
        }

        fn outputs(&self) -> Vec<OutputBounds> {
            self.outputs.lock().expect("outputs").clone()
        }
    }

    struct FakeMedia {
        calls: AtomicUsize,
    }

    impl MediaPort for FakeMedia {
        fn first_frame(&self, _video: &Path, _ffmpeg: &Path) -> Result<Image, RecordError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Image::bgra(320, 180).map_err(|error| RecordError::Save(error.to_string()))
        }
    }

    struct FakeStore {
        files: Mutex<Vec<RecordedFile>>,
    }

    impl RecordingStore for FakeStore {
        fn save(&self, file: RecordedFile) -> Result<SavedRecording, RecordError> {
            let saved = SavedRecording {
                capture_id: Some(1),
                path: file.media.clone(),
                file_name: file
                    .media
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("recording.mp4")
                    .to_owned(),
                bytes: 4,
                width: file.width,
                height: file.height,
                duration_ms: file.duration_ms,
                kind: file.kind,
            };
            self.files.lock().expect("files").push(file);
            Ok(saved)
        }
    }

    struct FakePublisher {
        published: Mutex<Vec<SavedRecording>>,
        cancelled: AtomicUsize,
        failures: Mutex<Vec<String>>,
    }

    impl RecordingPublisher for FakePublisher {
        fn failed(&self, message: &str) {
            self.failures
                .lock()
                .expect("failures")
                .push(message.to_owned());
        }

        fn published(&self, saved: &SavedRecording) {
            self.published
                .lock()
                .expect("published")
                .push(saved.clone());
        }

        fn cancelled(&self) {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeSink {
        events: Sender<AppEvent>,
    }

    impl EventSink for FakeSink {
        fn emit(&self, event: AppEvent) {
            let _ = self.events.try_send(event);
        }
    }

    struct Fixture {
        directory: TempDir,
        services: Services,
        picker: Arc<FakePicker>,
        border: Arc<FakeBorderHost>,
        controls: Arc<FakeControlsHost>,
        audio: Arc<FakeAudio>,
        runner: Arc<FakeRunner>,
        media: Arc<FakeMedia>,
        store: Arc<FakeStore>,
        publisher: Arc<FakePublisher>,
        sender: Sender<AppEvent>,
        events: Receiver<AppEvent>,
    }

    impl Fixture {
        fn new(
            target: Result<PickedTarget, RecordError>,
            fail_border: bool,
            fail_ffmpeg: bool,
        ) -> Self {
            let directory = tempdir().expect("temp");
            let services = Services::with_paths(paths(directory.path())).expect("services");
            let (sender, events) = unbounded();
            Self {
                picker: Arc::new(FakePicker {
                    result: target,
                    picks: AtomicUsize::new(0),
                }),
                border: Arc::new(FakeBorderHost {
                    shows: AtomicUsize::new(0),
                    closes: Arc::new(AtomicUsize::new(0)),
                    fail: fail_border,
                }),
                controls: Arc::new(FakeControlsHost {
                    shows: AtomicUsize::new(0),
                    closes: Arc::new(AtomicUsize::new(0)),
                    stops: Mutex::new(Vec::new()),
                    cancels: Mutex::new(Vec::new()),
                }),
                audio: Arc::new(FakeAudio {
                    starts: Mutex::new(Vec::new()),
                    stops: Arc::new(AtomicUsize::new(0)),
                }),
                runner: Arc::new(FakeRunner {
                    commands: Mutex::new(Vec::new()),
                    fail: AtomicBool::new(fail_ffmpeg),
                    stops: Arc::new(AtomicUsize::new(0)),
                    kills: Arc::new(AtomicUsize::new(0)),
                    encoder: VideoEncoder::H264Nvenc,
                    outputs: Mutex::new(Vec::new()),
                }),
                media: Arc::new(FakeMedia {
                    calls: AtomicUsize::new(0),
                }),
                store: Arc::new(FakeStore {
                    files: Mutex::new(Vec::new()),
                }),
                publisher: Arc::new(FakePublisher {
                    published: Mutex::new(Vec::new()),
                    cancelled: AtomicUsize::new(0),
                    failures: Mutex::new(Vec::new()),
                }),
                directory,
                services,
                sender,
                events,
            }
        }

        fn sink(&self) -> Arc<FakeSink> {
            Arc::new(FakeSink {
                events: self.sender.clone(),
            })
        }

        fn ports(&self) -> RecordingPorts {
            RecordingPorts {
                picker: Arc::clone(&self.picker) as Arc<dyn TargetPicker>,
                border: Arc::clone(&self.border) as Arc<dyn BorderHost>,
                controls: Arc::clone(&self.controls) as Arc<dyn ControlsHost>,
                ffmpeg: Arc::clone(&self.runner) as Arc<dyn FfmpegRunner>,
                audio: Arc::clone(&self.audio) as Arc<dyn AudioPort>,
                media: Arc::clone(&self.media) as Arc<dyn MediaPort>,
                store: Arc::clone(&self.store) as Arc<dyn RecordingStore>,
                events: self.sink() as Arc<dyn EventSink>,
                publisher: Arc::clone(&self.publisher) as Arc<dyn RecordingPublisher>,
            }
        }

        fn plan(
            &self,
            kind: RecordedKind,
            mode: SessionMode,
            border: bool,
            audio: bool,
        ) -> SessionPlan {
            let output = self.directory.path().join("capture.mp4");
            if kind == RecordedKind::Gif {
                std::fs::write(&output, b"stub").expect("stub");
            }
            SessionPlan {
                kind,
                mode,
                target: target(),
                output,
                converted: (kind == RecordedKind::Gif)
                    .then(|| self.directory.path().join("capture.gif")),
                settings: SessionSettings {
                    ffmpeg: PathBuf::from(r"C:\Tools\ffmpeg.exe"),
                    frame_rate: VideoFrameRate::Fps30,
                    quality: VideoQuality::High,
                    format: VideoFormat::Mp4H264,
                    gif: GifSettings::default(),
                    border,
                    audio: AudioOptions {
                        system: audio,
                        microphone: audio,
                        microphone_device: DEFAULT_MICROPHONE.to_owned(),
                    },
                    theme: OverlayTheme::Dark,
                },
            }
        }
    }

    fn paths(directory: &Path) -> Paths {
        Paths::resolve(
            &directory.join("exe"),
            &directory.join("Roaming"),
            &directory.join("Local"),
            &directory.join("Profile"),
            false,
        )
    }

    fn target() -> PickedTarget {
        PickedTarget {
            rect: Rect::new(Px(10), Px(20), Px(640), Px(360)),
            monitor: 3,
            window: None,
        }
    }

    fn wait_for_state(events: &Receiver<AppEvent>, wanted: RecordStateDto) {
        loop {
            match events.recv_timeout(Duration::from_secs(5)) {
                Ok(AppEvent::Record(Event::State(state))) if state.state == wanted => return,
                Ok(_) => {}
                Err(error) => panic!("the recording never reached {wanted:?}: {error}"),
            }
        }
    }

    fn drain_until_history(events: &Receiver<AppEvent>) -> (bool, bool) {
        let mut done = false;
        let mut history = false;
        for _ in 0..12 {
            match events.recv_timeout(Duration::from_millis(200)) {
                Ok(AppEvent::Record(Event::State(state))) => {
                    done |= state.state == RecordStateDto::Done;
                }
                Ok(AppEvent::History(crate::history::Event::Added { .. })) => history = true,
                Ok(_) => {}
                Err(_) => break,
            }
            if done && history {
                break;
            }
        }
        (done, history)
    }

    fn commands(runner: &FakeRunner) -> Vec<Vec<String>> {
        runner.commands.lock().expect("commands").clone()
    }

    #[test]
    fn the_control_bar_stop_button_ends_the_recording_and_the_bar_closes() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
        let session = Session::start(plan, fixture.ports()).expect("start");
        let control = session.control();
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        assert_eq!(fixture.controls.shows.load(Ordering::SeqCst), 1);
        let press = Arc::clone(&fixture.controls.stops.lock().expect("stops")[0]);
        press();
        session.wait().expect("finish");
        assert_eq!(control.status().state, RecordingState::Done);
        assert_eq!(fixture.controls.closes.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
    }

    fn start_recording_fixture(fixture: &Fixture) -> (Session, SessionControl, PathBuf) {
        let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, true, false);
        let output = plan.output.clone();
        std::fs::write(&output, b"bad take").expect("temp recording");
        let session = Session::start(plan, fixture.ports()).expect("start");
        let control = session.control();
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        (session, control, output)
    }

    #[test]
    fn cancelling_discards_the_recording_and_publishes_nothing() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let (session, control, output) = start_recording_fixture(&fixture);
        control.cancel();
        let error = session.wait().expect_err("cancelled");
        assert!(matches!(error, RecordError::Cancelled));
        assert!(!output.exists());
        assert!(fixture.store.files.lock().expect("files").is_empty());
        assert!(fixture
            .publisher
            .published
            .lock()
            .expect("published")
            .is_empty());
        assert_eq!(fixture.publisher.cancelled.load(Ordering::SeqCst), 1);
        assert_eq!(control.status().state, RecordingState::Idle);
        assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.controls.closes.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.runner.kills.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.runner.stops.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.media.calls.load(Ordering::SeqCst), 0);
        let (_, history) = drain_until_history(&fixture.events);
        assert!(!history);
    }

    #[test]
    fn the_bar_cancel_button_routes_to_the_cancel_command() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let (session, control, output) = start_recording_fixture(&fixture);
        let press = Arc::clone(&fixture.controls.cancels.lock().expect("cancels")[0]);
        press();
        let error = session.wait().expect_err("cancelled");
        assert!(matches!(error, RecordError::Cancelled));
        assert!(!output.exists());
        assert_eq!(control.status().state, RecordingState::Idle);
        assert!(fixture.store.files.lock().expect("files").is_empty());
    }

    #[test]
    fn cancelling_after_the_recording_is_stored_changes_nothing() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
        let session = Session::start(plan, fixture.ports()).expect("start");
        let control = session.control();
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        control.stop();
        session.wait().expect("finish");
        control.cancel();
        assert_eq!(control.status().state, RecordingState::Done);
        assert_eq!(fixture.store.files.lock().expect("files").len(), 1);
        assert_eq!(fixture.publisher.cancelled.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_cancel_queued_behind_a_stop_is_still_seen() {
        let (sender, inbox) = unbounded();
        sender.send(Command::Stop).expect("stop");
        assert!(!cancel_requested(&inbox));
        sender.send(Command::Stop).expect("stop");
        sender.send(Command::Cancel).expect("cancel");
        assert!(cancel_requested(&inbox));
    }

    #[test]
    fn the_cancel_action_is_registered_and_runs_without_a_recording() {
        let services = Services::new();
        assert_eq!(run(&ActionId::new("record_cancel"), &services), Ok(()));
    }

    #[test]
    fn a_saved_recording_is_handed_to_the_publisher() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
        let session = Session::start(plan, fixture.ports()).expect("start");
        let control = session.control();
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        control.stop();
        session.wait().expect("finish");
        let published = fixture.publisher.published.lock().expect("published");
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].capture_id, Some(1));
        assert_eq!(published[0].file_name, "capture.mp4");
    }

    #[test]
    fn a_temporary_recording_is_not_published() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let plan = fixture.plan(RecordedKind::Video, SessionMode::Temporary, false, false);
        let session = Session::start(plan, fixture.ports()).expect("start");
        let control = session.control();
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        control.stop();
        session.wait().expect("finish");
        assert!(fixture
            .publisher
            .published
            .lock()
            .expect("published")
            .is_empty());
    }

    #[test]
    fn the_saved_notice_names_the_file_size_and_duration() {
        let saved = SavedRecording {
            capture_id: Some(9),
            path: PathBuf::from(r"C:\Pics\Snap line\clip.mp4"),
            file_name: "clip.mp4".to_owned(),
            bytes: 1_572_864,
            width: 640,
            height: 360,
            duration_ms: 75_400,
            kind: RecordedKind::Video,
        };
        let thumbnail = PathBuf::from(r"C:\Thumbs\9.webp");
        let notice = recording_saved_notice(&saved, Some(thumbnail.clone()));
        assert_eq!(notice.title, "Recording saved");
        assert_eq!(notice.lines, ["clip.mp4", "1.5 MB, 1:15"]);
        assert_eq!(notice.tag.as_deref(), Some("capture-9"));
        assert_eq!(notice.image, Some(thumbnail));
        assert_eq!(
            notice.open.as_deref(),
            Some("file:///C:/Pics/Snap%20line/clip.mp4")
        );
        let labels: Vec<&str> = notice.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Open video", "Open folder"]);
    }

    #[test]
    fn start_stop_records_audio_saves_and_closes_everything() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, true, true);
        let output = plan.output.clone();
        let session = Session::start(plan, fixture.ports()).expect("start");
        let control = session.control();
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        control.stop();
        let outcome = session.wait().expect("finish");
        assert_eq!(outcome.path, output);
        assert_eq!(control.status().state, RecordingState::Done);
        assert!(control.status().duration_ms <= 5);
        assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.controls.closes.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.audio.stops.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.runner.stops.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.runner.kills.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.picker.picks.load(Ordering::SeqCst), 0);
        let sent = commands(&fixture.runner);
        assert_eq!(sent.len(), 1);
        let args = &sent[0];
        assert!(args.iter().any(|argument| argument == "h264_nvenc"));
        assert!(args.iter().any(|argument| argument
            == "ddagrab=framerate=30:output_idx=0:video_size=640x360:offset_x=10:offset_y=20"));
        assert!(args
            .iter()
            .any(|argument| argument == r"\\.\pipe\snapline-audio-0"));
        assert!(args
            .iter()
            .any(|argument| argument == r"\\.\pipe\snapline-audio-1"));
        assert!(args
            .iter()
            .any(|argument| argument == "[1:a][2:a]amix=inputs=2[aout]"));
        let starts = fixture.audio.starts.lock().expect("audio starts");
        assert_eq!(starts.len(), 1);
        assert!(starts[0].system);
        assert!(starts[0].microphone);
        let files = fixture.store.files.lock().expect("files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, RecordedKind::Video);
        assert_eq!(files[0].width, 640);
        assert_eq!(files[0].height, 360);
        assert_eq!(fixture.media.calls.load(Ordering::SeqCst), 1);
        let (done, history) = drain_until_history(&fixture.events);
        assert!(done);
        assert!(history);
    }

    #[test]
    fn the_status_events_follow_the_state_machine() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let session = Session::start(
            fixture.plan(RecordedKind::Video, SessionMode::Save, false, false),
            fixture.ports(),
        )
        .expect("start");
        let control = session.control();
        let mut states = Vec::new();
        loop {
            match fixture.events.recv_timeout(Duration::from_secs(5)) {
                Ok(AppEvent::Record(Event::State(state))) => {
                    let reached = state.state == RecordStateDto::Recording;
                    states.push(state.state);
                    if reached {
                        break;
                    }
                }
                Ok(_) => {}
                Err(error) => panic!("the recording never started: {error}"),
            }
        }
        control.stop();
        assert!(session.wait().is_ok());
        while let Ok(event) = fixture.events.recv_timeout(Duration::from_millis(300)) {
            if let AppEvent::Record(Event::State(state)) = event {
                states.push(state.state);
            }
        }
        states.dedup();
        assert_eq!(
            states,
            vec![
                RecordStateDto::Arming,
                RecordStateDto::Recording,
                RecordStateDto::Stopping,
                RecordStateDto::Encoding,
                RecordStateDto::Done,
            ]
        );
    }

    #[test]
    fn stopping_before_recording_starts_still_finishes() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let session = Session::start(
            fixture.plan(RecordedKind::Video, SessionMode::Save, false, false),
            fixture.ports(),
        )
        .expect("start");
        let control = session.control();
        control.stop();
        assert!(session.wait().is_ok());
        assert_eq!(control.status().state, RecordingState::Done);
        assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.audio.starts.lock().expect("audio").len(), 0);
    }

    fn run_on_outputs(
        outputs: Vec<OutputBounds>,
    ) -> (Fixture, Result<SessionOutcome, RecordError>) {
        let fixture = Fixture::new(Ok(target()), false, false);
        *fixture.runner.outputs.lock().expect("outputs") = outputs;
        let session = Session::start(
            fixture.plan(RecordedKind::Video, SessionMode::Save, false, false),
            fixture.ports(),
        )
        .expect("start");
        let control = session.control();
        std::thread::sleep(Duration::from_millis(60));
        control.stop();
        let result = session.wait();
        (fixture, result)
    }

    #[test]
    fn a_secondary_screen_uses_its_own_output_and_relative_offsets() {
        let outputs = vec![
            OutputBounds {
                index: 0,
                bounds: snapline_core::geometry::Rect::new(
                    snapline_core::geometry::Px(1920),
                    snapline_core::geometry::Px(0),
                    snapline_core::geometry::Px(1920),
                    snapline_core::geometry::Px(1080),
                ),
            },
            OutputBounds {
                index: 2,
                bounds: snapline_core::geometry::Rect::new(
                    snapline_core::geometry::Px(-30),
                    snapline_core::geometry::Px(0),
                    snapline_core::geometry::Px(1920),
                    snapline_core::geometry::Px(1080),
                ),
            },
        ];
        let (fixture, result) = run_on_outputs(outputs);
        assert!(result.is_ok());
        let sent = commands(&fixture.runner);
        assert!(sent[0].iter().any(|argument| argument
            == "ddagrab=framerate=30:output_idx=2:video_size=640x360:offset_x=40:offset_y=20"));
    }

    #[test]
    fn an_area_spanning_two_screens_fails_with_a_clear_message_and_no_ffmpeg() {
        let outputs = vec![
            OutputBounds {
                index: 0,
                bounds: snapline_core::geometry::Rect::new(
                    snapline_core::geometry::Px(-1620),
                    snapline_core::geometry::Px(0),
                    snapline_core::geometry::Px(1920),
                    snapline_core::geometry::Px(1080),
                ),
            },
            OutputBounds {
                index: 1,
                bounds: snapline_core::geometry::Rect::new(
                    snapline_core::geometry::Px(300),
                    snapline_core::geometry::Px(0),
                    snapline_core::geometry::Px(1920),
                    snapline_core::geometry::Px(1080),
                ),
            },
        ];
        let (fixture, result) = run_on_outputs(outputs);
        let error = result.expect_err("spans screens");
        assert!(error.to_string().contains("single screen"));
        assert!(commands(&fixture.runner).is_empty());
        let failures = fixture.publisher.failures.lock().expect("failures");
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("single screen"));
    }

    #[test]
    fn ffmpeg_reasons_keep_only_the_last_lines_of_stderr() {
        assert_eq!(ffmpeg_reason(""), "");
        assert_eq!(ffmpeg_reason("a\n\nb\nc\nd\n"), "b c d".to_owned());
    }

    #[test]
    fn a_border_failure_fails_the_recording_without_ffmpeg() {
        let fixture = Fixture::new(Ok(target()), true, false);
        let session = Session::start(
            fixture.plan(RecordedKind::Video, SessionMode::Save, true, true),
            fixture.ports(),
        )
        .expect("start");
        let control = session.control();
        let error = session.wait().expect_err("border failure");
        assert!(matches!(error, RecordError::Border(_)));
        assert_eq!(control.status().state, RecordingState::Failed);
        assert!(control.status().error.is_some());
        assert!(commands(&fixture.runner).is_empty());
        assert_eq!(fixture.audio.starts.lock().expect("audio").len(), 0);
        assert_eq!(fixture.store.files.lock().expect("files").len(), 0);
    }

    #[test]
    fn an_ffmpeg_start_failure_closes_the_border_and_audio() {
        let fixture = Fixture::new(Ok(target()), false, true);
        let session = Session::start(
            fixture.plan(RecordedKind::Video, SessionMode::Save, true, true),
            fixture.ports(),
        )
        .expect("start");
        let control = session.control();
        let error = session.wait().expect_err("ffmpeg failure");
        assert!(matches!(error, RecordError::FfmpegStart(_)));
        assert_eq!(control.status().state, RecordingState::Failed);
        assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.audio.stops.load(Ordering::SeqCst), 1);
        assert!(fixture.store.files.lock().expect("files").is_empty());
    }

    #[test]
    fn cancelling_the_picker_shows_no_border_and_starts_no_thread() {
        let fixture = Fixture::new(Err(RecordError::Cancelled), false, false);
        let result = begin_with(
            &fixture.services,
            fixture.ports(),
            RecordKindDto::Video,
            SessionMode::Save,
        )
        .expect("begin");
        assert!(result.is_none());
        assert_eq!(fixture.picker.picks.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 0);
        assert!(commands(&fixture.runner).is_empty());
    }

    #[test]
    fn an_empty_target_is_rejected_before_the_border() {
        let fixture = Fixture::new(
            Ok(PickedTarget {
                rect: Rect::ZERO,
                monitor: 1,
                window: None,
            }),
            false,
            false,
        );
        let error = match begin_with(
            &fixture.services,
            fixture.ports(),
            RecordKindDto::Video,
            SessionMode::Save,
        ) {
            Ok(_) => panic!("the empty target was accepted"),
            Err(error) => error,
        };
        assert_eq!(error, RecordError::NoTarget);
        assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_window_recording_uses_the_active_window() {
        let window = PickedTarget {
            rect: Rect::new(Px(0), Px(0), Px(320), Px(200)),
            monitor: 1,
            window: Some(PickedWindow {
                handle: 42,
                title: "Editor".to_owned(),
            }),
        };
        let fixture = Fixture::new(Ok(window.clone()), false, false);
        let mut plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
        plan.target = window.clone();
        assert_eq!(
            plan.target.recording_target(),
            RecordingTarget::window(WindowId::new(42), plan.target.rect)
        );
        assert_eq!(plan.target.title(), "Editor");
        let result = begin_with(
            &fixture.services,
            fixture.ports(),
            RecordKindDto::Window,
            SessionMode::Save,
        )
        .expect("begin");
        assert!(result.is_some());
        assert_eq!(fixture.picker.picks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_gif_recording_converts_the_lossless_capture_before_saving() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let plan = fixture.plan(RecordedKind::Gif, SessionMode::Save, false, false);
        let converted = plan.converted.clone().expect("converted");
        let session = Session::start(plan, fixture.ports()).expect("start");
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        session.control().stop();
        let outcome = session.wait().expect("finish");
        assert_eq!(outcome.path, converted);
        let sent = commands(&fixture.runner);
        assert_eq!(sent.len(), 2);
        assert!(sent[0].iter().any(|argument| argument == "libx264"));
        assert!(sent[0]
            .windows(2)
            .any(|pair| pair[0] == "-crf" && pair[1] == "0"));
        let filter = sent[1]
            .iter()
            .position(|argument| argument == "-filter_complex")
            .and_then(|index| sent[1].get(index + 1))
            .expect("gif filter");
        assert!(filter.contains("palettegen"));
        assert!(filter.contains("paletteuse"));
        assert!(filter.contains("fps=15"));
        let files = fixture.store.files.lock().expect("files");
        assert_eq!(files[0].kind, RecordedKind::Gif);
        assert_eq!(files[0].media, converted);
    }

    #[test]
    fn a_temporary_recording_is_never_saved_to_history() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let plan = fixture.plan(RecordedKind::Video, SessionMode::Temporary, false, false);
        let output = plan.output.clone();
        let session = Session::start(plan, fixture.ports()).expect("start");
        wait_for_state(&fixture.events, RecordStateDto::Recording);
        session.control().stop();
        let outcome = session.wait().expect("finish");
        assert_eq!(outcome.path, output);
        assert!(fixture.store.files.lock().expect("files").is_empty());
        assert_eq!(fixture.media.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn every_video_setting_reaches_the_request() {
        let mut settings = Settings::default();
        settings.capture.video_format = StoreVideoFormat::MkvH264;
        settings.capture.frame_rate = FrameRate::Fps60;
        settings.capture.video_quality = StoreVideoQuality::SmallFile;
        settings.capture.record_system_audio = true;
        settings.capture.record_microphone = true;
        settings.capture.recording_border = false;
        let directory = tempdir().expect("temp");
        let resolved = paths(directory.path());
        let mapped = session_settings(&settings, &resolved);
        assert_eq!(mapped.format, VideoFormat::MkvH264);
        assert_eq!(mapped.frame_rate, VideoFrameRate::Fps60);
        assert_eq!(mapped.quality, VideoQuality::SmallFile);
        assert!(mapped.audio.system);
        assert!(mapped.audio.microphone);
        assert!(!mapped.border);
        assert_eq!(mapped.ffmpeg, resolved.exe_dir.join("ffmpeg.exe"));
    }

    #[test]
    fn every_gif_setting_reaches_the_filter() {
        let mut settings = Settings::default();
        settings.capture.gif_frame_rate = StoreGifFrameRate::Fps20;
        settings.capture.gif_max_width = StoreGifMaxWidth::Px480;
        settings.capture.gif_optimise_palette = false;
        let mapped = gif_settings(&settings);
        assert_eq!(mapped.frame_rate, GifFrameRate::Fps20);
        assert_eq!(mapped.max_width, GifMaxWidth::Px480);
        assert!(!mapped.optimise_palette);
    }

    #[test]
    fn converting_a_recording_uses_the_palette_filter() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let input = fixture.directory.path().join("clip.mp4");
        let output = fixture.directory.path().join("clip.gif");
        std::fs::write(&input, b"stub").expect("stub");
        let converted = convert_with(
            fixture.runner.as_ref(),
            Path::new(r"C:\Tools\ffmpeg.exe"),
            &input,
            &output,
            GifSettings::default(),
        )
        .expect("convert");
        assert_eq!(converted, output);
        let sent = commands(&fixture.runner);
        assert_eq!(sent.len(), 1);
        let filter = sent[0]
            .iter()
            .position(|argument| argument == "-filter_complex")
            .and_then(|index| sent[0].get(index + 1))
            .expect("gif filter");
        assert!(filter.contains("palettegen=stats_mode=diff"));
        assert!(filter.contains("paletteuse"));
        assert_eq!(sent[0].last().map(String::as_str), output.to_str());
    }

    #[test]
    fn converting_a_missing_recording_reports_it() {
        let fixture = Fixture::new(Ok(target()), false, false);
        let error = convert_with(
            fixture.runner.as_ref(),
            Path::new(r"C:\Tools\ffmpeg.exe"),
            &fixture.directory.path().join("missing.mp4"),
            &fixture.directory.path().join("missing.gif"),
            GifSettings::default(),
        )
        .expect_err("missing");
        assert!(matches!(error, RecordError::Convert(_)));
        assert!(commands(&fixture.runner).is_empty());
    }

    fn store_services(directory: &Path) -> Services {
        let factory: Arc<crate::CodecFactory> = Arc::new(|| {
            let codec: Arc<dyn ImageCodec> = Arc::new(TestCodec);
            Ok(codec)
        });
        Services::with_paths(paths(directory))
            .expect("services")
            .with_codec_factory(factory)
    }

    #[test]
    fn a_saved_recording_gets_a_name_folder_history_row_and_thumbnail() {
        let directory = tempdir().expect("temp");
        let services = store_services(directory.path());
        let media = directory.path().join("source.mp4");
        std::fs::write(&media, b"video-bytes").expect("media");
        let saved = store_recording(
            &services,
            RecordedFile {
                media: media.clone(),
                first_frame: Image::bgra(640, 360).ok(),
                kind: RecordedKind::Video,
                width: 640,
                height: 360,
                duration_ms: 2_500,
                source_app: None,
                window_title: "Recording".to_owned(),
            },
        )
        .expect("save");
        assert!(!media.exists());
        assert!(saved.path.is_file());
        assert!(saved.file_name.ends_with(".mp4"));
        assert_eq!(saved.bytes, 11);
        assert_eq!(saved.duration_ms, 2_500);
        let resolved = services.paths().expect("paths");
        assert!(saved.path.starts_with(&resolved.captures_dir));
        let id = saved.capture_id.expect("id");
        let repo = HistoryRepo::open_read(&resolved.history_file).expect("read");
        let capture = repo.get(id).expect("get").expect("row");
        assert_eq!(capture.kind, CaptureKind::Video);
        assert_eq!(capture.duration_ms, Some(2_500));
        assert_eq!(capture.width, Some(640));
        assert_eq!(capture.height, Some(360));
        assert!(resolved.thumbs_dir.join(format!("{id}.webp")).is_file());
        assert_eq!(
            services
                .settings_snapshot()
                .expect("settings")
                .general
                .extra
                .get("counter")
                .and_then(toml::Value::as_integer),
            Some(1)
        );
    }

    #[test]
    fn a_gif_is_recorded_with_its_own_kind_and_extension() {
        let directory = tempdir().expect("temp");
        let services = store_services(directory.path());
        let media = directory.path().join("source.gif");
        std::fs::write(&media, b"gif-bytes").expect("media");
        let saved = store_recording(
            &services,
            RecordedFile {
                media,
                first_frame: None,
                kind: RecordedKind::Gif,
                width: 480,
                height: 270,
                duration_ms: 1_000,
                source_app: None,
                window_title: "Recording".to_owned(),
            },
        )
        .expect("save");
        assert!(saved.file_name.ends_with(".gif"));
        let repo =
            HistoryRepo::open_read(&services.paths().expect("paths").history_file).expect("read");
        let capture = repo
            .get(saved.capture_id.expect("id"))
            .expect("get")
            .expect("row");
        assert_eq!(capture.kind, CaptureKind::Gif);
    }

    #[test]
    fn status_dto_fills_dimensions_and_terminal_details() {
        let status = RecordStatus {
            state: RecordingState::Done,
            kind: Some(RecordedKind::Gif),
            elapsed_ms: 4_200,
            width: 800,
            height: 450,
            duration_ms: 4_200,
            capture_id: Some(9),
            path: Some(PathBuf::from(r"C:\Videos\clip.gif")),
            error: None,
        };
        let dto = status_dto(&status);
        assert_eq!(dto.state, RecordStateDto::Done);
        assert_eq!(dto.kind, Some(RecordKindDto::Gif));
        assert_eq!(dto.width, Some(800));
        assert_eq!(dto.height, Some(450));
        assert_eq!(dto.duration_ms, Some(4_200));
        assert_eq!(dto.capture_id, Some(9));
        assert_eq!(dto.path.as_deref(), Some(r"C:\Videos\clip.gif"));
        let idle = status_dto(&RecordStatus::default());
        assert_eq!(idle.state, RecordStateDto::Idle);
        assert_eq!(idle.width, None);
        assert_eq!(idle.duration_ms, None);
    }

    #[test]
    fn recording_actions_report_not_yet_without_ports() {
        let directory = tempdir().expect("temp");
        let services = Services::with_paths(paths(directory.path())).expect("services");
        let error = run(&ActionId::new("record_video"), &services).expect_err("not yet");
        assert_eq!(error, ServiceError::NotYet);
        assert_eq!(
            services.record().handle(RecordReq::State).expect("state"),
            RecordResp::Status {
                status: status_dto(&current_status())
            }
        );
        assert_eq!(
            services.record().handle(RecordReq::Stop).expect("stop"),
            RecordResp::Status {
                status: status_dto(&current_status())
            }
        );
    }

    #[test]
    fn a_cancelled_error_maps_to_an_invalid_service_error() {
        let error: ServiceError = RecordError::Cancelled.into();
        assert_eq!(error.code(), "invalid");
        assert!(matches!(
            ServiceError::from(RecordError::NotYet),
            ServiceError::NotYet
        ));
        assert_eq!(ServiceError::from(RecordError::Busy).code(), "busy");
    }
}
