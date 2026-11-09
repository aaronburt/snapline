use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use snapline_core::combine::{self as core_combine, CombineError, CombineOptions};
use snapline_core::image::{Image, ImageFormat};
use snapline_core::ports::{CodecError, ImageCodec};
use snapline_store::history::{CaptureId, CaptureKind, HistoryRepo};
use snapline_store::io::Paths;
use thiserror::Error;

use crate::saver::scale_to_width;
use crate::tools::ToolService;
use crate::{ServiceError, Services};

pub const PREVIEW_WIDTH: u32 = 960;
pub const PREVIEW_QUALITY: u8 = 90;
pub const MAX_PREVIEWS: usize = 16;
pub const MAX_PICKED_FILES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Source {
    History { id: CaptureId },
    Picked { id: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombineRequest {
    pub sources: Vec<Source>,
    pub options: CombineOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedImage {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub capture_id: CaptureId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

pub trait CombineHost: Send + Sync {
    fn pick_images(&self) -> Result<Vec<PathBuf>, ServiceError>;
    fn save_capture(&self, image: Image) -> Result<Option<CaptureId>, ServiceError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CombineToolError {
    #[error("{0}")]
    Layout(#[from] CombineError),
    #[error("The combined image could not be saved.")]
    Save,
}

impl From<CombineToolError> for ServiceError {
    fn from(error: CombineToolError) -> Self {
        match error {
            CombineToolError::Layout(error) => layout_error(error),
            CombineToolError::Save => Self::Io("The combined image could not be saved.".to_owned()),
        }
    }
}

#[derive(Default)]
struct Registry {
    next_job: u32,
    next_file: u32,
    previews: VecDeque<(u32, Preview)>,
    files: VecDeque<(u32, PathBuf, String)>,
}

impl Registry {
    fn insert_preview(&mut self, preview: Preview) -> u32 {
        let id = self.next_job.checked_add(1).unwrap_or(1);
        self.next_job = id;
        while self.previews.len() >= MAX_PREVIEWS {
            self.previews.pop_front();
        }
        self.previews.push_back((id, preview));
        id
    }

    fn insert_file(&mut self, path: PathBuf, name: String) -> u32 {
        let id = self.next_file.checked_add(1).unwrap_or(1);
        self.next_file = id;
        while self.files.len() >= MAX_PICKED_FILES {
            self.files.pop_front();
        }
        self.files.push_back((id, path, name));
        id
    }

    fn preview(&self, id: u32) -> Option<Preview> {
        if id == 0 {
            return None;
        }
        self.previews
            .iter()
            .find(|(stored, _)| *stored == id)
            .map(|(_, preview)| preview.clone())
    }

    fn file(&self, id: u32) -> Option<PathBuf> {
        if id == 0 {
            return None;
        }
        self.files
            .iter()
            .find(|(stored, _, _)| *stored == id)
            .map(|(_, path, _)| path.clone())
    }
}

fn shared_registry() -> Arc<Mutex<Registry>> {
    static REGISTRY: OnceLock<Arc<Mutex<Registry>>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(Registry::default())))
        .clone()
}

static SHARED_HOST: OnceLock<Arc<dyn CombineHost>> = OnceLock::new();

fn shared_host() -> Option<Arc<dyn CombineHost>> {
    SHARED_HOST.get().cloned()
}

pub fn install_host(host: Arc<dyn CombineHost>) {
    let _ = SHARED_HOST.set(host);
}

pub fn parse_id(raw: &str) -> Option<u32> {
    let digits = raw.strip_prefix('/').unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id = digits.parse::<u32>().ok()?;
    (id > 0).then_some(id)
}

pub fn preview(id: u32) -> Option<Preview> {
    if id == 0 {
        return None;
    }
    shared_registry().lock().ok()?.preview(id)
}

#[derive(Clone)]
pub struct CombineService {
    services: Services,
    host: Option<Arc<dyn CombineHost>>,
    registry: Arc<Mutex<Registry>>,
}

impl CombineService {
    pub fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
            host: shared_host(),
            registry: shared_registry(),
        }
    }

    pub fn with_host(services: &Services, host: Arc<dyn CombineHost>) -> Self {
        Self {
            services: services.clone(),
            host: Some(host),
            registry: shared_registry(),
        }
    }

    pub fn pick_files(&self) -> Result<Vec<PickedImage>, ServiceError> {
        let host = self.host.as_ref().ok_or(ServiceError::NotYet)?;
        let paths = host.pick_images()?;
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| ServiceError::Internal("The combiner is unavailable.".to_owned()))?;
        let mut picked = Vec::new();
        for path in paths {
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let id = registry.insert_file(path, name.clone());
            picked.push(PickedImage { id, name });
        }
        Ok(picked)
    }

    pub fn preview(&self, request: &CombineRequest) -> Result<u32, ServiceError> {
        let (images, codec) = self.images(&request.sources)?;
        let composed = core_combine::compose(&images, &request.options).map_err(layout_error)?;
        let preview = scale_to_width(&composed, PREVIEW_WIDTH)?;
        let (bytes, format) = encode_preview(&preview, codec.as_ref())?;
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| ServiceError::Internal("The combiner is unavailable.".to_owned()))?;
        Ok(registry.insert_preview(Preview {
            bytes,
            mime: format.mime(),
        }))
    }

    pub fn save(&self, request: &CombineRequest) -> Result<SaveResult, ServiceError> {
        let host = self.host.as_ref().ok_or(ServiceError::NotYet)?;
        let (images, _) = self.images(&request.sources)?;
        let composed = core_combine::compose(&images, &request.options).map_err(layout_error)?;
        let capture_id = host.save_capture(composed)?.ok_or(CombineToolError::Save)?;
        Ok(SaveResult { capture_id })
    }

    fn images(
        &self,
        sources: &[Source],
    ) -> Result<(Vec<Image>, Arc<dyn ImageCodec>), ServiceError> {
        if sources.is_empty() {
            return Err(layout_error(CombineError::Empty));
        }
        let paths = self.services.paths()?;
        let codec = self.services.codec()?;
        let mut images = Vec::with_capacity(sources.len());
        for source in sources {
            let path = match source {
                Source::History { id } => self.history_path(&paths, *id)?,
                Source::Picked { id } => self
                    .registry
                    .lock()
                    .map_err(|_| ServiceError::Internal("The combiner is unavailable.".to_owned()))?
                    .file(*id)
                    .ok_or(ServiceError::NotFound)?,
            };
            images.push(decode(&path, codec.as_ref())?);
        }
        Ok((images, codec))
    }

    fn history_path(&self, paths: &Paths, id: CaptureId) -> Result<PathBuf, ServiceError> {
        if id <= 0 {
            return Err(ServiceError::NotFound);
        }
        let repo = HistoryRepo::open_read(&paths.history_file).map_err(|error| {
            tracing::warn!(%error, "combine history could not be opened");
            ServiceError::Io("The History could not be read.".to_owned())
        })?;
        let capture = repo
            .get(id)
            .map_err(|error| {
                tracing::warn!(%error, "combine history capture could not be read");
                ServiceError::Io("The History could not be read.".to_owned())
            })?
            .ok_or(ServiceError::NotFound)?;
        if capture.kind != CaptureKind::Image {
            return Err(ServiceError::Invalid(
                "Only images can be combined.".to_owned(),
            ));
        }
        let path = PathBuf::from(capture.path);
        if !path.is_file() {
            return Err(ServiceError::NotFound);
        }
        Ok(path)
    }
}

