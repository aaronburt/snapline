#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]

use std::sync::{Arc, Mutex, OnceLock};

use snapline_core::actions::ActionId;
use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::ids::WindowId;
use snapline_core::image::Image;
use snapline_core::overlay::SnapSet;
use snapline_core::ports::{
    Clipboard, FrozenDesktop, FrozenMonitor, MagnifierSettings as CoreMagnifierSettings,
    MagnifierZoom as CoreMagnifierZoom, OverlayAction, OverlayError, OverlayHost, OverlayRequest,
    OverlayTheme, ScreenCapture,
};
use snapline_store::settings::capture::{CaptureDelay, FullscreenCapture, MagnifierZoom};
use snapline_store::settings::Settings;

use crate::{ServiceError, Services};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastRegion {
    pub monitor: u32,
    pub rect: Rect,
}

impl LastRegion {
    pub const KEY: &'static str = "last_region";

    fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "monitor": self.monitor,
            "x": self.rect.x.0,
            "y": self.rect.y.0,
            "width": self.rect.width.0,
            "height": self.rect.height.0,
        })
    }

    fn from_settings(settings: &Settings) -> Option<Self> {
        let table = settings.capture.extra.get(Self::KEY)?.as_table()?;
        let monitor = u32::try_from(table.get("monitor")?.as_integer()?).ok()?;
        let x = i32::try_from(table.get("x")?.as_integer()?).ok()?;
        let y = i32::try_from(table.get("y")?.as_integer()?).ok()?;
        let width = i32::try_from(table.get("width")?.as_integer()?).ok()?;
        let height = i32::try_from(table.get("height")?.as_integer()?).ok()?;
        let rect = Rect::new(Px(x), Px(y), Px(width), Px(height));
        if rect.is_empty() {
            return None;
        }
        Some(Self { monitor, rect })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    Cancelled,
    Copied { width: u32, height: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowChoice {
    pub id: WindowId,
    pub title: String,
    pub app: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowPick {
    Active,
    Window(WindowId),
}

pub const ACTIVE_WINDOW_LABEL: &str = "Active window";
pub const MENU_TITLE_LIMIT: usize = 60;

pub fn truncate_title(title: &str, limit: usize) -> String {
    let title = title.trim();
    if title.chars().count() <= limit {
        return title.to_owned();
    }
    let kept: String = title.chars().take(limit.saturating_sub(1)).collect();
    format!("{}\u{2026}", kept.trim_end())
}

pub fn choice_label(choice: &WindowChoice) -> String {
    let title = truncate_title(&choice.title, MENU_TITLE_LIMIT);
    match choice.app.as_deref().filter(|app| !app.is_empty()) {
        Some(app) => format!("{app} \u{2014} {title}"),
        None => title,
    }
}

pub fn menu_labels(choices: &[WindowChoice]) -> Vec<String> {
    std::iter::once(ACTIVE_WINDOW_LABEL.to_owned())
        .chain(choices.iter().map(choice_label))
        .collect()
}

pub fn resolve_pick(
    choices: &[WindowChoice],
    selected: Option<usize>,
) -> Result<Option<WindowPick>, ServiceError> {
    if choices.is_empty() {
        return Err(no_windows());
    }
    match selected {
        None => Ok(None),
        Some(0) => Ok(Some(WindowPick::Active)),
        Some(index) => choices
            .get(index - 1)
            .map(|choice| Some(WindowPick::Window(choice.id)))
            .ok_or_else(window_missing),
    }
}

pub trait Countdown: Send + Sync {
    fn run(&self, seconds: u8) -> Result<(), ServiceError>;
}

pub trait CaptureContext: Send + Sync {
    fn capture_settings(&self) -> Result<Settings, ServiceError>;
    fn save_last_region(&self, region: LastRegion) -> Result<(), ServiceError>;

    fn capture_complete(&self, _image: &Image, _sound: bool) {}

    fn foreground_window(&self) -> Option<WindowId> {
        None
    }

    fn window_under_cursor(&self) -> Option<WindowId> {
        None
    }

    fn list_windows(&self) -> Vec<WindowChoice> {
        Vec::new()
    }

    fn choose_window(&self, _choices: &[WindowChoice]) -> Result<Option<WindowPick>, ServiceError> {
        Ok(Some(WindowPick::Active))
    }

    fn window_at(&self, _point: Point) -> Option<WindowId> {
        None
    }

    fn process_name(&self, _window: WindowId) -> Option<String> {
        None
    }

    fn cursor_position(&self) -> Option<Point> {
        None
    }

    fn overlay_theme(&self) -> OverlayTheme {
        OverlayTheme::Dark
    }

    fn start_recording(&self, _gif: bool, _rect: Rect, _monitor: u32) -> Result<(), ServiceError> {
        Err(ServiceError::NotYet)
    }
}

impl CaptureContext for Services {
    fn capture_settings(&self) -> Result<Settings, ServiceError> {
        self.settings_snapshot()
    }

    fn save_last_region(&self, region: LastRegion) -> Result<(), ServiceError> {
        self.update_settings(&serde_json::json!({
            "capture": { "last_region": region.to_json() }
        }))
        .map(|_| ())
    }

    fn start_recording(&self, gif: bool, rect: Rect, monitor: u32) -> Result<(), ServiceError> {
        let kind = if gif {
            crate::dto::record::RecordKindDto::Gif
        } else {
            crate::dto::record::RecordKindDto::Video
        };
        crate::recording::start_recording_region(self, kind, rect, monitor)
            .map(|_| ())
            .map_err(|error| ServiceError::Invalid(error.to_string()))
    }
}

struct Inner {
    capture: Arc<dyn ScreenCapture + Send + Sync>,
    overlay: Arc<dyn OverlayHost + Send + Sync>,
    clipboard: Arc<dyn Clipboard + Send + Sync>,
    countdown: Arc<dyn Countdown>,
    context: Arc<dyn CaptureContext>,
    last_region: Mutex<Option<LastRegion>>,
}

pub(crate) struct Shot {
    pub image: Image,
    pub source_app: Option<String>,
    pub action: Option<snapline_core::ports::OverlayAction>,
}

#[derive(Clone)]
pub struct CaptureService {
    inner: Arc<Inner>,
}

impl CaptureService {
    pub fn new(
        capture: Arc<dyn ScreenCapture + Send + Sync>,
        overlay: Arc<dyn OverlayHost + Send + Sync>,
        clipboard: Arc<dyn Clipboard + Send + Sync>,
        countdown: Arc<dyn Countdown>,
        context: Arc<dyn CaptureContext>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                capture,
                overlay,
                clipboard,
                countdown,
                context,
                last_region: Mutex::new(None),
            }),
        }
    }

    pub fn capture_region(&self) -> Result<CaptureOutcome, ServiceError> {
        match self.region_result(true)? {
            Some(shot) => self.copy(shot.image),
            None => Ok(CaptureOutcome::Cancelled),
        }
    }

    pub(crate) fn region_result(&self, annotate: bool) -> Result<Option<Shot>, ServiceError> {
        let settings = self.inner.context.capture_settings()?;
        self.wait(settings.capture.capture_delay)?;
        let desktop = self.freeze()?;
        let request = OverlayRequest {
            desktop: desktop.clone(),
            annotate,
            upload_on_finish: settings.uploads.images.is_upload(),
            snap: SnapSet::default(),
            magnifier: magnifier(&settings),
            full_screen_crosshair: settings.capture.fullscreen_crosshair,
            theme: self.inner.context.overlay_theme(),
        };
        let result = match self.inner.overlay.select_region(request) {
            Ok(result) => result,
            Err(OverlayError::Cancelled) => return Ok(None),
            Err(error) => return Err(ServiceError::Invalid(error.to_string())),
        };
        if let Some(action @ (OverlayAction::Record | OverlayAction::Gif)) = result.action {
            let monitor = result
                .monitor
                .or_else(|| {
                    monitor_for(&desktop, None, result.selection.center())
                        .map(|frozen| frozen.monitor.id)
                })
                .ok_or_else(no_monitor)?;
            self.inner.context.start_recording(
                action == OverlayAction::Gif,
                result.selection,
                monitor,
            )?;
            return Ok(None);
        }
        let source_app = self.app_at(result.selection.center());
        let frozen = monitor_for(&desktop, result.monitor, result.selection.center())
            .or_else(|| desktop.monitors.first())
            .ok_or_else(no_monitor)?;
        let (image, rect) = if let Some(image) = result.image {
            (image, result.selection)
        } else {
            crop_to_monitor(frozen, result.selection)?
        };
        self.remember(LastRegion {
            monitor: frozen.monitor.id,
            rect,
        });
        self.inner
            .context
            .capture_complete(&image, settings.capture.shutter_sound);
        Ok(Some(Shot {
            image,
            source_app,
            action: result.action,
        }))
    }

    pub fn capture_window(&self) -> Result<CaptureOutcome, ServiceError> {
        match self.window_image()? {
            Some(shot) => self.copy(shot.image),
            None => Ok(CaptureOutcome::Cancelled),
        }
    }

    pub(crate) fn window_image(&self) -> Result<Option<Shot>, ServiceError> {
        let settings = self.inner.context.capture_settings()?;
        let choices = self.inner.context.list_windows();
        let foreground = self.inner.context.foreground_window();
        // Opened from the tray or the Quick Panel, the foreground can be the taskbar or one of
        // Snapline's own windows. Those are not capturable, so use the topmost window that is.
        let active = if choices.is_empty() {
            foreground
        } else {
            foreground
                .filter(|id| choices.iter().any(|choice| choice.id == *id))
                .or_else(|| choices.first().map(|choice| choice.id))
        };
        let Some(pick) = self.inner.context.choose_window(&choices)? else {
            return Ok(None);
        };
        self.wait(settings.capture.capture_delay)?;
        let window = match pick {
            WindowPick::Active => active.ok_or_else(window_missing)?,
            WindowPick::Window(id) => id,
        };
        self.window_image_id(
            window,
            settings.capture.keep_window_shadow,
            settings.capture.shutter_sound,
        )
    }

    pub fn capture_window_under_cursor(&self) -> Result<CaptureOutcome, ServiceError> {
        let settings = self.inner.context.capture_settings()?;
        self.wait(settings.capture.capture_delay)?;
        let Some(window) = self.inner.context.window_under_cursor() else {
            return Err(window_missing());
        };
        let shot = self.window_image_id(
            window,
            settings.capture.keep_window_shadow,
            settings.capture.shutter_sound,
        )?;
        match shot {
            Some(shot) => self.copy(shot.image),
            None => Ok(CaptureOutcome::Cancelled),
        }
    }

    pub fn capture_screen(&self) -> Result<CaptureOutcome, ServiceError> {
        match self.screen_image()? {
            Some(shot) => self.copy(shot.image),
            None => Ok(CaptureOutcome::Cancelled),
        }
    }

    pub(crate) fn screen_image(&self) -> Result<Option<Shot>, ServiceError> {
        let settings = self.inner.context.capture_settings()?;
        self.wait(settings.capture.capture_delay)?;
        let mut desktop = self.freeze()?;
        let (image, source_app) = match settings.capture.fullscreen_capture_uses {
            FullscreenCapture::AllMonitors => (compose_all(&desktop)?, None),
            FullscreenCapture::PrimaryMonitor => {
                let index = primary_index(&desktop);
                self.take_monitor_shot(&mut desktop, index)?
            }
            FullscreenCapture::MonitorUnderCursor => {
                let cursor = self.inner.context.cursor_position();
                let index = cursor_index(&desktop, cursor);
                self.take_monitor_shot(&mut desktop, index)?
            }
        };
        self.inner
            .context
            .capture_complete(&image, settings.capture.shutter_sound);
        Ok(Some(Shot {
            image,
            source_app,
            action: None,
        }))
    }

    fn take_monitor_shot(
        &self,
        desktop: &mut FrozenDesktop,
        index: Option<usize>,
    ) -> Result<(Image, Option<String>), ServiceError> {
        let centre = index
            .and_then(|index| desktop.monitors.get(index))
            .map(|item| item.monitor.bounds.center());
        let image = take_monitor(desktop, index)?;
        Ok((image, centre.and_then(|point| self.app_at(point))))
    }

    pub fn capture_last_region(&self) -> Result<CaptureOutcome, ServiceError> {
        match self.last_region_image()? {
            Some(shot) => self.copy(shot.image),
            None => Ok(CaptureOutcome::Cancelled),
        }
    }

    pub(crate) fn last_region_image(&self) -> Result<Option<Shot>, ServiceError> {
        let settings = self.inner.context.capture_settings()?;
        let region = self.last_region()?;
        self.wait(settings.capture.capture_delay)?;
        let desktop = self.freeze()?;
        let frozen = monitor_for(&desktop, Some(region.monitor), region.rect.center())
            .ok_or_else(display_changed)?;
        let (image, _) = crop_to_monitor(frozen, region.rect)?;
        let source_app = self.app_at(region.rect.center());
        self.inner
            .context
            .capture_complete(&image, settings.capture.shutter_sound);
        Ok(Some(Shot {
            image,
            source_app,
            action: None,
        }))
    }

    fn window_image_id(
        &self,
        window: WindowId,
        keep_shadow: bool,
        sound: bool,
    ) -> Result<Option<Shot>, ServiceError> {
        let image = self
            .inner
            .capture
            .window(window, keep_shadow)
            .map_err(capture_error)?;
        let source_app = self.inner.context.process_name(window);
        self.inner.context.capture_complete(&image, sound);
        Ok(Some(Shot {
            image,
            source_app,
            action: None,
        }))
    }

    fn freeze(&self) -> Result<FrozenDesktop, ServiceError> {
        self.inner.capture.freeze_all().map_err(capture_error)
    }

    fn copy(&self, image: Image) -> Result<CaptureOutcome, ServiceError> {
        let width = image.width();
        let height = image.height();
        self.inner
            .clipboard
            .set_image(&image)
            .map_err(|error| ServiceError::Invalid(error.to_string()))?;
        Ok(CaptureOutcome::Copied { width, height })
    }

    fn app_at(&self, point: Point) -> Option<String> {
        let window = self.inner.context.window_at(point)?;
        self.inner.context.process_name(window)
    }

    fn wait(&self, delay: CaptureDelay) -> Result<(), ServiceError> {
        let seconds = delay_seconds(delay);
        if seconds == 0 {
            return Ok(());
        }
        self.inner.countdown.run(seconds)
    }

    fn remember(&self, region: LastRegion) {
        if let Ok(mut slot) = self.inner.last_region.lock() {
            *slot = Some(region);
        }
        if let Err(error) = self.inner.context.save_last_region(region) {
            tracing::warn!(%error, "the last region could not be saved");
        }
    }

    fn last_region(&self) -> Result<LastRegion, ServiceError> {
        if let Some(region) = self.inner.last_region.lock().ok().and_then(|slot| *slot) {
            return Ok(region);
        }
        let settings = self.inner.context.capture_settings()?;
        LastRegion::from_settings(&settings).ok_or_else(|| {
            ServiceError::Invalid("There is no last region to capture yet.".to_owned())
        })
    }
}

