use snapline_core::actions::ActionId;
use snapline_core::geometry::Point;
use snapline_core::ports::{
    CaptureError, Clipboard, ClipboardError, FrozenDesktop, OverlayTheme, ScreenCapture,
};
use snapline_core::ruler::RulerMeasurement;
use thiserror::Error;

use crate::tools::ToolService;
use crate::ServiceError;

pub const RULER_ACTION: &str = "tool_ruler";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulerHostError {
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulerSelection {
    pub start: Point,
    pub end: Point,
    pub monitor: Option<u32>,
    pub copied: bool,
}

pub trait RulerHost: Send + Sync {
    fn measure(
        &self,
        desktop: &FrozenDesktop,
        theme: OverlayTheme,
    ) -> Result<RulerSelection, RulerHostError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RulerToolError {
    #[error("{0}")]
    Capture(CaptureError),
    #[error("{0}")]
    Clipboard(ClipboardError),
    #[error("The screen ruler was closed.")]
    Cancelled,
    #[error("{0}")]
    Host(String),
    #[error("There is no monitor to draw the ruler on.")]
    NoMonitors,
    #[error("{0}")]
    Invalid(String),
}

impl From<RulerToolError> for ServiceError {
    fn from(error: RulerToolError) -> Self {
        match error {
            RulerToolError::Capture(error) => Self::Internal(error.to_string()),
            RulerToolError::Clipboard(ClipboardError::Busy) => Self::Busy,
            RulerToolError::Clipboard(error) => Self::Internal(error.to_string()),
            RulerToolError::Cancelled => Self::Invalid("The screen ruler was closed.".to_owned()),
            RulerToolError::Host(message) => Self::Internal(message),
            RulerToolError::NoMonitors => {
                Self::Invalid("There is no monitor to draw the ruler on.".to_owned())
            }
            RulerToolError::Invalid(message) => Self::Invalid(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulerOutcome {
    pub measurement: RulerMeasurement,
    pub monitor: Option<u32>,
    pub copied: bool,
    pub text: String,
}

pub fn run(
    action: &ActionId,
    capture: &dyn ScreenCapture,
    host: &dyn RulerHost,
    clipboard: &dyn Clipboard,
    theme: OverlayTheme,
) -> Result<RulerOutcome, RulerToolError> {
    if action.as_str() != RULER_ACTION {
        return Err(RulerToolError::Invalid(format!(
            "{} is not the screen ruler action.",
            action.as_str()
        )));
    }
    if capture.monitors().is_empty() {
        return Err(RulerToolError::NoMonitors);
    }
    let desktop = capture.freeze_all().map_err(RulerToolError::Capture)?;
    let selection = host.measure(&desktop, theme).map_err(|error| match error {
        RulerHostError::Cancelled => RulerToolError::Cancelled,
        RulerHostError::Failed(message) => RulerToolError::Host(message),
    })?;
    let measurement = RulerMeasurement::measure(selection.start, selection.end);
    let text = measurement.copy_text();
    if !selection.copied {
        clipboard
            .set_text(&text)
            .map_err(RulerToolError::Clipboard)?;
    }
    Ok(RulerOutcome {
        measurement,
        monitor: selection.monitor,
        copied: true,
        text,
    })
}

impl ToolService {
    pub fn ruler(
        &self,
        capture: &dyn ScreenCapture,
        host: &dyn RulerHost,
        clipboard: &dyn Clipboard,
        theme: OverlayTheme,
    ) -> Result<RulerOutcome, ServiceError> {
        run(
            &ActionId::new(RULER_ACTION),
            capture,
            host,
            clipboard,
            theme,
        )
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use snapline_core::geometry::{DpiScale, Px, Rect};
    use snapline_core::image::Image;
    use snapline_core::ports::{FrozenMonitor, Monitor};

    use super::*;

    struct FakeCapture {
        monitors: Vec<Monitor>,
        freezes: AtomicUsize,
        fail: AtomicBool,
    }

    impl FakeCapture {
        fn one() -> Self {
            Self {
                monitors: vec![monitor(1, Rect::new(Px(0), Px(0), Px(1920), Px(1080)))],
                freezes: AtomicUsize::new(0),
                fail: AtomicBool::new(false),
            }
        }

        fn empty() -> Self {
            Self {
                monitors: Vec::new(),
                freezes: AtomicUsize::new(0),
                fail: AtomicBool::new(false),
            }
        }
    }

    impl ScreenCapture for FakeCapture {
        fn monitors(&self) -> Vec<Monitor> {
            self.monitors.clone()
        }

        fn freeze_all(&self) -> Result<FrozenDesktop, CaptureError> {
            self.freezes.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(CaptureError::Failed("no desktop".to_owned()));
            }
            Ok(FrozenDesktop {
                monitors: self
                    .monitors
                    .iter()
                    .cloned()
                    .map(|monitor| FrozenMonitor {
                        monitor,
                        image: std::sync::Arc::new(Image::bgra(4, 4).expect("image")),
                    })
                    .collect(),
            })
        }

        fn window(
            &self,
            _id: snapline_core::ids::WindowId,
            _keep_shadow: bool,
        ) -> Result<Image, CaptureError> {
            Err(CaptureError::WindowMissing)
        }
    }

    struct FakeHost {
        selection: Mutex<Option<RulerSelection>>,
        failure: Option<RulerHostError>,
        calls: AtomicUsize,
        themes: Mutex<Vec<OverlayTheme>>,
    }

    impl FakeHost {
        fn returning(selection: RulerSelection) -> Self {
            Self {
                selection: Mutex::new(Some(selection)),
                failure: None,
                calls: AtomicUsize::new(0),
                themes: Mutex::new(Vec::new()),
            }
        }

        fn failing(error: RulerHostError) -> Self {
            Self {
                selection: Mutex::new(None),
                failure: Some(error),
                calls: AtomicUsize::new(0),
                themes: Mutex::new(Vec::new()),
            }
        }
    }

    impl RulerHost for FakeHost {
        fn measure(
            &self,
            _desktop: &FrozenDesktop,
            theme: OverlayTheme,
        ) -> Result<RulerSelection, RulerHostError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.themes.lock().expect("themes").push(theme);
            if let Some(failure) = &self.failure {
                return Err(failure.clone());
            }
            self.selection
                .lock()
                .expect("selection")
                .clone()
                .ok_or_else(|| RulerHostError::Failed("no selection".to_owned()))
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        text: Mutex<Option<String>>,
        writes: AtomicUsize,
        fail: AtomicBool,
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

    fn monitor(id: u32, bounds: Rect) -> Monitor {
        Monitor {
            id,
            name: format!("Monitor {id}"),
            bounds,
            work_area: bounds,
            dpi: DpiScale::ONE,
            primary: id == 1,
        }
    }

    fn selection(start: (i32, i32), end: (i32, i32)) -> RulerSelection {
        RulerSelection {
            start: Point::new(Px(start.0), Px(start.1)),
            end: Point::new(Px(end.0), Px(end.1)),
            monitor: Some(1),
            copied: false,
        }
    }

    fn action() -> ActionId {
        ActionId::new(RULER_ACTION)
    }

    #[test]
    fn measuring_freezes_the_desktop_and_copies_the_size() {
        let capture = FakeCapture::one();
        let host = FakeHost::returning(selection((100, 100), (740, 580)));
        let clipboard = FakeClipboard::default();
        let outcome =
            run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark).expect("ruler");
        assert_eq!(capture.freezes.load(Ordering::SeqCst), 1);
        assert_eq!(outcome.text, "640 × 480");
        assert_eq!(outcome.measurement.width, Px(640));
        assert_eq!(outcome.measurement.height, Px(480));
        assert_eq!(outcome.monitor, Some(1));
        assert!(outcome.copied);
        assert_eq!(
            clipboard.text.lock().expect("text").as_deref(),
            Some("640 × 480")
        );
    }

    #[test]
    fn negative_drags_measure_the_same_size() {
        let capture = FakeCapture::one();
        let host = FakeHost::returning(selection((740, 580), (100, 100)));
        let clipboard = FakeClipboard::default();
        let outcome =
            run(&action(), &capture, &host, &clipboard, OverlayTheme::Light).expect("ruler");
        assert_eq!(outcome.text, "640 × 480");
        assert_eq!(
            host.themes.lock().expect("themes").as_slice(),
            &[OverlayTheme::Light]
        );
    }

    #[test]
    fn a_zero_length_measurement_copies_zero_by_zero() {
        let capture = FakeCapture::one();
        let host = FakeHost::returning(selection((50, 50), (50, 50)));
        let clipboard = FakeClipboard::default();
        let outcome =
            run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark).expect("ruler");
        assert_eq!(outcome.text, "0 × 0");
        assert_eq!(
            clipboard.text.lock().expect("text").as_deref(),
            Some("0 × 0")
        );
    }

    #[test]
    fn an_overlay_that_copied_itself_does_not_write_twice() {
        let capture = FakeCapture::one();
        let mut picked = selection((0, 0), (10, 20));
        picked.copied = true;
        let host = FakeHost::returning(picked);
        let clipboard = FakeClipboard::default();
        let outcome =
            run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark).expect("ruler");
        assert_eq!(outcome.text, "10 × 20");
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_clipboard_failure_is_reported() {
        let capture = FakeCapture::one();
        let host = FakeHost::returning(selection((0, 0), (10, 20)));
        let clipboard = FakeClipboard::default();
        clipboard.fail.store(true, Ordering::SeqCst);
        let error = run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark)
            .expect_err("clipboard failure");
        assert_eq!(error, RulerToolError::Clipboard(ClipboardError::Write));
    }

    #[test]
    fn cancelling_the_overlay_is_a_typed_error() {
        let capture = FakeCapture::one();
        let host = FakeHost::failing(RulerHostError::Cancelled);
        let clipboard = FakeClipboard::default();
        let error =
            run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark).expect_err("cancelled");
        assert_eq!(error, RulerToolError::Cancelled);
        assert_eq!(clipboard.writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_host_failure_keeps_its_message() {
        let capture = FakeCapture::one();
        let host = FakeHost::failing(RulerHostError::Failed("no device".to_owned()));
        let clipboard = FakeClipboard::default();
        let error = run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark)
            .expect_err("host failure");
        assert_eq!(error, RulerToolError::Host("no device".to_owned()));
    }

    #[test]
    fn no_monitors_stops_before_freezing() {
        let capture = FakeCapture::empty();
        let host = FakeHost::returning(selection((0, 0), (10, 10)));
        let clipboard = FakeClipboard::default();
        let error = run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark)
            .expect_err("no monitors");
        assert_eq!(error, RulerToolError::NoMonitors);
        assert_eq!(capture.freezes.load(Ordering::SeqCst), 0);
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_capture_failure_is_reported() {
        let capture = FakeCapture::one();
        capture.fail.store(true, Ordering::SeqCst);
        let host = FakeHost::returning(selection((0, 0), (10, 10)));
        let clipboard = FakeClipboard::default();
        let error = run(&action(), &capture, &host, &clipboard, OverlayTheme::Dark)
            .expect_err("capture failure");
        assert_eq!(
            error,
            RulerToolError::Capture(CaptureError::Failed("no desktop".to_owned()))
        );
    }

    #[test]
    fn another_action_is_rejected() {
        let capture = FakeCapture::one();
        let host = FakeHost::returning(selection((0, 0), (10, 10)));
        let clipboard = FakeClipboard::default();
        let error = run(
            &ActionId::new("tool_colour"),
            &capture,
            &host,
            &clipboard,
            OverlayTheme::Dark,
        )
        .expect_err("wrong action");
        assert!(matches!(error, RulerToolError::Invalid(_)));
        assert_eq!(capture.freezes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_service_method_returns_the_typed_outcome() {
        let services = crate::Services::new();
        let capture = FakeCapture::one();
        let host = FakeHost::returning(selection((10, 10), (810, 410)));
        let clipboard = FakeClipboard::default();
        let outcome = services
            .tool()
            .ruler(&capture, &host, &clipboard, OverlayTheme::Dark)
            .expect("ruler");
        assert_eq!(outcome.text, "800 × 400");
        assert_eq!(outcome.measurement.diagonal().round(), 894.0);
    }
}
