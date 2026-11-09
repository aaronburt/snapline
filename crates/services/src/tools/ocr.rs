use snapline_core::image::Image;
use snapline_core::ports::{Clipboard, ClipboardError, Ocr, OcrError};
use snapline_core::workflow::{TextPort, WorkflowError};
use snapline_store::history::CaptureId;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OcrToolError {
    #[error("{0}")]
    Ocr(OcrError),
    #[error("{0}")]
    Clipboard(ClipboardError),
    #[error("{0}")]
    History(String),
}

impl From<OcrError> for OcrToolError {
    fn from(error: OcrError) -> Self {
        Self::Ocr(error)
    }
}

impl From<ClipboardError> for OcrToolError {
    fn from(error: ClipboardError) -> Self {
        Self::Clipboard(error)
    }
}

impl From<OcrToolError> for crate::ServiceError {
    fn from(error: OcrToolError) -> Self {
        match error {
            OcrToolError::Ocr(error) => Self::Internal(error.to_string()),
            OcrToolError::Clipboard(ClipboardError::Busy) => Self::Busy,
            OcrToolError::Clipboard(error) => Self::Internal(error.to_string()),
            OcrToolError::History(message) => Self::Io(message),
        }
    }
}

pub trait OcrHistory: Send + Sync {
    fn set_ocr_text(&self, capture_id: CaptureId, text: &str) -> Result<(), OcrToolError>;
}

pub struct OcrHistoryFn<F>(F);

impl<F> OcrHistoryFn<F> {
    pub fn new(callback: F) -> Self {
        Self(callback)
    }
}

impl<F> OcrHistory for OcrHistoryFn<F>
where
    F: Fn(CaptureId, &str) -> Result<(), OcrToolError> + Send + Sync,
{
    fn set_ocr_text(&self, capture_id: CaptureId, text: &str) -> Result<(), OcrToolError> {
        (self.0)(capture_id, text)
    }
}

