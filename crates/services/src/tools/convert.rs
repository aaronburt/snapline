use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use snapline_core::ffmpeg_args::{
    self, ConvertFormat, ConvertRequest, FfmpegArgsError, FfmpegCommand, GifSettings,
};
use snapline_store::history::{CaptureKind, HistoryRepo, NewCapture};
use thiserror::Error;

use crate::dto::tool::{
    ConvertFormatDto, ConvertPhaseDto, ConvertProgressDto, ConvertSource, ConvertedDto, ToolResp,
};
use crate::tools::ToolService;
use crate::ServiceError;

pub const POLL_INTERVAL: Duration = Duration::from_millis(50);
const JOB_RETENTION: Duration = Duration::from_secs(300);
const VIDEO_EXTENSIONS: [&str; 9] = [
    "mp4", "mkv", "webm", "mov", "avi", "m4v", "wmv", "gif", "mpg",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConvertError {
    #[error("Choose a video to convert.")]
    MissingInput,
    #[error("That video could not be found.")]
    NotFound,
    #[error("The video path must be absolute.")]
    RelativePath,
    #[error("Only video files can be converted.")]
    UnsupportedFile,
    #[error("Choose a supported output format.")]
    UnsupportedFormat,
    #[error("The FFmpeg path must be absolute.")]
    FfmpegPath,
    #[error("FFmpeg is not available.")]
    NoBackend,
    #[error("The file picker is not available.")]
    NoFilePicker,
    #[error("The conversion was cancelled.")]
    Cancelled,
    #[error("The converted video could not be found.")]
    MissingOutput,
    #[error("The converted video could not be saved: {0}")]
    Save(String),
    #[error("History could not record the converted video: {0}")]
    History(String),
    #[error("The settings could not be read: {0}")]
    Settings(String),
    #[error("The video could not be converted. {0}")]
    Ffmpeg(String),
    #[error("{0}")]
    Args(#[from] FfmpegArgsError),
}

impl From<ConvertError> for ServiceError {
    fn from(error: ConvertError) -> Self {
        let message = error.to_string();
        match error {
            ConvertError::MissingInput
            | ConvertError::RelativePath
            | ConvertError::UnsupportedFile
            | ConvertError::UnsupportedFormat
            | ConvertError::FfmpegPath
            | ConvertError::Cancelled
            | ConvertError::Args(_) => Self::Invalid(message),
            ConvertError::NotFound => Self::NotFound,
            ConvertError::NoBackend | ConvertError::NoFilePicker => Self::NotYet,
            ConvertError::Save(_) | ConvertError::History(_) => Self::Io(message),
            ConvertError::MissingOutput | ConvertError::Settings(_) | ConvertError::Ffmpeg(_) => {
                Self::Internal(message)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BackendProgress {
    pub out_time_us: Option<u64>,
    pub frame: Option<u64>,
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendExit {
    pub success: bool,
    pub message: String,
}

impl BackendExit {
    pub fn success() -> Self {
        Self {
            success: true,
            message: String::new(),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
        }
    }
}

pub trait FfmpegProcess: Send {
    fn progress(&mut self) -> Option<BackendProgress>;
    fn finished(&mut self) -> Result<bool, ConvertError>;
    fn cancel(&mut self) -> Result<(), ConvertError>;
    fn wait(&mut self) -> Result<BackendExit, ConvertError>;
}

pub trait FfmpegBackend: Send + Sync {
    fn start(&self, command: FfmpegCommand) -> Result<Box<dyn FfmpegProcess>, ConvertError>;
}

pub trait ConvertFilePicker: Send + Sync {
    fn pick_video(&self) -> Result<Option<PathBuf>, ConvertError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertInput {
    pub path: PathBuf,
    pub capture_id: Option<i64>,
    pub duration_ms: Option<u64>,
    pub source_app: Option<String>,
}

impl ConvertInput {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            capture_id: None,
            duration_ms: None,
            source_app: None,
        }
    }
}

pub trait ConvertEnvironment: Send + Sync {
    fn resolve(&self, source: &ConvertSource) -> Result<ConvertInput, ConvertError>;
    fn ffmpeg(&self) -> Result<PathBuf, ConvertError>;
    fn output_path(
        &self,
        input: &ConvertInput,
        format: ConvertFormat,
    ) -> Result<PathBuf, ConvertError>;
    fn record(
        &self,
        input: &ConvertInput,
        output: &Path,
        format: ConvertFormat,
    ) -> Result<ConvertedDto, ConvertError>;
}

pub struct ServicesEnvironment {
    services: crate::Services,
}

impl ServicesEnvironment {
    pub fn new(services: &crate::Services) -> Self {
        Self {
            services: services.clone(),
        }
    }

    fn capture(&self, capture_id: i64) -> Result<ConvertInput, ConvertError> {
        if capture_id <= 0 {
            return Err(ConvertError::NotFound);
        }
        let paths = self
            .services
            .paths()
            .map_err(|error| ConvertError::History(error.to_string()))?;
        if !paths.history_file.is_file() {
            return Err(ConvertError::NotFound);
        }
        let repo = HistoryRepo::open_read(&paths.history_file)
            .map_err(|error| ConvertError::History(error.to_string()))?;
        let capture = repo
            .get(capture_id)
            .map_err(|error| ConvertError::History(error.to_string()))?
            .ok_or(ConvertError::NotFound)?;
        if capture.kind == CaptureKind::Image {
            return Err(ConvertError::UnsupportedFile);
        }
        let path = PathBuf::from(&capture.path);
        if !path.is_file() {
            return Err(ConvertError::NotFound);
        }
        Ok(ConvertInput {
            path,
            capture_id: Some(capture_id),
            duration_ms: capture
                .duration_ms
                .and_then(|value| u64::try_from(value).ok())
                .filter(|value| *value > 0),
            source_app: capture.source_app,
        })
    }
}

impl ConvertEnvironment for ServicesEnvironment {
    fn resolve(&self, source: &ConvertSource) -> Result<ConvertInput, ConvertError> {
        match source {
            ConvertSource::Capture { capture_id } => self.capture(*capture_id),
            ConvertSource::File { path } => file_input(path),
        }
    }

    fn ffmpeg(&self) -> Result<PathBuf, ConvertError> {
        let settings = self
            .services
            .settings_snapshot()
            .map_err(|error| ConvertError::Settings(error.to_string()))?;
        let paths = self
            .services
            .paths()
            .map_err(|error| ConvertError::Settings(error.to_string()))?;
        let ffmpeg = settings.capture.resolved_ffmpeg_path(&paths.exe_dir);
        if ffmpeg.is_absolute() {
            Ok(ffmpeg)
        } else {
            Err(ConvertError::FfmpegPath)
        }
    }

    fn output_path(
        &self,
        input: &ConvertInput,
        format: ConvertFormat,
    ) -> Result<PathBuf, ConvertError> {
        output_path(&input.path, format)
    }

    fn record(
        &self,
        input: &ConvertInput,
        output: &Path,
        format: ConvertFormat,
    ) -> Result<ConvertedDto, ConvertError> {
        let bytes = fs::metadata(output)
            .map_err(|error| ConvertError::Save(error.to_string()))?
            .len();
        let bytes = i64::try_from(bytes)
            .map_err(|_| ConvertError::Save("The file is too large.".to_owned()))?;
        let paths = self
            .services
            .paths()
            .map_err(|error| ConvertError::History(error.to_string()))?;
        let mut repo = HistoryRepo::open_write(&paths.history_file)
            .map_err(|error| ConvertError::History(error.to_string()))?;
        let capture_id = repo
            .record(NewCapture {
                kind: capture_kind(format),
                path: output.to_string_lossy().into_owned(),
                bytes,
                width: None,
                height: None,
                duration_ms: input
                    .duration_ms
                    .and_then(|value| i64::try_from(value).ok()),
                source_app: input.source_app.clone(),
                created_at: now_millis(),
                ocr_text: None,
            })
            .map_err(|error| ConvertError::History(error.to_string()))?;
        Ok(converted_dto(capture_id, output, format))
    }
}

fn file_input(value: &str) -> Result<ConvertInput, ConvertError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ConvertError::MissingInput);
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(ConvertError::RelativePath);
    }
    if !path.is_file() {
        return Err(ConvertError::NotFound);
    }
    if !is_video(&path) {
        return Err(ConvertError::UnsupportedFile);
    }
    Ok(ConvertInput::new(path))
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.as_str()))
}

fn output_path(input: &Path, format: ConvertFormat) -> Result<PathBuf, ConvertError> {
    let parent = input
        .parent()
        .ok_or_else(|| ConvertError::Save("The video has no folder.".to_owned()))?;
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("converted");
    let name = format!("{stem}.{}", format_extension(format));
    let chosen = snapline_core::naming::resolve_collision(&name, |candidate| {
        parent.join(candidate).exists()
    });
    Ok(parent.join(chosen))
}

fn format_extension(format: ConvertFormat) -> &'static str {
    match format {
        ConvertFormat::Gif(_) => "gif",
        ConvertFormat::WebmVp9 => "webm",
        ConvertFormat::Mp4Small => "mp4",
    }
}

