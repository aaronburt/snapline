#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use snapline_core::ports::ImageCodec;
use snapline_store::io::Paths;
use snapline_store::settings::{self as store_settings, LoadNotice, Settings};
use thiserror::Error;
use ts_rs::TS;

pub mod actions;
pub mod app;
pub mod capture;
pub mod clipboard_upload;
pub mod destinations;
pub mod dto;
pub mod editor;
pub mod event;
pub mod history;
pub mod hotkeys;
pub mod intake;
pub mod job;
pub mod recording;
pub mod saver;
pub mod settings;
pub mod shell_actions;
pub mod thumbs;
pub mod tools;
pub mod uninstall;
pub mod uploads;
pub mod worker;
pub mod workflow_runner;
pub mod workflows;

pub use event::AppEvent;
pub use job::{AreaJob, Job};

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ServiceError {
    #[error("This part of Snapline is not built yet.")]
    NotYet,
    #[error("{0}")]
    Invalid(String),
    #[error("The requested item was not found.")]
    NotFound,
    #[error("Snapline is busy. Try again.")]
    Busy,
    #[error("The request was not authorised.")]
    Auth,
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Internal(String),
}

impl ServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotYet | Self::Internal(_) => "internal",
            Self::Invalid(_) => "invalid",
            Self::NotFound => "not_found",
            Self::Busy => "busy",
            Self::Auth => "auth",
            Self::Io(_) => "io",
            Self::Network(_) => "network",
        }
    }
}

pub trait FolderPicker: Send + Sync {
    fn pick_folder(&self, current: Option<&Path>) -> Result<Option<PathBuf>, ServiceError>;
}

pub trait FilePicker: Send + Sync {
    fn pick_files(&self) -> Result<Vec<PathBuf>, ServiceError>;

    fn pick_ffmpeg(&self) -> Result<Option<PathBuf>, ServiceError> {
        Err(ServiceError::NotYet)
    }
}

pub type CodecFactory = dyn Fn() -> Result<Arc<dyn ImageCodec>, ServiceError> + Send + Sync;

struct SettingsState {
    paths: Option<Paths>,
    settings: Settings,
    notice: Option<LoadNotice>,
}

struct Inner {
    settings: Mutex<SettingsState>,
    picker: Mutex<Option<Arc<dyn FolderPicker>>>,
    file_picker: Mutex<Option<Arc<dyn FilePicker>>>,
    hotkeys: Mutex<hotkeys::HotkeysRuntime>,
    codec: Mutex<Option<Arc<CodecFactory>>>,
    history_host: Mutex<Option<Arc<dyn history::HistoryHost>>>,
    upload: Mutex<Option<uploads::UploadRuntime>>,
}

#[derive(Clone)]
pub struct Services {
    inner: Arc<Inner>,
}

impl fmt::Debug for Services {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Services").finish_non_exhaustive()
    }
}

impl Default for Services {
    fn default() -> Self {
        Self::new()
    }
}

impl Services {
    pub fn new() -> Self {
        Self::from_state(SettingsState {
            paths: None,
            settings: Settings::default(),
            notice: None,
        })
    }

    pub fn with_paths(paths: Paths) -> Result<Self, ServiceError> {
        let outcome = store_settings::load_from(&paths).map_err(settings_error)?;
        if let Some(notice) = &outcome.notice {
            tracing::warn!(
                message = notice.message,
                kept_file = %notice.kept_file.display(),
                "settings could not be read"
            );
        }
        Ok(Self::from_state(SettingsState {
            paths: Some(paths),
            settings: outcome.settings,
            notice: outcome.notice,
        }))
    }

    pub fn with_folder_picker(self, picker: Arc<dyn FolderPicker>) -> Self {
        if let Ok(mut slot) = self.inner.picker.lock() {
            *slot = Some(picker);
        }
        self
    }

    pub fn with_file_picker(self, picker: Arc<dyn FilePicker>) -> Self {
        if let Ok(mut slot) = self.inner.file_picker.lock() {
            *slot = Some(picker);
        }
        self
    }

    pub fn with_hotkey_input(self, input: Arc<dyn hotkeys::HotkeyInput>) -> Self {
        if let Ok(mut slot) = self.inner.hotkeys.lock() {
            slot.input = Some(input);
        }
        self
    }

