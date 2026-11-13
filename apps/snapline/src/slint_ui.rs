use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use snapline_core::actions::ActionId;
use snapline_core::colour::ColourFormat;
use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::image::Image;
use snapline_core::ports::{
    FrozenDesktop, FrozenMonitor, Notice, OverlayTheme, ScreenCapture as _,
};
use snapline_services::dto::settings::{SettingsDto, SettingsResp};
use snapline_services::recording::DEFAULT_MICROPHONE;
use snapline_services::tools::ocr::{OcrRequest, OcrTool};
use snapline_services::tools::ruler::{RulerHost, RulerHostError, RulerSelection, RulerToolError};
use snapline_services::{ServiceError, Services};
use snapline_ui::{Host, MonitorInfo, UiHandle};
use windows::Win32::Foundation::HWND;

use crate::ipc::settings::apply_update;

struct WindowsHost {
    services: Services,
}

impl WindowsHost {
    fn toast(&self, title: &str, body: impl Into<String>) {
        let notify = self
            .services
            .settings_snapshot()
            .is_ok_and(|settings| settings.general.notifications);
        if !notify {
            return;
        }
        if let Some(notifier) = self.services.notifier() {
            notifier.toast(Notice::new(title, body));
        }
    }
}

struct WinRulerHost;

impl RulerHost for WinRulerHost {
    fn measure(
        &self,
        desktop: &FrozenDesktop,
        theme: OverlayTheme,
    ) -> Result<RulerSelection, RulerHostError> {
        let request = snapline_overlay::ruler::RulerRequest::new(desktop.clone())
            .with_theme(to_overlay_theme(theme))
            .with_copy(Arc::new(|text| {
                let _ = snapline_win::clipboard::Clipboard::new().set_text(text);
            }));
        snapline_overlay::ruler::RulerHostImpl::new()
            .measure(request)
            .map(|measured| RulerSelection {
                start: measured.start,
                end: measured.end,
                monitor: measured.monitor,
                copied: measured.copied,
            })
            .map_err(|error| match error {
                snapline_overlay::OverlayError::Cancelled => RulerHostError::Cancelled,
                other => RulerHostError::Failed(other.to_string()),
            })
    }
}

impl Host for WindowsHost {
    fn browse_folder(&self, current: Option<&str>) -> Option<String> {
        match snapline_win::dialogs::pick_folder(current.map(Path::new)) {
            Ok(path) => path.map(|path| path.to_string_lossy().into_owned()),
            Err(error) => {
                tracing::error!(%error, "folder dialog failed");
                None
            }
        }
    }

    fn copy_text(&self, text: &str) -> Result<(), String> {
        snapline_win::clipboard::Clipboard::new()
            .set_text(text)
            .map_err(|error| error.to_string())
    }