fn capture_kind(format: ConvertFormat) -> CaptureKind {
    match format {
        ConvertFormat::Gif(_) => CaptureKind::Gif,
        ConvertFormat::WebmVp9 | ConvertFormat::Mp4Small => CaptureKind::Video,
    }
}

fn converted_dto(capture_id: i64, output: &Path, format: ConvertFormat) -> ConvertedDto {
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Converted video")
        .to_owned();
    ConvertedDto {
        capture_id,
        path: output.to_string_lossy().into_owned(),
        name,
        format: format_dto(format),
    }
}

pub fn core_format(format: ConvertFormatDto) -> ConvertFormat {
    match format {
        ConvertFormatDto::Gif => ConvertFormat::Gif(GifSettings::default()),
        ConvertFormatDto::WebmVp9 => ConvertFormat::WebmVp9,
        ConvertFormatDto::Mp4Small => ConvertFormat::Mp4Small,
    }
}

pub fn format_dto(format: ConvertFormat) -> ConvertFormatDto {
    match format {
        ConvertFormat::Gif(_) => ConvertFormatDto::Gif,
        ConvertFormat::WebmVp9 => ConvertFormatDto::WebmVp9,
        ConvertFormat::Mp4Small => ConvertFormatDto::Mp4Small,
    }
}