    pub fn with_prtsc_registry(self, registry: Arc<dyn hotkeys::PrtscRegistry>) -> Self {
        if let Ok(mut slot) = self.inner.hotkeys.lock() {
            slot.prtsc = Some(registry);
        }
        self
    }

    pub fn with_notifier(
        self,
        notifier: Arc<dyn snapline_core::ports::Notifier + Send + Sync>,
    ) -> Self {
        if let Ok(mut slot) = self.inner.hotkeys.lock() {
            slot.notifier = Some(notifier);
        }
        self
    }

    pub fn with_codec_factory(self, factory: Arc<CodecFactory>) -> Self {
        if let Ok(mut slot) = self.inner.codec.lock() {
            *slot = Some(factory);
        }
        self
    }

    pub fn with_history_host(self, host: Arc<dyn history::HistoryHost>) -> Self {
        if let Ok(mut slot) = self.inner.history_host.lock() {
            *slot = Some(host);
        }
        self
    }

    pub(crate) fn codec(&self) -> Result<Arc<dyn ImageCodec>, ServiceError> {
        let factory = self
            .inner
            .codec
            .lock()
            .map_err(|_| ServiceError::Internal("The image codec is unavailable.".to_owned()))?
            .clone()
            .ok_or(ServiceError::NotYet)?;
        factory()
    }

    pub(crate) fn history_host(&self) -> Option<Arc<dyn history::HistoryHost>> {
        self.inner
            .history_host
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
    }

    pub fn with_upload_runtime(self, runtime: uploads::UploadRuntime) -> Self {
        if let Ok(mut slot) = self.inner.upload.lock() {
            *slot = Some(runtime);
        }
        self
    }

    pub(crate) fn upload_runtime(&self) -> Option<uploads::UploadRuntime> {
        self.inner.upload.lock().ok().and_then(|slot| slot.clone())
    }