    fn copy_image_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        let codec = snapline_win::wic::WicCodec::new().map_err(|error| error.to_string())?;
        let image = codec.decode(bytes).map_err(|error| error.to_string())?;
        snapline_win::clipboard::Clipboard::new()
            .set_image(&image)
            .map_err(|error| error.to_string())
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        snapline_win::shell::open_url(url).map_err(|error| error.to_string())
    }

    fn save_png(&self, suggested_name: &str, bytes: &[u8]) -> Result<Option<String>, String> {
        snapline_win::dialogs::save_png(suggested_name, bytes)
            .map(|path| path.map(|path| path.to_string_lossy().into_owned()))
            .map_err(|error| error.to_string())
    }

    fn select_region_image(&self) -> Result<Option<Image>, String> {
        let settings = self
            .services
            .settings_snapshot()
            .map_err(|error| error.to_string())?;
        let desktop =
            snapline_win::capture::CaptureBackend::new(snapline_win::capture::CaptureOptions {
                include_cursor: settings.capture.include_cursor,
            })
            .freeze_all()
            .map_err(|error| error.to_string())?;
        let request = snapline_overlay::SelectRequest {
            desktop: desktop.clone(),
            annotate: false,
            upload_on_finish: false,
            snap: snapline_core::overlay::SnapSet::default(),
            magnifier: overlay_magnifier(
                settings.capture.magnifier,
                settings.capture.magnifier_zoom,
            ),
            full_screen_crosshair: settings.capture.fullscreen_crosshair,
            theme: current_overlay_theme(),
        };
        let result = match snapline_overlay::OverlayHostImpl::new().select_region(request) {
            Ok(result) => result,
            Err(snapline_overlay::OverlayError::Cancelled) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if let Some(image) = result.image {
            return Ok(Some(image));
        }
        crop_frozen(&desktop, result.selection, result.monitor).map(Some)
    }

    fn run_action(&self, action: &str) -> Result<(), String> {
        match action {
            "tool_colour" => {
                let settings = self
                    .services
                    .settings_snapshot()
                    .map_err(|error| error.to_string())?;
                let desktop = snapline_win::capture::CaptureBackend::new(
                    snapline_win::capture::CaptureOptions::default(),
                )
                .freeze_all()
                .map_err(|error| error.to_string())?;
                let request = snapline_overlay::picker::PickerRequest {
                    desktop,
                    magnifier: overlay_magnifier(
                        settings.capture.magnifier,
                        settings.capture.magnifier_zoom,
                    ),
                    format: ColourFormat::default(),
                    theme: current_overlay_theme(),
                };
                let result =
                    match snapline_overlay::picker::PickerHostImpl::new().pick(request, |text| {
                        snapline_win::clipboard::Clipboard::new()
                            .set_text(text)
                            .map_err(|error| error.to_string())
                    }) {
                        Ok(result) => result,
                        Err(snapline_overlay::picker::PickerError::Cancelled) => return Ok(()),
                        Err(error) => return Err(error.to_string()),
                    };
                self.toast("Colour copied", result.text);
                Ok(())
            }
            "tool_ruler" => {
                let capture = snapline_win::capture::CaptureBackend::new(
                    snapline_win::capture::CaptureOptions::default(),
                );
                let clipboard = snapline_win::clipboard::Clipboard::new();
                let theme = if snapline_win::theme::is_light() {
                    OverlayTheme::Light
                } else {
                    OverlayTheme::Dark
                };
                let outcome = match snapline_services::tools::ruler::run(
                    &ActionId::new(action),
                    &capture,
                    &WinRulerHost,
                    &clipboard,
                    theme,
                ) {
                    Ok(outcome) => outcome,
                    Err(RulerToolError::Cancelled) => return Ok(()),
                    Err(error) => return Err(error.to_string()),
                };
                self.toast("Measurement copied", outcome.text);
                Ok(())
            }
            "tool_ocr" => {
                let Some(image) = self.select_region_image()? else {
                    return Ok(());
                };
                let clipboard = snapline_win::clipboard::Clipboard::new();
                let outcome = OcrTool::new(&snapline_win::ocr::WindowsOcr, &clipboard)
                    .run(OcrRequest::new(&image))
                    .map_err(|error| error.to_string())?;
                if outcome.is_empty() {
                    return Err("No text was recognised in the selected area.".to_owned());
                }
                if let Some(body) = outcome.toast() {
                    self.toast("Text copied", body);
                }
                Ok(())
            }
            other => match snapline_services::actions::dispatch_notifying(
                &ActionId::new(other),
                &self.services,
            ) {
                Ok(()) => Ok(()),
                Err(ServiceError::Invalid(message)) if message.ends_with("was cancelled") => Ok(()),
                Err(error) => Err(error.to_string()),
            },
        }
    }

    fn report_error(&self, title: &str, message: &str) {
        if let Some(notifier) = self.services.notifier() {
            notifier.toast(Notice::new(title, message));
        }
    }

    fn is_light(&self) -> bool {
        snapline_win::theme::is_light()
    }

    fn configure_window(&self, hwnd: isize, light: bool) {
        crate::theme::apply_hwnd_dark_mode(HWND(hwnd as *mut c_void), light);
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        snapline_win::monitors::enumerate()
            .into_iter()
            .map(|monitor| MonitorInfo {
                name: monitor.name,
                x: monitor.bounds.x.0,
                y: monitor.bounds.y.0,
                width: monitor.bounds.width.0,
                height: monitor.bounds.height.0,
                primary: monitor.primary,
            })
            .collect()
    }

    fn microphones(&self) -> Vec<String> {
        let mut out = vec![DEFAULT_MICROPHONE.to_owned()];
        for device in snapline_win::audio::AudioCapture::microphones() {
            let name = device.name.trim();
            if !name.is_empty() && !out.iter().any(|existing| existing == name) {
                out.push(name.to_owned());
            }
        }
        out
    }

    fn update_settings(&self, services: &Services, patch: Value) -> Result<SettingsDto, String> {
        match apply_update(services, patch) {
            Ok(SettingsResp::Updated { settings }) => Ok(settings),
            Ok(_) => Err("unexpected settings response".to_owned()),
            Err(error) => Err(error.message),
        }
    }
}

