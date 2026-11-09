use std::time::Duration;

use snapline_core::image::Image;
use snapline_core::stitch::{self, StitchError, StitchOptions, StopReason, MIN_OVERLAP};
use snapline_store::settings::capture::ScrollLimit;
use snapline_store::settings::Settings;

use crate::tools::ToolService;
use crate::ServiceError;

pub const MIN_DELAY_MS: u32 = 100;
pub const MAX_DELAY_MS: u32 = 1_000;
pub const DEFAULT_MAX_HEIGHT: u32 = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollStop {
    Duplicate,
    Limit,
    Escape,
    MaxHeight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollingOutcome {
    pub image: Image,
    pub frames: usize,
    pub stop: ScrollStop,
    pub offsets: Vec<u32>,
    pub sticky_top: u32,
    pub sticky_bottom: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollingConfig {
    pub delay_ms: u32,
    pub limit: ScrollLimit,
    pub remove_sticky_headers: bool,
    pub max_height: u32,
}

impl Default for ScrollingConfig {
    fn default() -> Self {
        Self {
            delay_ms: 300,
            limit: ScrollLimit::Thirty,
            remove_sticky_headers: true,
            max_height: DEFAULT_MAX_HEIGHT,
        }
    }
}

impl ScrollingConfig {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            delay_ms: settings.capture.scroll_delay_ms,
            limit: settings.capture.scroll_limit,
            remove_sticky_headers: settings.capture.remove_sticky_headers,
            max_height: DEFAULT_MAX_HEIGHT,
        }
    }

    pub const fn max_scrolls(&self) -> Option<u32> {
        match self.limit {
            ScrollLimit::Ten => Some(10),
            ScrollLimit::Twenty => Some(20),
            ScrollLimit::Thirty => Some(30),
            ScrollLimit::Fifty => Some(50),
            ScrollLimit::Unlimited => None,
        }
    }

    pub fn delay(&self) -> u32 {
        self.delay_ms.clamp(MIN_DELAY_MS, MAX_DELAY_MS)
    }

    fn options(&self) -> StitchOptions {
        StitchOptions {
            remove_sticky: self.remove_sticky_headers,
            min_overlap: MIN_OVERLAP,
            max_height: self.max_height,
        }
    }
}

pub trait ScrollCapture: Send + Sync {
    fn capture(&self) -> Result<Image, ServiceError>;
}

pub trait Scroller: Send + Sync {
    fn scroll(&self) -> Result<(), ServiceError>;
    fn cancelled(&self) -> bool;
}