    pub(crate) fn hotkeys_state(
        &self,
    ) -> Result<MutexGuard<'_, hotkeys::HotkeysRuntime>, ServiceError> {
        self.inner
            .hotkeys
            .lock()
            .map_err(|_| ServiceError::Internal("Hotkeys are unavailable.".to_owned()))
    }

    fn from_state(state: SettingsState) -> Self {
        Self {
            inner: Arc::new(Inner {
                settings: Mutex::new(state),
                picker: Mutex::new(None),
                file_picker: Mutex::new(None),
                hotkeys: Mutex::new(hotkeys::HotkeysRuntime::default()),
                codec: Mutex::new(None),
                history_host: Mutex::new(None),
                upload: Mutex::new(None),
            }),
        }
    }

    fn lock_settings(&self) -> Result<MutexGuard<'_, SettingsState>, ServiceError> {
        self.inner
            .settings
            .lock()
            .map_err(|_| ServiceError::Internal("Settings are unavailable.".to_owned()))
    }

    pub fn settings_snapshot(&self) -> Result<Settings, ServiceError> {
        Ok(self.lock_settings()?.settings.clone())
    }

    pub fn paths(&self) -> Result<Paths, ServiceError> {
        self.lock_settings()?
            .paths
            .clone()
            .ok_or_else(|| ServiceError::Io("Snapline has no data folder.".to_owned()))
    }

    pub fn settings_notice(&self) -> Result<Option<LoadNotice>, ServiceError> {
        Ok(self.lock_settings()?.notice.clone())
    }

    pub fn update_settings(&self, patch: &serde_json::Value) -> Result<Settings, ServiceError> {
        let Some(object) = patch.as_object() else {
            return Err(ServiceError::Invalid(
                "Settings changes must be an object.".to_owned(),
            ));
        };
        let protected_capture_path = object
            .get("capture")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|capture| capture.contains_key("ffmpeg_path"));
        if protected_capture_path
            || object.contains_key("programs")
            || object.contains_key("watch_folders")
        {
            return Err(ServiceError::Invalid(
                "Choose protected file paths with their native picker.".to_owned(),
            ));
        }
        self.apply_settings_patch(patch)
    }

    pub(crate) fn update_ffmpeg_path(
        &self,
        path: Option<PathBuf>,
    ) -> Result<Settings, ServiceError> {
        if path
            .as_ref()
            .is_some_and(|path| !path.is_absolute() || !path.is_file())
        {
            return Err(ServiceError::Invalid(
                "Choose an existing FFmpeg executable.".to_owned(),
            ));
        }
        self.apply_settings_patch(&serde_json::json!({
            "capture": { "ffmpeg_path": path.map(|path| path.to_string_lossy().into_owned()) }
        }))
    }

    fn apply_settings_patch(&self, patch: &serde_json::Value) -> Result<Settings, ServiceError> {
        let mut state = self.lock_settings()?;
        let Some(object) = patch.as_object() else {
            return Err(ServiceError::Invalid(
                "Settings changes must be an object.".to_owned(),
            ));
        };
        let current = store_settings::to_toml_string(&state.settings).map_err(settings_error)?;
        let mut table: toml::Table = toml::from_str(&current).map_err(|error| {
            ServiceError::Internal(format!("The settings could not be prepared: {error}"))
        })?;
        merge_patch(&mut table, object)?;
        let mut settings: Settings = serde::Deserialize::deserialize(toml::Value::Table(table))
            .map_err(|error| {
                ServiceError::Invalid(format!("A settings value was not valid: {error}"))
            })?;
        if object
            .get("general")
            .and_then(|value| value.get("default_workflow"))
            .is_some()
        {
            let before =
                workflow_runner::default_from_general(&state.settings.general.default_workflow);
            let after = workflow_runner::default_from_general(&settings.general.default_workflow);
            if let Some(default) = settings
                .workflows
                .list
                .iter_mut()
                .find(|workflow| workflow.is_default())
            {
                for step in &before.steps {
                    if !after.steps.iter().any(|candidate| {
                        std::mem::discriminant(candidate) == std::mem::discriminant(step)
                    }) {
                        default.steps.retain(|candidate| {
                            std::mem::discriminant(candidate) != std::mem::discriminant(step)
                        });
                    }
                }
                for (index, step) in after.steps.iter().enumerate() {
                    if !default.steps.iter().any(|candidate| {
                        std::mem::discriminant(candidate) == std::mem::discriminant(step)
                    }) {
                        let position = after.steps[index + 1..]
                            .iter()
                            .find_map(|next| {
                                default.steps.iter().position(|candidate| {
                                    std::mem::discriminant(candidate)
                                        == std::mem::discriminant(next)
                                })
                            })
                            .unwrap_or(default.steps.len());
                        default.steps.insert(position, step.clone());
                    }
                }
            }
        }
        settings.version = store_settings::CURRENT_VERSION;
        let paths = state.paths.as_ref().ok_or_else(|| {
            ServiceError::Io(
                "Settings cannot be saved because Snapline has no settings folder.".to_owned(),
            )
        })?;
        store_settings::save_to(paths, &settings).map_err(settings_error)?;
        state.settings = settings.clone();
        state.notice = None;
        Ok(settings)
    }

    pub fn pick_folder(&self, current: Option<&str>) -> Result<Option<String>, ServiceError> {
        let picker = self
            .inner
            .picker
            .lock()
            .map_err(|_| ServiceError::Internal("The folder picker is unavailable.".to_owned()))?
            .clone()
            .ok_or(ServiceError::NotYet)?;
        let path = picker.pick_folder(current.map(Path::new))?;
        Ok(path.map(|path| path.to_string_lossy().into_owned()))
    }

    pub(crate) fn pick_files(&self) -> Result<Vec<PathBuf>, ServiceError> {
        let picker = self
            .inner
            .file_picker
            .lock()
            .map_err(|_| ServiceError::Internal("The file picker is unavailable.".to_owned()))?
            .clone()
            .ok_or(ServiceError::NotYet)?;
        picker.pick_files()
    }

    pub(crate) fn pick_ffmpeg(&self) -> Result<Option<PathBuf>, ServiceError> {
        let picker = self
            .inner
            .file_picker
            .lock()
            .map_err(|_| ServiceError::Internal("The file picker is unavailable.".to_owned()))?
            .clone()
            .ok_or(ServiceError::NotYet)?;
        picker.pick_ffmpeg()
    }

    pub fn notifier(&self) -> Option<Arc<dyn snapline_core::ports::Notifier + Send + Sync>> {
        self.inner
            .hotkeys
            .lock()
            .ok()
            .and_then(|runtime| runtime.notifier.clone())
    }

    pub fn app(&self) -> app::AppService {
        app::AppService::new(self)
    }

    pub fn settings(&self) -> settings::SettingsService {
        settings::SettingsService::new(self)
    }

    pub fn hotkeys(&self) -> hotkeys::HotkeysService {
        hotkeys::HotkeysService::new(self)
    }

    pub fn history(&self) -> history::HistoryService {
        history::HistoryService::new(self)
    }

    pub fn thumbs(&self) -> thumbs::ThumbService {
        thumbs::ThumbService::new(self)
    }

    pub fn upload(&self) -> uploads::UploadService {
        match self.upload_runtime() {
            Some(runtime) => uploads::UploadService::with_runtime(self, runtime),
            None => uploads::UploadService::new(self),
        }
    }

    pub fn workflow(&self) -> workflows::WorkflowService {
        workflows::WorkflowService::new(self)
    }

    pub fn record(&self) -> recording::RecordService {
        recording::RecordService::new(self)
    }

    pub fn tool(&self) -> tools::ToolService {
        tools::ToolService::new(self)
    }
}

