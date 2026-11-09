use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use snapline_core::image::{Image, ImageFormat};
use snapline_core::ports::{CodecError, ImageCodec};
use snapline_store::history::{CaptureId, HistoryRepo};
use snapline_store::io::{write_atomic, Paths};
use thiserror::Error;

use crate::saver::{scale_to_width, THUMBNAIL_WIDTH};
use crate::Services;

pub const THUMBNAIL_QUALITY: u8 = 90;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ThumbError {
    #[error("The requested item was not found.")]
    NotFound,
    #[error("The thumbnail could not be prepared.")]
    Unavailable,
}

pub fn parse_id(raw: &str) -> Option<CaptureId> {
    let digits = raw.strip_prefix('/').unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id = digits.parse::<CaptureId>().ok()?;
    (id > 0).then_some(id)
}

pub fn thumbnail_path(thumbs_dir: &Path, id: CaptureId, format: ImageFormat) -> PathBuf {
    thumbs_dir.join(format!("{id}.{}", format.extension()))
}

pub fn existing_thumbnail(thumbs_dir: &Path, id: CaptureId) -> Option<(PathBuf, ImageFormat)> {
    if id <= 0 {
        return None;
    }
    for format in [ImageFormat::WebP, ImageFormat::Jpeg] {
        let path = thumbnail_path(thumbs_dir, id, format);
        if path.is_file() {
            return Some((path, format));
        }
    }
    None
}

pub fn write_thumbnail(
    capture_path: &Path,
    thumbs_dir: &Path,
    id: CaptureId,
    quality: u8,
    codec: &dyn ImageCodec,
) -> Result<(PathBuf, ImageFormat), ThumbError> {
    if id <= 0 {
        return Err(ThumbError::NotFound);
    }
    let bytes = match std::fs::read(capture_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(ThumbError::NotFound),
        Err(_) => return Err(ThumbError::Unavailable),
    };
    let image = codec.decode(&bytes).map_err(|_| ThumbError::Unavailable)?;
    let thumbnail = scale_to_width(&image, THUMBNAIL_WIDTH).map_err(|_| ThumbError::Unavailable)?;
    let (thumbnail, format) = encode_thumbnail(&thumbnail, quality, codec)?;
    let path = thumbnail_path(thumbs_dir, id, format);
    write_atomic(&path, &thumbnail).map_err(|_| ThumbError::Unavailable)?;
    for other in [ImageFormat::WebP, ImageFormat::Jpeg] {
        let stale = thumbnail_path(thumbs_dir, id, other);
        if stale != path && stale.is_file() {
            let _ = std::fs::remove_file(stale);
        }
    }
    Ok((path, format))
}

fn encode_thumbnail(
    image: &Image,
    quality: u8,
    codec: &dyn ImageCodec,
) -> Result<(Vec<u8>, ImageFormat), ThumbError> {
    match codec.encode(image, ImageFormat::WebP, quality) {
        Ok(bytes) => Ok((bytes, ImageFormat::WebP)),
        Err(CodecError::Unsupported) => codec
            .encode(image, ImageFormat::Jpeg, quality)
            .map(|bytes| (bytes, ImageFormat::Jpeg))
            .map_err(|_| ThumbError::Unavailable),
        Err(_) => Err(ThumbError::Unavailable),
    }
}

pub fn load(
    paths: &Paths,
    id: CaptureId,
    quality: u8,
    codec: &dyn ImageCodec,
) -> Result<Thumbnail, ThumbError> {
    if id <= 0 {
        return Err(ThumbError::NotFound);
    }
    if let Some((path, format)) = existing_thumbnail(&paths.thumbs_dir, id) {
        return read_thumbnail(&path, format);
    }
    let capture = capture_file(paths, id)?;
    let (path, format) = write_thumbnail(&capture, &paths.thumbs_dir, id, quality, codec)?;
    read_thumbnail(&path, format)
}

fn capture_file(paths: &Paths, id: CaptureId) -> Result<PathBuf, ThumbError> {
    let repo = HistoryRepo::open_read(&paths.history_file).map_err(|_| ThumbError::NotFound)?;
    let capture = repo.get(id).map_err(|_| ThumbError::Unavailable)?;
    let capture = capture.ok_or(ThumbError::NotFound)?;
    let path = PathBuf::from(capture.path);
    if !path.is_file() {
        return Err(ThumbError::NotFound);
    }
    Ok(path)
}