pub fn delay_seconds(delay: CaptureDelay) -> u8 {
    match delay {
        CaptureDelay::Off => 0,
        CaptureDelay::OneSecond => 1,
        CaptureDelay::ThreeSeconds => 3,
        CaptureDelay::FiveSeconds => 5,
    }
}

static SERVICE: OnceLock<Mutex<Option<CaptureService>>> = OnceLock::new();

pub fn install(service: CaptureService) -> Result<(), ServiceError> {
    let slot = SERVICE.get_or_init(|| Mutex::new(None));
    let mut guard = slot
        .lock()
        .map_err(|_| ServiceError::Internal("The capture service is unavailable.".to_owned()))?;
    if guard.is_some() {
        return Err(ServiceError::Internal(
            "The capture service is already installed.".to_owned(),
        ));
    }
    *guard = Some(service);
    Ok(())
}

pub fn installed() -> Result<CaptureService, ServiceError> {
    let slot = SERVICE.get().ok_or(ServiceError::NotYet)?;
    slot.lock()
        .map_err(|_| ServiceError::Internal("The capture service is unavailable.".to_owned()))?
        .clone()
        .ok_or(ServiceError::NotYet)
}

enum Routed {
    Region,
    Window,
    Screen,
    LastRegion,
}

pub fn run(action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    let routed = match action.as_str() {
        "capture_region" => Routed::Region,
        "capture_window" => Routed::Window,
        "capture_screen" => Routed::Screen,
        "capture_last_region" => Routed::LastRegion,
        _ => return Err(ServiceError::NotYet),
    };
    let service = installed()?;
    let outcome = match routed {
        Routed::Region => service.capture_region()?,
        Routed::Window => service.capture_window()?,
        Routed::Screen => service.capture_screen()?,
        Routed::LastRegion => service.capture_last_region()?,
    };
    match outcome {
        CaptureOutcome::Cancelled | CaptureOutcome::Copied { .. } => Ok(()),
    }
}

