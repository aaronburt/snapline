use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use snapline_core::image::Image;
use snapline_core::ports::Clipboard;
use snapline_store::settings::uploads::DestinationRef;
use snapline_store::settings::Settings;
use thiserror::Error;

use crate::dto::upload::{UploadReq, UploadResp};
use crate::saver::{CaptureRequest, SaveOutcome};
use crate::uploads::UploadService;
use crate::{CodecFactory, ServiceError, Services};

pub const SOURCE_APP: &str = "Clipboard";
pub const WINDOW_TITLE: &str = "Clipboard";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    Upload,
    Cancel,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmError {
    #[error("The confirmation dialog could not be shown.")]
    Unavailable,
}

pub trait ConfirmHost: Send + Sync {
    fn confirm(&self, message: &str) -> Result<Confirmation, ConfirmError>;
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardUploadError {
    #[error("There is no image on the clipboard.")]
    NoImage,
    #[error("The clipboard has more than one file. Copy a single image and try again.")]
    MultipleFiles,
    #[error("The clipboard could not be read.")]
    Clipboard,
    #[error("The image on the clipboard could not be read.")]
    Read,
    #[error("The file on the clipboard is not a supported image.")]
    Decode,
    #[error("Choose an upload destination for images in Settings.")]
    NoDestination,
    #[error("The image could not be saved.")]
    Save,
    #[error("The upload could not be started.")]
    Upload,
    #[error("The confirmation dialog could not be shown.")]
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardUploadRequest {
    pub ask: bool,
    pub destination: String,
    pub label: String,
}

impl ClipboardUploadRequest {
    pub fn new(ask: bool, destination: impl Into<String>) -> Self {
        let destination = destination.into();
        Self {
            ask,
            label: destination.clone(),
            destination,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn from_settings(settings: &Settings) -> Self {
        Self::new(
            settings.uploads.ask_before_clipboard_upload,
            settings.uploads.images.as_str(),
        )
    }

    fn has_destination(&self) -> bool {
        DestinationRef::is_upload_id(&self.destination)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardUploadOutcome {
    Queued {
        capture_id: i64,
        upload_id: u64,
        destination: String,
    },
    Cancelled,
}

pub trait ClipboardSource: Send + Sync {
    fn image(&self) -> Result<Option<Image>, ClipboardUploadError>;

    fn paths(&self) -> Result<Vec<PathBuf>, ClipboardUploadError> {
        Ok(Vec::new())
    }
}

pub trait ClipboardFilePaths: Send + Sync {
    fn paths(&self) -> Result<Vec<PathBuf>, ClipboardUploadError>;
}

pub struct CoreClipboardSource<C> {
    clipboard: Arc<C>,
    paths: Option<Arc<dyn ClipboardFilePaths>>,
}

impl<C: Clipboard + Send + Sync> CoreClipboardSource<C> {
    pub fn new(clipboard: Arc<C>) -> Self {
        Self {
            clipboard,
            paths: None,
        }
    }

    pub fn with_paths(mut self, paths: Arc<dyn ClipboardFilePaths>) -> Self {
        self.paths = Some(paths);
        self
    }
}

impl<C: Clipboard + Send + Sync> ClipboardSource for CoreClipboardSource<C> {
    fn image(&self) -> Result<Option<Image>, ClipboardUploadError> {
        match self.clipboard.get_image() {
            Ok(image) => Ok(image),
            Err(error) => {
                tracing::warn!(%error, "the clipboard image could not be read");
                Err(ClipboardUploadError::Clipboard)
            }
        }
    }

    fn paths(&self) -> Result<Vec<PathBuf>, ClipboardUploadError> {
        match &self.paths {
            Some(paths) => paths.paths(),
            None => Ok(Vec::new()),
        }
    }
}

pub trait CaptureSaver: Send + Sync {
    fn save(&self, request: CaptureRequest) -> Result<SaveOutcome, ServiceError>;
}

impl CaptureSaver for crate::worker::Handle {
    fn save(&self, request: CaptureRequest) -> Result<SaveOutcome, ServiceError> {
        self.save_capture(request)
    }
}

pub trait UploadQueue: Send + Sync {
    fn enqueue(&self, capture_id: i64, destination: &str) -> Result<u64, ServiceError>;
}

impl UploadQueue for UploadService {
    fn enqueue(&self, capture_id: i64, destination: &str) -> Result<u64, ServiceError> {
        let response = self.handle(UploadReq::UploadElsewhere {
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

#[derive(Clone)]
pub struct ClipboardUpload {
    source: Arc<dyn ClipboardSource>,
    codec: Arc<CodecFactory>,
    saver: Arc<dyn CaptureSaver>,
    queue: Arc<dyn UploadQueue>,
    confirm: Arc<dyn ConfirmHost>,
}

impl ClipboardUpload {
    pub fn new(
        source: Arc<dyn ClipboardSource>,
        codec: Arc<CodecFactory>,
        saver: Arc<dyn CaptureSaver>,
        queue: Arc<dyn UploadQueue>,
        confirm: Arc<dyn ConfirmHost>,
    ) -> Self {
        Self {
            source,
            codec,
            saver,
            queue,
            confirm,
        }
    }

    pub fn run(
        &self,
        request: &ClipboardUploadRequest,
    ) -> Result<ClipboardUploadOutcome, ClipboardUploadError> {
        if !request.has_destination() {
            return Err(ClipboardUploadError::NoDestination);
        }
        let image = self.read_image()?;
        if request.ask {
            let message = format!("Upload the image on the clipboard to {}?", request.label);
            match self
                .confirm
                .confirm(&message)
                .map_err(|_| ClipboardUploadError::Confirm)?
            {
                Confirmation::Cancel => return Ok(ClipboardUploadOutcome::Cancelled),
                Confirmation::Upload => {}
            }
        }
        let capture =
            CaptureRequest::new(image, Some(SOURCE_APP.to_owned()), WINDOW_TITLE.to_owned());
        let saved = self.saver.save(capture).map_err(|error| {
            tracing::warn!(%error, "the clipboard capture could not be saved");
            ClipboardUploadError::Save
        })?;
        let capture_id = saved.capture_id.ok_or(ClipboardUploadError::Save)?;
        let upload_id = self
            .queue
            .enqueue(capture_id, &request.destination)
            .map_err(|error| {
                tracing::warn!(%error, "the clipboard capture could not be queued for upload");
                ClipboardUploadError::Upload
            })?;
        Ok(ClipboardUploadOutcome::Queued {
            capture_id,
            upload_id,
            destination: request.destination.clone(),
        })
    }

    fn read_image(&self) -> Result<Image, ClipboardUploadError> {
        if let Some(image) = self.source.image()? {
            return Ok(image);
        }
        let paths = self.source.paths()?;
        match paths.as_slice() {
            [] => Err(ClipboardUploadError::NoImage),
            [path] => {
                let bytes = std::fs::read(path).map_err(|error| {
                    tracing::warn!(%error, "the clipboard file could not be read");
                    ClipboardUploadError::Read
                })?;
                let codec = (self.codec)().map_err(|error| {
                    tracing::warn!(%error, "the image codec is unavailable");
                    ClipboardUploadError::Decode
                })?;
                codec.decode(&bytes).map_err(|error| {
                    tracing::warn!(%error, "the clipboard file could not be decoded");
                    ClipboardUploadError::Decode
                })
            }
            _ => Err(ClipboardUploadError::MultipleFiles),
        }
    }
}

static SERVICE: OnceLock<Mutex<Option<ClipboardUpload>>> = OnceLock::new();

pub fn install(service: ClipboardUpload) -> Result<(), ServiceError> {
    let slot = SERVICE.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| {
        ServiceError::Internal("The clipboard upload service is unavailable.".to_owned())
    })?;
    if guard.is_some() {
        return Err(ServiceError::Internal(
            "The clipboard upload service is already installed.".to_owned(),
        ));
    }
    *guard = Some(service);
    Ok(())
}

pub fn installed() -> Result<ClipboardUpload, ServiceError> {
    let slot = SERVICE.get().ok_or(ServiceError::NotYet)?;
    slot.lock()
        .map_err(|_| {
            ServiceError::Internal("The clipboard upload service is unavailable.".to_owned())
        })?
        .clone()
        .ok_or(ServiceError::NotYet)
}

pub fn run(services: &Services) -> Result<(), ServiceError> {
    let settings = services.settings_snapshot()?;
    let request = ClipboardUploadRequest::from_settings(&settings);
    installed()?
        .run(&request)
        .map(|_| ())
        .map_err(|error| ServiceError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use snapline_core::image::ImageFormat;
    use snapline_core::ports::{CodecError, ImageCodec};

    use super::*;

    struct FakeClipboard {
        image: Mutex<Option<Image>>,
        paths: Mutex<Vec<PathBuf>>,
        failed: AtomicBool,
        image_calls: AtomicUsize,
    }

    impl FakeClipboard {
        fn new(image: Option<Image>, paths: Vec<PathBuf>) -> Self {
            Self {
                image: Mutex::new(image),
                paths: Mutex::new(paths),
                failed: AtomicBool::new(false),
                image_calls: AtomicUsize::new(0),
            }
        }
    }

    impl ClipboardSource for FakeClipboard {
        fn image(&self) -> Result<Option<Image>, ClipboardUploadError> {
            self.image_calls.fetch_add(1, Ordering::SeqCst);
            if self.failed.load(Ordering::SeqCst) {
                return Err(ClipboardUploadError::Clipboard);
            }
            Ok(self.image.lock().expect("image").clone())
        }

        fn paths(&self) -> Result<Vec<PathBuf>, ClipboardUploadError> {
            Ok(self.paths.lock().expect("paths").clone())
        }
    }

    struct FakeCodec {
        image: Image,
        decoded: Mutex<Vec<Vec<u8>>>,
        failed: AtomicBool,
    }

    impl FakeCodec {
        fn new(image: Image) -> Self {
            Self {
                image,
                decoded: Mutex::new(Vec::new()),
                failed: AtomicBool::new(false),
            }
        }
    }

    impl ImageCodec for FakeCodec {
        fn encode(
            &self,
            _image: &Image,
            _format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            Err(CodecError::Encode("not used".to_owned()))
        }

        fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError> {
            self.decoded.lock().expect("decoded").push(bytes.to_vec());
            if self.failed.load(Ordering::SeqCst) {
                return Err(CodecError::Decode("not an image".to_owned()));
            }
            Ok(self.image.clone())
        }
    }

    struct FakeConfirm {
        choice: Confirmation,
        messages: Mutex<Vec<String>>,
        failed: AtomicBool,
    }

    impl FakeConfirm {
        fn new(choice: Confirmation) -> Self {
            Self {
                choice,
                messages: Mutex::new(Vec::new()),
                failed: AtomicBool::new(false),
            }
        }
    }

    impl ConfirmHost for FakeConfirm {
        fn confirm(&self, message: &str) -> Result<Confirmation, ConfirmError> {
            self.messages
                .lock()
                .expect("messages")
                .push(message.to_owned());
            if self.failed.load(Ordering::SeqCst) {
                return Err(ConfirmError::Unavailable);
            }
            Ok(self.choice)
        }
    }

    struct FakeSaver {
        capture_id: Option<i64>,
        requests: Mutex<Vec<CaptureRequest>>,
        failed: AtomicBool,
    }

    impl FakeSaver {
        fn new(capture_id: Option<i64>) -> Self {
            Self {
                capture_id,
                requests: Mutex::new(Vec::new()),
                failed: AtomicBool::new(false),
            }
        }
    }

    impl CaptureSaver for FakeSaver {
        fn save(&self, request: CaptureRequest) -> Result<SaveOutcome, ServiceError> {
            self.requests.lock().expect("requests").push(request);
            if self.failed.load(Ordering::SeqCst) {
                return Err(ServiceError::Internal("not shown".to_owned()));
            }
            Ok(SaveOutcome {
                capture_id: self.capture_id,
                path: None,
                file_name: None,
                bytes: 4,
                width: 2,
                height: 2,
                copied: false,
                saved: true,
                next_counter: None,
            })
        }
    }

    struct FakeQueue {
        upload_id: u64,
        calls: Mutex<Vec<(i64, String)>>,
        failed: AtomicBool,
    }

    impl FakeQueue {
        fn new(upload_id: u64) -> Self {
            Self {
                upload_id,
                calls: Mutex::new(Vec::new()),
                failed: AtomicBool::new(false),
            }
        }
    }

    impl UploadQueue for FakeQueue {
        fn enqueue(&self, capture_id: i64, destination: &str) -> Result<u64, ServiceError> {
            self.calls
                .lock()
                .expect("calls")
                .push((capture_id, destination.to_owned()));
            if self.failed.load(Ordering::SeqCst) {
                return Err(ServiceError::Internal("not shown".to_owned()));
            }
            Ok(self.upload_id)
        }
    }

    fn image() -> Image {
        let mut image = Image::bgra(2, 2).expect("image");
        image.set_pixel(0, 0, [1, 2, 3, 255]).expect("pixel");
        image
    }

    struct Fixture {
        source: Arc<FakeClipboard>,
        codec: Arc<FakeCodec>,
        confirm: Arc<FakeConfirm>,
        saver: Arc<FakeSaver>,
        queue: Arc<FakeQueue>,
    }

    impl Fixture {
        fn new(image: Option<Image>, paths: Vec<PathBuf>, choice: Confirmation) -> Self {
            Self {
                source: Arc::new(FakeClipboard::new(image, paths)),
                codec: Arc::new(FakeCodec::new(Self::decoded())),
                confirm: Arc::new(FakeConfirm::new(choice)),
                saver: Arc::new(FakeSaver::new(Some(7))),
                queue: Arc::new(FakeQueue::new(11)),
            }
        }

        fn decoded() -> Image {
            let mut image = Image::bgra(3, 1).expect("image");
            image.set_pixel(0, 0, [9, 9, 9, 255]).expect("pixel");
            image
        }

        fn service(&self) -> ClipboardUpload {
            let codec = Arc::clone(&self.codec);
            let factory: Arc<CodecFactory> =
                Arc::new(move || Ok(Arc::clone(&codec) as Arc<dyn ImageCodec>));
            ClipboardUpload::new(
                Arc::clone(&self.source) as Arc<dyn ClipboardSource>,
                factory,
                Arc::clone(&self.saver) as Arc<dyn CaptureSaver>,
                Arc::clone(&self.queue) as Arc<dyn UploadQueue>,
                Arc::clone(&self.confirm) as Arc<dyn ConfirmHost>,
            )
        }

        fn saver_requests(&self) -> Vec<CaptureRequest> {
            self.saver.requests.lock().expect("requests").clone()
        }

        fn queue_calls(&self) -> Vec<(i64, String)> {
            self.queue.calls.lock().expect("calls").clone()
        }

        fn confirm_messages(&self) -> Vec<String> {
            self.confirm.messages.lock().expect("messages").clone()
        }
    }

    fn request(ask: bool) -> ClipboardUploadRequest {
        ClipboardUploadRequest::new(ask, "imgur")
    }

    #[test]
    fn an_image_is_confirmed_saved_and_queued() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
        let outcome = fixture.service().run(&request(true)).expect("upload");
        assert_eq!(
            outcome,
            ClipboardUploadOutcome::Queued {
                capture_id: 7,
                upload_id: 11,
                destination: "imgur".to_owned(),
            }
        );
        assert_eq!(
            fixture.confirm_messages(),
            vec!["Upload the image on the clipboard to imgur?".to_owned()]
        );
        let requests = fixture.saver_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].source_app.as_deref(), Some(SOURCE_APP));
        assert_eq!(requests[0].window_title, WINDOW_TITLE);
        assert_eq!(fixture.queue_calls(), vec![(7, "imgur".to_owned())]);
    }

    #[test]
    fn a_single_file_path_is_decoded_and_queued() {
        let directory = tempfile::tempdir().expect("temp");
        let path = directory.path().join("clip.png");
        std::fs::write(&path, b"pixels").expect("write");
        let fixture = Fixture::new(None, vec![path], Confirmation::Upload);
        let outcome = fixture.service().run(&request(true)).expect("upload");
        assert!(matches!(
            outcome,
            ClipboardUploadOutcome::Queued {
                capture_id: 7,
                upload_id: 11,
                ..
            }
        ));
        assert_eq!(
            fixture.codec.decoded.lock().expect("decoded").as_slice(),
            [b"pixels".to_vec()].as_slice()
        );
        let requests = fixture.saver_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            (requests[0].image.width(), requests[0].image.height()),
            (3, 1)
        );
        assert_eq!(requests[0].source_app.as_deref(), Some(SOURCE_APP));
        assert_eq!(fixture.queue_calls(), vec![(7, "imgur".to_owned())]);
    }

    #[test]
    fn cancelling_saves_nothing_and_queues_nothing() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Cancel);
        let outcome = fixture.service().run(&request(true)).expect("cancel");
        assert_eq!(outcome, ClipboardUploadOutcome::Cancelled);
        assert_eq!(fixture.confirm_messages().len(), 1);
        assert!(fixture.saver_requests().is_empty());
        assert!(fixture.queue_calls().is_empty());
    }

    #[test]
    fn no_image_is_a_typed_error() {
        let fixture = Fixture::new(None, Vec::new(), Confirmation::Upload);
        let error = fixture.service().run(&request(true)).expect_err("no image");
        assert_eq!(error, ClipboardUploadError::NoImage);
        assert_eq!(error.to_string(), "There is no image on the clipboard.");
        assert!(fixture.confirm_messages().is_empty());
        assert!(fixture.saver_requests().is_empty());
        assert!(fixture.queue_calls().is_empty());
    }

    #[test]
    fn a_multi_file_clipboard_is_rejected() {
        let fixture = Fixture::new(
            None,
            vec![PathBuf::from("one.png"), PathBuf::from("two.png")],
            Confirmation::Upload,
        );
        let error = fixture.service().run(&request(true)).expect_err("files");
        assert_eq!(error, ClipboardUploadError::MultipleFiles);
        assert_eq!(
            error.to_string(),
            "The clipboard has more than one file. Copy a single image and try again."
        );
        assert!(fixture.confirm_messages().is_empty());
        assert!(fixture.saver_requests().is_empty());
        assert!(fixture.queue_calls().is_empty());
    }

    #[test]
    fn asking_can_be_turned_off() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Cancel);
        let outcome = fixture.service().run(&request(false)).expect("upload");
        assert!(matches!(
            outcome,
            ClipboardUploadOutcome::Queued { capture_id: 7, .. }
        ));
        assert!(fixture.confirm_messages().is_empty());
        assert_eq!(fixture.saver_requests().len(), 1);
        assert_eq!(fixture.queue_calls(), vec![(7, "imgur".to_owned())]);
    }

    #[test]
    fn the_capture_source_app_is_clipboard() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
        fixture.service().run(&request(false)).expect("upload");
        let requests = fixture.saver_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].source_app.as_deref(), Some("Clipboard"));
    }

    #[test]
    fn the_selected_destination_is_used_in_the_message_and_the_queue() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
        let request = ClipboardUploadRequest::new(true, "s3-production");
        let outcome = fixture.service().run(&request).expect("upload");
        assert_eq!(
            outcome,
            ClipboardUploadOutcome::Queued {
                capture_id: 7,
                upload_id: 11,
                destination: "s3-production".to_owned(),
            }
        );
        assert_eq!(
            fixture.confirm_messages(),
            vec!["Upload the image on the clipboard to s3-production?".to_owned()]
        );
        assert_eq!(fixture.queue_calls(), vec![(7, "s3-production".to_owned())]);
    }

    #[test]
    fn a_destination_label_is_used_in_the_message() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
        let request = ClipboardUploadRequest::new(true, "imgur").with_label("Imgur");
        fixture.service().run(&request).expect("upload");
        assert_eq!(
            fixture.confirm_messages(),
            vec!["Upload the image on the clipboard to Imgur?".to_owned()]
        );
        assert_eq!(fixture.queue_calls(), vec![(7, "imgur".to_owned())]);
    }

    #[test]
    fn save_locally_only_is_not_an_upload_destination() {
        for destination in ["", "none", "save_locally_only", "  "] {
            let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
            let error = fixture
                .service()
                .run(&ClipboardUploadRequest::new(true, destination))
                .expect_err("destination");
            assert_eq!(
                error,
                ClipboardUploadError::NoDestination,
                "{destination:?}"
            );
            assert_eq!(fixture.source.image_calls.load(Ordering::SeqCst), 0);
            assert!(fixture.confirm_messages().is_empty());
        }
    }

    #[test]
    fn the_settings_decide_the_dialog_and_the_destination() {
        let mut settings = Settings::default();
        assert!(settings.uploads.ask_before_clipboard_upload);
        settings.uploads.images = DestinationRef::new("imgur");
        let request = ClipboardUploadRequest::from_settings(&settings);
        assert!(request.ask);
        assert_eq!(request.destination, "imgur");

        settings.uploads.ask_before_clipboard_upload = false;
        let request = ClipboardUploadRequest::from_settings(&settings);
        assert!(!request.ask);
    }

    #[test]
    fn no_bytes_or_paths_are_reported_in_errors() {
        let directory = tempfile::tempdir().expect("temp");
        let path = directory.path().join("secret-file-name.png");
        std::fs::write(&path, b"TOP-SECRET-BYTES").expect("write");
        let fixture = Fixture::new(None, vec![path.clone()], Confirmation::Upload);
        fixture.codec.failed.store(true, Ordering::SeqCst);
        let error = fixture.service().run(&request(true)).expect_err("decode");
        let message = error.to_string();
        assert_eq!(
            message,
            "The file on the clipboard is not a supported image."
        );
        assert!(!message.contains("TOP-SECRET-BYTES"));
        assert!(!message.contains("secret-file-name"));
    }

    #[test]
    fn a_missing_capture_id_is_a_save_error() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
        let saver = Arc::new(FakeSaver::new(None));
        let codec = Arc::clone(&fixture.codec);
        let factory: Arc<CodecFactory> =
            Arc::new(move || Ok(Arc::clone(&codec) as Arc<dyn ImageCodec>));
        let service = ClipboardUpload::new(
            Arc::clone(&fixture.source) as Arc<dyn ClipboardSource>,
            factory,
            saver as Arc<dyn CaptureSaver>,
            Arc::clone(&fixture.queue) as Arc<dyn UploadQueue>,
            Arc::clone(&fixture.confirm) as Arc<dyn ConfirmHost>,
        );
        let error = service.run(&request(false)).expect_err("save");
        assert_eq!(error, ClipboardUploadError::Save);
        assert!(fixture.queue_calls().is_empty());
    }

    #[test]
    fn a_failed_confirmation_stops_the_upload() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
        fixture.confirm.failed.store(true, Ordering::SeqCst);
        let error = fixture.service().run(&request(true)).expect_err("dialog");
        assert_eq!(error, ClipboardUploadError::Confirm);
        assert!(fixture.saver_requests().is_empty());
        assert!(fixture.queue_calls().is_empty());
    }

    #[test]
    fn a_failed_queue_stops_after_saving() {
        let fixture = Fixture::new(Some(image()), Vec::new(), Confirmation::Upload);
        fixture.queue.failed.store(true, Ordering::SeqCst);
        let error = fixture.service().run(&request(false)).expect_err("queue");
        assert_eq!(error, ClipboardUploadError::Upload);
        assert_eq!(fixture.saver_requests().len(), 1);
    }

    #[test]
    fn a_failed_clipboard_read_is_a_typed_error() {
        let fixture = Fixture::new(None, Vec::new(), Confirmation::Upload);
        fixture.source.failed.store(true, Ordering::SeqCst);
        let error = fixture
            .service()
            .run(&request(true))
            .expect_err("clipboard");
        assert_eq!(error, ClipboardUploadError::Clipboard);
        assert_eq!(error.to_string(), "The clipboard could not be read.");
    }
}