pub fn copied_toast(lines: usize) -> String {
    if lines == 1 {
        "Copied 1 line of text".to_owned()
    } else {
        format!("Copied {lines} lines of text")
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OcrRequest<'a> {
    pub image: &'a Image,
    pub language: Option<&'a str>,
    pub capture_id: Option<CaptureId>,
}

impl<'a> OcrRequest<'a> {
    pub fn new(image: &'a Image) -> Self {
        Self {
            image,
            language: None,
            capture_id: None,
        }
    }

    pub fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    pub fn for_capture(mut self, capture_id: CaptureId) -> Self {
        self.capture_id = Some(capture_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrOutcome {
    pub text: String,
    pub lines: usize,
    pub copied: bool,
    pub stored: bool,
}

impl OcrOutcome {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn toast(&self) -> Option<String> {
        if self.copied {
            Some(copied_toast(self.lines))
        } else {
            None
        }
    }
}

pub struct OcrTool<'a> {
    ocr: &'a dyn Ocr,
    clipboard: &'a dyn Clipboard,
    history: Option<&'a dyn OcrHistory>,
}

impl<'a> OcrTool<'a> {
    pub fn new(ocr: &'a dyn Ocr, clipboard: &'a dyn Clipboard) -> Self {
        Self {
            ocr,
            clipboard,
            history: None,
        }
    }

    pub fn with_history(mut self, history: &'a dyn OcrHistory) -> Self {
        self.history = Some(history);
        self
    }

    pub fn run(&self, request: OcrRequest<'_>) -> Result<OcrOutcome, OcrToolError> {
        let recognised = self.ocr.recognise(request.image, request.language)?;
        if recognised.is_empty() {
            return Ok(OcrOutcome {
                text: String::new(),
                lines: 0,
                copied: false,
                stored: false,
            });
        }
        let text = recognised.text();
        let lines = recognised.lines.len();
        self.clipboard.set_text(&text)?;
        let stored = match (request.capture_id, self.history) {
            (Some(capture_id), Some(history)) => {
                history.set_ocr_text(capture_id, &text)?;
                true
            }
            _ => false,
        };
        Ok(OcrOutcome {
            text,
            lines,
            copied: true,
            stored,
        })
    }
}

pub struct OcrTextPort<'a> {
    ocr: &'a dyn Ocr,
    language: Option<&'a str>,
}

impl<'a> OcrTextPort<'a> {
    pub fn new(ocr: &'a dyn Ocr) -> Self {
        Self {
            ocr,
            language: None,
        }
    }

    pub fn with_language(ocr: &'a dyn Ocr, language: &'a str) -> Self {
        Self {
            ocr,
            language: Some(language),
        }
    }
}

impl TextPort for OcrTextPort<'_> {
    fn extract(&self, image: &Image) -> Result<String, WorkflowError> {
        self.ocr
            .recognise(image, self.language)
            .map(|recognised| recognised.text())
            .map_err(|error| WorkflowError::Failed(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use snapline_core::ports::OcrText;

    use super::*;

    struct FakeOcr {
        result: Result<OcrText, OcrError>,
        languages: Mutex<Vec<Option<String>>>,
        sizes: Mutex<Vec<(u32, u32)>>,
    }

    impl FakeOcr {
        fn new(lines: &[&str]) -> Self {
            Self {
                result: Ok(OcrText {
                    lines: lines.iter().map(|line| (*line).to_owned()).collect(),
                }),
                languages: Mutex::new(Vec::new()),
                sizes: Mutex::new(Vec::new()),
            }
        }

        fn failing(error: OcrError) -> Self {
            Self {
                result: Err(error),
                languages: Mutex::new(Vec::new()),
                sizes: Mutex::new(Vec::new()),
            }
        }
    }

    impl Ocr for FakeOcr {
        fn recognise(&self, image: &Image, language: Option<&str>) -> Result<OcrText, OcrError> {
            self.languages
                .lock()
                .expect("languages")
                .push(language.map(str::to_owned));
            self.sizes
                .lock()
                .expect("sizes")
                .push((image.width(), image.height()));
            self.result.clone()
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        text: Mutex<Option<String>>,
        writes: AtomicUsize,
        fail: AtomicBool,
    }

    impl FakeClipboard {
        fn text(&self) -> Option<String> {
            self.text.lock().expect("text").clone()
        }
    }

    impl Clipboard for FakeClipboard {
        fn set_image(&self, _image: &Image) -> Result<(), ClipboardError> {
            Err(ClipboardError::Write)
        }

        fn set_text(&self, text: &str) -> Result<(), ClipboardError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(ClipboardError::Write);
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            *self.text.lock().expect("text") = Some(text.to_owned());
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct FakeHistory {
        stored: Mutex<Vec<(CaptureId, String)>>,
        fail: AtomicBool,
    }

    impl OcrHistory for FakeHistory {
        fn set_ocr_text(&self, capture_id: CaptureId, text: &str) -> Result<(), OcrToolError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(OcrToolError::History("no database".to_owned()));
            }
            self.stored
                .lock()
                .expect("stored")
                .push((capture_id, text.to_owned()));
            Ok(())
        }
    }

    fn image() -> Image {
        Image::bgra(4, 2).expect("image")
    }

    #[test]
    fn joins_recognised_lines_with_carriage_return_newline() {
        let image = image();
        let ocr = FakeOcr::new(&["one", "two", "three"]);
        let clipboard = FakeClipboard::default();
        let outcome = OcrTool::new(&ocr, &clipboard)
            .run(OcrRequest::new(&image))
            .expect("outcome");
        assert_eq!(outcome.text, "one\r\ntwo\r\nthree");
        assert_eq!(clipboard.text().as_deref(), Some("one\r\ntwo\r\nthree"));
    }

    #[test]
    fn counts_lines_for_the_toast() {
        let image = image();
        let ocr = FakeOcr::new(&["one", "two", "three"]);
        let clipboard = FakeClipboard::default();
        let outcome = OcrTool::new(&ocr, &clipboard)
            .run(OcrRequest::new(&image))
            .expect("outcome");
        assert_eq!(outcome.lines, 3);
        assert!(outcome.copied);
        assert_eq!(outcome.toast().as_deref(), Some("Copied 3 lines of text"));
    }

    #[test]
    fn singular_toast_names_one_line() {
        assert_eq!(copied_toast(1), "Copied 1 line of text");
        assert_eq!(copied_toast(0), "Copied 0 lines of text");
        assert_eq!(copied_toast(2), "Copied 2 lines of text");
        assert_eq!(copied_toast(21), "Copied 21 lines of text");
    }

    #[test]
    fn empty_result_is_not_copied_or_stored() {
        let image = image();
        let ocr = FakeOcr::new(&[]);
        let clipboard = FakeClipboard::default();
        let history = FakeHistory::default();
        let outcome = OcrTool::new(&ocr, &clipboard)
            .with_history(&history)
            .run(OcrRequest::new(&image).for_capture(7))
            .expect("outcome");
        assert!(outcome.is_empty());
        assert_eq!(outcome.lines, 0);
        assert!(!outcome.copied);
        assert!(!outcome.stored);
        assert_eq!(outcome.toast(), None);
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
        assert!(history.stored.lock().expect("stored").is_empty());
    }

    #[test]
    fn blank_lines_are_an_empty_result() {
        let image = image();
        let ocr = FakeOcr::new(&["", "   "]);
        let clipboard = FakeClipboard::default();
        let outcome = OcrTool::new(&ocr, &clipboard)
            .run(OcrRequest::new(&image))
            .expect("outcome");
        assert!(outcome.is_empty());
        assert_eq!(outcome.toast(), None);
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn no_language_error_keeps_the_core_text() {
        let image = image();
        let ocr = FakeOcr::failing(OcrError::NoLanguage);
        let clipboard = FakeClipboard::default();
        let error = OcrTool::new(&ocr, &clipboard)
            .run(OcrRequest::new(&image))
            .expect_err("no language");
        assert_eq!(error, OcrToolError::Ocr(OcrError::NoLanguage));
        assert_eq!(
            error.to_string(),
            "Windows has no text recognition language installed. Add one in Settings > Time & language > Language & region."
        );
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn saved_capture_text_goes_to_the_history_writer() {
        let image = image();
        let ocr = FakeOcr::new(&["hello", "world"]);
        let clipboard = FakeClipboard::default();
        let history = FakeHistory::default();
        let outcome = OcrTool::new(&ocr, &clipboard)
            .with_history(&history)
            .run(OcrRequest::new(&image).for_capture(42))
            .expect("outcome");
        assert!(outcome.stored);
        assert_eq!(
            history.stored.lock().expect("stored").as_slice(),
            &[(42, "hello\r\nworld".to_owned())]
        );
    }

    #[test]
    fn saved_capture_without_a_writer_still_copies() {
        let image = image();
        let ocr = FakeOcr::new(&["hello"]);
        let clipboard = FakeClipboard::default();
        let outcome = OcrTool::new(&ocr, &clipboard)
            .run(OcrRequest::new(&image).for_capture(42))
            .expect("outcome");
        assert!(outcome.copied);
        assert!(!outcome.stored);
    }

    #[test]
    fn clipboard_failure_is_reported() {
        let image = image();
        let ocr = FakeOcr::new(&["hello"]);
        let clipboard = FakeClipboard::default();
        clipboard.fail.store(true, Ordering::SeqCst);
        let error = OcrTool::new(&ocr, &clipboard)
            .run(OcrRequest::new(&image))
            .expect_err("clipboard failure");
        assert_eq!(error, OcrToolError::Clipboard(ClipboardError::Write));
    }

    #[test]
    fn requested_language_reaches_the_recogniser() {
        let image = image();
        let ocr = FakeOcr::new(&["hello"]);
        let clipboard = FakeClipboard::default();
        OcrTool::new(&ocr, &clipboard)
            .run(OcrRequest::new(&image).with_language("en-GB"))
            .expect("outcome");
        assert_eq!(
            ocr.languages.lock().expect("languages").as_slice(),
            &[Some("en-GB".to_owned())]
        );
        assert_eq!(ocr.sizes.lock().expect("sizes").as_slice(), &[(4, 2)]);
    }

    #[test]
    fn history_failure_is_reported() {
        let image = image();
        let ocr = FakeOcr::new(&["hello"]);
        let clipboard = FakeClipboard::default();
        let history = FakeHistory::default();
        history.fail.store(true, Ordering::SeqCst);
        let error = OcrTool::new(&ocr, &clipboard)
            .with_history(&history)
            .run(OcrRequest::new(&image).for_capture(1))
            .expect_err("history failure");
        assert_eq!(error, OcrToolError::History("no database".to_owned()));
    }

    #[test]
    fn workflow_port_reads_text() {
        let image = image();
        let ocr = FakeOcr::new(&["alpha", "beta"]);
        let port = OcrTextPort::new(&ocr);
        assert_eq!(port.extract(&image), Ok("alpha\r\nbeta".to_owned()));
    }

    #[test]
    fn workflow_port_reports_no_language() {
        let image = image();
        let ocr = FakeOcr::failing(OcrError::NoLanguage);
        let port = OcrTextPort::new(&ocr);
        assert_eq!(
            port.extract(&image),
            Err(WorkflowError::Failed(OcrError::NoLanguage.to_string()))
        );
    }
}
