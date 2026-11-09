use std::path::{Path, PathBuf};
use std::sync::Arc;

use snapline_core::image::{Image, ImageFormat};
use snapline_core::naming::{self, NameContext, NameFormat, TokenSource};
use snapline_core::ports::{Clipboard, Clock, CodecError, ImageCodec, Notice, Notifier, Sound};
use snapline_store::history::{CaptureId, CaptureKind, HistoryRepo, NewCapture};
use snapline_store::io;
use snapline_store::settings::general::{
    General, ImageFormat as SettingsImageFormat, Subfolders, WorkflowStep, WorkflowStepKind,
};
use snapline_store::settings::Settings;
use thiserror::Error;
use time::OffsetDateTime;

use crate::ServiceError;

pub const THUMBNAIL_WIDTH: u32 = 320;
pub const COUNTER_KEY: &str = "counter";
pub const SAVED_TITLE: &str = "Saved";

#[derive(Clone)]
pub struct ServicePorts {
    pub codec: Arc<dyn ImageCodec>,
    pub clipboard: Arc<dyn Clipboard>,
    pub notifier: Arc<dyn Notifier>,
    pub sound: Arc<dyn Sound>,
    pub clock: Arc<dyn Clock>,
}

impl ServicePorts {
    pub fn new(
        codec: Arc<dyn ImageCodec>,
        clipboard: Arc<dyn Clipboard>,
        notifier: Arc<dyn Notifier>,
        sound: Arc<dyn Sound>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            codec,
            clipboard,
            notifier,
            sound,
            clock,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_local(&self) -> OffsetDateTime {
        OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
    }
}

pub trait SettingsAccess: Send + Sync {
    fn settings(&self) -> Result<Settings, ServiceError>;
    fn set_counter(&self, value: u32) -> Result<(), ServiceError>;
}

impl SettingsAccess for crate::Services {
    fn settings(&self) -> Result<Settings, ServiceError> {
        self.settings_snapshot()
    }

    fn set_counter(&self, value: u32) -> Result<(), ServiceError> {
        self.update_settings(&serde_json::json!({
            "general": { "counter": i64::from(value) }
        }))
        .map(|_| ())
    }
}

pub trait RandomSource: Send + Sync {
    fn chars(&self) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadRandomSource;

impl RandomSource for ThreadRandomSource {
    fn chars(&self) -> String {
        naming::ThreadRandom.random_chars()
    }
}

struct NamingRandom<'a>(&'a dyn RandomSource);

impl TokenSource for NamingRandom<'_> {
    fn random_chars(&self) -> String {
        self.0.chars()
    }
}

#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub image: Image,
    pub source_app: Option<String>,
    pub window_title: String,
    pub(crate) workflow_save: bool,
}