fn decode(path: &Path, codec: &dyn ImageCodec) -> Result<Image, ServiceError> {
    let bytes = fs::read(path).map_err(|error| {
        tracing::warn!(%error, "combine image could not be read");
        ServiceError::Invalid("An image could not be read.".to_owned())
    })?;
    codec.decode(&bytes).map_err(|error| {
        tracing::warn!(%error, "combine image could not be decoded");
        ServiceError::Invalid("An image could not be read.".to_owned())
    })
}

fn encode_preview(
    image: &Image,
    codec: &dyn ImageCodec,
) -> Result<(Vec<u8>, ImageFormat), ServiceError> {
    match codec.encode(image, ImageFormat::Png, 100) {
        Ok(bytes) => Ok((bytes, ImageFormat::Png)),
        Err(CodecError::Unsupported) => codec
            .encode(image, ImageFormat::WebP, PREVIEW_QUALITY)
            .map(|bytes| (bytes, ImageFormat::WebP))
            .map_err(|error| {
                tracing::warn!(%error, "combine preview could not be encoded");
                ServiceError::Internal("The preview could not be made.".to_owned())
            }),
        Err(error) => {
            tracing::warn!(%error, "combine preview could not be encoded");
            Err(ServiceError::Internal(
                "The preview could not be made.".to_owned(),
            ))
        }
    }
}

