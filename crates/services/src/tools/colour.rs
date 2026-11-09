use std::sync::Mutex;

use snapline_core::colour::{ColourFormat, Rgb};
use snapline_core::geometry::Point;
use snapline_core::ports::{CaptureError, Clipboard, ClipboardError, FrozenDesktop, ScreenCapture};

use crate::ServiceError;

pub const ACTION: &str = "tool_colour";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickedColour {
    pub colour: Rgb,
    pub format: ColourFormat,
    pub position: Point,
    pub monitor: Option<u32>,
}

pub trait ColourPicker: Send + Sync {
    fn pick(
        &self,
        desktop: &FrozenDesktop,
        format: ColourFormat,
    ) -> Result<PickedColour, ServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColourOutcome {
    pub colour: Rgb,
    pub format: ColourFormat,
    pub text: String,
    pub position: Point,
    pub monitor: Option<u32>,
}

pub struct ColourTool<'a> {
    capture: &'a dyn ScreenCapture,
    picker: &'a dyn ColourPicker,
    clipboard: &'a dyn Clipboard,
    format: Mutex<ColourFormat>,
}

impl<'a> ColourTool<'a> {
    pub fn new(
        capture: &'a dyn ScreenCapture,
        picker: &'a dyn ColourPicker,
        clipboard: &'a dyn Clipboard,
    ) -> Self {
        Self {
            capture,
            picker,
            clipboard,
            format: Mutex::new(ColourFormat::default()),
        }
    }

    pub fn with_format(self, format: ColourFormat) -> Self {
        Self {
            format: Mutex::new(format),
            ..self
        }
    }

    pub fn format(&self) -> ColourFormat {
        self.format
            .lock()
            .map_or(ColourFormat::default(), |format| *format)
    }

    pub fn run(&self) -> Result<ColourOutcome, ServiceError> {
        let desktop = self
            .capture
            .freeze_all()
            .map_err(|error| capture_error(&error))?;
        self.pick(&desktop)
    }

    pub fn pick(&self, desktop: &FrozenDesktop) -> Result<ColourOutcome, ServiceError> {
        let picked = self.picker.pick(desktop, self.format())?;
        let text = picked.colour.display(picked.format);
        self.clipboard
            .set_text(&text)
            .map_err(|error| clipboard_error(&error))?;
        if let Ok(mut format) = self.format.lock() {
            *format = picked.format;
        }
        Ok(ColourOutcome {
            colour: picked.colour,
            format: picked.format,
            text,
            position: picked.position,
            monitor: picked.monitor,
        })
    }
}

pub fn run(
    capture: &dyn ScreenCapture,
    picker: &dyn ColourPicker,
    clipboard: &dyn Clipboard,
    format: ColourFormat,
) -> Result<ColourOutcome, ServiceError> {
    ColourTool::new(capture, picker, clipboard)
        .with_format(format)
        .run()
}

fn capture_error(error: &CaptureError) -> ServiceError {
    ServiceError::Invalid(error.to_string())
}