pub trait Waiter: Send + Sync {
    fn wait(&self, millis: u32);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadWaiter;

impl Waiter for ThreadWaiter {
    fn wait(&self, millis: u32) {
        std::thread::sleep(Duration::from_millis(u64::from(millis)));
    }
}

pub fn run(
    capture: &dyn ScrollCapture,
    scroller: &dyn Scroller,
    waiter: &dyn Waiter,
    config: &ScrollingConfig,
) -> Result<ScrollingOutcome, ServiceError> {
    let mut frames = vec![capture.capture()?];
    let max_scrolls = config.max_scrolls();
    let mut scrolls = 0_u32;
    let stop;
    loop {
        if scroller.cancelled() {
            stop = ScrollStop::Escape;
            break;
        }
        if let Some(limit) = max_scrolls {
            if scrolls >= limit {
                stop = ScrollStop::Limit;
                break;
            }
        }
        scroller.scroll()?;
        scrolls = scrolls.saturating_add(1);
        waiter.wait(config.delay());
        let next = capture.capture()?;
        let duplicate = frames
            .last()
            .map(|previous| previous == &next)
            .unwrap_or(false);
        frames.push(next);
        if duplicate {
            stop = ScrollStop::Duplicate;
            break;
        }
    }
    let stitched = stitch::stitch(&frames, &config.options()).map_err(stitch_error)?;
    let stop = match stitched.meta.stop {
        StopReason::MaxHeight => ScrollStop::MaxHeight,
        StopReason::Duplicate => ScrollStop::Duplicate,
        StopReason::CapturesEnded => stop,
    };
    Ok(ScrollingOutcome {
        image: stitched.image,
        frames: stitched.meta.frames_used,
        stop,
        offsets: stitched.meta.offsets,
        sticky_top: stitched.meta.sticky_top,
        sticky_bottom: stitched.meta.sticky_bottom,
    })
}

fn stitch_error(error: StitchError) -> ServiceError {
    match error {
        StitchError::WidthMismatch => {
            ServiceError::Invalid("The scrolling captures have different widths.".to_owned())
        }
        other => ServiceError::Internal(other.to_string()),
    }
}

impl ToolService {
    pub fn scrolling(
        &self,
        capture: &dyn ScrollCapture,
        scroller: &dyn Scroller,
        waiter: &dyn Waiter,
    ) -> Result<ScrollingOutcome, ServiceError> {
        let settings = self._services.settings_snapshot()?;
        run(
            capture,
            scroller,
            waiter,
            &ScrollingConfig::from_settings(&settings),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use snapline_core::image::Image;

    use super::*;

    struct FakeCapture {
        frames: Mutex<Vec<Image>>,
    }

    impl FakeCapture {
        fn new(frames: Vec<Image>) -> Self {
            Self {
                frames: Mutex::new(frames),
            }
        }
    }

    impl ScrollCapture for FakeCapture {
        fn capture(&self) -> Result<Image, ServiceError> {
            let mut frames = self
                .frames
                .lock()
                .map_err(|_| ServiceError::Internal("The fake capture was poisoned.".to_owned()))?;
            if frames.is_empty() {
                return Err(ServiceError::Internal(
                    "The fake capture ran out of frames.".to_owned(),
                ));
            }
            Ok(frames.remove(0))
        }
    }

    #[derive(Default)]
    struct FakeScroller {
        state: Mutex<ScrollerState>,
    }

    #[derive(Default)]
    struct ScrollerState {
        scrolls: usize,
        esc_after: Option<usize>,
    }

    impl FakeScroller {
        fn with_escape_after(scrolls: usize) -> Self {
            Self {
                state: Mutex::new(ScrollerState {
                    scrolls: 0,
                    esc_after: Some(scrolls),
                }),
            }
        }

        fn scrolls(&self) -> usize {
            self.state.lock().map(|state| state.scrolls).unwrap_or(0)
        }
    }

    impl Scroller for FakeScroller {
        fn scroll(&self) -> Result<(), ServiceError> {
            let mut state = self
                .state
                .lock()
                .map_err(|_| ServiceError::Internal("The fake scroller was poisoned.".to_owned()))?;
            state.scrolls += 1;
            Ok(())
        }

        fn cancelled(&self) -> bool {
            self.state
                .lock()
                .map(|state| state.esc_after.map(|limit| state.scrolls >= limit).unwrap_or(false))
                .unwrap_or(false)
        }
    }

    #[derive(Default)]
    struct FakeClock {
        waits: Mutex<Vec<u32>>,
    }

    impl FakeClock {
        fn waits(&self) -> Vec<u32> {
            self.waits.lock().map(|waits| waits.clone()).unwrap_or_default()
        }
    }

    impl Waiter for FakeClock {
        fn wait(&self, millis: u32) {
            if let Ok(mut waits) = self.waits.lock() {
                waits.push(millis);
            }
        }
    }

    fn page(width: u32, height: u32) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                let pixel = [
                    u8::try_from(y % 251).unwrap_or(0),
                    u8::try_from(x % 241).unwrap_or(0),
                    u8::try_from((y / 251) % 239).unwrap_or(0),
                    255,
                ];
                image.set_pixel(x, y, pixel).expect("pixel");
            }
        }
        image
    }

    fn viewport(source: &Image, top: u32, height: u32) -> Image {
        let mut image = Image::bgra(source.width(), height).expect("viewport");
        for y in 0..height {
            if let Some(row) = source.row(top + y) {
                if let Some(target) = image.row_mut(y) {
                    target.copy_from_slice(row);
                }
            }
        }
        image
    }

    fn band_pixel(y: u32, x: u32) -> [u8; 4] {
        [
            u8::try_from(y % 7).unwrap_or(0),
            0,
            u8::try_from(x % 5).unwrap_or(0),
            255,
        ]
    }

    fn framed(source: &Image, top: u32, height: u32, header: u32, footer: u32) -> Image {
        let mut image = Image::bgra(source.width(), height).expect("frame");
        for y in 0..height {
            for x in 0..source.width() {
                let pixel = if y < header {
                    band_pixel(0, x)
                } else if y >= height.saturating_sub(footer) {
                    band_pixel(1, x)
                } else {
                    source.pixel(x, top + y).expect("pixel")
                };
                image.set_pixel(x, y, pixel).expect("pixel");
            }
        }
        image
    }

    fn config(limit: ScrollLimit) -> ScrollingConfig {
        ScrollingConfig {
            delay_ms: 300,
            limit,
            remove_sticky_headers: true,
            max_height: DEFAULT_MAX_HEIGHT,
        }
    }

    #[test]
    fn two_identical_frames_stop_the_capture() {
        let source = page(8, 80);
        let repeated = viewport(&source, 0, 40);
        let capture = FakeCapture::new(vec![repeated.clone(), repeated.clone()]);
        let scroller = FakeScroller::default();
        let clock = FakeClock::default();
        let outcome = run(
            &capture,
            &scroller,
            &clock,
            &config(ScrollLimit::Thirty),
        )
        .expect("scroll");
        assert_eq!(outcome.stop, ScrollStop::Duplicate);
        assert_eq!(outcome.frames, 1);
        assert_eq!(outcome.image, repeated);
        assert_eq!(scroller.scrolls(), 1);
        assert_eq!(clock.waits(), vec![300]);
    }