fn layout_error(error: CombineError) -> ServiceError {
    match error {
        CombineError::Empty => ServiceError::Invalid("Choose at least one image.".to_owned()),
        CombineError::InvalidSpacing => {
            ServiceError::Invalid("Spacing must be between 0 and 64 px.".to_owned())
        }
        CombineError::InvalidHex => ServiceError::Invalid(
            "Enter the background colour as a hex value, for example #1A2B3C.".to_owned(),
        ),
        CombineError::Overflow | CombineError::Allocate => {
            ServiceError::Invalid("The combined image is too large.".to_owned())
        }
    }
}

impl ToolService {
    pub fn combine_pick_files(&self) -> Result<Vec<PickedImage>, ServiceError> {
        CombineService::new(&self._services).pick_files()
    }

    pub fn combine_preview(
        &self,
        sources: Vec<Source>,
        options: CombineOptions,
    ) -> Result<u32, ServiceError> {
        CombineService::new(&self._services).preview(&CombineRequest { sources, options })
    }

    pub fn combine_save(
        &self,
        sources: Vec<Source>,
        options: CombineOptions,
    ) -> Result<SaveResult, ServiceError> {
        CombineService::new(&self._services).save(&CombineRequest { sources, options })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use snapline_core::combine::{Background, Layout};
    use snapline_store::history::NewCapture;
    use tempfile::TempDir;

    use super::*;

    struct FakeCodec {
        png: bool,
        sizes: Mutex<Vec<(u32, u32)>>,
        formats: Mutex<Vec<ImageFormat>>,
    }

    impl FakeCodec {
        fn new() -> Self {
            Self {
                png: true,
                sizes: Mutex::new(Vec::new()),
                formats: Mutex::new(Vec::new()),
            }
        }

        fn sizes(&self) -> Vec<(u32, u32)> {
            self.sizes.lock().expect("sizes").clone()
        }

        fn formats(&self) -> Vec<ImageFormat> {
            self.formats.lock().expect("formats").clone()
        }
    }

    impl ImageCodec for FakeCodec {
        fn encode(
            &self,
            image: &Image,
            format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            self.formats.lock().expect("formats").push(format);
            self.sizes
                .lock()
                .expect("sizes")
                .push((image.width(), image.height()));
            if format == ImageFormat::Png && !self.png {
                return Err(CodecError::Unsupported);
            }
            Ok(vec![format as u8, 1, 2, 3])
        }

        fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError> {
            let (width, height) = match bytes.first() {
                Some(b'a') => (100, 50),
                Some(b'b') => (40, 80),
                Some(b'w') => (2000, 1000),
                _ => return Err(CodecError::Decode("unknown test image".to_owned())),
            };
            Image::bgra(width, height).map_err(|error| CodecError::Decode(error.to_string()))
        }
    }

    #[derive(Default)]
    struct FakeHost {
        files: Mutex<Vec<PathBuf>>,
        saved: Mutex<Option<(u32, u32)>>,
        picks: AtomicUsize,
    }

    impl CombineHost for FakeHost {
        fn pick_images(&self) -> Result<Vec<PathBuf>, ServiceError> {
            self.picks.fetch_add(1, Ordering::SeqCst);
            Ok(self.files.lock().expect("files").clone())
        }

        fn save_capture(&self, image: Image) -> Result<Option<CaptureId>, ServiceError> {
            *self.saved.lock().expect("saved") = Some((image.width(), image.height()));
            Ok(Some(99))
        }
    }

    struct Fixture {
        _directory: TempDir,
        services: Services,
        paths: Paths,
        codec: Arc<FakeCodec>,
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
        let codec = Arc::new(FakeCodec::new());
        let factory_codec = codec.clone();
        let services = Services::with_paths(paths.clone())
            .expect("services")
            .with_codec_factory(Arc::new(move || {
                Ok(factory_codec.clone() as Arc<dyn ImageCodec>)
            }));
        Fixture {
            _directory: directory,
            services,
            paths,
            codec,
        }
    }

    impl Fixture {
        fn service(&self, host: Option<Arc<dyn CombineHost>>) -> CombineService {
            CombineService {
                services: self.services.clone(),
                host,
                registry: Arc::new(Mutex::new(Registry::default())),
            }
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

    fn request(sources: Vec<Source>) -> CombineRequest {
        CombineRequest {
            sources,
            options: CombineOptions::default(),
        }
    }

    fn history(id: CaptureId) -> Source {
        Source::History { id }
    }

    #[test]
    fn only_positive_numbers_are_job_ids() {
        assert_eq!(parse_id("/1"), Some(1));
        assert_eq!(parse_id("42"), Some(42));
        assert_eq!(parse_id("/007"), Some(7));
        assert_eq!(parse_id("/"), None);
        assert_eq!(parse_id(""), None);
        assert_eq!(parse_id("/0"), None);
        assert_eq!(parse_id("0"), None);
        assert_eq!(parse_id("/-1"), None);
        assert_eq!(parse_id("/+1"), None);
        assert_eq!(parse_id("/abc"), None);
        assert_eq!(parse_id("/1a"), None);
        assert_eq!(parse_id("/1.5"), None);
        assert_eq!(parse_id("/1/2"), None);
        assert_eq!(parse_id("//1"), None);
        assert_eq!(parse_id("/../1"), None);
        assert_eq!(parse_id("/%31"), None);
        assert_eq!(parse_id("/4294967296"), None);
        assert_eq!(parse_id("/ 1"), None);
        assert_eq!(preview(0), None);
    }

    #[test]
    fn an_empty_request_is_invalid() {
        let fixture = fixture();
        let service = fixture.service(None);
        let error = service.preview(&request(Vec::new())).expect_err("empty");
        assert!(matches!(error, ServiceError::Invalid(_)));
        assert_eq!(error.code(), "invalid");
    }

    #[test]
    fn a_history_preview_returns_a_numeric_job_and_a_png() {
        let fixture = fixture();
        let id = fixture.capture("one.png", b'w', CaptureKind::Image);
        let service = fixture.service(None);
        let job = service
            .preview(&request(vec![history(id)]))
            .expect("preview");
        assert!(job > 0);
        let stored = service
            .registry
            .lock()
            .expect("registry")
            .preview(job)
            .expect("stored");
        assert_eq!(stored.mime, "image/png");
        assert_eq!(stored.bytes.first(), Some(&(ImageFormat::Png as u8)));
        assert_eq!(fixture.codec.sizes(), vec![(PREVIEW_WIDTH, 480)]);
        assert_eq!(fixture.codec.formats(), vec![ImageFormat::Png]);
    }

    #[test]
    fn a_preview_falls_back_to_webp_when_png_is_unsupported() {
        let fixture = fixture();
        let id = fixture.capture("one.png", b'a', CaptureKind::Image);
        let codec = Arc::new(FakeCodec {
            png: false,
            ..FakeCodec::new()
        });
        let factory_codec = codec.clone();
        let services = Services::with_paths(fixture.paths.clone())
            .expect("services")
            .with_codec_factory(Arc::new(move || {
                Ok(factory_codec.clone() as Arc<dyn ImageCodec>)
            }));
        let service = CombineService {
            services,
            host: None,
            registry: Arc::new(Mutex::new(Registry::default())),
        };
        let job = service
            .preview(&request(vec![history(id)]))
            .expect("preview");
        let stored = service
            .registry
            .lock()
            .expect("registry")
            .preview(job)
            .expect("stored");
        assert_eq!(stored.mime, "image/webp");
        assert_eq!(codec.formats(), vec![ImageFormat::Png, ImageFormat::WebP]);
    }

    #[test]
    fn saved_images_use_the_full_size_and_the_host_pipeline() {
        let fixture = fixture();
        let first = fixture.capture("one.png", b'a', CaptureKind::Image);
        let second = fixture.capture("two.png", b'b', CaptureKind::Image);
        let host = Arc::new(FakeHost::default());
        let service = fixture.service(Some(host.clone()));
        let result = service
            .save(&request(vec![history(first), history(second)]))
            .expect("save");
        assert_eq!(result.capture_id, 99);
        assert_eq!(*host.saved.lock().expect("saved"), Some((148, 80)));
        assert!(fixture.codec.sizes().is_empty());
    }

    #[test]
    fn saving_needs_a_host() {
        let fixture = fixture();
        let id = fixture.capture("one.png", b'a', CaptureKind::Image);
        assert_eq!(
            fixture
                .service(None)
                .save(&request(vec![history(id)]))
                .expect_err("host"),
            ServiceError::NotYet
        );
    }

    #[test]
    fn picked_files_get_positive_ids_and_compose() {
        let fixture = fixture();
        let path = fixture.paths.captures_dir.join("picked.png");
        std::fs::write(&path, [b'a']).expect("picked file");
        let host = Arc::new(FakeHost {
            files: Mutex::new(vec![path.clone()]),
            ..FakeHost::default()
        });
        let service = fixture.service(Some(host.clone()));
        let picked = service.pick_files().expect("pick");
        assert_eq!(picked.len(), 1);
        assert!(picked[0].id > 0);
        assert_eq!(picked[0].name, "picked.png");
        assert_eq!(host.picks.load(Ordering::SeqCst), 1);
        let job = service
            .preview(&request(vec![Source::Picked { id: picked[0].id }]))
            .expect("preview");
        assert!(job > 0);
    }

    #[test]
    fn picked_files_need_the_picker() {
        let fixture = fixture();
        assert_eq!(
            fixture.service(None).pick_files().expect_err("picker"),
            ServiceError::NotYet
        );
    }

    #[test]
    fn missing_and_unknown_sources_are_not_found() {
        let fixture = fixture();
        let service = fixture.service(None);
        assert_eq!(
            service
                .preview(&request(vec![history(9)]))
                .expect_err("missing"),
            ServiceError::NotFound
        );
        assert_eq!(
            service
                .preview(&request(vec![Source::Picked { id: 7 }]))
                .expect_err("picked"),
            ServiceError::NotFound
        );
        assert_eq!(
            service
                .preview(&request(vec![history(0)]))
                .expect_err("zero"),
            ServiceError::NotFound
        );
    }

    #[test]
    fn video_captures_cannot_be_combined() {
        let fixture = fixture();
        let id = fixture.capture("clip.mp4", b'a', CaptureKind::Video);
        let error = fixture
            .service(None)
            .preview(&request(vec![history(id)]))
            .expect_err("video");
        assert!(matches!(error, ServiceError::Invalid(_)));
    }

    #[test]
    fn layout_limits_are_reported_as_invalid() {
        let fixture = fixture();
        let id = fixture.capture("one.png", b'a', CaptureKind::Image);
        let service = fixture.service(None);
        let mut spacing = request(vec![history(id)]);
        spacing.options.spacing = 65;
        let error = service.preview(&spacing).expect_err("spacing");
        assert_eq!(error.code(), "invalid");
        let mut background = request(vec![history(id)]);
        background.options.background = Background::Custom("#nope".to_owned());
        let error = service.preview(&background).expect_err("hex");
        assert_eq!(error.code(), "invalid");
    }

    #[test]
    fn stacked_layouts_are_passed_through_to_the_combiner() {
        let fixture = fixture();
        let first = fixture.capture("one.png", b'a', CaptureKind::Image);
        let second = fixture.capture("two.png", b'b', CaptureKind::Image);
        let host = Arc::new(FakeHost::default());
        let service = fixture.service(Some(host.clone()));
        let mut combine = request(vec![history(first), history(second)]);
        combine.options.layout = Layout::Stacked;
        combine.options.spacing = 6;
        let result = service.save(&combine).expect("save");
        assert_eq!(result.capture_id, 99);
        assert_eq!(*host.saved.lock().expect("saved"), Some((100, 136)));
    }

    #[test]
    fn the_registry_drops_the_oldest_preview() {
        let fixture = fixture();
        let service = fixture.service(None);
        let stored = Preview {
            bytes: vec![1],
            mime: "image/png",
        };
        let mut registry = service.registry.lock().expect("registry");
        let mut ids = Vec::new();
        for _ in 0..(MAX_PREVIEWS + 2) {
            ids.push(registry.insert_preview(stored.clone()));
        }
        assert!(registry.preview(ids[0]).is_none());
        assert!(registry.preview(ids[1]).is_none());
        assert!(registry.preview(ids[ids.len() - 1]).is_some());
    }
}
