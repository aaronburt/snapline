use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use snapline_core::image::Image;
use snapline_core::ports::Clipboard;
use snapline_store::history::{CaptureKind, HistoryRepo};
use snapline_store::settings::uploads::DestinationRef;

use crate::dto::upload::UploadResp;
use crate::saver::{CaptureRequest, SaveOutcome};
use crate::uploads::UploadService;
use crate::{ServiceError, Services};

pub const TOOL_EDITOR: &str = "tool_editor";
pub const CLIPBOARD_TITLE: &str = "Clipboard image";
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorAction {
    Copy,
    Save,
    Pin,
    Upload,
}

#[derive(Debug, Clone)]
pub struct EditorResult {
    pub action: Option<EditorAction>,
    pub image: Option<Image>,
}

impl EditorResult {
    pub fn cancelled() -> Self {
        Self {
            action: None,
            image: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorOutcome {
    pub action: Option<EditorAction>,
    pub capture_id: Option<i64>,
    pub path: Option<PathBuf>,
    pub cancelled: bool,
}

impl EditorOutcome {
    pub fn cancelled() -> Self {
        Self {
            action: None,
            capture_id: None,
            path: None,
            cancelled: true,
        }
    }

    fn performed(action: EditorAction) -> Self {
        Self {
            action: Some(action),
            capture_id: None,
            path: None,
            cancelled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSource {
    File,
    Clipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinOutcome {
    Pinned(PinSource),
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum EditorClipboardContent {
    Image(Image),
    Files(Vec<PathBuf>),
}

pub trait EditorHost: Send + Sync {
    fn open(&self, image: Image) -> Result<EditorResult, ServiceError>;
}

pub trait EditorFileDialog: Send + Sync {
    fn pick_image(&self) -> Result<Option<PathBuf>, ServiceError>;
}

pub trait EditorClipboardSource: Send + Sync {
    fn read(&self) -> Result<Option<EditorClipboardContent>, ServiceError>;
}

pub trait EditorSaver: Send + Sync {
    fn save(&self, image: Image, title: String) -> Result<SaveOutcome, ServiceError>;
}

pub trait EditorUploader: Send + Sync {
    fn enqueue(&self, capture_id: i64, destination: &str) -> Result<u64, ServiceError>;
}

pub trait EditorPin: Send + Sync {
    fn pin(&self, image: Image) -> Result<(), ServiceError>;
}

impl EditorSaver for crate::worker::Handle {
    fn save(&self, image: Image, title: String) -> Result<SaveOutcome, ServiceError> {
        self.save_capture(CaptureRequest::new(
            image,
            Some(String::from("Snapline")),
            title,
        ))
    }
}

impl EditorUploader for UploadService {
    fn enqueue(&self, capture_id: i64, destination: &str) -> Result<u64, ServiceError> {
        let response = self.handle(crate::dto::upload::UploadReq::UploadElsewhere {
            capture_id,
            dest: destination.to_owned(),
        })?;
        match response {
            UploadResp::Accepted { id } => Ok(id),
            _ => Err(ServiceError::Internal(
                "The upload could not be queued.".to_owned(),
            )),
        }
    }
}

pub struct EditorPorts {
    pub host: Arc<dyn EditorHost>,
    pub file_dialog: Arc<dyn EditorFileDialog>,
    pub clipboard: Arc<dyn Clipboard + Send + Sync>,
    pub clipboard_source: Arc<dyn EditorClipboardSource>,
    pub saver: Arc<dyn EditorSaver>,
    pub uploader: Arc<dyn EditorUploader>,
    pub pin: Arc<dyn EditorPin>,
}

static PORTS: OnceLock<Arc<EditorPorts>> = OnceLock::new();

pub fn install(ports: EditorPorts) -> Result<(), ServiceError> {
    PORTS
        .set(Arc::new(ports))
        .map_err(|_| ServiceError::Internal("The image editor is already available.".to_owned()))
}

pub fn annotate_image(image: &Image) -> Result<Image, snapline_core::workflow::WorkflowError> {
    let ports = installed().ok_or(snapline_core::workflow::WorkflowError::NotAvailableYet)?;
    let result = ports
        .host
        .open(image.clone())
        .map_err(|error| snapline_core::workflow::WorkflowError::Failed(error.to_string()))?;
    result
        .image
        .ok_or(snapline_core::workflow::WorkflowError::Cancelled)
}

pub fn is_available() -> bool {
    PORTS.get().is_some()
}

fn installed() -> Option<Arc<EditorPorts>> {
    PORTS.get().cloned()
}

pub struct EditorService {
    services: Services,
    ports: Option<Arc<EditorPorts>>,
}

impl EditorService {
    pub fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
            ports: installed(),
        }
    }

    pub fn with_ports(services: &Services, ports: EditorPorts) -> Self {
        Self {
            services: services.clone(),
            ports: Some(Arc::new(ports)),
        }
    }

    pub fn available(&self) -> bool {
        self.ports.is_some()
    }

    pub fn open_history(&self, capture_id: i64) -> Result<EditorOutcome, ServiceError> {
        self.ports()?;
        let (image, title) = self.history_image(capture_id)?;
        self.edit(image, title)
    }

    pub fn open_file(&self) -> Result<EditorOutcome, ServiceError> {
        let path = self.ports()?.file_dialog.pick_image()?;
        let Some(path) = path else {
            return Ok(EditorOutcome::cancelled());
        };
        let (image, title) = file_image(&path, self.services.codec()?)?;
        self.edit(image, title)
    }

    pub fn open_clipboard(&self) -> Result<EditorOutcome, ServiceError> {
        let (image, title) = self.clipboard_image()?;
        self.edit(image, title)
    }

    pub fn pin_available(&self) -> bool {
        self.ports.is_some()
    }

    pub fn pin_file(&self) -> Result<PinOutcome, ServiceError> {
        let path = self.ports()?.file_dialog.pick_image()?;
        let Some(path) = path else {
            return Ok(PinOutcome::Cancelled);
        };
        let (image, _title) = file_image(&path, self.services.codec()?)?;
        self.ports()?.pin.pin(image)?;
        Ok(PinOutcome::Pinned(PinSource::File))
    }

    pub fn pin_history(&self, capture_id: i64) -> Result<PinOutcome, ServiceError> {
        let (image, _) = self.history_image(capture_id)?;
        self.ports()?.pin.pin(image)?;
        Ok(PinOutcome::Pinned(PinSource::File))
    }

    pub fn pin_clipboard(&self) -> Result<PinOutcome, ServiceError> {
        let (image, _title) = self.clipboard_image()?;
        self.ports()?.pin.pin(image)?;
        Ok(PinOutcome::Pinned(PinSource::Clipboard))
    }

    fn clipboard_image(&self) -> Result<(Image, String), ServiceError> {
        let Some(content) = self.ports()?.clipboard_source.read()? else {
            return Err(ServiceError::Invalid(
                "The clipboard does not contain an image.".to_owned(),
            ));
        };
        match content {
            EditorClipboardContent::Image(image) => Ok((image, String::from(CLIPBOARD_TITLE))),
            EditorClipboardContent::Files(paths) => {
                let Some(path) = paths.first() else {
                    return Err(ServiceError::Invalid(
                        "The clipboard does not contain an image.".to_owned(),
                    ));
                };
                file_image(path, self.services.codec()?)
            }
        }
    }

    fn edit(&self, image: Image, title: String) -> Result<EditorOutcome, ServiceError> {
        let result = self.ports()?.host.open(image)?;
        let Some(action) = result.action else {
            return Ok(EditorOutcome::cancelled());
        };
        let image = result.image.ok_or_else(|| {
            ServiceError::Internal("The editor did not return an image.".to_owned())
        })?;
        self.apply(action, image, &title)
    }

    fn apply(
        &self,
        action: EditorAction,
        image: Image,
        title: &str,
    ) -> Result<EditorOutcome, ServiceError> {
        let ports = self.ports()?;
        match action {
            EditorAction::Copy => {
                ports
                    .clipboard
                    .set_image(&image)
                    .map_err(|error| ServiceError::Invalid(error.to_string()))?;
                Ok(EditorOutcome::performed(EditorAction::Copy))
            }
            EditorAction::Pin => {
                ports.pin.pin(image)?;
                Ok(EditorOutcome::performed(EditorAction::Pin))
            }
            EditorAction::Save => {
                let saved = ports.saver.save(image, title.to_owned())?;
                Ok(saved_outcome(EditorAction::Save, saved)?)
            }
            EditorAction::Upload => {
                let saved = ports.saver.save(image, title.to_owned())?;
                let capture_id = saved.capture_id.ok_or_else(not_saved)?;
                let destination = self.default_destination()?;
                let _id = ports.uploader.enqueue(capture_id, &destination)?;
                Ok(EditorOutcome {
                    action: Some(EditorAction::Upload),
                    capture_id: Some(capture_id),
                    path: saved.path,
                    cancelled: false,
                })
            }
        }
    }

    fn default_destination(&self) -> Result<String, ServiceError> {
        let settings = self.services.settings_snapshot()?;
        let destination = settings.uploads.images.as_str().trim().to_owned();
        if !DestinationRef::is_upload_id(&destination) {
            return Err(ServiceError::Invalid(
                "Choose an upload destination for images in Settings.".to_owned(),
            ));
        }
        Ok(destination)
    }

    fn history_image(&self, capture_id: i64) -> Result<(Image, String), ServiceError> {
        if capture_id <= 0 {
            return Err(ServiceError::NotFound);
        }
        let paths = self.services.paths()?;
        if !paths.history_file.is_file() {
            return Err(ServiceError::NotFound);
        }
        let repo = HistoryRepo::open_read(&paths.history_file).map_err(history_error)?;
        let capture = repo
            .get(capture_id)
            .map_err(history_error)?
            .ok_or(ServiceError::NotFound)?;
        if capture.kind != CaptureKind::Image {
            return Err(ServiceError::Invalid(
                "Only images can be opened in the editor.".to_owned(),
            ));
        }
        let path = PathBuf::from(&capture.path);
        let title = capture.name().to_owned();
        let image = decode_path(&path, self.services.codec()?)?;
        Ok((image, title))
    }

    fn ports(&self) -> Result<&EditorPorts, ServiceError> {
        self.ports.as_deref().ok_or(ServiceError::NotYet)
    }
}

fn saved_outcome(action: EditorAction, saved: SaveOutcome) -> Result<EditorOutcome, ServiceError> {
    let capture_id = saved.capture_id.ok_or_else(not_saved)?;
    Ok(EditorOutcome {
        action: Some(action),
        capture_id: Some(capture_id),
        path: saved.path,
        cancelled: false,
    })
}

fn not_saved() -> ServiceError {
    ServiceError::Invalid("The image could not be saved.".to_owned())
}

fn history_error(error: snapline_store::history::HistoryError) -> ServiceError {
    tracing::warn!(%error, "the editor could not read the History");
    ServiceError::Io("The History could not be read.".to_owned())
}

fn file_image(
    path: &Path,
    codec: Arc<dyn snapline_core::ports::ImageCodec>,
) -> Result<(Image, String), ServiceError> {
    let title = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("Image"));
    let image = decode_path(path, codec)?;
    Ok((image, title))
}

fn decode_path(
    path: &Path,
    codec: Arc<dyn snapline_core::ports::ImageCodec>,
) -> Result<Image, ServiceError> {
    if !path.is_absolute() {
        return Err(ServiceError::Invalid(
            "The image path must be absolute.".to_owned(),
        ));
    }
    if !path.is_file() {
        return Err(ServiceError::NotFound);
    }
    if !is_image(path) {
        return Err(ServiceError::Invalid(
            "Only PNG, JPEG, WebP and BMP images can be opened.".to_owned(),
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        tracing::warn!(%error, "the editor image could not be read");
        ServiceError::Io("The image could not be read.".to_owned())
    })?;
    codec.decode(&bytes).map_err(|error| {
        tracing::warn!(%error, "the editor image could not be decoded");
        ServiceError::Invalid("The image could not be read.".to_owned())
    })
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension.as_str()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use snapline_core::image::ImageFormat;
    use snapline_core::ports::{
        ClipboardError, CodecError, ImageCodec, Notice, Notifier, Sound, SoundId,
    };
    use snapline_store::history::{CaptureId, NewCapture};
    use snapline_store::io::Paths;
    use tempfile::TempDir;

    use super::*;
    use crate::dto::tool::{ToolReq, ToolResp};
    use crate::saver::SaveOutcome;

    struct FakeCodec;

    impl ImageCodec for FakeCodec {
        fn encode(
            &self,
            _image: &Image,
            _format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            Ok(vec![1, 2, 3, 4])
        }

        fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError> {
            let (width, height) = match bytes.first() {
                Some(b'a') => (100, 50),
                Some(b'b') => (40, 80),
                Some(b'c') => (20, 10),
                _ => return Err(CodecError::Decode("unknown test image".to_owned())),
            };
            Image::bgra(width, height).map_err(|error| CodecError::Decode(error.to_string()))
        }
    }

    struct Silenced;

    impl Notifier for Silenced {
        fn toast(&self, _notice: Notice) {}
    }

    impl Sound for Silenced {
        fn play(&self, _sound: SoundId) {}
    }

    struct FakeHost {
        calls: Mutex<Vec<(u32, u32)>>,
        result: Mutex<EditorResult>,
    }

    impl FakeHost {
        fn with_result(action: EditorAction, image: Image) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Mutex::new(EditorResult {
                    action: Some(action),
                    image: Some(image),
                }),
            }
        }
    }

    impl Default for FakeHost {
        fn default() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Mutex::new(EditorResult::cancelled()),
            }
        }
    }

    impl EditorHost for FakeHost {
        fn open(&self, image: Image) -> Result<EditorResult, ServiceError> {
            self.calls
                .lock()
                .expect("calls")
                .push((image.width(), image.height()));
            Ok(self.result.lock().expect("result").clone())
        }
    }

    #[derive(Default)]
    struct FakeDialog {
        path: Mutex<Option<PathBuf>>,
    }

    impl EditorFileDialog for FakeDialog {
        fn pick_image(&self) -> Result<Option<PathBuf>, ServiceError> {
            Ok(self.path.lock().expect("path").clone())
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        images: Mutex<Vec<(u32, u32)>>,
    }

    impl Clipboard for FakeClipboard {
        fn set_image(&self, image: &Image) -> Result<(), ClipboardError> {
            self.images
                .lock()
                .expect("images")
                .push((image.width(), image.height()));
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
    struct FakeSource {
        content: Mutex<Option<EditorClipboardContent>>,
    }

    impl EditorClipboardSource for FakeSource {
        fn read(&self) -> Result<Option<EditorClipboardContent>, ServiceError> {
            Ok(self.content.lock().expect("content").clone())
        }
    }

    #[derive(Default)]
    struct FakeSaver {
        calls: Mutex<Vec<(u32, u32, String)>>,
        capture_id: Mutex<Option<i64>>,
        path: Mutex<Option<PathBuf>>,
    }

    impl EditorSaver for FakeSaver {
        fn save(&self, image: Image, title: String) -> Result<SaveOutcome, ServiceError> {
            self.calls
                .lock()
                .expect("calls")
                .push((image.width(), image.height(), title));
            let capture_id = *self.capture_id.lock().expect("capture");
            let path = self.path.lock().expect("path").clone();
            Ok(SaveOutcome {
                capture_id,
                path: path.clone(),
                file_name: path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned()),
                bytes: 12,
                width: image.width(),
                height: image.height(),
                copied: false,
                saved: capture_id.is_some(),
                next_counter: None,
            })
        }
    }

    #[derive(Default)]
    struct FakeUploader {
        calls: Mutex<Vec<(i64, String)>>,
    }

    impl EditorUploader for FakeUploader {
        fn enqueue(&self, capture_id: i64, destination: &str) -> Result<u64, ServiceError> {
            self.calls
                .lock()
                .expect("calls")
                .push((capture_id, destination.to_owned()));
            Ok(7)
        }
    }

    #[derive(Default)]
    struct FakePin {
        pins: AtomicUsize,
    }

    impl EditorPin for FakePin {
        fn pin(&self, _image: Image) -> Result<(), ServiceError> {
            self.pins.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Ports {
        host: Arc<FakeHost>,
        dialog: Arc<FakeDialog>,
        clipboard: Arc<FakeClipboard>,
        source: Arc<FakeSource>,
        saver: Arc<FakeSaver>,
        uploader: Arc<FakeUploader>,
        pin: Arc<FakePin>,
    }

    impl Default for Ports {
        fn default() -> Self {
            Self {
                host: Arc::new(FakeHost::default()),
                dialog: Arc::new(FakeDialog::default()),
                clipboard: Arc::new(FakeClipboard::default()),
                source: Arc::new(FakeSource::default()),
                saver: Arc::new(FakeSaver::default()),
                uploader: Arc::new(FakeUploader::default()),
                pin: Arc::new(FakePin::default()),
            }
        }
    }

    impl Ports {
        fn editor_ports(&self) -> EditorPorts {
            EditorPorts {
                host: self.host.clone(),
                file_dialog: self.dialog.clone(),
                clipboard: self.clipboard.clone(),
                clipboard_source: self.source.clone(),
                saver: self.saver.clone(),
                uploader: self.uploader.clone(),
                pin: self.pin.clone(),
            }
        }
    }

    struct Fixture {
        _directory: TempDir,
        services: Services,
        paths: Paths,
    }

    fn image(width: u32, height: u32) -> Image {
        Image::bgra(width, height).expect("image")
    }

    fn fixture() -> Fixture {
        let directory = tempfile::tempdir().expect("temp");
        let paths = Paths::resolve(
            &directory.path().join("exe"),
            &directory.path().join("roaming"),
            &directory.path().join("local"),
            &directory.path().join("profile"),
            false,
        );
        std::fs::create_dir_all(&paths.captures_dir).expect("captures");
        let services = Services::with_paths(paths.clone())
            .expect("services")
            .with_codec_factory(Arc::new(|| Ok(Arc::new(FakeCodec) as Arc<dyn ImageCodec>)));
        Fixture {
            _directory: directory,
            services,
            paths,
        }
    }

    impl Fixture {
        fn editor(&self, ports: &Ports) -> EditorService {
            EditorService::with_ports(&self.services, ports.editor_ports())
        }

        fn capture(&self, name: &str, marker: u8, kind: CaptureKind) -> CaptureId {
            let path = self.paths.captures_dir.join(name);
            std::fs::write(&path, [marker]).expect("capture file");
            let mut repo = HistoryRepo::open_write(&self.paths.history_file).expect("history");
            repo.record(NewCapture {
                kind,
                path: path.to_string_lossy().into_owned(),
                bytes: 1,
                width: Some(100),
                height: Some(50),
                duration_ms: None,
                source_app: Some("test.exe".to_owned()),
                created_at: 1,
                ocr_text: None,
            })
            .expect("record")
        }
    }

    #[test]
    fn history_opening_decodes_the_saved_image_and_copies_the_result() {
        let fixture = fixture();
        let id = fixture.capture("one.png", b'a', CaptureKind::Image);
        let mut ports = Ports::default();
        ports.host = Arc::new(FakeHost::with_result(EditorAction::Copy, image(11, 7)));
        let outcome = fixture.editor(&ports).open_history(id).expect("open");
        assert_eq!(outcome.action, Some(EditorAction::Copy));
        assert!(!outcome.cancelled);
        assert_eq!(*ports.host.calls.lock().expect("calls"), vec![(100, 50)]);
        assert_eq!(
            *ports.clipboard.images.lock().expect("images"),
            vec![(11, 7)]
        );
    }

    #[test]
    fn file_opening_picks_decodes_and_saves_through_the_saver() {
        let fixture = fixture();
        let path = fixture.paths.captures_dir.join("picked.png");
        std::fs::write(&path, [b'b']).expect("file");
        let mut ports = Ports::default();
        *ports.dialog.path.lock().expect("path") = Some(path);
        *ports.saver.capture_id.lock().expect("capture") = Some(42);
        *ports.saver.path.lock().expect("saved") = Some(PathBuf::from(r"C:\Shots\edit.png"));
        ports.host = Arc::new(FakeHost::with_result(EditorAction::Save, image(30, 10)));
        let outcome = fixture.editor(&ports).open_file().expect("open");
        assert_eq!(outcome.action, Some(EditorAction::Save));
        assert_eq!(outcome.capture_id, Some(42));
        assert_eq!(
            outcome.path.as_deref(),
            Some(Path::new(r"C:\Shots\edit.png"))
        );
        assert_eq!(
            *ports.saver.calls.lock().expect("calls"),
            vec![(30, 10, String::from("picked.png"))]
        );
    }

    #[test]
    fn clipboard_opening_uses_an_image_or_a_single_file_path() {
        let fixture = fixture();
        let mut ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Image(image(5, 6)));
        ports.host = Arc::new(FakeHost::with_result(EditorAction::Copy, image(5, 6)));
        let outcome = fixture.editor(&ports).open_clipboard().expect("clipboard");
        assert_eq!(outcome.action, Some(EditorAction::Copy));
        assert_eq!(*ports.host.calls.lock().expect("calls"), vec![(5, 6)]);

        let path = fixture.paths.captures_dir.join("clip.png");
        std::fs::write(&path, [b'c']).expect("file");
        let mut ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Files(vec![path]));
        ports.host = Arc::new(FakeHost::with_result(EditorAction::Copy, image(2, 2)));
        let outcome = fixture.editor(&ports).open_clipboard().expect("clipboard");
        assert_eq!(outcome.action, Some(EditorAction::Copy));
        assert_eq!(*ports.host.calls.lock().expect("calls"), vec![(20, 10)]);
    }

    #[test]
    fn a_cancelled_dialog_returns_a_cancelled_outcome_without_opening_the_editor() {
        let fixture = fixture();
        let ports = Ports::default();
        let outcome = fixture.editor(&ports).open_file().expect("open");
        assert!(outcome.cancelled);
        assert_eq!(outcome.action, None);
        assert!(ports.host.calls.lock().expect("calls").is_empty());
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let fixture = fixture();
        let ports = Ports::default();
        *ports.dialog.path.lock().expect("path") =
            Some(fixture.paths.captures_dir.join("missing.png"));
        assert_eq!(
            fixture.editor(&ports).open_file().expect_err("missing"),
            ServiceError::NotFound
        );
    }

    #[test]
    fn non_images_are_rejected_from_files_history_and_the_clipboard() {
        let fixture = fixture();
        let text = fixture.paths.captures_dir.join("notes.txt");
        std::fs::write(&text, b"text").expect("file");
        let ports = Ports::default();
        *ports.dialog.path.lock().expect("path") = Some(text.clone());
        assert!(matches!(
            fixture.editor(&ports).open_file(),
            Err(ServiceError::Invalid(_))
        ));

        let video = fixture.capture("clip.mp4", b'a', CaptureKind::Video);
        assert!(matches!(
            fixture.editor(&ports).open_history(video),
            Err(ServiceError::Invalid(_))
        ));

        let ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Files(vec![text]));
        assert!(matches!(
            fixture.editor(&ports).open_clipboard(),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn every_editor_action_routes_through_its_port() {
        let fixture = fixture();
        fixture
            .services
            .update_settings(&serde_json::json!({
                "uploads": { "images": "imgur" }
            }))
            .expect("settings");

        let mut ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Image(image(3, 4)));
        *ports.saver.capture_id.lock().expect("capture") = Some(9);
        *ports.saver.path.lock().expect("path") = Some(PathBuf::from(r"C:\Shots\e.png"));

        ports.host = Arc::new(FakeHost::with_result(EditorAction::Copy, image(3, 4)));
        fixture.editor(&ports).open_clipboard().expect("copy");
        assert_eq!(
            *ports.clipboard.images.lock().expect("images"),
            vec![(3, 4)]
        );

        ports.host = Arc::new(FakeHost::with_result(EditorAction::Save, image(3, 4)));
        let saved = fixture.editor(&ports).open_clipboard().expect("save");
        assert_eq!(saved.capture_id, Some(9));

        ports.host = Arc::new(FakeHost::with_result(EditorAction::Upload, image(3, 4)));
        fixture.editor(&ports).open_clipboard().expect("upload");
        assert_eq!(
            *ports.uploader.calls.lock().expect("uploads"),
            vec![(9, String::from("imgur"))]
        );

        ports.host = Arc::new(FakeHost::with_result(EditorAction::Pin, image(3, 4)));
        fixture.editor(&ports).open_clipboard().expect("pin");
        assert_eq!(ports.pin.pins.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_saved_editor_result_is_recorded_in_history() {
        let fixture = fixture();
        let worker = crate::worker::Worker::start(fixture.services.clone(), || {
            Ok(crate::saver::ServicePorts::new(
                Arc::new(FakeCodec),
                Arc::new(FakeClipboard::default()),
                Arc::new(Silenced),
                Arc::new(Silenced),
                Arc::new(crate::saver::SystemClock),
            ))
        })
        .expect("worker");
        let mut ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Image(image(30, 20)));
        ports.host = Arc::new(FakeHost::with_result(EditorAction::Save, image(30, 20)));
        let editor = EditorService::with_ports(
            &fixture.services,
            EditorPorts {
                host: ports.host.clone(),
                file_dialog: ports.dialog.clone(),
                clipboard: ports.clipboard.clone(),
                clipboard_source: ports.source.clone(),
                saver: Arc::new(worker.handle.clone()),
                uploader: ports.uploader.clone(),
                pin: ports.pin.clone(),
            },
        );
        let outcome = editor.open_clipboard().expect("save");
        let capture_id = outcome.capture_id.expect("capture");
        let repo = HistoryRepo::open_read(&fixture.paths.history_file).expect("history");
        let capture = repo.get(capture_id).expect("get").expect("capture");
        assert_eq!(capture.kind, CaptureKind::Image);
        assert_eq!(capture.width, Some(30));
        assert_eq!(capture.height, Some(20));
        assert!(Path::new(&capture.path).is_file());
        worker.join();
    }

    #[test]
    fn upload_requires_a_real_destination_and_a_saved_capture() {
        let fixture = fixture();
        let mut ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Image(image(3, 4)));
        *ports.saver.capture_id.lock().expect("capture") = Some(9);
        ports.host = Arc::new(FakeHost::with_result(EditorAction::Upload, image(3, 4)));
        assert!(matches!(
            fixture.editor(&ports).open_clipboard(),
            Err(ServiceError::Invalid(_))
        ));

        *ports.saver.capture_id.lock().expect("capture") = None;
        fixture
            .services
            .update_settings(&serde_json::json!({
                "uploads": { "images": "imgur" }
            }))
            .expect("settings");
        assert!(matches!(
            fixture.editor(&ports).open_clipboard(),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn unavailable_ports_report_not_yet() {
        let fixture = fixture();
        let service = EditorService::new(&fixture.services);
        assert!(!service.available());
        assert_eq!(
            service.open_file().expect_err("ports"),
            ServiceError::NotYet
        );
        assert_eq!(
            fixture
                .services
                .tool()
                .handle(ToolReq::EditorOpenFile)
                .expect_err("ports"),
            ServiceError::NotYet
        );
        assert_eq!(
            fixture
                .services
                .tool()
                .handle(ToolReq::Available {
                    tool: String::from(TOOL_EDITOR),
                })
                .expect("available"),
            ToolResp::Available { available: false }
        );
        assert_eq!(
            crate::actions::dispatch(
                &snapline_core::actions::ActionId::new(TOOL_EDITOR),
                &fixture.services,
            )
            .expect_err("action"),
            ServiceError::NotYet
        );
    }

    #[test]
    fn a_cancelled_editor_result_does_not_run_an_action() {
        let fixture = fixture();
        let ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Image(image(3, 4)));
        let outcome = fixture.editor(&ports).open_clipboard().expect("open");
        assert!(outcome.cancelled);
        assert!(ports.clipboard.images.lock().expect("images").is_empty());
        assert!(ports.saver.calls.lock().expect("calls").is_empty());
        assert!(ports.uploader.calls.lock().expect("calls").is_empty());
        assert_eq!(ports.pin.pins.load(Ordering::SeqCst), 0);
    }
}