pub fn percent(duration_ms: Option<u64>, out_time_us: Option<u64>) -> Option<f64> {
    let total = duration_ms?;
    if total == 0 {
        return None;
    }
    let elapsed_ms = out_time_us.unwrap_or(0) / 1_000;
    Some((elapsed_ms as f64 / total as f64 * 100.0).clamp(0.0, 99.0))
}

fn progress_dto(progress: BackendProgress, duration_ms: Option<u64>) -> ConvertProgressDto {
    ConvertProgressDto {
        phase: if progress.done {
            ConvertPhaseDto::Saving
        } else {
            ConvertPhaseDto::Converting
        },
        percent: if progress.done {
            Some(100.0)
        } else {
            percent(duration_ms, progress.out_time_us)
        },
        out_time_ms: progress.out_time_us.map(|value| value / 1_000),
        speed: progress.speed,
        message: None,
    }
}

fn saving_progress() -> ConvertProgressDto {
    ConvertProgressDto {
        phase: ConvertPhaseDto::Saving,
        percent: Some(100.0),
        out_time_ms: None,
        speed: None,
        message: None,
    }
}

fn done_progress() -> ConvertProgressDto {
    ConvertProgressDto {
        phase: ConvertPhaseDto::Done,
        percent: Some(100.0),
        out_time_ms: None,
        speed: None,
        message: None,
    }
}