fn magnifier(settings: &Settings) -> CoreMagnifierSettings {
    CoreMagnifierSettings {
        enabled: settings.capture.magnifier,
        zoom: match settings.capture.magnifier_zoom {
            MagnifierZoom::X2 => CoreMagnifierZoom::X2,
            MagnifierZoom::X4 => CoreMagnifierZoom::X4,
            MagnifierZoom::X8 => CoreMagnifierZoom::X8,
        },
    }
}

fn monitor_for(desktop: &FrozenDesktop, id: Option<u32>, point: Point) -> Option<&FrozenMonitor> {
    id.and_then(|id| desktop.monitors.iter().find(|item| item.monitor.id == id))
        .or_else(|| {
            desktop
                .monitors
                .iter()
                .find(|item| item.monitor.bounds.contains(point))
        })
}

fn primary_index(desktop: &FrozenDesktop) -> Option<usize> {
    desktop
        .monitors
        .iter()
        .position(|item| item.monitor.primary)
        .or_else(|| (!desktop.monitors.is_empty()).then_some(0))
}

fn cursor_index(desktop: &FrozenDesktop, cursor: Option<Point>) -> Option<usize> {
    cursor
        .and_then(|point| {
            desktop
                .monitors
                .iter()
                .position(|item| item.monitor.bounds.contains(point))
        })
        .or_else(|| primary_index(desktop))
}

fn take_monitor(desktop: &mut FrozenDesktop, index: Option<usize>) -> Result<Image, ServiceError> {
    let index = index.ok_or_else(no_monitor)?;
    let image = desktop.monitors.swap_remove(index).image;
    Ok(Arc::try_unwrap(image).unwrap_or_else(|shared| (*shared).clone()))
}

fn compose_all(desktop: &FrozenDesktop) -> Result<Image, ServiceError> {
    let Some(first) = desktop.monitors.first() else {
        return Err(no_monitor());
    };
    let mut bounds = first.monitor.bounds;
    for item in &desktop.monitors[1..] {
        bounds = bounds.union(item.monitor.bounds);
    }
    let width = u32::try_from(bounds.width.0).map_err(|_| no_monitor())?;
    let height = u32::try_from(bounds.height.0).map_err(|_| no_monitor())?;
    let mut output = Image::bgra(width, height).map_err(image_error)?;
    for item in &desktop.monitors {
        let x = item.monitor.bounds.x.0 - bounds.x.0;
        let y = item.monitor.bounds.y.0 - bounds.y.0;
        blit(&mut output, &item.image, x, y)?;
    }
    Ok(output)
}

fn blit(dest: &mut Image, source: &Image, x: i32, y: i32) -> Result<(), ServiceError> {
    let left = u32::try_from(x).map_err(|_| layout_changed())?;
    let top = u32::try_from(y).map_err(|_| layout_changed())?;
    let right = left
        .checked_add(source.width())
        .ok_or_else(layout_changed)?;
    let bottom = top
        .checked_add(source.height())
        .ok_or_else(layout_changed)?;
    if right > dest.width() || bottom > dest.height() {
        return Err(layout_changed());
    }
    let start = usize::try_from(left).map_err(|_| layout_changed())? * 4;
    for row in 0..source.height() {
        let from = source.row(row).ok_or_else(layout_changed)?;
        let to = dest.row_mut(top + row).ok_or_else(layout_changed)?;
        to[start..start + from.len()].copy_from_slice(from);
    }
    Ok(())
}

fn crop_to_monitor(frozen: &FrozenMonitor, selection: Rect) -> Result<(Image, Rect), ServiceError> {
    let clipped = selection
        .intersection(frozen.monitor.bounds)
        .ok_or_else(empty_area)?;
    let local = clipped.translated(Point::new(
        Px(-frozen.monitor.bounds.x.0),
        Px(-frozen.monitor.bounds.y.0),
    ));
    Ok((crop_image(&frozen.image, local)?, clipped))
}

fn crop_image(image: &Image, rect: Rect) -> Result<Image, ServiceError> {
    if rect.is_empty() {
        return Err(empty_area());
    }
    let left = u32::try_from(rect.x.0).map_err(|_| empty_area())?;
    let top = u32::try_from(rect.y.0).map_err(|_| empty_area())?;
    let width = u32::try_from(rect.width.0).map_err(|_| empty_area())?;
    let height = u32::try_from(rect.height.0).map_err(|_| empty_area())?;
    let right = left.checked_add(width).ok_or_else(empty_area)?;
    let bottom = top.checked_add(height).ok_or_else(empty_area)?;
    if right > image.width() || bottom > image.height() {
        return Err(empty_area());
    }
    let stride = usize::try_from(width).map_err(|_| empty_area())? * 4;
    let rows = usize::try_from(height).map_err(|_| empty_area())?;
    let mut pixels = Vec::with_capacity(stride * rows);
    let start = usize::try_from(left).map_err(|_| empty_area())? * 4;
    for row in 0..height {
        let source = image.row(top + row).ok_or_else(empty_area)?;
        pixels.extend_from_slice(&source[start..start + stride]);
    }
    Image::new(width, height, stride, pixels).map_err(image_error)
}

