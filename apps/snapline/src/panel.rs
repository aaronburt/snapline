use std::mem::size_of;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DWM_WINDOW_CORNER_PREFERENCE,
};

use crate::theme;
use crate::windows::WindowManager;

pub const PANEL_LABEL: &str = "panel";
pub const PANEL_WIDTH: f64 = 344.0;
const MARGIN: f64 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrayRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl TrayRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_tauri(rect: tauri::Rect) -> Self {
        let (x, y) = match rect.position {
            tauri::Position::Physical(position) => (f64::from(position.x), f64::from(position.y)),
            tauri::Position::Logical(position) => (position.x, position.y),
        };
        let (width, height) = match rect.size {
            tauri::Size::Physical(size) => (f64::from(size.width), f64::from(size.height)),
            tauri::Size::Logical(size) => (size.width, size.height),
        };
        Self::new(x, y, width, height)
    }

    fn centre_x(self) -> f64 {
        self.x + self.width / 2.0
    }

    fn centre_y(self) -> f64 {
        self.y + self.height / 2.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskbarEdge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PanelState {
    tray: Option<TrayRect>,
    height: Option<f64>,
    ready: bool,
}

static STATE: Mutex<PanelState> = Mutex::new(PanelState {
    tray: None,
    height: None,
    ready: false,
});

fn with_state<T>(apply: impl FnOnce(&mut PanelState) -> T) -> T {
    let mut state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    apply(&mut state)
}

pub fn taskbar_edge(work: Bounds, monitor: Bounds) -> TaskbarEdge {
    if work.y > monitor.y {
        TaskbarEdge::Top
    } else if work.x > monitor.x {
        TaskbarEdge::Left
    } else if work.right() < monitor.right() {
        TaskbarEdge::Right
    } else {
        TaskbarEdge::Bottom
    }
}

pub fn placement(
    icon: TrayRect,
    work: Bounds,
    monitor: Bounds,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let minimum_x = work.x + MARGIN;
    let maximum_x = (work.right() - MARGIN - width).max(minimum_x);
    let minimum_y = work.y + MARGIN;
    let maximum_y = (work.bottom() - MARGIN - height).max(minimum_y);
    let icon_y = (icon.centre_y() - height / 2.0).clamp(minimum_y, maximum_y);
    let (x, y) = match taskbar_edge(work, monitor) {
        TaskbarEdge::Bottom => (maximum_x, maximum_y),
        TaskbarEdge::Top => (maximum_x, minimum_y),
        TaskbarEdge::Left => (minimum_x, icon_y),
        TaskbarEdge::Right => (maximum_x, icon_y),
    };
    (x.clamp(minimum_x, maximum_x), y.clamp(minimum_y, maximum_y))
}

fn work_bounds(monitor: &tauri::Monitor) -> Bounds {
    let work = monitor.work_area();
    Bounds::new(
        f64::from(work.position.x),
        f64::from(work.position.y),
        f64::from(work.size.width),
        f64::from(work.size.height),
    )
}

fn monitor_bounds(monitor: &tauri::Monitor) -> Bounds {
    let position = monitor.position();
    let size = monitor.size();
    Bounds::new(
        f64::from(position.x),
        f64::from(position.y),
        f64::from(size.width),
        f64::from(size.height),
    )
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_u32(value: f64) -> u32 {
    value.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

#[allow(clippy::cast_possible_truncation)]
fn to_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

pub fn show(app: &AppHandle, x: f64, y: f64, width: f64, height: f64) {
    if crate::native_panel::enabled() {
        crate::native_panel::show(app, TrayRect::new(x, y, width, height));
    } else {
        show_web(app, x, y, width, height);
    }
}

pub fn place_native(app: &AppHandle, tray: TrayRect, width: f64, height: f64) -> (i32, i32) {
    let Some(monitor) = app
        .monitor_from_point(tray.centre_x(), tray.centre_y())
        .ok()
        .flatten()
    else {
        return (to_i32(tray.x), to_i32(tray.y));
    };
    let (x, y) = placement(
        tray,
        work_bounds(&monitor),
        monitor_bounds(&monitor),
        width,
        height,
    );
    (to_i32(x), to_i32(y))
}

pub fn show_web(app: &AppHandle, x: f64, y: f64, width: f64, height: f64) {
    let tray = TrayRect::new(x, y, width, height);
    let existing = app.get_webview_window(PANEL_LABEL).is_some();
    with_state(|state| {
        state.tray = Some(tray);
        if !existing {
            state.height = None;
            state.ready = false;
        }
    });
    let manager = app.state::<WindowManager>();
    if let Err(error) = manager.open_panel(app) {
        tracing::error!(%error, "could not open the quick panel");
        return;
    }
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        apply_rounded_corners(&window);
        let height = with_state(|state| state.height);
        if let Some(height) = height {
            if let Err(error) = place(&window, height) {
                tracing::error!(%error, "could not place the quick panel");
            }
        }
    }
}

pub fn ready(window: &WebviewWindow) -> tauri::Result<()> {
    let height = with_state(|state| {
        state.ready = true;
        state.height
    });
    if let Some(height) = height {
        place(window, height)?;
        reveal(window)?;
    }
    Ok(())
}

pub fn resize(window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    if !height.is_finite() {
        return Ok(());
    }
    let height = height.max(1.0);
    let ready = with_state(|state| {
        state.height = Some(height);
        state.ready
    });
    apply_rounded_corners(window);
    place(window, height)?;
    if ready {
        reveal(window)?;
    }
    Ok(())
}

fn reveal(window: &WebviewWindow) -> tauri::Result<()> {
    window.show()?;
    window.set_focus()?;
    Ok(())
}

fn place(window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    let stored = with_state(|state| state.tray);
    let icon = match stored {
        Some(tray) => tray,
        None => {
            let work = window.current_monitor()?.map_or_else(
                || Bounds::new(0.0, 0.0, 1.0, 1.0),
                |monitor| work_bounds(&monitor),
            );
            TrayRect::new(work.right() - 1.0, work.bottom() - 1.0, 0.0, 0.0)
        }
    };
    let Some(monitor) = window.monitor_from_point(icon.centre_x(), icon.centre_y())? else {
        return Ok(());
    };
    let work = work_bounds(&monitor);
    let screen = monitor_bounds(&monitor);
    let scale = monitor.scale_factor();
    let physical_width = PANEL_WIDTH * scale;
    let physical_height = height * scale;
    let (x, y) = placement(icon, work, screen, physical_width, physical_height);
    window.set_size(PhysicalSize::new(
        to_u32(physical_width),
        to_u32(physical_height),
    ))?;
    window.set_position(PhysicalPosition::new(to_i32(x), to_i32(y)))?;
    Ok(())
}

pub fn apply_rounded_corners(window: &WebviewWindow) {
    if !theme::is_windows_11() {
        return;
    }
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let preference = DWMWCP_ROUND;
    // SAFETY: hwnd is this window's live native handle and the pointer reads a live preference value.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&preference).cast(),
            u32::try_from(size_of::<DWM_WINDOW_CORNER_PREFERENCE>()).unwrap_or(4),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    const WIDTH: f64 = 344.0;
    const HEIGHT: f64 = 600.0;

    fn icon() -> TrayRect {
        TrayRect::new(1850.0, 1040.0, 24.0, 24.0)
    }

    fn middle_icon() -> TrayRect {
        TrayRect::new(48.0, 528.0, 24.0, 24.0)
    }

    #[test]
    fn bottom_taskbar_sits_above_the_tray_icon() {
        let work = Bounds::new(0.0, 0.0, 1920.0, 1040.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Bottom);
        assert_eq!(
            placement(icon(), work, MONITOR, WIDTH, HEIGHT),
            (1564.0, 428.0)
        );
    }

    #[test]
    fn top_taskbar_sits_below_the_taskbar() {
        let work = Bounds::new(0.0, 40.0, 1920.0, 1040.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Top);
        assert_eq!(
            placement(icon(), work, MONITOR, WIDTH, HEIGHT),
            (1564.0, 52.0)
        );
    }

    #[test]
    fn left_taskbar_sits_next_to_the_icon() {
        let work = Bounds::new(48.0, 0.0, 1872.0, 1080.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Left);
        assert_eq!(
            placement(middle_icon(), work, MONITOR, WIDTH, HEIGHT),
            (60.0, 240.0)
        );
    }

    #[test]
    fn right_taskbar_sits_next_to_the_icon() {
        let work = Bounds::new(0.0, 0.0, 1872.0, 1080.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Right);
        assert_eq!(
            placement(middle_icon(), work, MONITOR, WIDTH, HEIGHT),
            (1516.0, 240.0)
        );
    }

    #[test]
    fn vertical_placement_stays_inside_the_work_area() {
        let work = Bounds::new(48.0, 0.0, 1872.0, 1080.0);
        let high = TrayRect::new(48.0, 4.0, 24.0, 24.0);
        assert_eq!(placement(high, work, MONITOR, WIDTH, HEIGHT).1, 12.0);
        let low = TrayRect::new(48.0, 1050.0, 24.0, 24.0);
        assert_eq!(placement(low, work, MONITOR, WIDTH, HEIGHT).1, 468.0);
    }
}