pub fn run_conversion(
    backend: &dyn FfmpegBackend,
    environment: &dyn ConvertEnvironment,
    source: &ConvertSource,
    format: ConvertFormatDto,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(ConvertProgressDto),
) -> Result<ConvertedDto, ConvertError> {
    let input = environment.resolve(source)?;
    let ffmpeg = environment.ffmpeg()?;
    let output_format = core_format(format);
    let output = environment.output_path(&input, output_format)?;
    let command = ffmpeg_args::convert_command(&ConvertRequest {
        ffmpeg,
        input: input.path.clone(),
        output: output.clone(),
        format: output_format,
    })?;
    let mut process = backend.start(command)?;
    loop {
        if cancel.load(Ordering::SeqCst) {
            if let Err(error) = process.cancel() {
                tracing::warn!(%error, "the converter could not stop FFmpeg");
            }
            let _ = fs::remove_file(&output);
            return Err(ConvertError::Cancelled);
        }
        while let Some(progress) = process.progress() {
            on_progress(progress_dto(progress, input.duration_ms));
        }
        if process.finished()? {
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    let outcome = process.wait()?;
    if !outcome.success {
        let _ = fs::remove_file(&output);
        return Err(ConvertError::Ffmpeg(outcome.message));
    }
    if !output.is_file() {
        return Err(ConvertError::MissingOutput);
    }
    on_progress(saving_progress());
    let converted = environment.record(&input, &output, output_format)?;
    on_progress(done_progress());
    Ok(converted)
}

struct JobState {
    phase: ConvertPhaseDto,
    percent: Option<f64>,
    out_time_ms: Option<u64>,
    speed: Option<f64>,
    message: Option<String>,
    converted: Option<ConvertedDto>,
}

impl JobState {
    fn new() -> Self {
        Self {
            phase: ConvertPhaseDto::Converting,
            percent: None,
            out_time_ms: None,
            speed: None,
            message: None,
            converted: None,
        }
    }

    fn progress(&self) -> ConvertProgressDto {
        ConvertProgressDto {
            phase: self.phase,
            percent: self.percent,
            out_time_ms: self.out_time_ms,
            speed: self.speed,
            message: self.message.clone(),
        }
    }
}

struct Job {
    cancel: AtomicBool,
    state: Mutex<JobState>,
    started: Instant,
}

enum JobOutcome {
    Running,
    Done(ConvertedDto),
    Failed(String),
    Cancelled,
}

impl Job {
    fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
            state: Mutex::new(JobState::new()),
            started: Instant::now(),
        }
    }

    fn progressed(&self, progress: ConvertProgressDto) {
        let mut state = lock(&self.state);
        if matches!(
            state.phase,
            ConvertPhaseDto::Converting | ConvertPhaseDto::Saving
        ) {
            state.phase = progress.phase;
            state.percent = progress.percent;
            state.out_time_ms = progress.out_time_ms;
            state.speed = progress.speed;
            state.message = progress.message;
        }
    }

    fn finish(&self, converted: ConvertedDto) {
        let mut state = lock(&self.state);
        state.phase = ConvertPhaseDto::Done;
        state.percent = Some(100.0);
        state.message = None;
        state.converted = Some(converted);
    }

    fn fail(&self, message: String) {
        let mut state = lock(&self.state);
        state.phase = ConvertPhaseDto::Failed;
        state.message = Some(message);
    }

    fn cancelled(&self) {
        let mut state = lock(&self.state);
        state.phase = ConvertPhaseDto::Cancelled;
        state.message = Some("The conversion was cancelled.".to_owned());
    }

    fn progress(&self) -> ConvertProgressDto {
        lock(&self.state).progress()
    }

    fn outcome(&self) -> JobOutcome {
        let state = lock(&self.state);
        match state.phase {
            ConvertPhaseDto::Done => state
                .converted
                .clone()
                .map_or(JobOutcome::Running, JobOutcome::Done),
            ConvertPhaseDto::Failed => JobOutcome::Failed(
                state
                    .message
                    .clone()
                    .unwrap_or_else(|| "The video could not be converted.".to_owned()),
            ),
            ConvertPhaseDto::Cancelled => JobOutcome::Cancelled,
            ConvertPhaseDto::Converting | ConvertPhaseDto::Saving => JobOutcome::Running,
        }
    }

    fn is_finished(&self) -> bool {
        matches!(
            lock(&self.state).phase,
            ConvertPhaseDto::Done | ConvertPhaseDto::Failed | ConvertPhaseDto::Cancelled
        )
    }
}

static JOBS: OnceLock<Mutex<HashMap<u64, Arc<Job>>>> = OnceLock::new();
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);
static BACKEND: OnceLock<Mutex<Option<Arc<dyn FfmpegBackend>>>> = OnceLock::new();
static PICKER: OnceLock<Mutex<Option<Arc<dyn ConvertFilePicker>>>> = OnceLock::new();

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn jobs() -> &'static Mutex<HashMap<u64, Arc<Job>>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn backend_slot() -> &'static Mutex<Option<Arc<dyn FfmpegBackend>>> {
    BACKEND.get_or_init(|| Mutex::new(None))
}

fn picker_slot() -> &'static Mutex<Option<Arc<dyn ConvertFilePicker>>> {
    PICKER.get_or_init(|| Mutex::new(None))
}

pub fn install_backend(backend: Arc<dyn FfmpegBackend>) {
    *lock(backend_slot()) = Some(backend);
}

pub fn install_file_picker(picker: Arc<dyn ConvertFilePicker>) {
    *lock(picker_slot()) = Some(picker);
}

fn installed_backend() -> Result<Arc<dyn FfmpegBackend>, ConvertError> {
    lock(backend_slot()).clone().ok_or(ConvertError::NoBackend)
}