fn settings_error(error: store_settings::SettingsError) -> ServiceError {
    ServiceError::Io(error.to_string())
}

fn merge_patch(
    target: &mut toml::Table,
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ServiceError> {
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
            continue;
        }
        match value {
            serde_json::Value::Object(object) => {
                if let Some(toml::Value::Table(existing)) = target.get_mut(key) {
                    merge_patch(existing, object)?;
                } else {
                    let mut table = toml::Table::new();
                    merge_patch(&mut table, object)?;
                    target.insert(key.clone(), toml::Value::Table(table));
                }
            }
            other => {
                target.insert(key.clone(), json_to_toml(other)?);
            }
        }
    }
    Ok(())
}

fn json_to_toml(value: &serde_json::Value) -> Result<toml::Value, ServiceError> {
    match value {
        serde_json::Value::Bool(value) => Ok(toml::Value::Boolean(*value)),
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(toml::Value::Integer(value))
            } else if let Some(value) = number.as_f64() {
                Ok(toml::Value::Float(value))
            } else {
                Err(ServiceError::Invalid(
                    "A settings number was not valid.".to_owned(),
                ))
            }
        }
        serde_json::Value::String(value) => Ok(toml::Value::String(value.clone())),
        serde_json::Value::Array(items) => items
            .iter()
            .map(json_to_toml)
            .collect::<Result<Vec<_>, _>>()
            .map(toml::Value::Array),
        serde_json::Value::Object(object) => {
            let mut table = toml::Table::new();
            for (key, item) in object {
                if item.is_null() {
                    continue;
                }
                table.insert(key.clone(), json_to_toml(item)?);
            }
            Ok(toml::Value::Table(table))
        }
        serde_json::Value::Null => Err(ServiceError::Invalid(
            "A settings value cannot be empty.".to_owned(),
        )),
    }
}

pub fn export_bindings(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let config = ts_rs::Config::new().with_out_dir(dir);
    dto::app::AppReq::export_all(&config)?;
    dto::app::AppResp::export_all(&config)?;
    dto::settings::SettingsReq::export_all(&config)?;
    dto::settings::SettingsResp::export_all(&config)?;
    dto::hotkeys::HotkeysReq::export_all(&config)?;
    dto::hotkeys::HotkeysResp::export_all(&config)?;
    dto::history::HistoryReq::export_all(&config)?;
    dto::history::HistoryResp::export_all(&config)?;
    dto::upload::UploadReq::export_all(&config)?;
    dto::upload::UploadResp::export_all(&config)?;
    dto::workflow::WorkflowReq::export_all(&config)?;
    dto::workflow::WorkflowResp::export_all(&config)?;
    dto::record::RecordReq::export_all(&config)?;
    dto::record::RecordResp::export_all(&config)?;
    dto::tool::ToolReq::export_all(&config)?;
    dto::tool::ToolResp::export_all(&config)?;
    AppEvent::export_all(&config)?;
    Ok(())
}