fn no_monitor() -> ServiceError {
    ServiceError::Invalid("There is no monitor to capture.".to_owned())
}

fn empty_area() -> ServiceError {
    ServiceError::Invalid("The selected area is empty.".to_owned())
}

fn layout_changed() -> ServiceError {
    ServiceError::Invalid("The display layout changed during the capture.".to_owned())
}

fn display_changed() -> ServiceError {
    ServiceError::Invalid(
        "The display layout changed, so the last region is no longer available.".to_owned(),
    )
}

fn no_windows() -> ServiceError {
    ServiceError::Invalid("There are no windows to capture.".to_owned())
}

fn window_missing() -> ServiceError {
    ServiceError::Invalid(snapline_core::ports::CaptureError::WindowMissing.to_string())
}

fn capture_error(error: snapline_core::ports::CaptureError) -> ServiceError {
    ServiceError::Invalid(error.to_string())
}

fn image_error(error: snapline_core::image::ImageError) -> ServiceError {
    ServiceError::Internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use snapline_core::geometry::DpiScale;
    use snapline_core::overlay::{Rgb, Tool};
    use snapline_core::ports::{
        CaptureError, ClipboardError, Monitor, OverlayAction as PortOverlayAction,
        OverlayError as PortOverlayError, OverlayResult as PortOverlayResult,
    };
    use snapline_store::io::Paths;

    use super::*;

    struct TestContext {
        services: Services,
        cursor: Mutex<Option<Point>>,
        foreground: Mutex<Option<WindowId>>,
        under_cursor: Mutex<Option<WindowId>>,
        choices: Mutex<Vec<WindowChoice>>,
        selection: Mutex<Option<Option<usize>>>,
        menus: Mutex<Vec<Vec<String>>>,
        at_point: Mutex<Vec<(Rect, WindowId)>>,
        apps: Mutex<HashMap<WindowId, String>>,
        theme: OverlayTheme,
        feedback: Mutex<Vec<(u32, u32, bool)>>,
        recordings: Mutex<Vec<(bool, Rect, u32)>>,
    }

    impl CaptureContext for TestContext {
        fn capture_complete(&self, image: &Image, sound: bool) {
            self.feedback
                .lock()
                .expect("feedback")
                .push((image.width(), image.height(), sound));
        }
        fn capture_settings(&self) -> Result<Settings, ServiceError> {
            self.services.settings_snapshot()
        }

        fn save_last_region(&self, region: LastRegion) -> Result<(), ServiceError> {
            <Services as CaptureContext>::save_last_region(&self.services, region)
        }

        fn start_recording(&self, gif: bool, rect: Rect, monitor: u32) -> Result<(), ServiceError> {
            self.recordings
                .lock()
                .expect("recordings")
                .push((gif, rect, monitor));
            Ok(())
        }

        fn foreground_window(&self) -> Option<WindowId> {
            *self.foreground.lock().expect("foreground")
        }

        fn window_under_cursor(&self) -> Option<WindowId> {
            *self.under_cursor.lock().expect("under cursor")
        }

        fn list_windows(&self) -> Vec<WindowChoice> {
            self.choices.lock().expect("choices").clone()
        }

        fn choose_window(
            &self,
            choices: &[WindowChoice],
        ) -> Result<Option<WindowPick>, ServiceError> {
            let Some(selection) = *self.selection.lock().expect("selection") else {
                return Ok(Some(WindowPick::Active));
            };
            self.menus.lock().expect("menus").push(menu_labels(choices));
            resolve_pick(choices, selection)
        }

        fn window_at(&self, point: Point) -> Option<WindowId> {
            self.at_point
                .lock()
                .expect("at point")
                .iter()
                .find(|(rect, _)| rect.contains(point))
                .map(|(_, window)| *window)
        }

        fn process_name(&self, window: WindowId) -> Option<String> {
            self.apps.lock().expect("apps").get(&window).cloned()
        }

        fn cursor_position(&self) -> Option<Point> {
            *self.cursor.lock().expect("cursor")
        }

        fn overlay_theme(&self) -> OverlayTheme {
            self.theme
        }
    }

    struct FakeCapture {
        desktop: FrozenDesktop,
        windows: Mutex<HashMap<WindowId, Image>>,
        calls: Mutex<Vec<(WindowId, bool)>>,
        freezes: AtomicUsize,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ScreenCapture for FakeCapture {
        fn monitors(&self) -> Vec<Monitor> {
            self.desktop
                .monitors
                .iter()
                .map(|item| item.monitor.clone())
                .collect()
        }

        fn freeze_all(&self) -> Result<FrozenDesktop, CaptureError> {
            self.freezes.fetch_add(1, Ordering::SeqCst);
            self.log.lock().expect("log").push("freeze");
            Ok(self.desktop.clone())
        }

        fn window(&self, id: WindowId, keep_shadow: bool) -> Result<Image, CaptureError> {
            self.calls.lock().expect("calls").push((id, keep_shadow));
            self.windows
                .lock()
                .expect("windows")
                .get(&id)
                .cloned()
                .ok_or(CaptureError::WindowMissing)
        }
    }

    struct FakeOverlay {
        result: Mutex<Result<PortOverlayResult, PortOverlayError>>,
        requests: Mutex<Vec<OverlayRequest>>,
    }

    impl OverlayHost for FakeOverlay {
        fn select_region(
            &self,
            request: OverlayRequest,
        ) -> Result<PortOverlayResult, PortOverlayError> {
            self.requests.lock().expect("requests").push(request);
            self.result.lock().expect("result").clone()
        }
    }

    struct FakeClipboard {
        images: Mutex<Vec<Image>>,
    }

    impl Clipboard for FakeClipboard {
        fn set_image(&self, image: &Image) -> Result<(), ClipboardError> {
            self.images.lock().expect("images").push(image.clone());
            Ok(())
        }

        fn set_text(&self, _text: &str) -> Result<(), ClipboardError> {
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    struct FakeCountdown {
        calls: Mutex<Vec<u8>>,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Countdown for FakeCountdown {
        fn run(&self, seconds: u8) -> Result<(), ServiceError> {
            self.calls.lock().expect("countdown").push(seconds);
            self.log.lock().expect("log").push("countdown");
            Ok(())
        }
    }

    struct Harness {
        service: CaptureService,
        capture: Arc<FakeCapture>,
        overlay: Arc<FakeOverlay>,
        clipboard: Arc<FakeClipboard>,
        countdown: Arc<FakeCountdown>,
        context: Arc<TestContext>,
        services: Services,
    }

    fn paths(directory: &Path) -> Paths {
        Paths::resolve(
            &directory.join("exe"),
            &directory.join("Roaming"),
            &directory.join("Local"),
            &directory.join("Profile"),
            false,
        )
    }

    fn monitor(id: u32, x: i32, y: i32, width: i32, height: i32, primary: bool) -> Monitor {
        let bounds = Rect::new(Px(x), Px(y), Px(width), Px(height));
        Monitor {
            id,
            name: format!("Display {id}"),
            bounds,
            work_area: bounds,
            dpi: DpiScale::ONE,
            primary,
        }
    }

    fn desktop_of(items: Vec<(Monitor, Image)>) -> FrozenDesktop {
        FrozenDesktop {
            monitors: items
                .into_iter()
                .map(|(monitor, image)| FrozenMonitor {
                    monitor,
                    image: Arc::new(image),
                })
                .collect(),
        }
    }

    fn pattern_pixel(x: u32, y: u32) -> [u8; 4] {
        [
            u8::try_from(x % 251).expect("byte"),
            u8::try_from(y % 241).expect("byte"),
            0x33,
            0xff,
        ]
    }

    fn pattern(width: u32, height: u32) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                image.set_pixel(x, y, pattern_pixel(x, y)).expect("pixel");
            }
        }
        image
    }

    fn solid(width: u32, height: u32, value: [u8; 4]) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                image.set_pixel(x, y, value).expect("pixel");
            }
        }
        image
    }

    fn selected(rect: Rect, monitor: u32) -> Result<PortOverlayResult, PortOverlayError> {
        Ok(PortOverlayResult {
            action: Some(PortOverlayAction::Copy),
            selection: rect,
            monitor: Some(monitor),
            tool: Tool::Rect,
            colour: Rgb::default(),
            image: None,
        })
    }

    fn harness(
        directory: &Path,
        desktop: FrozenDesktop,
        result: Result<PortOverlayResult, PortOverlayError>,
    ) -> Harness {
        let services = Services::with_paths(paths(directory)).expect("services");
        let context = Arc::new(TestContext {
            services: services.clone(),
            cursor: Mutex::new(None),
            foreground: Mutex::new(None),
            under_cursor: Mutex::new(None),
            choices: Mutex::new(Vec::new()),
            selection: Mutex::new(None),
            menus: Mutex::new(Vec::new()),
            at_point: Mutex::new(Vec::new()),
            apps: Mutex::new(HashMap::new()),
            theme: OverlayTheme::Dark,
            feedback: Mutex::new(Vec::new()),
            recordings: Mutex::new(Vec::new()),
        });
        let log = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::new(FakeCapture {
            desktop,
            windows: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
            freezes: AtomicUsize::new(0),
            log: Arc::clone(&log),
        });
        let overlay = Arc::new(FakeOverlay {
            result: Mutex::new(result),
            requests: Mutex::new(Vec::new()),
        });
        let clipboard = Arc::new(FakeClipboard {
            images: Mutex::new(Vec::new()),
        });
        let countdown = Arc::new(FakeCountdown {
            calls: Mutex::new(Vec::new()),
            log: Arc::clone(&log),
        });
        let service = CaptureService::new(
            Arc::clone(&capture) as Arc<dyn ScreenCapture + Send + Sync>,
            Arc::clone(&overlay) as Arc<dyn OverlayHost + Send + Sync>,
            Arc::clone(&clipboard) as Arc<dyn Clipboard + Send + Sync>,
            Arc::clone(&countdown) as Arc<dyn Countdown>,
            Arc::clone(&context) as Arc<dyn CaptureContext>,
        );
        Harness {
            service,
            capture,
            overlay,
            clipboard,
            countdown,
            context,
            services,
        }
    }

    fn last_region_table(services: &Services) -> toml::Table {
        services
            .settings_snapshot()
            .expect("settings")
            .capture
            .extra
            .get(LastRegion::KEY)
            .and_then(|value| value.as_table())
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn region_selection_crops_at_physical_pixels_and_copies() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(
            monitor(7, 100, 100, 1000, 600, true),
            pattern(1000, 600),
        )]);
        let selection = Rect::new(Px(250), Px(200), Px(300), Px(150));
        let harness = harness(directory.path(), desktop, selected(selection, 7));

        let outcome = harness.service.capture_region().expect("region");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 300,
                height: 150
            }
        );

        let copies = harness.clipboard.images.lock().expect("images");
        assert_eq!(copies.len(), 1);
        assert_eq!(
            harness
                .context
                .feedback
                .lock()
                .expect("feedback")
                .as_slice(),
            &[(300, 150, true)]
        );
        let copied = &copies[0];
        assert_eq!(copied.width(), 300);
        assert_eq!(copied.height(), 150);
        assert_eq!(copied.pixel(0, 0).expect("pixel"), pattern_pixel(150, 100));
        assert_eq!(
            copied.pixel(299, 149).expect("pixel"),
            pattern_pixel(449, 249)
        );

        let requests = harness.overlay.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].annotate);
        assert!(requests[0].magnifier.enabled);
        assert_eq!(requests[0].magnifier.zoom, CoreMagnifierZoom::X4);
        assert!(!requests[0].full_screen_crosshair);
        assert_eq!(requests[0].desktop.monitors.len(), 1);

        let table = last_region_table(&harness.services);
        assert_eq!(
            table.get("monitor").and_then(toml::Value::as_integer),
            Some(7)
        );
        assert_eq!(table.get("x").and_then(toml::Value::as_integer), Some(250));
        assert_eq!(table.get("y").and_then(toml::Value::as_integer), Some(200));
        assert_eq!(
            table.get("width").and_then(toml::Value::as_integer),
            Some(300)
        );
        assert_eq!(
            table.get("height").and_then(toml::Value::as_integer),
            Some(150)
        );
    }

    #[test]
    fn region_result_preserves_the_toolbar_output_choice() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(monitor(7, 0, 0, 10, 10, true), pattern(10, 10))]);
        let mut result = selected(Rect::new(Px(1), Px(1), Px(5), Px(5)), 7).expect("selection");
        result.action = Some(PortOverlayAction::Save);
        let harness = harness(directory.path(), desktop, Ok(result));
        let shot = harness
            .service
            .region_result(true)
            .expect("capture")
            .expect("selection");
        assert_eq!((shot.image.width(), shot.image.height()), (5, 5));
        assert_eq!(shot.action, Some(PortOverlayAction::Save));
        assert!(harness
            .clipboard
            .images
            .lock()
            .expect("clipboard")
            .is_empty());
    }

    #[test]
    fn a_record_choice_starts_a_recording_of_the_selection_and_takes_no_screenshot() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(monitor(7, 0, 0, 10, 10, true), pattern(10, 10))]);
        let selection = Rect::new(Px(1), Px(1), Px(5), Px(5));
        for (action, gif) in [
            (PortOverlayAction::Record, false),
            (PortOverlayAction::Gif, true),
        ] {
            let mut result = selected(selection, 7).expect("selection");
            result.action = Some(action);
            let harness = harness(directory.path(), desktop.clone(), Ok(result));
            let shot = harness.service.region_result(true).expect("capture");
            assert!(shot.is_none());
            assert_eq!(
                *harness.context.recordings.lock().expect("recordings"),
                vec![(gif, selection, 7)]
            );
            assert!(harness
                .context
                .feedback
                .lock()
                .expect("feedback")
                .is_empty());
            assert!(harness
                .clipboard
                .images
                .lock()
                .expect("clipboard")
                .is_empty());
        }
    }

    #[test]
    fn region_toolbar_matches_the_image_destination() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(monitor(7, 0, 0, 10, 10, true), pattern(10, 10))]);
        let selection = Rect::new(Px(1), Px(1), Px(5), Px(5));
        let harness = harness(directory.path(), desktop, selected(selection, 7));
        harness.service.region_result(true).expect("local capture");
        assert!(!harness.overlay.requests.lock().expect("requests")[0].upload_on_finish);
        harness
            .services
            .update_settings(&serde_json::json!({ "uploads": { "images": "my-destination" } }))
            .expect("settings");
        harness.service.region_result(true).expect("upload capture");
        assert!(harness.overlay.requests.lock().expect("requests")[1].upload_on_finish);
    }

    #[test]
    fn region_selection_is_clamped_to_the_monitor() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(
            monitor(3, 100, 100, 1000, 600, true),
            pattern(1000, 600),
        )]);
        let selection = Rect::new(Px(900), Px(650), Px(300), Px(200));
        let harness = harness(directory.path(), desktop, selected(selection, 3));

        let outcome = harness.service.capture_region().expect("region");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 200,
                height: 50
            }
        );
        let table = last_region_table(&harness.services);
        assert_eq!(
            table.get("width").and_then(toml::Value::as_integer),
            Some(200)
        );
        assert_eq!(
            table.get("height").and_then(toml::Value::as_integer),
            Some(50)
        );
    }

    #[test]
    fn cancelling_the_overlay_copies_nothing() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(monitor(1, 0, 0, 800, 600, true), pattern(800, 600))]);
        let harness = harness(directory.path(), desktop, Err(PortOverlayError::Cancelled));

        let outcome = harness.service.capture_region().expect("region");
        assert_eq!(outcome, CaptureOutcome::Cancelled);
        assert!(harness.clipboard.images.lock().expect("images").is_empty());
        assert!(last_region_table(&harness.services).is_empty());
    }

    #[test]
    fn window_capture_uses_the_foreground_window_and_keep_shadow() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = harness(
            directory.path(),
            desktop_of(vec![]),
            selected(Rect::ZERO, 1),
        );
        harness
            .services
            .update_settings(&serde_json::json!({
                "capture": { "keep_window_shadow": false }
            }))
            .expect("settings");
        *harness.context.foreground.lock().expect("foreground") = Some(WindowId::new(42));
        harness
            .capture
            .windows
            .lock()
            .expect("windows")
            .insert(WindowId::new(42), solid(120, 80, [9, 8, 7, 255]));

        let outcome = harness.service.capture_window().expect("window");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 120,
                height: 80
            }
        );
        assert_eq!(
            *harness.capture.calls.lock().expect("calls"),
            vec![(WindowId::new(42), false)]
        );
        let copies = harness.clipboard.images.lock().expect("images");
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].pixel(0, 0).expect("pixel"), [9, 8, 7, 255]);
    }

    fn window_choice(id: u64, title: &str, app: Option<&str>) -> WindowChoice {
        WindowChoice {
            id: WindowId::new(id),
            title: title.to_owned(),
            app: app.map(str::to_owned),
        }
    }

    fn window_menu_harness(directory: &std::path::Path, selection: Option<usize>) -> Harness {
        let harness = harness(directory, desktop_of(vec![]), selected(Rect::ZERO, 1));
        *harness.context.foreground.lock().expect("foreground") = Some(WindowId::new(7));
        *harness.context.choices.lock().expect("choices") = vec![
            window_choice(7, "Notes", Some("notepad.exe")),
            window_choice(8, "Browser", Some("chrome.exe")),
        ];
        *harness.context.selection.lock().expect("selection") = Some(selection);
        let mut windows = harness.capture.windows.lock().expect("windows");
        windows.insert(WindowId::new(42), solid(10, 10, [1, 1, 1, 255]));
        windows.insert(WindowId::new(7), solid(20, 20, [2, 2, 2, 255]));
        windows.insert(WindowId::new(8), solid(30, 30, [3, 3, 3, 255]));
        drop(windows);
        harness
    }

    #[test]
    fn the_chosen_window_handle_is_the_one_captured() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = window_menu_harness(directory.path(), Some(2));
        let outcome = harness.service.capture_window().expect("window");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 30,
                height: 30
            }
        );
        assert_eq!(
            harness.capture.calls.lock().expect("calls")[0].0,
            WindowId::new(8)
        );
        let menus = harness.context.menus.lock().expect("menus");
        assert_eq!(
            menus[0],
            vec![
                "Active window".to_owned(),
                "notepad.exe \u{2014} Notes".to_owned(),
                "chrome.exe \u{2014} Browser".to_owned(),
            ]
        );
    }

    #[test]
    fn the_active_window_entry_captures_the_window_in_front_before_the_menu() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = window_menu_harness(directory.path(), Some(0));
        harness.service.capture_window().expect("window");
        assert_eq!(
            harness.capture.calls.lock().expect("calls")[0].0,
            WindowId::new(7)
        );
    }

    #[test]
    fn an_unlisted_foreground_window_falls_back_to_the_topmost_listed_one() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = window_menu_harness(directory.path(), Some(0));
        *harness.context.foreground.lock().expect("foreground") = Some(WindowId::new(42));
        harness.service.capture_window().expect("window");
        assert_eq!(
            harness.capture.calls.lock().expect("calls")[0].0,
            WindowId::new(7)
        );
    }

    #[test]
    fn dismissing_the_window_menu_cancels_without_an_error() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = window_menu_harness(directory.path(), None);
        let outcome = harness.service.capture_window().expect("window");
        assert_eq!(outcome, CaptureOutcome::Cancelled);
        assert!(harness.capture.calls.lock().expect("calls").is_empty());
        assert!(harness.clipboard.images.lock().expect("images").is_empty());
    }

    #[test]
    fn an_empty_window_list_reports_that_there_is_nothing_to_capture() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = window_menu_harness(directory.path(), Some(0));
        harness.context.choices.lock().expect("choices").clear();
        let error = harness.service.capture_window().expect_err("empty");
        assert_eq!(error.to_string(), "There are no windows to capture.");
    }

    #[test]
    fn long_titles_are_truncated_for_the_menu() {
        assert_eq!(truncate_title("short", 10), "short");
        assert_eq!(truncate_title("  padded  ", 10), "padded");
        let long = "x".repeat(100);
        let truncated = truncate_title(&long, MENU_TITLE_LIMIT);
        assert_eq!(truncated.chars().count(), MENU_TITLE_LIMIT);
        assert!(truncated.ends_with('\u{2026}'));
        assert_eq!(truncate_title("ab\u{e9}def", 4), "ab\u{e9}\u{2026}");
    }

    #[test]
    fn labels_show_the_app_when_it_is_known() {
        assert_eq!(
            choice_label(&window_choice(1, "Doc", Some("word.exe"))),
            "word.exe \u{2014} Doc"
        );
        assert_eq!(choice_label(&window_choice(1, "Doc", None)), "Doc");
        assert_eq!(choice_label(&window_choice(1, "Doc", Some(""))), "Doc");
    }

    #[test]
    fn picks_resolve_to_the_matching_entry() {
        let choices = vec![window_choice(7, "A", None), window_choice(8, "B", None)];
        assert_eq!(resolve_pick(&choices, None), Ok(None));
        assert_eq!(
            resolve_pick(&choices, Some(0)),
            Ok(Some(WindowPick::Active))
        );
        assert_eq!(
            resolve_pick(&choices, Some(2)),
            Ok(Some(WindowPick::Window(WindowId::new(8))))
        );
        assert!(resolve_pick(&choices, Some(3)).is_err());
        assert!(resolve_pick(&[], Some(0)).is_err());
    }

    #[test]
    fn window_capture_under_the_cursor_uses_the_tool_window() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = harness(
            directory.path(),
            desktop_of(vec![]),
            selected(Rect::ZERO, 1),
        );
        *harness.context.under_cursor.lock().expect("under cursor") = Some(WindowId::new(9));
        harness
            .capture
            .windows
            .lock()
            .expect("windows")
            .insert(WindowId::new(9), solid(64, 32, [4, 4, 4, 255]));

        let outcome = harness
            .service
            .capture_window_under_cursor()
            .expect("window");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 64,
                height: 32
            }
        );
        assert_eq!(
            *harness.capture.calls.lock().expect("calls"),
            vec![(WindowId::new(9), true)]
        );
    }

    fn place_app(harness: &Harness, id: u64, rect: Rect, name: &str) {
        harness
            .context
            .at_point
            .lock()
            .expect("at point")
            .push((rect, WindowId::new(id)));
        harness
            .context
            .apps
            .lock()
            .expect("apps")
            .insert(WindowId::new(id), name.to_owned());
    }

    #[test]
    fn a_region_capture_records_the_app_under_the_selection() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(monitor(1, 0, 0, 800, 600, true), pattern(800, 600))]);
        let selection = Rect::new(Px(500), Px(100), Px(100), Px(100));
        let harness = harness(directory.path(), desktop, selected(selection, 1));
        place_app(
            &harness,
            1,
            Rect::new(Px(0), Px(0), Px(400), Px(600)),
            "chrome.exe",
        );
        place_app(
            &harness,
            2,
            Rect::new(Px(400), Px(0), Px(400), Px(600)),
            "firefox.exe",
        );
        *harness.context.foreground.lock().expect("foreground") = Some(WindowId::new(1));
        let shot = harness
            .service
            .region_result(true)
            .expect("capture")
            .expect("selection");
        assert_eq!(shot.source_app.as_deref(), Some("firefox.exe"));
    }

    #[test]
    fn a_region_over_no_app_does_not_fall_back_to_the_foreground_window() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(monitor(1, 0, 0, 800, 600, true), pattern(800, 600))]);
        let selection = Rect::new(Px(500), Px(100), Px(100), Px(100));
        let harness = harness(directory.path(), desktop, selected(selection, 1));
        place_app(
            &harness,
            1,
            Rect::new(Px(0), Px(0), Px(400), Px(600)),
            "snapline.exe",
        );
        *harness.context.foreground.lock().expect("foreground") = Some(WindowId::new(1));
        let shot = harness
            .service
            .region_result(true)
            .expect("capture")
            .expect("selection");
        assert_eq!(shot.source_app, None);
    }

    #[test]
    fn a_window_capture_records_the_captured_window_app() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = harness(
            directory.path(),
            desktop_of(vec![]),
            selected(Rect::ZERO, 1),
        );
        *harness.context.foreground.lock().expect("foreground") = Some(WindowId::new(42));
        harness
            .capture
            .windows
            .lock()
            .expect("windows")
            .insert(WindowId::new(42), solid(120, 80, [9, 8, 7, 255]));
        harness
            .context
            .apps
            .lock()
            .expect("apps")
            .insert(WindowId::new(42), "code.exe".to_owned());
        let shot = harness
            .service
            .window_image()
            .expect("capture")
            .expect("window");
        assert_eq!(shot.source_app.as_deref(), Some("code.exe"));
    }

    #[test]
    fn a_single_monitor_capture_records_the_app_at_its_centre() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![
            (
                monitor(1, 0, 0, 800, 600, true),
                solid(800, 600, [1, 1, 1, 255]),
            ),
            (
                monitor(2, 800, 0, 640, 480, false),
                solid(640, 480, [2, 2, 2, 255]),
            ),
        ]);
        let harness = harness(directory.path(), desktop, selected(Rect::ZERO, 1));
        harness
            .services
            .update_settings(&serde_json::json!({
                "capture": { "fullscreen_capture_uses": "primary_monitor" }
            }))
            .expect("settings");
        place_app(
            &harness,
            1,
            Rect::new(Px(0), Px(0), Px(800), Px(600)),
            "game.exe",
        );
        place_app(
            &harness,
            2,
            Rect::new(Px(800), Px(0), Px(640), Px(480)),
            "slack.exe",
        );
        *harness.context.cursor.lock().expect("cursor") = Some(Point::new(Px(900), Px(100)));
        let shot = harness
            .service
            .screen_image()
            .expect("capture")
            .expect("screen");
        assert_eq!(shot.source_app.as_deref(), Some("game.exe"));
    }

    #[test]
    fn an_all_monitors_capture_has_no_source_app() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(
            monitor(1, 0, 0, 800, 600, true),
            solid(800, 600, [1, 1, 1, 255]),
        )]);
        let harness = harness(directory.path(), desktop, selected(Rect::ZERO, 1));
        harness
            .services
            .update_settings(&serde_json::json!({
                "capture": { "fullscreen_capture_uses": "all_monitors" }
            }))
            .expect("settings");
        place_app(
            &harness,
            1,
            Rect::new(Px(0), Px(0), Px(800), Px(600)),
            "game.exe",
        );
        *harness.context.cursor.lock().expect("cursor") = Some(Point::new(Px(100), Px(100)));
        let shot = harness
            .service
            .screen_image()
            .expect("capture")
            .expect("screen");
        assert_eq!(shot.source_app, None);
    }

    #[test]
    fn a_last_region_capture_records_the_app_under_the_region() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(monitor(1, 0, 0, 800, 600, true), pattern(800, 600))]);
        let selection = Rect::new(Px(100), Px(100), Px(100), Px(100));
        let harness = harness(directory.path(), desktop, selected(selection, 1));
        harness.service.capture_region().expect("region");
        place_app(
            &harness,
            1,
            Rect::new(Px(0), Px(0), Px(400), Px(600)),
            "chrome.exe",
        );
        let shot = harness
            .service
            .last_region_image()
            .expect("capture")
            .expect("region");
        assert_eq!(shot.source_app.as_deref(), Some("chrome.exe"));
    }

    #[test]
    fn window_capture_without_a_window_reports_it() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = harness(
            directory.path(),
            desktop_of(vec![]),
            selected(Rect::ZERO, 1),
        );
        let error = harness.service.capture_window().expect_err("missing");
        assert_eq!(error.to_string(), "The requested window no longer exists.");
        assert!(harness.clipboard.images.lock().expect("images").is_empty());
    }

    #[test]
    fn screen_capture_uses_the_monitor_under_the_cursor() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![
            (
                monitor(1, 0, 0, 800, 600, true),
                solid(800, 600, [1, 1, 1, 255]),
            ),
            (
                monitor(2, 800, 0, 640, 480, false),
                solid(640, 480, [2, 2, 2, 255]),
            ),
        ]);
        let harness = harness(directory.path(), desktop, selected(Rect::ZERO, 1));
        *harness.context.cursor.lock().expect("cursor") = Some(Point::new(Px(900), Px(100)));

        let outcome = harness.service.capture_screen().expect("screen");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 640,
                height: 480
            }
        );
        let copies = harness.clipboard.images.lock().expect("images");
        assert_eq!(copies[0].pixel(0, 0).expect("pixel"), [2, 2, 2, 255]);
    }

    #[test]
    fn screen_capture_falls_back_to_the_primary_monitor() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![
            (
                monitor(1, 0, 0, 800, 600, false),
                solid(800, 600, [1, 1, 1, 255]),
            ),
            (
                monitor(2, 800, 0, 640, 480, true),
                solid(640, 480, [2, 2, 2, 255]),
            ),
        ]);
        let harness = harness(directory.path(), desktop, selected(Rect::ZERO, 1));
        harness
            .services
            .update_settings(&serde_json::json!({
                "capture": { "fullscreen_capture_uses": "primary_monitor" }
            }))
            .expect("settings");

        let outcome = harness.service.capture_screen().expect("screen");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 640,
                height: 480
            }
        );
    }

    #[test]
    fn screen_capture_composes_all_monitors_at_the_real_layout() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![
            (
                monitor(1, 0, 0, 100, 80, true),
                solid(100, 80, [1, 1, 1, 255]),
            ),
            (
                monitor(2, 200, -40, 60, 50, false),
                solid(60, 50, [2, 2, 2, 255]),
            ),
        ]);
        let harness = harness(directory.path(), desktop, selected(Rect::ZERO, 1));
        harness
            .services
            .update_settings(&serde_json::json!({
                "capture": { "fullscreen_capture_uses": "all_monitors" }
            }))
            .expect("settings");

        let outcome = harness.service.capture_screen().expect("screen");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 260,
                height: 120
            }
        );
        let copies = harness.clipboard.images.lock().expect("images");
        let image = &copies[0];
        assert_eq!(image.pixel(10, 50).expect("pixel"), [1, 1, 1, 255]);
        assert_eq!(image.pixel(205, 5).expect("pixel"), [2, 2, 2, 255]);
        assert_eq!(image.pixel(150, 5).expect("pixel"), [0, 0, 0, 0]);
    }

    #[test]
    fn last_region_reuses_the_selection_without_the_overlay() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(
            monitor(7, 100, 100, 1000, 600, true),
            pattern(1000, 600),
        )]);
        let selection = Rect::new(Px(250), Px(200), Px(300), Px(150));
        let harness = harness(directory.path(), desktop, selected(selection, 7));
        let first = harness.service.capture_region().expect("region");
        *harness.overlay.result.lock().expect("result") = Err(PortOverlayError::Cancelled);

        let second = harness.service.capture_last_region().expect("last region");
        assert_eq!(first, second);
        assert_eq!(harness.overlay.requests.lock().expect("requests").len(), 1);
        assert_eq!(harness.clipboard.images.lock().expect("images").len(), 2);
    }

    #[test]
    fn last_region_survives_in_settings_for_a_new_service() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(
            monitor(7, 100, 100, 1000, 600, true),
            pattern(1000, 600),
        )]);
        let selection = Rect::new(Px(250), Px(200), Px(300), Px(150));
        let harness = harness(directory.path(), desktop, selected(selection, 7));
        harness.service.capture_region().expect("region");

        let reloaded = Services::with_paths(paths(directory.path())).expect("reloaded");
        let context = Arc::new(TestContext {
            services: reloaded,
            cursor: Mutex::new(None),
            foreground: Mutex::new(None),
            under_cursor: Mutex::new(None),
            choices: Mutex::new(Vec::new()),
            selection: Mutex::new(None),
            menus: Mutex::new(Vec::new()),
            at_point: Mutex::new(Vec::new()),
            apps: Mutex::new(HashMap::new()),
            theme: OverlayTheme::Dark,
            feedback: Mutex::new(Vec::new()),
            recordings: Mutex::new(Vec::new()),
        });
        let overlay = Arc::new(FakeOverlay {
            result: Mutex::new(Err(PortOverlayError::Cancelled)),
            requests: Mutex::new(Vec::new()),
        });
        let clipboard = Arc::new(FakeClipboard {
            images: Mutex::new(Vec::new()),
        });
        let second = CaptureService::new(
            Arc::clone(&harness.capture) as Arc<dyn ScreenCapture + Send + Sync>,
            overlay as Arc<dyn OverlayHost + Send + Sync>,
            Arc::clone(&clipboard) as Arc<dyn Clipboard + Send + Sync>,
            Arc::clone(&harness.countdown) as Arc<dyn Countdown>,
            context as Arc<dyn CaptureContext>,
        );
        let outcome = second.capture_last_region().expect("last region");
        assert_eq!(
            outcome,
            CaptureOutcome::Copied {
                width: 300,
                height: 150
            }
        );
        let copies = clipboard.images.lock().expect("images");
        assert_eq!(
            copies[0].pixel(0, 0).expect("pixel"),
            pattern_pixel(150, 100)
        );
    }

    #[test]
    fn last_region_without_a_stored_region_reports_it() {
        let directory = tempfile::tempdir().expect("temp");
        let harness = harness(
            directory.path(),
            desktop_of(vec![]),
            selected(Rect::ZERO, 1),
        );
        let error = harness
            .service
            .capture_last_region()
            .expect_err("no region");
        assert_eq!(error.to_string(), "There is no last region to capture yet.");
        assert_eq!(harness.capture.freezes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn capture_delay_maps_to_seconds() {
        assert_eq!(delay_seconds(CaptureDelay::Off), 0);
        assert_eq!(delay_seconds(CaptureDelay::OneSecond), 1);
        assert_eq!(delay_seconds(CaptureDelay::ThreeSeconds), 3);
        assert_eq!(delay_seconds(CaptureDelay::FiveSeconds), 5);
    }

    #[test]
    fn the_countdown_runs_before_the_freeze_only_when_delay_is_on() {
        let directory = tempfile::tempdir().expect("temp");
        let desktop = desktop_of(vec![(
            monitor(1, 0, 0, 800, 600, true),
            solid(800, 600, [1, 1, 1, 255]),
        )]);
        let harness = harness(directory.path(), desktop, selected(Rect::ZERO, 1));

        harness.service.capture_screen().expect("immediate");
        assert!(harness.countdown.calls.lock().expect("calls").is_empty());
        assert_eq!(harness.capture.freezes.load(Ordering::SeqCst), 1);

        harness
            .services
            .update_settings(&serde_json::json!({
                "capture": { "capture_delay": "3s" }
            }))
            .expect("settings");
        harness.service.capture_screen().expect("delayed");
        assert_eq!(*harness.countdown.calls.lock().expect("calls"), vec![3]);
        assert_eq!(harness.capture.freezes.load(Ordering::SeqCst), 2);
        assert_eq!(
            *harness.countdown.log.lock().expect("log"),
            vec!["freeze", "countdown", "freeze"]
        );
    }

    #[test]
    fn unknown_capture_actions_stay_not_yet() {
        let error =
            run(&ActionId::new("capture_scrolling"), &Services::new()).expect_err("removed");
        assert_eq!(error, ServiceError::NotYet);
        let error = run(&ActionId::new("capture_nonsense"), &Services::new()).expect_err("unknown");
        assert_eq!(error, ServiceError::NotYet);
    }
}