impl CaptureRequest {
    pub fn new(image: Image, source_app: Option<String>, window_title: impl Into<String>) -> Self {
        Self {
            image,
            source_app,
            window_title: window_title.into(),
            workflow_save: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOutcome {
    pub capture_id: Option<CaptureId>,
    pub path: Option<PathBuf>,
    pub file_name: Option<String>,
    pub bytes: i64,
    pub width: u32,
    pub height: u32,
    pub copied: bool,
    pub saved: bool,
    pub next_counter: Option<u32>,
}

impl SaveOutcome {
    fn unsaved(copied: bool, width: u32, height: u32) -> Self {
        Self {
            capture_id: None,
            path: None,
            file_name: None,
            bytes: 0,
            width,
            height,
            copied,
            saved: false,
            next_counter: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("{0}")]
    Settings(String),
    #[error("{0}")]
    Codec(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    History(String),
}

impl From<SaveError> for ServiceError {
    fn from(error: SaveError) -> Self {
        match error {
            SaveError::Settings(message) => Self::Internal(message),
            SaveError::Codec(message) | SaveError::Io(message) | SaveError::History(message) => {
                Self::Io(message)
            }
        }
    }
}

pub struct Saver {
    settings: Arc<dyn SettingsAccess>,
    ports: ServicePorts,
    thumbs_dir: PathBuf,
    user_profile: PathBuf,
    random: Arc<dyn RandomSource>,
}

impl Saver {
    pub fn new(
        settings: Arc<dyn SettingsAccess>,
        ports: ServicePorts,
        thumbs_dir: impl Into<PathBuf>,
        user_profile: impl Into<PathBuf>,
    ) -> Self {
        Self {
            settings,
            ports,
            thumbs_dir: thumbs_dir.into(),
            user_profile: user_profile.into(),
            random: Arc::new(ThreadRandomSource),
        }
    }

    pub fn with_random_source(mut self, random: Arc<dyn RandomSource>) -> Self {
        self.random = random;
        self
    }

    pub fn run(
        &self,
        repo: &mut HistoryRepo,
        request: CaptureRequest,
    ) -> Result<SaveOutcome, SaveError> {
        let settings = self
            .settings
            .settings()
            .map_err(|error| SaveError::Settings(error.to_string()))?;
        let general = &settings.general;
        let now = self.ports.clock.now_local();
        let image = request.image;
        let copied = if !request.workflow_save
            && step_enabled(&general.default_workflow, WorkflowStepKind::CopyImage)
        {
            match self.ports.clipboard.set_image(&image) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "the capture could not be copied to the clipboard");
                    false
                }
            }
        } else {
            false
        };

        if !request.workflow_save
            && !step_enabled(&general.default_workflow, WorkflowStepKind::Save)
        {
            return Ok(SaveOutcome::unsaved(copied, image.width(), image.height()));
        }

        let counter = naming::next_counter(counter_from(general));
        let format = image_format(general.image_format);
        let quality = quality_for(format, general.lossy_quality);
        let folder = self.folder(general, request.source_app.as_deref(), now);
        let context = NameContext {
            app: request.source_app.clone(),
            window: request.window_title,
            timestamp: now,
            counter,
            width: image.width(),
            height: image.height(),
        };
        let source = NamingRandom(self.random.as_ref());
        let file_name = naming::file_name(
            &general.name_pattern,
            &context,
            &source,
            naming_format(format),
        );
        let file_name =
            naming::resolve_collision(&file_name, |candidate| folder.join(candidate).exists());
        let path = folder.join(&file_name);

        let encoded = self
            .ports
            .codec
            .encode(&image, format, quality)
            .map_err(|error| SaveError::Codec(error.to_string()))?;
        let bytes = i64::try_from(encoded.len())
            .map_err(|_| SaveError::Io("The encoded image is too large.".to_owned()))?;
        io::write_atomic(&path, &encoded).map_err(|error| SaveError::Io(error.to_string()))?;

        let next_counter = counter;
        if let Err(error) = self.settings.set_counter(next_counter) {
            tracing::warn!(%error, "the capture counter could not be saved");
        }

        let created_at = now.unix_timestamp().saturating_mul(1000) + i64::from(now.millisecond());
        let id = repo
            .record(NewCapture {
                kind: CaptureKind::Image,
                path: path.to_string_lossy().into_owned(),
                bytes,
                width: Some(i64::from(image.width())),
                height: Some(i64::from(image.height())),
                duration_ms: None,
                source_app: request.source_app.clone(),
                created_at,
                ocr_text: None,
            })
            .map_err(|error| SaveError::History(error.to_string()))?;

        if let Err(error) = self.write_thumbnail(id, &image, quality) {
            tracing::warn!(%error, "the thumbnail could not be written");
        }

        if general.notifications {
            self.ports
                .notifier
                .toast(Notice::for_capture(SAVED_TITLE, &file_name, id));
        }

        Ok(SaveOutcome {
            capture_id: Some(id),
            path: Some(path),
            file_name: Some(file_name),
            bytes,
            width: image.width(),
            height: image.height(),
            copied,
            saved: true,
            next_counter: Some(next_counter),
        })
    }

    fn folder(&self, general: &General, source_app: Option<&str>, now: OffsetDateTime) -> PathBuf {
        let mut folder = PathBuf::from(expand_profile(&general.save_folder, &self.user_profile));
        match general.subfolders {
            Subfolders::ByMonth => {
                folder.push(format!("{:04}-{:02}", now.year(), now.month() as u8))
            }
            Subfolders::ByDay => folder.push(format!(
                "{:04}-{:02}-{:02}",
                now.year(),
                now.month() as u8,
                now.day()
            )),
            Subfolders::ByApp => {
                folder.push(naming::sanitise_file_name(&naming::app_label(source_app)))
            }
            Subfolders::None => {}
        }
        folder
    }

    fn write_thumbnail(
        &self,
        id: CaptureId,
        image: &Image,
        quality: u8,
    ) -> Result<PathBuf, SaveError> {
        let thumbnail = scale_to_width(image, THUMBNAIL_WIDTH)?;
        let (bytes, extension) =
            match self
                .ports
                .codec
                .encode(&thumbnail, ImageFormat::WebP, quality)
            {
                Ok(bytes) => (bytes, "webp"),
                Err(CodecError::Unsupported) => {
                    let bytes = self
                        .ports
                        .codec
                        .encode(&thumbnail, ImageFormat::Jpeg, quality)
                        .map_err(|error| SaveError::Codec(error.to_string()))?;
                    (bytes, "jpg")
                }
                Err(error) => return Err(SaveError::Codec(error.to_string())),
            };
        let path = self.thumbs_dir.join(format!("{id}.{extension}"));
        io::write_atomic(&path, &bytes).map_err(|error| SaveError::Io(error.to_string()))?;
        Ok(path)
    }
}

pub fn expand_profile(value: &str, profile: &Path) -> String {
    value.replace("%USERPROFILE%", &profile.to_string_lossy())
}

pub fn step_enabled(steps: &[WorkflowStep], kind: WorkflowStepKind) -> bool {
    steps.iter().any(|step| step.kind == kind && step.enabled)
}

pub fn counter_from(general: &General) -> u32 {
    general
        .extra
        .get(COUNTER_KEY)
        .and_then(toml::Value::as_integer)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0)
}

fn image_format(format: SettingsImageFormat) -> ImageFormat {
    match format {
        SettingsImageFormat::Png => ImageFormat::Png,
        SettingsImageFormat::Jpeg => ImageFormat::Jpeg,
        SettingsImageFormat::Webp => ImageFormat::WebP,
    }
}

fn quality_for(format: ImageFormat, lossy_quality: u8) -> u8 {
    match format {
        ImageFormat::Png => 100,
        ImageFormat::Jpeg | ImageFormat::WebP => lossy_quality,
    }
}

fn naming_format(format: ImageFormat) -> NameFormat {
    match format {
        ImageFormat::Png => NameFormat::Png,
        ImageFormat::Jpeg => NameFormat::Jpeg,
        ImageFormat::WebP => NameFormat::WebP,
    }
}
pub fn scale_to_width(image: &Image, target: u32) -> Result<Image, SaveError> {
    let width = image.width();
    let height = image.height();
    let target = target.max(1);
    if width <= target {
        return Ok(image.clone());
    }
    let target_height = u32::try_from(
        (u64::from(height) * u64::from(target) + u64::from(width) / 2) / u64::from(width),
    )
    .map_err(|_| SaveError::Io("The thumbnail dimensions are not supported.".to_owned()))?
    .max(1);
    let mut output =
        Image::bgra(target, target_height).map_err(|error| SaveError::Io(error.to_string()))?;
    let source = image.pixels();
    let stride = image.stride();
    for y in 0..target_height {
        let start_y = u64::from(y) * u64::from(height) / u64::from(target_height);
        let end_y = (((u64::from(y) + 1) * u64::from(height) / u64::from(target_height))
            .max(start_y + 1))
        .min(u64::from(height));
        for x in 0..target {
            let start_x = u64::from(x) * u64::from(width) / u64::from(target);
            let end_x = (((u64::from(x) + 1) * u64::from(width) / u64::from(target))
                .max(start_x + 1))
            .min(u64::from(width));
            let mut sums = [0_u64; 4];
            let mut count = 0_u64;
            for source_y in start_y..end_y {
                let row = usize::try_from(source_y)
                    .map_err(|_| SaveError::Io("The thumbnail row is not supported.".to_owned()))?
                    .saturating_mul(stride);
                for source_x in start_x..end_x {
                    let offset = row
                        + usize::try_from(source_x)
                            .map_err(|_| {
                                SaveError::Io("The thumbnail column is not supported.".to_owned())
                            })?
                            .saturating_mul(4);
                    let pixel = source.get(offset..offset + 4).ok_or_else(|| {
                        SaveError::Io("The image buffer is too short for a thumbnail.".to_owned())
                    })?;
                    for channel in 0..4 {
                        sums[channel] += u64::from(pixel[channel]);
                    }
                    count += 1;
                }
            }
            let count = count.max(1);
            let pixel = [
                u8::try_from(sums[0] / count).unwrap_or(0),
                u8::try_from(sums[1] / count).unwrap_or(0),
                u8::try_from(sums[2] / count).unwrap_or(0),
                u8::try_from(sums[3] / count).unwrap_or(0),
            ];
            output
                .set_pixel(x, y, pixel)
                .map_err(|error| SaveError::Io(error.to_string()))?;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use snapline_core::ports::{ClipboardError, Notice};
    use snapline_store::history::{HistoryFilter, HistoryRepo};
    use tempfile::TempDir;
    use time::macros::datetime;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Call {
        format: ImageFormat,
        quality: u8,
        width: u32,
        height: u32,
    }

    struct FakeCodec {
        calls: Mutex<Vec<Call>>,
        fail: AtomicBool,
        webp: AtomicBool,
    }

    impl FakeCodec {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail: AtomicBool::new(false),
                webp: AtomicBool::new(true),
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl ImageCodec for FakeCodec {
        fn encode(
            &self,
            image: &Image,
            format: ImageFormat,
            quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            self.calls.lock().expect("calls").push(Call {
                format,
                quality,
                width: image.width(),
                height: image.height(),
            });
            if self.fail.load(Ordering::SeqCst) {
                return Err(CodecError::Encode("boom".to_owned()));
            }
            if format == ImageFormat::WebP && !self.webp.load(Ordering::SeqCst) {
                return Err(CodecError::Unsupported);
            }
            Ok(vec![7_u8; 32])
        }

        fn decode(&self, _bytes: &[u8]) -> Result<Image, CodecError> {
            Err(CodecError::Decode("not used".to_owned()))
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        images: AtomicUsize,
    }

    impl Clipboard for FakeClipboard {
        fn set_image(&self, _image: &Image) -> Result<(), ClipboardError> {
            self.images.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn set_text(&self, _text: &str) -> Result<(), ClipboardError> {
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct FakeNotifier {
        notices: Mutex<Vec<Notice>>,
    }

    impl Notifier for FakeNotifier {
        fn toast(&self, notice: Notice) {
            self.notices.lock().expect("notices").push(notice);
        }
    }

    #[derive(Default)]
    struct FakeSound {
        played: AtomicUsize,
    }

    impl Sound for FakeSound {
        fn play(&self, _sound: snapline_core::ports::SoundId) {
            self.played.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FixedClock(OffsetDateTime);

    impl Clock for FixedClock {
        fn now_local(&self) -> OffsetDateTime {
            self.0
        }
    }

    struct FakeSettings {
        settings: Mutex<Settings>,
        counters: Mutex<Vec<u32>>,
    }

    impl FakeSettings {
        fn new(settings: Settings) -> Self {
            Self {
                settings: Mutex::new(settings),
                counters: Mutex::new(Vec::new()),
            }
        }

        fn snapshot(&self) -> Settings {
            self.settings.lock().expect("settings").clone()
        }

        fn counters(&self) -> Vec<u32> {
            self.counters.lock().expect("counters").clone()
        }
    }

    impl SettingsAccess for FakeSettings {
        fn settings(&self) -> Result<Settings, ServiceError> {
            Ok(self.snapshot())
        }

        fn set_counter(&self, value: u32) -> Result<(), ServiceError> {
            let mut settings = self.settings.lock().expect("settings");
            settings.general.extra.insert(
                COUNTER_KEY.to_owned(),
                toml::Value::Integer(i64::from(value)),
            );
            self.counters.lock().expect("counters").push(value);
            Ok(())
        }
    }

    struct Fixture {
        _directory: TempDir,
        settings: Arc<FakeSettings>,
        codec: Arc<FakeCodec>,
        clipboard: Arc<FakeClipboard>,
        notifier: Arc<FakeNotifier>,
        sound: Arc<FakeSound>,
        repo: HistoryRepo,
        thumbs: PathBuf,
        profile: PathBuf,
    }

    fn fixed_time() -> OffsetDateTime {
        datetime!(2026-11-08 14:31:07 UTC)
    }

    fn image(width: u32, height: u32) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                let value = u8::try_from((x + y) % 256).unwrap_or(0);
                image.set_pixel(x, y, [value, 0, 0, 255]).expect("pixel");
            }
        }
        image
    }

    impl Fixture {
        fn new(settings: Settings) -> Self {
            let directory = tempfile::tempdir().expect("temp dir");
            let repo =
                HistoryRepo::open_write(&directory.path().join("history.db")).expect("history");
            let profile = directory.path().join("Profile");
            let thumbs = directory.path().join("thumbs");
            let settings = Arc::new(FakeSettings::new(settings));
            let codec = Arc::new(FakeCodec::new());
            let clipboard = Arc::new(FakeClipboard::default());
            let notifier = Arc::new(FakeNotifier::default());
            let sound = Arc::new(FakeSound::default());
            Self {
                _directory: directory,
                settings,
                codec,
                clipboard,
                notifier,
                sound,
                repo,
                thumbs,
                profile,
            }
        }

        fn ports(&self) -> ServicePorts {
            ServicePorts::new(
                self.codec.clone(),
                self.clipboard.clone(),
                self.notifier.clone(),
                self.sound.clone(),
                Arc::new(FixedClock(fixed_time())),
            )
        }

        fn saver(&self) -> Saver {
            Saver::new(
                self.settings.clone(),
                self.ports(),
                self.thumbs.clone(),
                self.profile.clone(),
            )
        }

        fn run(&mut self, request: CaptureRequest) -> Result<SaveOutcome, SaveError> {
            let saver = self.saver();
            saver.run(&mut self.repo, request)
        }

        fn default_request() -> CaptureRequest {
            CaptureRequest::new(
                image(1280, 720),
                Some("chrome.exe".to_owned()),
                "Pull-request-42",
            )
        }

        fn base_folder(&self) -> PathBuf {
            self.profile.join("Pictures").join("Snapline")
        }

        fn count(&self) -> usize {
            self.repo
                .page(HistoryFilter::All, None)
                .expect("page")
                .items
                .len()
        }
    }

    fn settings_with_folder() -> Settings {
        let mut settings = Settings::default();
        settings.general.save_folder = "%USERPROFILE%\\Pictures\\Snapline".to_owned();
        settings
    }

    #[test]
    fn the_default_pattern_and_month_subfolder_are_used() {
        let mut fixture = Fixture::new(settings_with_folder());
        let outcome = fixture.run(Fixture::default_request()).expect("save");
        assert!(outcome.saved);
        assert_eq!(
            outcome.file_name.as_deref(),
            Some("chrome_2026-11-08_14-31-07.png")
        );
        let path = outcome.path.as_ref().expect("path");
        let expected_folder = fixture.base_folder().join("2026-11");
        assert_eq!(path.parent(), Some(expected_folder.as_path()));
        assert!(path.is_file());
        assert_eq!(outcome.bytes, 32);
        assert_eq!(outcome.width, 1280);
        assert_eq!(outcome.height, 720);
        assert_eq!(fixture.count(), 1);
        assert_eq!(outcome.next_counter, Some(1));
        assert_eq!(fixture.settings.counters(), vec![1]);
        assert_eq!(
            fixture.settings.snapshot().general.extra.get(COUNTER_KEY),
            Some(&toml::Value::Integer(1))
        );
    }

    #[test]
    fn the_filename_uses_the_configured_pattern_and_counter() {
        let mut settings = settings_with_folder();
        settings.general.name_pattern = "{app}_{window}_{n}_{w}x{h}_{rand}".to_owned();
        settings
            .general
            .extra
            .insert(COUNTER_KEY.to_owned(), toml::Value::Integer(41));
        let mut fixture = Fixture::new(settings);
        let saver = Saver::new(
            fixture.settings.clone(),
            fixture.ports(),
            fixture.thumbs.clone(),
            fixture.profile.clone(),
        )
        .with_random_source(Arc::new(FixedRandom::new("k3f9")));
        let outcome = saver
            .run(&mut fixture.repo, Fixture::default_request())
            .expect("save");
        assert_eq!(
            outcome.file_name.as_deref(),
            Some("chrome_Pull-request-42_0042_1280x720_k3f9.png")
        );
        assert_eq!(outcome.next_counter, Some(42));
    }

    struct FixedRandom(String);

    impl FixedRandom {
        fn new(value: &str) -> Self {
            Self(value.to_owned())
        }
    }

    impl RandomSource for FixedRandom {
        fn chars(&self) -> String {
            self.0.clone()
        }
    }

    #[test]
    fn every_subfolder_mode_is_honoured() {
        for (mode, suffix) in [
            (Subfolders::ByMonth, Some("2026-11")),
            (Subfolders::ByDay, Some("2026-11-08")),
            (Subfolders::ByApp, Some("chrome")),
            (Subfolders::None, None),
        ] {
            let mut settings = settings_with_folder();
            settings.general.subfolders = mode;
            let mut fixture = Fixture::new(settings);
            let outcome = fixture.run(Fixture::default_request()).expect("save");
            let expected = match suffix {
                Some(suffix) => fixture.base_folder().join(suffix),
                None => fixture.base_folder(),
            };
            assert_eq!(
                outcome.path.as_ref().and_then(|path| path.parent()),
                Some(expected.as_path()),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn the_image_format_and_lossy_quality_reach_the_codec() {
        for (format, extension) in [
            (SettingsImageFormat::Png, "png"),
            (SettingsImageFormat::Jpeg, "jpg"),
            (SettingsImageFormat::Webp, "webp"),
        ] {
            let mut settings = settings_with_folder();
            settings.general.image_format = format;
            settings.general.lossy_quality = 75;
            settings.general.name_pattern = "shot".to_owned();
            let mut fixture = Fixture::new(settings);
            let outcome = fixture.run(Fixture::default_request()).expect("save");
            let expected_name = format!("shot.{extension}");
            assert_eq!(outcome.file_name.as_deref(), Some(expected_name.as_str()));
            let calls = fixture.codec.calls();
            let expected_quality = match format {
                SettingsImageFormat::Png => 100,
                SettingsImageFormat::Jpeg | SettingsImageFormat::Webp => 75,
            };
            assert_eq!(calls[0].quality, expected_quality);
            assert_eq!(
                calls[0].format,
                match format {
                    SettingsImageFormat::Png => ImageFormat::Png,
                    SettingsImageFormat::Jpeg => ImageFormat::Jpeg,
                    SettingsImageFormat::Webp => ImageFormat::WebP,
                }
            );
        }
    }

    #[test]
    fn collisions_append_the_next_number() {
        let mut settings = settings_with_folder();
        settings.general.name_pattern = "shot".to_owned();
        let mut fixture = Fixture::new(settings);
        let folder = fixture.base_folder().join("2026-11");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("shot.png"), b"old").expect("old");
        std::fs::write(folder.join("shot (2).png"), b"old").expect("old");
        let outcome = fixture.run(Fixture::default_request()).expect("save");
        assert_eq!(outcome.file_name.as_deref(), Some("shot (3).png"));
        assert!(folder.join("shot (3).png").is_file());
    }

    #[test]
    fn copying_runs_only_when_the_step_is_on() {
        let mut fixture = Fixture::new(settings_with_folder());
        fixture.run(Fixture::default_request()).expect("save");
        assert_eq!(fixture.clipboard.images.load(Ordering::SeqCst), 1);
        assert!(
            fixture
                .run(Fixture::default_request())
                .expect("second")
                .copied
        );

        let mut settings = settings_with_folder();
        for step in &mut settings.general.default_workflow {
            if step.kind == WorkflowStepKind::CopyImage {
                step.enabled = false;
            }
        }
        let mut fixture = Fixture::new(settings);
        let outcome = fixture.run(Fixture::default_request()).expect("save");
        assert!(!outcome.copied);
        assert_eq!(fixture.clipboard.images.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn saving_is_skipped_when_the_save_step_is_off() {
        let mut settings = settings_with_folder();
        for step in &mut settings.general.default_workflow {
            if step.kind == WorkflowStepKind::Save {
                step.enabled = false;
            }
        }
        let mut fixture = Fixture::new(settings);
        let outcome = fixture.run(Fixture::default_request()).expect("run");
        assert!(!outcome.saved);
        assert_eq!(outcome.capture_id, None);
        assert_eq!(outcome.path, None);
        assert_eq!(outcome.next_counter, None);
        assert_eq!(fixture.count(), 0);
        assert!(fixture.notifier.notices.lock().expect("notices").is_empty());
        assert_eq!(fixture.clipboard.images.load(Ordering::SeqCst), 1);
        assert!(!fixture.base_folder().exists());
    }

    #[test]
    fn a_saved_capture_is_recorded_thumbnailed_and_toasted() {
        let mut fixture = Fixture::new(settings_with_folder());
        let outcome = fixture.run(Fixture::default_request()).expect("save");
        let id = outcome.capture_id.expect("id");
        let recorded = fixture.repo.get(id).expect("get").expect("row");
        assert_eq!(
            recorded.path,
            outcome
                .path
                .as_ref()
                .expect("path")
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(recorded.bytes, 32);
        assert_eq!(recorded.kind, CaptureKind::Image);
        assert_eq!(recorded.source_app.as_deref(), Some("chrome.exe"));
        assert_eq!(
            recorded.created_at,
            fixed_time().unix_timestamp().saturating_mul(1000)
        );
        let thumbnail = fixture.thumbs.join(format!("{id}.webp"));
        assert!(thumbnail.is_file());
        let thumb_call = fixture
            .codec
            .calls()
            .into_iter()
            .find(|call| call.width == 320)
            .expect("thumbnail call");
        assert_eq!(thumb_call.height, 180);
        let notices = fixture.notifier.notices.lock().expect("notices");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "Saved");
        assert_eq!(
            notices[0].body,
            outcome.file_name.as_deref().expect("file name")
        );
        assert_eq!(notices[0].capture_id, Some(id));
    }

    #[test]
    fn notifications_are_silent_when_turned_off() {
        let mut settings = settings_with_folder();
        settings.general.notifications = false;
        let mut fixture = Fixture::new(settings);
        fixture.run(Fixture::default_request()).expect("save");
        assert!(fixture.notifier.notices.lock().expect("notices").is_empty());
    }

    #[test]
    fn a_codec_failure_leaves_no_file_row_or_counter_change() {
        let mut fixture = Fixture::new(settings_with_folder());
        fixture.codec.fail.store(true, Ordering::SeqCst);
        let error = fixture
            .run(Fixture::default_request())
            .expect_err("codec failure");
        assert!(matches!(error, SaveError::Codec(_)));
        assert_eq!(fixture.count(), 0);
        assert_eq!(fixture.settings.counters(), Vec::<u32>::new());
        assert!(!fixture.thumbs.exists());
        assert_eq!(fixture.clipboard.images.load(Ordering::SeqCst), 1);
        assert!(fixture.notifier.notices.lock().expect("notices").is_empty());
    }

    #[test]
    fn a_missing_webp_thumbnail_falls_back_to_jpeg() {
        let mut fixture = Fixture::new(settings_with_folder());
        fixture.codec.webp.store(false, Ordering::SeqCst);
        let outcome = fixture.run(Fixture::default_request()).expect("save");
        let id = outcome.capture_id.expect("id");
        assert!(fixture.thumbs.join(format!("{id}.jpg")).is_file());
        assert!(!fixture.thumbs.join(format!("{id}.webp")).exists());
    }

    #[test]
    fn scaling_keeps_the_aspect_ratio_and_never_enlarges() {
        let wide = scale_to_width(&image(1000, 500), 320).expect("scale");
        assert_eq!((wide.width(), wide.height()), (320, 160));
        let tiny = scale_to_width(&image(100, 50), 320).expect("scale");
        assert_eq!((tiny.width(), tiny.height()), (100, 50));
    }

    #[test]
    fn profile_tokens_are_expanded_in_the_save_folder() {
        assert_eq!(
            expand_profile(
                "%USERPROFILE%\\Pictures\\Snapline",
                Path::new("C:\\Users\\Aaron")
            ),
            "C:\\Users\\Aaron\\Pictures\\Snapline"
        );
        assert_eq!(
            expand_profile("D:\\Shots", Path::new("C:\\Users\\Aaron")),
            "D:\\Shots"
        );
    }

    #[test]
    fn the_counter_is_read_from_the_general_extra_table() {
        let mut general = General::default();
        assert_eq!(counter_from(&general), 0);
        general
            .extra
            .insert(COUNTER_KEY.to_owned(), toml::Value::Integer(9999));
        assert_eq!(counter_from(&general), 9999);
        general
            .extra
            .insert(COUNTER_KEY.to_owned(), toml::Value::Integer(-1));
        assert_eq!(counter_from(&general), 0);
    }
}