fn read_thumbnail(path: &Path, format: ImageFormat) -> Result<Thumbnail, ThumbError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(ThumbError::NotFound),
        Err(_) => return Err(ThumbError::Unavailable),
    };
    Ok(Thumbnail {
        bytes,
        mime: format.mime(),
    })
}

pub struct ThumbService {
    services: Services,
}

impl ThumbService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }

    pub fn get(&self, id: CaptureId) -> Result<Thumbnail, ThumbError> {
        if id <= 0 {
            return Err(ThumbError::NotFound);
        }
        let paths = self.services.paths().map_err(|_| ThumbError::Unavailable)?;
        if let Some((path, format)) = existing_thumbnail(&paths.thumbs_dir, id) {
            return read_thumbnail(&path, format);
        }
        let quality = self
            .services
            .settings_snapshot()
            .map(|settings| settings.general.lossy_quality)
            .unwrap_or(THUMBNAIL_QUALITY);
        let codec = self.services.codec().map_err(|_| ThumbError::Unavailable)?;
        load(&paths, id, quality, codec.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapline_core::image::Image;
    use snapline_store::history::{CaptureKind, NewCapture};
    use tempfile::tempdir;

    fn test_paths(directory: &Path) -> Paths {
        Paths::resolve(
            &directory.join("exe"),
            &directory.join("roaming"),
            &directory.join("local"),
            &directory.join("user"),
            false,
        )
    }

    fn record_capture(paths: &Paths, capture_path: &Path) -> CaptureId {
        let mut repo = HistoryRepo::open_write(&paths.history_file).expect("history");
        repo.record(NewCapture {
            kind: CaptureKind::Image,
            path: capture_path.to_string_lossy().into_owned(),
            bytes: 1,
            width: Some(640),
            height: Some(480),
            duration_ms: None,
            source_app: Some("test.exe".to_owned()),
            created_at: 1,
            ocr_text: None,
        })
        .expect("capture")
    }

    fn write_thumbnail_file(paths: &Paths, id: CaptureId, format: ImageFormat, bytes: &[u8]) {
        let path = thumbnail_path(&paths.thumbs_dir, id, format);
        std::fs::create_dir_all(&paths.thumbs_dir).expect("thumb folder");
        std::fs::write(path, bytes).expect("thumb file");
    }

    struct FakeCodec {
        webp: bool,
        sizes: std::sync::Mutex<Vec<(u32, u32)>>,
    }

    impl FakeCodec {
        fn new(webp: bool) -> Self {
            Self {
                webp,
                sizes: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn sizes(&self) -> Vec<(u32, u32)> {
            self.sizes.lock().expect("sizes").clone()
        }
    }

    impl ImageCodec for FakeCodec {
        fn encode(
            &self,
            image: &Image,
            format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            self.sizes
                .lock()
                .expect("sizes")
                .push((image.width(), image.height()));
            if format == ImageFormat::WebP && !self.webp {
                return Err(CodecError::Unsupported);
            }
            Ok(vec![7_u8; 8])
        }

        fn decode(&self, _bytes: &[u8]) -> Result<Image, CodecError> {
            Image::bgra(640, 480).map_err(|error| CodecError::Decode(error.to_string()))
        }
    }

    struct UnusedCodec;

    impl ImageCodec for UnusedCodec {
        fn encode(
            &self,
            _image: &Image,
            _format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            Err(CodecError::Unsupported)
        }

        fn decode(&self, _bytes: &[u8]) -> Result<Image, CodecError> {
            Err(CodecError::Decode("The codec is not used.".to_owned()))
        }
    }

    #[test]
    fn only_positive_numbers_are_ids() {
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
        assert_eq!(parse_id("/9223372036854775808"), None);
        assert_eq!(parse_id("/ 1"), None);
    }

    #[test]
    fn the_path_uses_the_encoder_extension() {
        let directory = tempdir().expect("temp");
        let thumbs = directory.path();
        assert_eq!(
            thumbnail_path(thumbs, 7, ImageFormat::WebP),
            thumbs.join("7.webp")
        );
        assert_eq!(
            thumbnail_path(thumbs, 7, ImageFormat::Jpeg),
            thumbs.join("7.jpg")
        );
        assert_eq!(ImageFormat::WebP.mime(), "image/webp");
        assert_eq!(ImageFormat::Jpeg.mime(), "image/jpeg");
    }

    #[test]
    fn invalid_ids_are_not_found_before_any_disk_access() {
        let directory = tempdir().expect("temp");
        let paths = test_paths(directory.path());
        for id in [0, -1, -500, CaptureId::MIN] {
            assert_eq!(
                load(&paths, id, 90, &UnusedCodec),
                Err(ThumbError::NotFound)
            );
        }
        assert!(!paths.history_file.exists());
        assert!(!paths.thumbs_dir.exists());
    }

    #[test]
    fn an_existing_webp_thumbnail_is_served_without_rewriting() {
        let directory = tempdir().expect("temp");
        let paths = test_paths(directory.path());
        let id = record_capture(&paths, &directory.path().join("missing.png"));
        write_thumbnail_file(&paths, id, ImageFormat::WebP, b"existing webp");
        let thumbnail = load(&paths, id, 90, &UnusedCodec).expect("thumbnail");
        assert_eq!(thumbnail.bytes, b"existing webp");
        assert_eq!(thumbnail.mime, "image/webp");
        let path = thumbnail_path(&paths.thumbs_dir, id, ImageFormat::WebP);
        assert_eq!(std::fs::read(path).expect("kept"), b"existing webp");
    }

    #[test]
    fn an_existing_jpeg_thumbnail_is_served_as_jpeg() {
        let directory = tempdir().expect("temp");
        let paths = test_paths(directory.path());
        let id = record_capture(&paths, &directory.path().join("missing.png"));
        write_thumbnail_file(&paths, id, ImageFormat::Jpeg, b"existing jpeg");
        let thumbnail = load(&paths, id, 90, &UnusedCodec).expect("thumbnail");
        assert_eq!(thumbnail.bytes, b"existing jpeg");
        assert_eq!(thumbnail.mime, "image/jpeg");
    }

    #[test]
    fn a_missing_capture_is_not_found() {
        let directory = tempdir().expect("temp");
        let paths = test_paths(directory.path());
        let id = record_capture(&paths, &directory.path().join("missing.png"));
        assert_eq!(
            load(&paths, id, 90, &UnusedCodec),
            Err(ThumbError::NotFound)
        );
        assert!(existing_thumbnail(&paths.thumbs_dir, id).is_none());
    }

    #[test]
    fn an_unknown_id_is_not_found() {
        let directory = tempdir().expect("temp");
        let paths = test_paths(directory.path());
        record_capture(&paths, &directory.path().join("missing.png"));
        assert_eq!(load(&paths, 2, 90, &UnusedCodec), Err(ThumbError::NotFound));
        assert_eq!(
            load(&paths, CaptureId::MAX, 90, &UnusedCodec),
            Err(ThumbError::NotFound)
        );
    }

    #[test]
    fn a_valid_capture_regenerates_a_320px_webp_thumbnail() {
        let directory = tempdir().expect("temp");
        let paths = test_paths(directory.path());
        let capture_path = directory.path().join("capture.png");
        std::fs::write(&capture_path, b"source").expect("capture file");
        let id = record_capture(&paths, &capture_path);
        let codec = FakeCodec::new(true);
        let thumbnail = load(&paths, id, 90, &codec).expect("thumbnail");
        assert_eq!(thumbnail.mime, "image/webp");
        assert_eq!(codec.sizes(), vec![(THUMBNAIL_WIDTH, 240)]);
        let (path, format) = existing_thumbnail(&paths.thumbs_dir, id).expect("written");
        assert_eq!(format, ImageFormat::WebP);
        assert_eq!(std::fs::read(path).expect("bytes"), thumbnail.bytes);
    }

    #[test]
    fn a_codec_without_webp_writes_a_jpeg_thumbnail() {
        let directory = tempdir().expect("temp");
        let paths = test_paths(directory.path());
        let capture_path = directory.path().join("capture.png");
        std::fs::write(&capture_path, b"source").expect("capture file");
        let id = record_capture(&paths, &capture_path);
        let codec = FakeCodec::new(false);
        let thumbnail = load(&paths, id, 90, &codec).expect("thumbnail");
        assert_eq!(thumbnail.mime, "image/jpeg");
        assert_eq!(
            codec.sizes(),
            vec![(THUMBNAIL_WIDTH, 240), (THUMBNAIL_WIDTH, 240)]
        );
        let (path, format) = existing_thumbnail(&paths.thumbs_dir, id).expect("written");
        assert_eq!(format, ImageFormat::Jpeg);
        assert_eq!(std::fs::read(path).expect("bytes"), thumbnail.bytes);
    }
}