    #[test]
    fn the_scroll_limit_stops_the_capture() {
        let source = page(8, 400);
        let frames: Vec<Image> = (0..=10)
            .map(|step| viewport(&source, step * 10, 40))
            .collect();
        let capture = FakeCapture::new(frames);
        let scroller = FakeScroller::default();
        let clock = FakeClock::default();
        let outcome = run(&capture, &scroller, &clock, &config(ScrollLimit::Ten))
            .expect("scroll");
        assert_eq!(outcome.stop, ScrollStop::Limit);
        assert_eq!(outcome.frames, 11);
        assert_eq!(scroller.scrolls(), 10);
        assert_eq!(clock.waits().len(), 10);
    }

    #[test]
    fn escape_stops_the_capture() {
        let source = page(8, 400);
        let frames: Vec<Image> = (0..=10)
            .map(|step| viewport(&source, step * 10, 40))
            .collect();
        let capture = FakeCapture::new(frames);
        let scroller = FakeScroller::with_escape_after(2);
        let clock = FakeClock::default();
        let outcome = run(
            &capture,
            &scroller,
            &clock,
            &config(ScrollLimit::Unlimited),
        )
        .expect("scroll");
        assert_eq!(outcome.stop, ScrollStop::Escape);
        assert_eq!(outcome.frames, 3);
        assert_eq!(scroller.scrolls(), 2);
    }

    #[test]
    fn sticky_headers_and_footers_are_removed() {
        let source = page(8, 140);
        let frames: Vec<Image> = [0_u32, 20, 40]
            .iter()
            .map(|top| framed(&source, *top, 40, 4, 3))
            .collect();
        let mut captures = frames.clone();
        captures.push(frames[2].clone());
        let capture = FakeCapture::new(captures);
        let scroller = FakeScroller::default();
        let clock = FakeClock::default();
        let outcome = run(
            &capture,
            &scroller,
            &clock,
            &config(ScrollLimit::Thirty),
        )
        .expect("scroll");
        assert_eq!(outcome.stop, ScrollStop::Duplicate);
        assert_eq!(outcome.sticky_top, 4);
        assert_eq!(outcome.sticky_bottom, 3);
        assert_eq!(outcome.image.height(), 80);
        for x in 0..8 {
            assert_eq!(outcome.image.pixel(x, 0), Ok(band_pixel(0, x)));
            let last = outcome.image.height() - 1;
            assert_eq!(outcome.image.pixel(x, last), Ok(band_pixel(1, x)));
        }
        for y in 0..73 {
            assert_eq!(outcome.image.row(4 + y), source.row(4 + y));
        }
    }

    #[test]
    fn the_configured_delay_is_used_and_clamped() {
        let mut settings = Settings::default();
        settings.capture.scroll_delay_ms = 1_500;
        settings.capture.scroll_limit = ScrollLimit::Twenty;
        settings.capture.remove_sticky_headers = false;
        let config = ScrollingConfig::from_settings(&settings);
        assert_eq!(config.delay(), 1_000);
        assert_eq!(config.limit, ScrollLimit::Twenty);
        assert!(!config.remove_sticky_headers);

        let source = page(8, 100);
        let capture = FakeCapture::new(vec![
            viewport(&source, 0, 40),
            viewport(&source, 20, 40),
            viewport(&source, 20, 40),
        ]);
        let clock = FakeClock::default();
        run(
            &capture,
            &FakeScroller::default(),
            &clock,
            &config,
        )
        .expect("scroll");
        assert_eq!(clock.waits(), vec![1_000, 1_000]);
    }

    #[test]
    fn every_limit_maps_to_its_count() {
        assert_eq!(
            ScrollingConfig {
                limit: ScrollLimit::Ten,
                ..ScrollingConfig::default()
            }
            .max_scrolls(),
            Some(10)
        );
        assert_eq!(
            ScrollingConfig {
                limit: ScrollLimit::Twenty,
                ..ScrollingConfig::default()
            }
            .max_scrolls(),
            Some(20)
        );
        assert_eq!(
            ScrollingConfig {
                limit: ScrollLimit::Thirty,
                ..ScrollingConfig::default()
            }
            .max_scrolls(),
            Some(30)
        );
        assert_eq!(
            ScrollingConfig {
                limit: ScrollLimit::Fifty,
                ..ScrollingConfig::default()
            }
            .max_scrolls(),
            Some(50)
        );
        assert_eq!(
            ScrollingConfig {
                limit: ScrollLimit::Unlimited,
                ..ScrollingConfig::default()
            }
            .max_scrolls(),
            None
        );
    }

    #[test]
    fn a_width_mismatch_is_reported_as_invalid() {
        let capture = FakeCapture::new(vec![page(8, 40), page(9, 40)]);
        let error = run(
            &capture,
            &FakeScroller::with_escape_after(1),
            &FakeClock::default(),
            &config(ScrollLimit::Thirty),
        )
        .expect_err("width mismatch");
        assert!(matches!(error, ServiceError::Invalid(_)));
    }
}