fn current_overlay_theme() -> snapline_overlay::Theme {
    if snapline_win::theme::is_light() {
        snapline_overlay::Theme::Light
    } else {
        snapline_overlay::Theme::Dark
    }
}

fn to_overlay_theme(theme: OverlayTheme) -> snapline_overlay::Theme {
    match theme {
        OverlayTheme::Light => snapline_overlay::Theme::Light,
        OverlayTheme::Dark => snapline_overlay::Theme::Dark,
    }
}

fn overlay_magnifier(
    enabled: bool,
    zoom: snapline_store::settings::capture::MagnifierZoom,
) -> snapline_overlay::MagnifierSettings {
    snapline_overlay::MagnifierSettings {
        enabled,
        zoom: match zoom {
            snapline_store::settings::capture::MagnifierZoom::X2 => {
                snapline_overlay::MagnifierZoom::X2
            }
            snapline_store::settings::capture::MagnifierZoom::X4 => {
                snapline_overlay::MagnifierZoom::X4
            }
            snapline_store::settings::capture::MagnifierZoom::X8 => {
                snapline_overlay::MagnifierZoom::X8
            }
        },
    }
}

fn crop_frozen(
    desktop: &FrozenDesktop,
    selection: Rect,
    monitor: Option<u32>,
) -> Result<Image, String> {
    let frozen = select_monitor(desktop, monitor, selection.center())
        .ok_or_else(|| "There is no monitor to capture.".to_owned())?;
    let clipped = selection
        .intersection(frozen.monitor.bounds)
        .ok_or_else(|| "The selected area is empty.".to_owned())?;
    let local = clipped.translated(Point::new(
        Px(-frozen.monitor.bounds.x.0),
        Px(-frozen.monitor.bounds.y.0),
    ));
    if local.is_empty() {
        return Err("The selected area is empty.".to_owned());
    }
    let left = u32::try_from(local.x.0).map_err(|_| "The selected area is empty.".to_owned())?;
    let top = u32::try_from(local.y.0).map_err(|_| "The selected area is empty.".to_owned())?;
    let width =
        u32::try_from(local.width.0).map_err(|_| "The selected area is empty.".to_owned())?;
    let height =
        u32::try_from(local.height.0).map_err(|_| "The selected area is empty.".to_owned())?;
    let right = left
        .checked_add(width)
        .ok_or_else(|| "The selected area is empty.".to_owned())?;
    let bottom = top
        .checked_add(height)
        .ok_or_else(|| "The selected area is empty.".to_owned())?;
    if right > frozen.image.width() || bottom > frozen.image.height() {
        return Err("The selected area is empty.".to_owned());
    }
    let stride = usize::try_from(width).map_err(|_| "The selected area is empty.".to_owned())? * 4;
    let rows = usize::try_from(height).map_err(|_| "The selected area is empty.".to_owned())?;
    let mut pixels = Vec::with_capacity(stride * rows);
    let start = usize::try_from(left).map_err(|_| "The selected area is empty.".to_owned())? * 4;
    for row in 0..height {
        let source = frozen
            .image
            .row(top + row)
            .ok_or_else(|| "The selected area is empty.".to_owned())?;
        pixels.extend_from_slice(&source[start..start + stride]);
    }
    Image::new(width, height, stride, pixels).map_err(|error| error.to_string())
}

fn select_monitor<'a>(
    desktop: &'a FrozenDesktop,
    monitor: Option<u32>,
    point: Point,
) -> Option<&'a FrozenMonitor> {
    monitor
        .and_then(|id| desktop.monitors.iter().find(|item| item.monitor.id == id))
        .or_else(|| {
            desktop
                .monitors
                .iter()
                .find(|item| item.monitor.bounds.contains(point))
        })
        .or_else(|| desktop.monitors.iter().find(|item| item.monitor.primary))
        .or_else(|| desktop.monitors.first())
}

pub fn start(services: Services) -> Result<UiHandle, String> {
    let host = Arc::new(WindowsHost {
        services: services.clone(),
    });
    snapline_ui::runner::start(services, host)
}

pub fn web_ui_requested() -> bool {
    std::env::var_os("SNAPLINE_WEB_UI").is_some_and(|value| value != "0")
}