fn installed_picker() -> Result<Arc<dyn ConvertFilePicker>, ConvertError> {
    lock(picker_slot())
        .clone()
        .ok_or(ConvertError::NoFilePicker)
}

fn register_job() -> (u64, Arc<Job>) {
    let id = NEXT_JOB.fetch_add(1, Ordering::SeqCst);
    let job = Arc::new(Job::new());
    let mut map = lock(jobs());
    map.retain(|_, existing| !existing.is_finished() || existing.started.elapsed() < JOB_RETENTION);
    map.insert(id, Arc::clone(&job));
    (id, job)
}

fn job_of(job_id: u64) -> Result<Arc<Job>, ServiceError> {
    lock(jobs())
        .get(&job_id)
        .cloned()
        .ok_or(ServiceError::NotFound)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

impl ToolService {
    pub fn convert_pick_file(&self) -> Result<ToolResp, ServiceError> {
        let picker = installed_picker()?;
        let path = picker.pick_video()?;
        Ok(ToolResp::ConvertFilePicked {
            path: path.map(|path| path.to_string_lossy().into_owned()),
        })
    }

    pub fn convert_start(
        &self,
        source: ConvertSource,
        format: ConvertFormatDto,
    ) -> Result<ToolResp, ServiceError> {
        let backend = installed_backend()?;
        let environment = Arc::new(ServicesEnvironment::new(&self._services));
        let (job_id, job) = register_job();
        let task_job = Arc::clone(&job);
        let spawned = thread::Builder::new()
            .name("snapline-convert".to_owned())
            .spawn(move || {
                let mut callback = |progress| task_job.progressed(progress);
                let result = run_conversion(
                    backend.as_ref(),
                    environment.as_ref(),
                    &source,
                    format,
                    &task_job.cancel,
                    &mut callback,
                );
                match result {
                    Ok(converted) => task_job.finish(converted),
                    Err(ConvertError::Cancelled) => task_job.cancelled(),
                    Err(error) => task_job.fail(error.to_string()),
                }
            });
        if spawned.is_err() {
            job.fail("The conversion could not be started.".to_owned());
            return Err(ServiceError::Busy);
        }
        Ok(ToolResp::ConvertStarted { job_id })
    }

    pub fn convert_progress(&self, job_id: u64) -> Result<ToolResp, ServiceError> {
        let job = job_of(job_id)?;
        Ok(ToolResp::ConvertProgressed {
            job_id,
            progress: job.progress(),
        })
    }

    pub fn convert_cancel(&self, job_id: u64) -> Result<ToolResp, ServiceError> {
        let job = job_of(job_id)?;
        if job.is_finished() {
            return Err(ServiceError::Invalid(
                "The conversion has already finished.".to_owned(),
            ));
        }
        job.cancel.store(true, Ordering::SeqCst);
        Ok(ToolResp::ConvertCancelled { job_id })
    }

    pub fn convert_result(&self, job_id: u64) -> Result<ToolResp, ServiceError> {
        let job = job_of(job_id)?;
        match job.outcome() {
            JobOutcome::Done(converted) => Ok(ToolResp::Converted { converted }),
            JobOutcome::Failed(message) => Err(ServiceError::Internal(message)),
            JobOutcome::Cancelled => Err(ServiceError::Invalid(
                "The conversion was cancelled.".to_owned(),
            )),
            JobOutcome::Running => Err(ServiceError::Busy),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use snapline_store::io::Paths;
    use tempfile::tempdir;

    use super::*;
    use crate::Services;

    struct FakeProcess {
        state: Arc<FakeState>,
    }

    struct FakeState {
        commands: Mutex<Vec<FfmpegCommand>>,
        progress: Mutex<VecDeque<BackendProgress>>,
        exit: Mutex<BackendExit>,
        cancelled: AtomicBool,
    }

    struct FakeBackend {
        state: Arc<FakeState>,
    }

    impl FakeBackend {
        fn new(progress: Vec<BackendProgress>, exit: BackendExit) -> Self {
            Self {
                state: Arc::new(FakeState {
                    commands: Mutex::new(Vec::new()),
                    progress: Mutex::new(progress.into()),
                    exit: Mutex::new(exit),
                    cancelled: AtomicBool::new(false),
                }),
            }
        }

        fn commands(&self) -> Vec<FfmpegCommand> {
            lock(&self.state.commands).clone()
        }

        fn cancelled(&self) -> bool {
            self.state.cancelled.load(Ordering::SeqCst)
        }
    }

    impl FfmpegBackend for FakeBackend {
        fn start(&self, command: FfmpegCommand) -> Result<Box<dyn FfmpegProcess>, ConvertError> {
            lock(&self.state.commands).push(command);
            Ok(Box::new(FakeProcess {
                state: Arc::clone(&self.state),
            }))
        }
    }

    impl FfmpegProcess for FakeProcess {
        fn progress(&mut self) -> Option<BackendProgress> {
            lock(&self.state.progress).pop_front()
        }

        fn finished(&mut self) -> Result<bool, ConvertError> {
            Ok(lock(&self.state.progress).is_empty())
        }

        fn cancel(&mut self) -> Result<(), ConvertError> {
            self.state.cancelled.store(true, Ordering::SeqCst);
            lock(&self.state.progress).clear();
            Ok(())
        }

        fn wait(&mut self) -> Result<BackendExit, ConvertError> {
            Ok(lock(&self.state.exit).clone())
        }
    }

    struct FakeEnvironment {
        input: Result<ConvertInput, ConvertError>,
        ffmpeg: PathBuf,
        output: PathBuf,
        recorded: Mutex<Vec<(PathBuf, ConvertFormat)>>,
    }

    impl FakeEnvironment {
        fn new(input: ConvertInput, output: PathBuf) -> Self {
            Self {
                input: Ok(input),
                ffmpeg: PathBuf::from(r"C:\Tools\ffmpeg.exe"),
                output,
                recorded: Mutex::new(Vec::new()),
            }
        }

        fn recorded(&self) -> Vec<(PathBuf, ConvertFormat)> {
            lock(&self.recorded).clone()
        }
    }

    impl ConvertEnvironment for FakeEnvironment {
        fn resolve(&self, _source: &ConvertSource) -> Result<ConvertInput, ConvertError> {
            self.input.clone()
        }

        fn ffmpeg(&self) -> Result<PathBuf, ConvertError> {
            Ok(self.ffmpeg.clone())
        }

        fn output_path(
            &self,
            _input: &ConvertInput,
            _format: ConvertFormat,
        ) -> Result<PathBuf, ConvertError> {
            Ok(self.output.clone())
        }

        fn record(
            &self,
            _input: &ConvertInput,
            output: &Path,
            format: ConvertFormat,
        ) -> Result<ConvertedDto, ConvertError> {
            lock(&self.recorded).push((output.to_path_buf(), format));
            Ok(converted_dto(42, output, format))
        }
    }

    fn args(command: &FfmpegCommand) -> Vec<String> {
        command
            .args()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    }

    fn source(path: &Path) -> ConvertSource {
        ConvertSource::File {
            path: path.to_string_lossy().into_owned(),
        }
    }

    fn fixture(format: ConvertFormatDto) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempdir().expect("directory");
        let input = directory.path().join("recording.mp4");
        fs::write(&input, b"video").expect("input");
        let output = directory.path().join(format!(
            "recording.{}",
            match format {
                ConvertFormatDto::Gif => "gif",
                ConvertFormatDto::WebmVp9 => "webm",
                ConvertFormatDto::Mp4Small => "mp4",
            }
        ));
        fs::write(&output, b"converted").expect("output");
        (directory, input, output)
    }

    #[test]
    fn every_output_format_builds_the_expected_command_and_records_to_history() {
        for (format, container, codec, crf) in [
            (ConvertFormatDto::Gif, "gif", None, None),
            (ConvertFormatDto::WebmVp9, "webm", Some("libvpx-vp9"), None),
            (
                ConvertFormatDto::Mp4Small,
                "mp4",
                Some("libx264"),
                Some("28"),
            ),
        ] {
            let (_directory, input, output) = fixture(format);
            let backend = FakeBackend::new(Vec::new(), BackendExit::success());
            let environment =
                FakeEnvironment::new(ConvertInput::new(input.clone()), output.clone());
            let cancel = AtomicBool::new(false);
            let mut updates = Vec::new();
            let converted = run_conversion(
                &backend,
                &environment,
                &source(&input),
                format,
                &cancel,
                &mut |progress| updates.push(progress),
            )
            .expect("conversion");
            assert_eq!(converted.capture_id, 42);
            assert_eq!(converted.format, format);
            assert_eq!(
                converted.name,
                output.file_name().unwrap().to_string_lossy().into_owned()
            );
            let commands = backend.commands();
            let command = commands.first().expect("command");
            let arguments = args(command);
            assert!(has_pair(&arguments, "-f", container));
            if let Some(codec) = codec {
                assert!(has_pair(&arguments, "-c:v", codec));
            }
            if let Some(crf) = crf {
                assert!(has_pair(&arguments, "-crf", crf));
            }
            let recorded = environment.recorded();
            assert_eq!(recorded.len(), 1);
            assert_eq!(recorded[0].0, output);
            assert_eq!(recorded[0].1, core_format(format));
            assert_eq!(
                updates.last().expect("progress").phase,
                ConvertPhaseDto::Done
            );
        }
    }

    #[test]
    fn progress_maps_out_time_against_the_capture_duration() {
        let (_directory, input, output) = fixture(ConvertFormatDto::Mp4Small);
        let backend = FakeBackend::new(
            vec![
                BackendProgress {
                    out_time_us: Some(500_000),
                    ..BackendProgress::default()
                },
                BackendProgress {
                    out_time_us: Some(1_500_000),
                    speed: Some(1.5),
                    ..BackendProgress::default()
                },
            ],
            BackendExit::success(),
        );
        let mut info = ConvertInput::new(input.clone());
        info.duration_ms = Some(4_000);
        let environment = FakeEnvironment::new(info, output);
        let cancel = AtomicBool::new(false);
        let mut updates = Vec::new();
        run_conversion(
            &backend,
            &environment,
            &source(&input),
            ConvertFormatDto::Mp4Small,
            &cancel,
            &mut |progress| updates.push(progress),
        )
        .expect("conversion");
        assert_eq!(updates[0].percent, Some(12.5));
        assert_eq!(updates[1].percent, Some(37.5));
        assert_eq!(updates[1].out_time_ms, Some(1_500));
        assert_eq!(updates[1].speed, Some(1.5));
        assert_eq!(updates[2].phase, ConvertPhaseDto::Saving);
        assert_eq!(updates[2].percent, Some(100.0));
        assert_eq!(updates[3].phase, ConvertPhaseDto::Done);
        assert_eq!(percent(None, Some(1_000_000)), None);
        assert_eq!(percent(Some(0), Some(1_000_000)), None);
        assert_eq!(percent(Some(1_000), Some(10_000_000)), Some(99.0));
    }

    #[test]
    fn cancelling_kills_ffmpeg_and_removes_the_partial_output() {
        let (_directory, input, output) = fixture(ConvertFormatDto::WebmVp9);
        let backend = FakeBackend::new(Vec::new(), BackendExit::success());
        let environment = FakeEnvironment::new(ConvertInput::new(input.clone()), output.clone());
        let cancel = AtomicBool::new(true);
        let error = run_conversion(
            &backend,
            &environment,
            &source(&input),
            ConvertFormatDto::WebmVp9,
            &cancel,
            &mut |_| {},
        )
        .expect_err("cancelled");
        assert_eq!(error, ConvertError::Cancelled);
        assert!(backend.cancelled());
        assert!(!output.exists());
        assert!(environment.recorded().is_empty());
        assert_eq!(ServiceError::from(error).code(), "invalid");
    }

    #[test]
    fn an_ffmpeg_failure_maps_to_a_typed_error_and_removes_the_output() {
        let (_directory, input, output) = fixture(ConvertFormatDto::Gif);
        let backend = FakeBackend::new(
            Vec::new(),
            BackendExit::failure("Invalid data found when processing input"),
        );
        let environment = FakeEnvironment::new(ConvertInput::new(input.clone()), output.clone());
        let cancel = AtomicBool::new(false);
        let error = run_conversion(
            &backend,
            &environment,
            &source(&input),
            ConvertFormatDto::Gif,
            &cancel,
            &mut |_| {},
        )
        .expect_err("failure");
        assert_eq!(
            error,
            ConvertError::Ffmpeg("Invalid data found when processing input".to_owned())
        );
        assert!(!output.exists());
        assert_eq!(ServiceError::from(error).code(), "internal");
    }

    #[test]
    fn missing_and_relative_file_inputs_are_rejected() {
        let services = Services::new();
        let environment = ServicesEnvironment::new(&services);
        assert_eq!(
            environment.resolve(&ConvertSource::File {
                path: "   ".to_owned()
            }),
            Err(ConvertError::MissingInput)
        );
        assert_eq!(
            environment.resolve(&ConvertSource::File {
                path: "recording.mp4".to_owned()
            }),
            Err(ConvertError::RelativePath)
        );
        let directory = tempdir().expect("directory");
        assert_eq!(
            environment.resolve(&ConvertSource::File {
                path: directory
                    .path()
                    .join("missing.mp4")
                    .to_string_lossy()
                    .into_owned()
            }),
            Err(ConvertError::NotFound)
        );
        let image = directory.path().join("shot.png");
        fs::write(&image, b"image").expect("image");
        assert_eq!(
            environment.resolve(&ConvertSource::File {
                path: image.to_string_lossy().into_owned()
            }),
            Err(ConvertError::UnsupportedFile)
        );
    }

    #[test]
    fn missing_captures_and_blank_capture_ids_are_not_found() {
        let directory = tempdir().expect("directory");
        let services = Services::with_paths(Paths::resolve(
            &directory.path().join("exe"),
            &directory.path().join("roaming"),
            &directory.path().join("local"),
            &directory.path().join("user"),
            false,
        ))
        .expect("services");
        let environment = ServicesEnvironment::new(&services);
        assert_eq!(
            environment.resolve(&ConvertSource::Capture { capture_id: 0 }),
            Err(ConvertError::NotFound)
        );
        assert_eq!(
            environment.resolve(&ConvertSource::Capture { capture_id: 7 }),
            Err(ConvertError::NotFound)
        );
    }

    #[test]
    fn a_converted_file_is_recorded_in_history() {
        let directory = tempdir().expect("directory");
        let paths = Paths::resolve(
            &directory.path().join("exe"),
            &directory.path().join("roaming"),
            &directory.path().join("local"),
            &directory.path().join("user"),
            false,
        );
        let services = Services::with_paths(paths.clone()).expect("services");
        let environment = ServicesEnvironment::new(&services);
        let input = directory.path().join("recording.mp4");
        let output = directory.path().join("recording.gif");
        fs::write(&input, b"video").expect("input");
        fs::write(&output, b"converted").expect("output");
        let mut info = ConvertInput::new(input.clone());
        info.duration_ms = Some(2_000);
        info.source_app = Some("chrome.exe".to_owned());
        let converted = environment
            .record(&info, &output, ConvertFormat::Gif(GifSettings::default()))
            .expect("record");
        assert_eq!(converted.format, ConvertFormatDto::Gif);
        assert_eq!(converted.name, "recording.gif");
        let repo = HistoryRepo::open_read(&paths.history_file).expect("read");
        let capture = repo
            .get(converted.capture_id)
            .expect("get")
            .expect("capture");
        assert_eq!(capture.kind, CaptureKind::Gif);
        assert_eq!(capture.path, output.to_string_lossy().into_owned());
        assert_eq!(capture.duration_ms, Some(2_000));
        assert_eq!(capture.source_app.as_deref(), Some("chrome.exe"));
    }

    #[test]
    fn the_output_name_avoids_the_source_and_existing_files() {
        let directory = tempdir().expect("directory");
        let input = directory.path().join("clip.mp4");
        fs::write(&input, b"video").expect("input");
        let first = output_path(&input, ConvertFormat::Mp4Small).expect("output");
        assert_eq!(first, directory.path().join("clip (2).mp4"));
        fs::write(&first, b"converted").expect("first");
        let second = output_path(&input, ConvertFormat::Mp4Small).expect("output");
        assert_eq!(second, directory.path().join("clip (3).mp4"));
        let gif = output_path(&input, ConvertFormat::Gif(GifSettings::default())).expect("output");
        assert_eq!(gif, directory.path().join("clip.gif"));
    }

    #[test]
    fn converter_requests_keep_the_existing_qr_variants() {
        let make = serde_json::to_value(crate::dto::tool::ToolReq::QrMake {
            text: "hello".to_owned(),
        })
        .expect("request");
        assert_eq!(
            make,
            serde_json::json!({ "kind": "qrMake", "data": { "text": "hello" } })
        );
        let start = serde_json::to_value(crate::dto::tool::ToolReq::ConvertStart {
            source: ConvertSource::File {
                path: r"C:\Videos\clip.mp4".to_owned(),
            },
            format: ConvertFormatDto::Mp4Small,
        })
        .expect("request");
        assert_eq!(
            start,
            serde_json::json!({
                "kind": "convertStart",
                "data": {
                    "source": { "kind": "file", "data": { "path": "C:\\Videos\\clip.mp4" } },
                    "format": "mp4Small"
                }
            })
        );
    }
}