fn clipboard_error(error: &ClipboardError) -> ServiceError {
    match error {
        ClipboardError::Busy => ServiceError::Busy,
        other => ServiceError::Internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use snapline_core::geometry::{DpiScale, Px, Rect};
    use snapline_core::image::Image;
    use snapline_core::ports::{FrozenMonitor, Monitor};

    use super::*;

    struct FakeCapture {
        result: Result<FrozenDesktop, CaptureError>,
        freezes: AtomicUsize,
    }

    impl FakeCapture {
        fn new() -> Self {
            Self {
                result: Ok(desktop()),
                freezes: AtomicUsize::new(0),
            }
        }

        fn failing(error: CaptureError) -> Self {
            Self {
                result: Err(error),
                freezes: AtomicUsize::new(0),
            }
        }
    }

    impl ScreenCapture for FakeCapture {
        fn monitors(&self) -> Vec<Monitor> {
            self.result.as_ref().map_or_else(
                |_| Vec::new(),
                |desktop| {
                    desktop
                        .monitors
                        .iter()
                        .map(|frozen| frozen.monitor.clone())
                        .collect()
                },
            )
        }

        fn freeze_all(&self) -> Result<FrozenDesktop, CaptureError> {
            self.freezes.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }

        fn window(
            &self,
            _id: snapline_core::ids::WindowId,
            _keep_shadow: bool,
        ) -> Result<Image, CaptureError> {
            Err(CaptureError::WindowMissing)
        }
    }

    struct FakePicker {
        result: PickedColour,
        failure: Option<String>,
        formats: Mutex<Vec<ColourFormat>>,
        monitors: Mutex<Vec<usize>>,
    }

    impl FakePicker {
        fn new(colour: Rgb, format: ColourFormat) -> Self {
            Self {
                result: PickedColour {
                    colour,
                    format,
                    position: Point::new(Px(280), Px(160)),
                    monitor: Some(1),
                },
                failure: None,
                formats: Mutex::new(Vec::new()),
                monitors: Mutex::new(Vec::new()),
            }
        }

        fn failing(error: ServiceError) -> Self {
            Self {
                result: PickedColour {
                    colour: Rgb::new(0, 0, 0),
                    format: ColourFormat::Hex,
                    position: Point::ORIGIN,
                    monitor: None,
                },
                failure: Some(error.to_string()),
                formats: Mutex::new(Vec::new()),
                monitors: Mutex::new(Vec::new()),
            }
        }

        fn formats(&self) -> Vec<ColourFormat> {
            self.formats
                .lock()
                .map_or_else(|_| Vec::new(), |value| value.clone())
        }
    }

    impl ColourPicker for FakePicker {
        fn pick(
            &self,
            desktop: &FrozenDesktop,
            format: ColourFormat,
        ) -> Result<PickedColour, ServiceError> {
            self.formats.lock().expect("formats").push(format);
            self.monitors
                .lock()
                .expect("monitors")
                .push(desktop.monitors.len());
            if let Some(message) = &self.failure {
                return Err(ServiceError::Invalid(message.clone()));
            }
            Ok(self.result)
        }
    }

    struct FakeClipboard {
        text: Mutex<Option<String>>,
        fail: Option<ClipboardError>,
        writes: AtomicUsize,
    }

    impl FakeClipboard {
        fn new() -> Self {
            Self {
                text: Mutex::new(None),
                fail: None,
                writes: AtomicUsize::new(0),
            }
        }

        fn failing(error: ClipboardError) -> Self {
            Self {
                text: Mutex::new(None),
                fail: Some(error),
                writes: AtomicUsize::new(0),
            }
        }

        fn text(&self) -> Option<String> {
            self.text.lock().expect("text").clone()
        }
    }

    impl Clipboard for FakeClipboard {
        fn set_image(&self, _image: &Image) -> Result<(), ClipboardError> {
            Err(ClipboardError::Write)
        }

        fn set_text(&self, text: &str) -> Result<(), ClipboardError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = &self.fail {
                return Err(error.clone());
            }
            *self.text.lock().expect("text") = Some(text.to_owned());
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    fn monitor() -> Monitor {
        Monitor {
            id: 1,
            name: "Test".to_owned(),
            bounds: Rect::new(Px(0), Px(0), Px(1920), Px(1080)),
            work_area: Rect::new(Px(0), Px(0), Px(1920), Px(1040)),
            dpi: DpiScale::ONE,
            primary: true,
        }
    }

    fn desktop() -> FrozenDesktop {
        FrozenDesktop {
            monitors: vec![FrozenMonitor {
                monitor: monitor(),
                image: Image::bgra(4, 4).expect("image"),
            }],
        }
    }

    #[test]
    fn run_freezes_picks_and_copies_hex_by_default() {
        let capture = FakeCapture::new();
        let picker = FakePicker::new(Rgb::new(229, 72, 77), ColourFormat::Hex);
        let clipboard = FakeClipboard::new();
        let tool = ColourTool::new(&capture, &picker, &clipboard);
        let outcome = tool.run().expect("outcome");
        assert_eq!(capture.freezes.load(Ordering::SeqCst), 1);
        assert_eq!(picker.formats(), vec![ColourFormat::Hex]);
        assert_eq!(picker.monitors.lock().expect("monitors").as_slice(), &[1]);
        assert_eq!(clipboard.text().as_deref(), Some("#E5484D"));
        assert_eq!(outcome.text, "#E5484D");
        assert_eq!(outcome.colour, Rgb::new(229, 72, 77));
        assert_eq!(outcome.format, ColourFormat::Hex);
        assert_eq!(outcome.position, Point::new(Px(280), Px(160)));
        assert_eq!(outcome.monitor, Some(1));
    }

    #[test]
    fn the_picker_format_is_copied_and_remembered() {
        let capture = FakeCapture::new();
        let picker = FakePicker::new(Rgb::new(18, 52, 86), ColourFormat::Hsl);
        let clipboard = FakeClipboard::new();
        let tool = ColourTool::new(&capture, &picker, &clipboard);
        let first = tool.run().expect("first");
        assert_eq!(first.format, ColourFormat::Hsl);
        assert_eq!(clipboard.text().as_deref(), Some("hsl(210, 65%, 20%)"));
        assert_eq!(tool.format(), ColourFormat::Hsl);
        let second = tool.run().expect("second");
        assert_eq!(second.format, ColourFormat::Hsl);
        assert_eq!(picker.formats(), vec![ColourFormat::Hex, ColourFormat::Hsl]);
    }

    #[test]
    fn the_seeded_format_reaches_the_picker() {
        let capture = FakeCapture::new();
        let picker = FakePicker::new(Rgb::new(18, 52, 86), ColourFormat::Rgb);
        let clipboard = FakeClipboard::new();
        let tool = ColourTool::new(&capture, &picker, &clipboard).with_format(ColourFormat::Hsl);
        assert_eq!(tool.format(), ColourFormat::Hsl);
        let outcome = tool.run().expect("outcome");
        assert_eq!(picker.formats(), vec![ColourFormat::Hsl]);
        assert_eq!(outcome.text, "rgb(18, 52, 86)");
        assert_eq!(clipboard.text().as_deref(), Some("rgb(18, 52, 86)"));
    }

    #[test]
    fn a_clipboard_failure_is_busy_and_keeps_the_old_format() {
        let capture = FakeCapture::new();
        let picker = FakePicker::new(Rgb::new(1, 2, 3), ColourFormat::Hsl);
        let clipboard = FakeClipboard::failing(ClipboardError::Busy);
        let tool = ColourTool::new(&capture, &picker, &clipboard);
        let error = tool.run().expect_err("busy");
        assert_eq!(error, ServiceError::Busy);
        assert_eq!(tool.format(), ColourFormat::Hex);
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 1);
        assert_eq!(clipboard.text(), None);
    }

    #[test]
    fn a_picker_failure_does_not_touch_the_clipboard() {
        let capture = FakeCapture::new();
        let picker = FakePicker::failing(ServiceError::Invalid("cancelled".to_owned()));
        let clipboard = FakeClipboard::new();
        let error = ColourTool::new(&capture, &picker, &clipboard)
            .run()
            .expect_err("picker failure");
        assert_eq!(error, ServiceError::Invalid("cancelled".to_owned()));
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_capture_failure_does_not_open_the_picker() {
        let capture = FakeCapture::failing(CaptureError::Unavailable("no screen".to_owned()));
        let picker = FakePicker::new(Rgb::new(1, 2, 3), ColourFormat::Hex);
        let clipboard = FakeClipboard::new();
        let error = ColourTool::new(&capture, &picker, &clipboard)
            .run()
            .expect_err("capture failure");
        assert!(matches!(error, ServiceError::Invalid(_)));
        assert_eq!(picker.formats(), Vec::new());
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_action_entry_uses_the_supplied_format() {
        let capture = FakeCapture::new();
        let picker = FakePicker::new(Rgb::new(255, 214, 10), ColourFormat::Rgb);
        let clipboard = FakeClipboard::new();
        let outcome = run(&capture, &picker, &clipboard, ColourFormat::Rgb).expect("outcome");
        assert_eq!(picker.formats(), vec![ColourFormat::Rgb]);
        assert_eq!(outcome.text, "rgb(255, 214, 10)");
        assert_eq!(outcome.format, ColourFormat::Rgb);
    }

    #[test]
    fn the_action_id_matches_the_catalogue() {
        assert!(snapline_core::actions::CATALOGUE
            .iter()
            .any(|definition| definition.id == ACTION));
    }

    #[test]
    fn clipboard_internal_errors_are_reported() {
        let capture = FakeCapture::new();
        let picker = FakePicker::new(Rgb::new(1, 2, 3), ColourFormat::Hex);
        let clipboard = FakeClipboard::failing(ClipboardError::Write);
        let error = ColourTool::new(&capture, &picker, &clipboard)
            .run()
            .expect_err("write failure");
        assert!(matches!(error, ServiceError::Internal(_)));
    }
}
