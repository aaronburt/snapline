use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use time::OffsetDateTime;

use crate::io;

pub mod capture;
pub mod general;
pub mod hotkeys;
pub mod migrate;
pub mod uploads;
pub mod window;
pub mod workflows;

pub use capture::Capture;
pub use general::General;
pub use hotkeys::Hotkeys;
pub use uploads::Uploads;
pub use window::Window;
pub use workflows::{default_workflow, Workflows};

pub const CURRENT_VERSION: u32 = migrate::CURRENT_VERSION;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub general: General,
    pub capture: Capture,
    pub hotkeys: Hotkeys,
    pub uploads: Uploads,
    pub workflows: Workflows,
    pub window: Window,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            general: General::default(),
            capture: Capture::default(),
            hotkeys: Hotkeys::default(),
            uploads: Uploads::default(),
            workflows: Workflows::default(),
            window: Window::default(),
            extra: toml::Table::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadNotice {
    pub message: &'static str,
    pub kept_file: PathBuf,
}

impl LoadNotice {
    pub const MESSAGE: &'static str =
        "Settings could not be read, so defaults are in use. The old file was kept.";

    pub fn new(kept_file: PathBuf) -> Self {
        Self {
            message: Self::MESSAGE,
            kept_file,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadOutcome {
    pub settings: Settings,
    pub notice: Option<LoadNotice>,
}

impl LoadOutcome {
    pub fn defaults() -> Self {
        Self {
            settings: Settings::default(),
            notice: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("could not read the settings file: {0}")]
    Read(#[source] std::io::Error),
    #[error("could not keep the unreadable settings file: {0}")]
    KeepBadFile(#[source] std::io::Error),
    #[error("could not write the settings file: {0}")]
    Write(#[source] std::io::Error),
    #[error("could not serialise the settings file: {0}")]
    Serialise(#[source] toml::ser::Error),
    #[error("settings version {0} is newer than this build supports")]
    FutureVersion(u32),
}

pub fn load(path: &Path) -> Result<LoadOutcome, SettingsError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadOutcome::defaults());
        }
        Err(error) => return Err(SettingsError::Read(error)),
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return keep_bad_file(path),
    };
    let table = match toml::from_str::<toml::Table>(&text) {
        Ok(table) => table,
        Err(_) => return keep_bad_file(path),
    };
    let version = match migrate::version_of(&table) {
        Ok(version) => version,
        Err(_) => return keep_bad_file(path),
    };
    if version > CURRENT_VERSION {
        return Err(SettingsError::FutureVersion(version));
    }
    let migrated = match migrate::migrate(table, version) {
        Ok(table) => table,
        Err(migrate::MigrationError::FutureVersion(version)) => {
            return Err(SettingsError::FutureVersion(version));
        }
        Err(_) => return keep_bad_file(path),
    };
    match Settings::deserialize(toml::Value::Table(migrated)) {
        Ok(mut settings) => {
            settings.version = CURRENT_VERSION;
            Ok(LoadOutcome {
                settings,
                notice: None,
            })
        }
        Err(_) => keep_bad_file(path),
    }
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
    let text = to_toml_string(settings)?;
    io::write_atomic(path, text.as_bytes()).map_err(SettingsError::Write)
}

pub fn load_from(paths: &io::Paths) -> Result<LoadOutcome, SettingsError> {
    load(&paths.settings_file)
}

pub fn save_to(paths: &io::Paths, settings: &Settings) -> Result<(), SettingsError> {
    save(&paths.settings_file, settings)
}

pub fn to_toml_string(settings: &Settings) -> Result<String, SettingsError> {
    let mut value = toml::Value::try_from(settings).map_err(SettingsError::Serialise)?;
    if let toml::Value::Table(table) = &mut value {
        table.insert(
            "version".to_owned(),
            toml::Value::Integer(i64::from(CURRENT_VERSION)),
        );
    }
    normalise(&mut value);
    toml::to_string_pretty(&value).map_err(SettingsError::Serialise)
}

fn normalise(value: &mut toml::Value) {
    match value {
        toml::Value::Table(table) => {
            let mut scalars = toml::Table::new();
            let mut arrays = toml::Table::new();
            let mut tables = toml::Table::new();
            for (key, mut value) in std::mem::take(table) {
                normalise(&mut value);
                match value {
                    toml::Value::Table(_) => tables.insert(key, value),
                    toml::Value::Array(_) => arrays.insert(key, value),
                    _ => scalars.insert(key, value),
                };
            }
            for (key, value) in arrays {
                scalars.insert(key, value);
            }
            for (key, value) in tables {
                scalars.insert(key, value);
            }
            *table = scalars;
        }
        toml::Value::Array(items) => {
            for item in items {
                normalise(item);
            }
        }
        _ => {}
    }
}

pub fn bad_file_path(path: &Path) -> PathBuf {
    let seconds = OffsetDateTime::now_utc().unix_timestamp();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.toml");
    path.with_file_name(format!("{name}.bad-{seconds}"))
}

fn keep_bad_file(path: &Path) -> Result<LoadOutcome, SettingsError> {
    let kept_file = bad_file_path(path);
    std::fs::copy(path, &kept_file).map_err(SettingsError::KeepBadFile)?;
    Ok(LoadOutcome {
        settings: Settings::default(),
        notice: Some(LoadNotice::new(kept_file)),
    })
}

#[cfg(test)]
mod tests {
    use super::capture::{
        CaptureDelay, FrameRate, FullscreenCapture, GifFrameRate, GifMaxWidth, MagnifierZoom,
        VideoFormat, VideoQuality,
    };
    use super::general::{ImageFormat, LinkFormat, Subfolders, WorkflowStepKind};
    use super::hotkeys::DEFAULT_BINDINGS;
    use super::uploads::{KeepHistory, Proxy, Retry, Timeout};
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn defaults_match_the_plan() {
        let settings = Settings::default();
        assert_eq!(settings.version, 2);
        assert!(settings.general.launch_at_signin);
        assert!(settings.general.start_minimised);
        assert!(settings.general.notifications);
        assert_eq!(
            settings.general.save_folder,
            "%USERPROFILE%\\Pictures\\Snapline"
        );
        assert_eq!(settings.general.subfolders, Subfolders::ByMonth);
        assert_eq!(settings.general.image_format, ImageFormat::Png);
        assert_eq!(settings.general.lossy_quality, 90);
        assert_eq!(settings.general.zoom, 100);
        assert_eq!(
            settings.general.name_pattern,
            "{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}"
        );
        assert_eq!(
            settings
                .general
                .default_workflow
                .iter()
                .map(|step| (step.kind, step.enabled))
                .collect::<Vec<_>>(),
            vec![
                (WorkflowStepKind::CopyImage, true),
                (WorkflowStepKind::Annotate, false),
                (WorkflowStepKind::Save, true),
                (WorkflowStepKind::Upload, true),
                (WorkflowStepKind::CopyLink, true),
                (WorkflowStepKind::PlaySound, true),
            ]
        );
        assert_eq!(settings.general.copied_link_format, LinkFormat::Direct);

        assert!(!settings.capture.include_cursor);
        assert_eq!(settings.capture.capture_delay, CaptureDelay::Off);
        assert!(settings.capture.shutter_sound);
        assert!(settings.capture.keep_window_shadow);
        assert!(settings.capture.freeze_screen);
        assert!(settings.capture.magnifier);
        assert_eq!(settings.capture.magnifier_zoom, MagnifierZoom::X4);
        assert!(settings.capture.snap_to_windows);
        assert!(settings.capture.show_size_and_position);
        assert!(!settings.capture.fullscreen_crosshair);
        assert_eq!(
            settings.capture.fullscreen_capture_uses,
            FullscreenCapture::MonitorUnderCursor
        );
        assert_eq!(settings.capture.video_format, VideoFormat::Mp4H264);
        assert_eq!(settings.capture.frame_rate, FrameRate::Fps30);
        assert_eq!(settings.capture.video_quality, VideoQuality::High);
        assert!(settings.capture.record_system_audio);
        assert!(!settings.capture.record_microphone);
        assert_eq!(settings.capture.microphone_device, "Default microphone");
        assert!(settings.capture.recording_border);
        assert_eq!(settings.capture.ffmpeg_path, None);
        assert_eq!(
            settings
                .capture
                .resolved_ffmpeg_path(Path::new("C:\\Apps\\Snapline")),
            PathBuf::from("C:\\Apps\\Snapline\\ffmpeg.exe")
        );
        assert_eq!(settings.capture.gif_frame_rate, GifFrameRate::Fps15);
        assert_eq!(settings.capture.gif_max_width, GifMaxWidth::Px800);
        assert!(settings.capture.gif_optimise_palette);

        assert_eq!(settings.hotkeys.bindings.len(), DEFAULT_BINDINGS.len());
        for (action, chord) in DEFAULT_BINDINGS {
            assert_eq!(
                settings.hotkeys.bindings.get(*action).map(String::as_str),
                Some(*chord),
                "{action}"
            );
        }
        assert!(settings.hotkeys.take_over_prtsc);
        assert!(settings.hotkeys.warn_about_clashes);
        assert_eq!(settings.hotkeys.restore, super::hotkeys::Restore::default());

        assert_eq!(
            settings.uploads.images.as_str(),
            uploads::DestinationRef::SAVE_LOCALLY_ONLY
        );
        assert_eq!(
            settings.uploads.videos_and_gifs.as_str(),
            uploads::DestinationRef::SAVE_LOCALLY_ONLY
        );
        assert_eq!(
            settings.uploads.text_and_code.as_str(),
            uploads::DestinationRef::SAVE_LOCALLY_ONLY
        );
        assert_eq!(
            settings.uploads.other_files.as_str(),
            uploads::DestinationRef::SAVE_LOCALLY_ONLY
        );
        assert_eq!(
            settings.uploads.link_shortener.as_str(),
            uploads::DestinationRef::NONE
        );
        assert!(settings.uploads.destinations.is_empty());
        assert!(settings.uploads.strip_exif);
        assert!(settings.uploads.ask_before_clipboard_upload);
        assert_eq!(settings.uploads.retry, Retry::ThreeTimes);
        assert_eq!(settings.uploads.timeout, Timeout::Secs60);
        assert_eq!(settings.uploads.keep_history, KeepHistory::Forever);
        assert_eq!(settings.uploads.proxy, Proxy::System);
        assert_eq!(settings.uploads.proxy_url, None);

        assert!(settings.workflows.list.is_empty());
        assert!(settings.window.main.is_empty());
    }

    #[test]
    fn settings_round_trip() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        let mut settings = Settings::default();
        settings.general.notifications = false;
        settings.general.lossy_quality = 75;
        settings.capture.include_cursor = true;
        settings.capture.video_quality = VideoQuality::Balanced;
        settings.capture.ffmpeg_path = Some(PathBuf::from("C:\\Tools\\ffmpeg.exe"));
        settings
            .hotkeys
            .bindings
            .insert("capture_region".to_owned(), "Ctrl+Shift+1".to_owned());
        settings.uploads.retry = Retry::Once;
        settings.uploads.proxy = Proxy::Custom;
        settings.uploads.proxy_url = Some("http://127.0.0.1:8080".to_owned());
        settings.window.main.x = Some(120);
        settings.window.main.y = Some(80);
        settings.window.main.width = Some(1280);
        settings.window.main.height = Some(880);

        save(&path, &settings).expect("save");
        let loaded = load(&path).expect("load");
        assert!(loaded.notice.is_none());
        assert_eq!(loaded.settings, settings);
    }

    #[test]
    fn unknown_keys_are_kept_on_save() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(
            &path,
            concat!(
                "version = 1\n",
                "unknown_top = \"kept\"\n",
                "[general]\n",
                "notifications = false\n",
                "unknown_general = 7\n",
                "[capture]\n",
                "unknown_capture = true\n",
            ),
        )
        .expect("write");
        let loaded = load(&path).expect("load");
        assert!(!loaded.settings.general.notifications);
        let output = directory.path().join("saved.toml");
        save(&output, &loaded.settings).expect("save");
        let text = std::fs::read_to_string(&output).expect("read");
        assert!(text.contains("unknown_top"));
        assert!(text.contains("unknown_general"));
        assert!(text.contains("unknown_capture"));
        let reloaded = load(&output).expect("reload");
        assert_eq!(reloaded.settings, loaded.settings);
    }

    #[test]
    fn a_bad_file_is_kept_aside_and_defaults_are_used() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "this is not toml\n[general\n").expect("write");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded.settings, Settings::default());
        let notice = loaded.notice.expect("notice");
        assert_eq!(
            notice.message,
            "Settings could not be read, so defaults are in use. The old file was kept."
        );
        assert!(notice.kept_file.exists());
        let name = notice
            .kept_file
            .file_name()
            .expect("file name")
            .to_string_lossy();
        assert!(name.starts_with("settings.toml.bad-"));
    }

    #[test]
    fn a_missing_file_is_defaults_without_a_notice() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.notice.is_none());
    }

    #[test]
    fn a_version_one_file_gains_the_default_workflow() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(
            &path,
            concat!(
                "version = 1\n",
                "[general]\n",
                "notifications = false\n",
                "[workflows]\n",
                "unknown_workflows = \"kept\"\n",
            ),
        )
        .expect("write");

        let loaded = load(&path).expect("load");
        assert!(loaded.notice.is_none());
        assert_eq!(loaded.settings.version, 2);
        assert_eq!(loaded.settings.workflows.list.len(), 5);
        let workflow = &loaded.settings.workflows.list[0];
        assert_eq!(
            workflow.id.as_str(),
            snapline_core::workflow::DEFAULT_WORKFLOW_ID
        );
        assert_eq!(
            workflow.name,
            snapline_core::workflow::DEFAULT_WORKFLOW_NAME
        );
        assert!(workflow.enabled);
        assert_eq!(workflow.hotkey, None);
        assert_eq!(
            workflow.steps,
            vec![
                snapline_core::workflow::Step::CopyImage,
                snapline_core::workflow::Step::Save,
                snapline_core::workflow::Step::Upload { dest: None },
                snapline_core::workflow::Step::CopyLink {
                    format: snapline_core::link_format::LinkFormat::Direct,
                },
                snapline_core::workflow::Step::PlaySound,
            ]
        );
        assert_eq!(
            loaded.settings.workflows.extra.get("unknown_workflows"),
            Some(&toml::Value::String("kept".to_owned()))
        );
    }

    #[test]
    fn repeated_loads_do_not_duplicate_the_default_workflow() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "version = 1\n").expect("write");

        let first = load(&path).expect("first load");
        let second = load(&path).expect("second load");
        assert_eq!(first.settings.workflows.list.len(), 5);
        assert_eq!(second.settings.workflows.list.len(), 5);
        assert_eq!(first.settings, second.settings);

        save(&path, &first.settings).expect("save");
        let reloaded = load(&path).expect("reload");
        assert_eq!(reloaded.settings.workflows.list.len(), 5);
        assert_eq!(reloaded.settings, first.settings);
    }

    #[test]
    fn a_version_two_file_keeps_an_empty_workflow_list() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "version = 2\n[workflows]\nlist = []\n").expect("write");

        let loaded = load(&path).expect("load");
        assert!(loaded.notice.is_none());
        assert_eq!(loaded.settings.version, 2);
        assert!(loaded.settings.workflows.list.is_empty());
    }

    #[test]
    fn a_file_from_before_scrolling_capture_was_removed_still_loads() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(
            &path,
            r#"version = 2

[capture]
scroll_delay_ms = 500
scroll_limit = "ten"
remove_sticky_headers = false

[hotkeys.bindings]
capture_scrolling = "Ctrl+Alt+S"
capture_region = "PrtSc"

[[workflows.list]]
id = "old-scroll"
name = "Old scroll"
enabled = true
steps = [{ step = "capture", kind = "scrolling" }, { step = "save" }]
"#,
        )
        .expect("write");

        let loaded = load(&path).expect("load");
        assert!(loaded.notice.is_none());
        assert_eq!(
            loaded.settings.hotkeys.bindings.get("capture_region"),
            Some(&"PrtSc".to_owned())
        );
        assert_eq!(
            loaded.settings.workflows.list[0].steps,
            vec![
                snapline_core::workflow::Step::Capture {
                    kind: snapline_core::workflow::CaptureKind::Region
                },
                snapline_core::workflow::Step::Save,
            ]
        );
        assert!(!DEFAULT_BINDINGS
            .iter()
            .any(|(action, _)| *action == "capture_scrolling"));
    }
}
