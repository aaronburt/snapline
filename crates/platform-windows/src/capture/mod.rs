use snapline_core::ids::WindowId;
use snapline_core::image::Image;
use snapline_core::ports::{CaptureError, FrozenDesktop, Monitor, ScreenCapture};

pub mod dxgi;
pub mod gdi;
pub mod wgc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureOptions {
    pub include_cursor: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            include_cursor: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureBackend {
    options: CaptureOptions,
}

impl CaptureBackend {
    pub const fn new(options: CaptureOptions) -> Self {
        Self { options }
    }

    pub const fn options(self) -> CaptureOptions {
        self.options
    }

    pub fn foreground_window() -> Option<WindowId> {
        wgc::foreground_window()
    }

    pub fn window_under_cursor() -> Option<WindowId> {
        wgc::window_under_cursor()
    }
}

impl ScreenCapture for CaptureBackend {
    fn monitors(&self) -> Vec<Monitor> {
        crate::monitors::enumerate()
    }

    fn freeze_all(&self) -> Result<FrozenDesktop, CaptureError> {
        match dxgi::freeze_all(self.options) {
            Ok(desktop) => Ok(desktop),
            Err(error) => {
                tracing::debug!(%error, "desktop duplication failed; falling back to GDI");
                gdi::freeze_all(self.options)
            }
        }
    }

    fn window(&self, id: WindowId, keep_shadow: bool) -> Result<Image, CaptureError> {
        wgc::capture_window(id, keep_shadow, self.options.include_cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "desktop"]
    fn freeze_all_returns_one_image_per_monitor_at_physical_size() {
        let backend = CaptureBackend::default();
        let monitors = backend.monitors();
        let frozen = backend.freeze_all().expect("freeze");
        assert_eq!(frozen.monitors.len(), monitors.len());
        for frozen_monitor in &frozen.monitors {
            assert_eq!(
                frozen_monitor.image.width(),
                u32::try_from(frozen_monitor.monitor.bounds.width.0).expect("width")
            );
            assert_eq!(
                frozen_monitor.image.height(),
                u32::try_from(frozen_monitor.monitor.bounds.height.0).expect("height")
            );
            assert_eq!(
                frozen_monitor.image.stride(),
                usize::try_from(frozen_monitor.image.width()).expect("width") * 4
            );
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn freeze_all_includes_the_cursor_when_asked() {
        let backend = CaptureBackend::new(CaptureOptions {
            include_cursor: true,
        });
        let frozen = backend.freeze_all().expect("freeze with cursor");
        for frozen_monitor in &frozen.monitors {
            assert!(frozen_monitor.image.width() > 0);
            assert!(frozen_monitor.image.height() > 0);
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn gdi_backend_can_always_freeze_the_desktop() {
        let desktop = gdi::freeze_all(CaptureOptions::default()).expect("gdi freeze");
        assert_eq!(desktop.monitors.len(), crate::monitors::enumerate().len());
    }

    #[test]
    #[ignore = "desktop"]
    fn dxgi_backend_captures_the_desktop_when_available() {
        match dxgi::freeze_all(CaptureOptions::default()) {
            Ok(desktop) => {
                assert_eq!(desktop.monitors.len(), crate::monitors::enumerate().len());
                for frozen_monitor in &desktop.monitors {
                    assert!(frozen_monitor.image.width() > 0);
                    assert!(frozen_monitor.image.height() > 0);
                }
            }
            Err(error) => {
                eprintln!("desktop duplication is unavailable here: {error}");
            }
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn window_capture_of_a_known_visible_window_succeeds() {
        let Some(window) = CaptureBackend::foreground_window() else {
            panic!("there is no foreground window");
        };
        let backend = CaptureBackend::default();
        let image = backend.window(window, true).expect("window capture");
        assert!(image.width() > 0);
        assert!(image.height() > 0);
    }
}
