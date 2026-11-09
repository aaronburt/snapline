use std::fs;
use std::path::Path;

use snapline_core::image::{Image, ImageFormat};
use snapline_core::ports::{Clipboard, ClipboardError, CodecError, ImageCodec};
use snapline_core::qr::{self as core_qr, QrError, QrScan};
use thiserror::Error;

use crate::dto::tool::ToolResp;
use crate::tools::ToolService;
use crate::ServiceError;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QrToolError {
    #[error("{0}")]
    Qr(#[from] QrError),
    #[error("{0}")]
    Clipboard(#[from] ClipboardError),
    #[error("{0}")]
    Codec(#[from] CodecError),
    #[error("Saving a QR code as a PNG is not available.")]
    NotYet,
    #[error("The QR code could not be saved: {0}")]
    Save(String),
}

impl From<QrToolError> for ServiceError {
    fn from(error: QrToolError) -> Self {
        match error {
            QrToolError::Qr(error) => service_error(error),
            QrToolError::Clipboard(ClipboardError::Busy) => Self::Busy,
            QrToolError::Clipboard(error) => Self::Internal(error.to_string()),
            QrToolError::Codec(error) => Self::Internal(error.to_string()),
            QrToolError::NotYet => Self::NotYet,
            QrToolError::Save(message) => Self::Io(message),
        }
    }
}

fn service_error(error: QrError) -> ServiceError {
    match error {
        QrError::NotFound => ServiceError::NotFound,
        other => ServiceError::Invalid(other.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MadeQr {
    pub svg: String,
    pub link: Option<String>,
}

impl MadeQr {
    pub fn is_url(&self) -> bool {
        self.link.is_some()
    }
}

pub fn make(text: &str) -> Result<MadeQr, QrError> {
    Ok(MadeQr {
        svg: core_qr::make_svg(text)?,
        link: core_qr::link_of(text),
    })
}

pub fn scan(image: &Image) -> Result<QrScan, QrError> {
    core_qr::scan(image)
}

pub struct QrTool<'a> {
    clipboard: &'a dyn Clipboard,
    codec: Option<&'a dyn ImageCodec>,
}

impl<'a> QrTool<'a> {
    pub fn new(clipboard: &'a dyn Clipboard) -> Self {
        Self {
            clipboard,
            codec: None,
        }
    }

    pub fn with_codec(mut self, codec: &'a dyn ImageCodec) -> Self {
        self.codec = Some(codec);
        self
    }

    pub fn make(&self, text: &str) -> Result<MadeQr, QrToolError> {
        Ok(make(text)?)
    }

    pub fn copy_image(&self, text: &str) -> Result<(), QrToolError> {
        let image = core_qr::make_image(text)?;
        self.clipboard.set_image(&image)?;
        Ok(())
    }

    pub fn save_png(&self, text: &str, path: &Path) -> Result<(), QrToolError> {
        let codec = self.codec.ok_or(QrToolError::NotYet)?;
        let image = core_qr::make_image(text)?;
        let bytes = codec.encode(&image, ImageFormat::Png, 0)?;
        fs::write(path, bytes).map_err(|error| QrToolError::Save(error.to_string()))?;
        Ok(())
    }

    pub fn scan(&self, image: &Image) -> Result<QrScan, QrToolError> {
        Ok(core_qr::scan(image)?)
    }

    pub fn scan_and_copy(&self, image: &Image) -> Result<QrScan, QrToolError> {
        let scanned = core_qr::scan(image)?;
        self.clipboard.set_text(&scanned.text)?;
        Ok(scanned)
    }
}

impl ToolService {
    pub fn qr_make(&self, text: &str) -> Result<ToolResp, ServiceError> {
        let made = make(text).map_err(service_error)?;
        Ok(ToolResp::QrMade {
            svg: made.svg,
            link: made.link,
        })
    }

    pub fn qr_copy_image(&self, text: &str) -> Result<ToolResp, ServiceError> {
        let image = core_qr::make_image(text).map_err(service_error)?;
        let codec = self._services.codec()?;
        let bytes = codec
            .encode(&image, ImageFormat::Png, 0)
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        let host = self
            ._services
            .history_host()
            .ok_or(ServiceError::NotYet)?;
        host.set_clipboard_image(&bytes)?;
        Ok(ToolResp::QrCopied)
    }

    pub fn qr_open(&self, link: &str) -> Result<ToolResp, ServiceError> {
        let link = core_qr::link_of(link).ok_or_else(|| {
            ServiceError::Invalid("Only http and https links can be opened.".to_owned())
        })?;
        let host = self
            ._services
            .history_host()
            .ok_or(ServiceError::NotYet)?;
        host.open_url(&link)?;
        Ok(ToolResp::QrOpened)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use snapline_core::ports::CodecError;

    use super::*;
    use crate::dto::tool::ToolReq;
    use crate::history::HistoryHost;
    use crate::Services;

    #[derive(Default)]
    struct FakeClipboard {
        image: Mutex<Option<Image>>,
        text: Mutex<Option<String>>,
        fail: Mutex<bool>,
    }

    impl Clipboard for FakeClipboard {
        fn set_image(&self, image: &Image) -> Result<(), ClipboardError> {
            if *self.fail.lock().expect("fail") {
                return Err(ClipboardError::Write);
            }
            *self.image.lock().expect("image") = Some(image.clone());
            Ok(())
        }

        fn set_text(&self, text: &str) -> Result<(), ClipboardError> {
            if *self.fail.lock().expect("fail") {
                return Err(ClipboardError::Write);
            }
            *self.text.lock().expect("text") = Some(text.to_owned());
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    struct FakeCodec {
        formats: Mutex<Vec<ImageFormat>>,
        bytes: Vec<u8>,
    }

    impl FakeCodec {
        fn new(bytes: &[u8]) -> Self {
            Self {
                formats: Mutex::new(Vec::new()),
                bytes: bytes.to_vec(),
            }
        }
    }

    impl ImageCodec for FakeCodec {
        fn encode(
            &self,
            _image: &Image,
            format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            self.formats.lock().expect("formats").push(format);
            Ok(self.bytes.clone())
        }

        fn decode(&self, _bytes: &[u8]) -> Result<Image, CodecError> {
            Err(CodecError::Unsupported)
        }
    }

    #[derive(Default)]
    struct FakeHost {
        image: Mutex<Option<Vec<u8>>>,
        opened: Mutex<Option<String>>,
    }

    impl HistoryHost for FakeHost {
        fn confirm_delete(&self, _message: &str) -> Result<bool, ServiceError> {
            Ok(true)
        }

        fn set_clipboard_image(&self, bytes: &[u8]) -> Result<(), ServiceError> {
            *self.image.lock().expect("image") = Some(bytes.to_vec());
            Ok(())
        }

        fn show_in_folder(&self, _path: &Path) -> Result<(), ServiceError> {
            Ok(())
        }

        fn open_file(&self, _path: &Path) -> Result<(), ServiceError> {
            Ok(())
        }

        fn open_url(&self, url: &str) -> Result<(), ServiceError> {
            *self.opened.lock().expect("opened") = Some(url.to_owned());
            Ok(())
        }

        fn recycle_file(&self, _path: &Path) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    fn services_with_host(host: &Arc<FakeHost>) -> Services {
        Services::new()
            .with_codec_factory(Arc::new(|| {
                Ok(Arc::new(FakeCodec::new(b"png")) as Arc<dyn ImageCodec>)
            }))
            .with_history_host(host.clone())
    }

    #[test]
    fn make_returns_svg_and_detects_a_link() {
        let made = make("https://example.com").expect("made");
        assert!(made.svg.starts_with("<svg "));
        assert_eq!(made.link.as_deref(), Some("https://example.com"));
        assert!(made.is_url());
        assert_eq!(make("hello").expect("made").link, None);
    }

    #[test]
    fn copy_image_puts_a_black_and_white_image_on_the_clipboard() {
        let clipboard = FakeClipboard::default();
        QrTool::new(&clipboard).copy_image("hello").expect("copy");
        let image = clipboard
            .image
            .lock()
            .expect("image")
            .clone()
            .expect("image");
        assert_eq!(image.width(), image.height());
        assert!(image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel == [0, 0, 0, 255]));
        assert!(image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel == [255, 255, 255, 255]));
    }

    #[test]
    fn save_png_encodes_with_the_app_codec() {
        let clipboard = FakeClipboard::default();
        let codec = FakeCodec::new(b"png bytes");
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("qr.png");
        QrTool::new(&clipboard)
            .with_codec(&codec)
            .save_png("hello", &path)
            .expect("save");
        assert_eq!(fs::read(&path).expect("read"), b"png bytes");
        assert_eq!(
            codec.formats.lock().expect("formats").as_slice(),
            &[ImageFormat::Png]
        );
    }

    #[test]
    fn save_without_a_codec_is_not_yet() {
        let clipboard = FakeClipboard::default();
        let directory = tempfile::tempdir().expect("directory");
        let error = QrTool::new(&clipboard)
            .save_png("hello", &directory.path().join("qr.png"))
            .expect_err("no codec");
        assert_eq!(error, QrToolError::NotYet);
        assert_eq!(ServiceError::from(error), ServiceError::NotYet);
    }

    #[test]
    fn scan_returns_the_text_and_link() {
        let clipboard = FakeClipboard::default();
        let image = core_qr::make_image("https://example.com").expect("image");
        let scanned = QrTool::new(&clipboard).scan(&image).expect("scan");
        assert_eq!(scanned.text, "https://example.com");
        assert_eq!(scanned.link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn scan_and_copy_writes_the_text() {
        let clipboard = FakeClipboard::default();
        let image = core_qr::make_image("copied text").expect("image");
        let scanned = QrTool::new(&clipboard)
            .scan_and_copy(&image)
            .expect("scan");
        assert_eq!(scanned.text, "copied text");
        assert_eq!(
            clipboard.text.lock().expect("text").as_deref(),
            Some("copied text")
        );
    }

    #[test]
    fn clipboard_failures_are_reported() {
        let clipboard = FakeClipboard::default();
        *clipboard.fail.lock().expect("fail") = true;
        let image = core_qr::make_image("hello").expect("image");
        assert_eq!(
            QrTool::new(&clipboard).copy_image("hello"),
            Err(QrToolError::Clipboard(ClipboardError::Write))
        );
        assert_eq!(
            QrTool::new(&clipboard).scan_and_copy(&image),
            Err(QrToolError::Clipboard(ClipboardError::Write))
        );
    }

    #[test]
    fn not_found_maps_to_the_not_found_code() {
        assert_eq!(service_error(QrError::NotFound), ServiceError::NotFound);
        assert_eq!(
            service_error(QrError::EmptyText).code(),
            "invalid"
        );
        assert_eq!(
            service_error(QrError::UnsupportedImage).code(),
            "invalid"
        );
    }

    #[test]
    fn tool_make_round_trips_through_the_ipc_shape() {
        let services = Services::new();
        let response = services
            .tool()
            .handle(ToolReq::QrMake {
                text: "https://example.com".to_owned(),
            })
            .expect("make");
        match response {
            ToolResp::QrMade { svg, link } => {
                assert!(svg.starts_with("<svg "));
                assert_eq!(link.as_deref(), Some("https://example.com"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn tool_scan_is_not_yet_without_an_overlay_image() {
        let services = Services::new();
        assert_eq!(
            services.tool().handle(ToolReq::QrScan { capture_id: None }),
            Err(ServiceError::NotYet)
        );
        assert_eq!(
            services.tool().handle(ToolReq::QrScan { capture_id: Some(1) }),
            Err(ServiceError::NotYet)
        );
    }

    #[test]
    fn tool_copy_image_encodes_and_sets_the_clipboard() {
        let host = Arc::new(FakeHost::default());
        let services = services_with_host(&host);
        assert_eq!(
            services.tool().handle(ToolReq::QrCopyImage {
                text: "hello".to_owned(),
            }),
            Ok(ToolResp::QrCopied)
        );
        assert_eq!(
            host.image.lock().expect("image").as_deref(),
            Some(b"png".as_slice())
        );
    }

    #[test]
    fn tool_copy_image_needs_a_host_and_codec() {
        let services = Services::new();
        assert_eq!(
            services.tool().handle(ToolReq::QrCopyImage {
                text: "hello".to_owned(),
            }),
            Err(ServiceError::NotYet)
        );
    }

    #[test]
    fn tool_open_allows_only_http_and_https() {
        let host = Arc::new(FakeHost::default());
        let services = Services::new().with_history_host(host.clone());
        assert_eq!(
            services.tool().handle(ToolReq::QrOpen {
                link: "https://example.com".to_owned(),
            }),
            Ok(ToolResp::QrOpened)
        );
        assert_eq!(
            host.opened.lock().expect("opened").as_deref(),
            Some("https://example.com")
        );
        let error = services
            .tool()
            .handle(ToolReq::QrOpen {
                link: "ftp://example.com".to_owned(),
            })
            .expect_err("ftp");
        assert!(matches!(error, ServiceError::Invalid(_)));
    }

    #[test]
    fn tool_copy_text_and_save_wait_for_the_app_host() {
        let services = Services::new();
        assert_eq!(
            services.tool().handle(ToolReq::QrCopyText {
                text: "hello".to_owned(),
            }),
            Err(ServiceError::NotYet)
        );
        assert_eq!(
            services.tool().handle(ToolReq::QrSavePng {
                text: "hello".to_owned(),
            }),
            Err(ServiceError::NotYet)
        );
    }
}
