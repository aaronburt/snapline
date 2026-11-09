use std::ffi::c_void;
use std::mem::size_of;
use std::sync::mpsc;
use std::thread::JoinHandle;

use snapline_core::geometry::{DpiScale, Point, Px, Rect};
use snapline_core::overlay::SnapSet;
use snapline_core::ports::{FrozenDesktop, Monitor};
use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    ValidateRect, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IsWindowVisible,
    PostMessageW, PostQuitMessage, RegisterClassExW, SetWindowDisplayAffinity, ShowWindow,
    TranslateMessage, UnregisterClassW, UpdateLayeredWindow, HTTRANSPARENT, MA_NOACTIVATE, MSG,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WDA_EXCLUDEFROMCAPTURE, WINDOW_DISPLAY_AFFINITY, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::window::Context;
use crate::{MagnifierSettings, OverlayError, Theme};

const CLASS_NAME: &str = "SnaplineRecordBorder";

pub const BORDER_WIDTH: i32 = 2;
pub const CLICK_TOLERANCE: i32 = 2;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RecordBorderError {
    #[error("The recorded area is empty.")]
    Empty,
    #[error("The recording border thread could not be started.")]
    Thread,
    #[error("The recording border window could not be created: {0}")]
    Window(String),
    #[error("The recording border could not be excluded from capture: {0}")]
    Affinity(String),
    #[error("The recording border could not be drawn: {0}")]
    Paint(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RecordPickError {
    #[error("The recording area picker was cancelled.")]
    Cancelled,
    #[error("There is no monitor to draw the recording area picker on.")]
    NoMonitors,
    #[error("The recording area picker thread could not be started.")]
    Thread,
    #[error("The recording area picker failed: {0}")]
    Failed(String),
}

impl From<OverlayError> for RecordPickError {
    fn from(error: OverlayError) -> Self {
        match error {
            OverlayError::NoMonitors => Self::NoMonitors,
            OverlayError::Thread => Self::Thread,
            OverlayError::Cancelled => Self::Cancelled,
            OverlayError::Device(message) => Self::Failed(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordWindow {
    pub handle: isize,
    pub title: String,
}

impl RecordWindow {
    pub fn new(handle: isize, title: impl Into<String>) -> Self {
        Self {
            handle,
            title: title.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordTarget {
    pub rect: Rect,
    pub monitor: u32,
    pub window: Option<RecordWindow>,
}

impl RecordTarget {
    pub fn region(rect: Rect, monitor: u32) -> Self {
        Self {
            rect,
            monitor,
            window: None,
        }
    }

    pub fn window(rect: Rect, monitor: u32, window: RecordWindow) -> Self {
        Self {
            rect,
            monitor,
            window: Some(window),
        }
    }

    pub const fn is_window(&self) -> bool {
        self.window.is_some()
    }
}

pub fn resolve_record_target(
    rect: Rect,
    monitor: u32,
    window: Option<RecordWindow>,
) -> RecordTarget {
    match window {
        Some(window) => RecordTarget::window(rect, monitor, window),
        None => RecordTarget::region(rect, monitor),
    }
}

pub fn is_click(start: Point, end: Point) -> bool {
    (end.x.get() - start.x.get()).abs() <= CLICK_TOLERANCE
        && (end.y.get() - start.y.get()).abs() <= CLICK_TOLERANCE
}

#[derive(Debug, Clone)]
pub struct RecordPickRequest {
    pub desktop: FrozenDesktop,
    pub snap: SnapSet,
    pub magnifier: MagnifierSettings,
    pub theme: Theme,
}

impl RecordPickRequest {
    pub fn new(desktop: FrozenDesktop) -> Self {
        Self {
            desktop,
            snap: SnapSet::default(),
            magnifier: MagnifierSettings::default(),
            theme: Theme::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderGeometry {
    pub rect: Rect,
    pub thickness: i32,
}

pub fn border_width(_scale: f32) -> i32 {
    BORDER_WIDTH
}

pub fn border_rect(area: Rect) -> Rect {
    Rect::from_edges(
        area.left() - Px(BORDER_WIDTH),
        area.top() - Px(BORDER_WIDTH),
        area.right() + Px(BORDER_WIDTH),
        area.bottom() + Px(BORDER_WIDTH),
    )
}

pub fn border_geometry(area: Rect, bounds: Rect, dpi: DpiScale) -> BorderGeometry {
    BorderGeometry {
        rect: border_rect(area).clamp(bounds),
        thickness: border_width(dpi.scale()),
    }
}

pub fn border_colour(theme: Theme) -> [f32; 4] {
    theme.palette().red
}

pub fn window_ex_style() -> WINDOW_EX_STYLE {
    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
}

pub const fn window_style() -> WINDOW_STYLE {
    WS_POPUP
}

pub const WINDOW_AFFINITY: WINDOW_DISPLAY_AFFINITY = WDA_EXCLUDEFROMCAPTURE;

pub fn pick_target(request: RecordPickRequest) -> Result<RecordTarget, RecordPickError> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name(String::from("snapline-record-pick"))
        .spawn(move || {
            let result = run_picker(request);
            let _ = sender.send(result);
        })
        .map_err(|_| RecordPickError::Thread)?;
    receiver.recv().map_err(|_| RecordPickError::Thread)?
}

fn run_picker(request: RecordPickRequest) -> Result<RecordTarget, RecordPickError> {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated picker thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut context = Box::new(Context::build_record(request)?);
    context.activate()?;
    Context::run_message_loop();
    let target = context.record_target.take();
    context.destroy();
    target.ok_or(RecordPickError::Cancelled)
}

pub struct RecordBorder;

impl RecordBorder {
    pub fn show(
        area: Rect,
        monitor: Monitor,
        theme: Theme,
    ) -> Result<RecordBorderHandle, RecordBorderError> {
        if area.is_empty() {
            return Err(RecordBorderError::Empty);
        }
        let (sender, receiver) = mpsc::channel();
        let bounds = monitor.bounds;
        let dpi = monitor.dpi;
        let thread = std::thread::Builder::new()
            .name(String::from("snapline-record-border"))
            .spawn(move || run_border(area, bounds, dpi, theme, sender))
            .map_err(|_| RecordBorderError::Thread)?;
        match receiver.recv() {
            Ok(Ok(handle)) => Ok(RecordBorderHandle {
                hwnd: handle,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(RecordBorderError::Thread)
            }
        }
    }
}

pub struct RecordBorderHandle {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
}

impl RecordBorderHandle {
    pub const fn hwnd(&self) -> isize {
        self.hwnd
    }

    pub fn is_visible(&self) -> bool {
        // SAFETY: reads the state of a window created by this process; a destroyed handle returns false.
        unsafe { IsWindowVisible(HWND(self.hwnd as *mut c_void)).as_bool() }
    }

    pub fn close(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        // SAFETY: posts a close request to the window owned by the border thread.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        let _ = thread.join();
    }
}

impl Drop for RecordBorderHandle {
    fn drop(&mut self) {
        self.close();
    }
}

impl crate::OverlayHostImpl {
    pub fn pick_record_target(
        &self,
        request: RecordPickRequest,
    ) -> Result<RecordTarget, RecordPickError> {
        pick_target(request)
    }
}

fn run_border(
    area: Rect,
    bounds: Rect,
    dpi: DpiScale,
    theme: Theme,
    sender: mpsc::Sender<Result<isize, RecordBorderError>>,
) {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated border thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    match create_border(area, bounds, dpi, theme) {
        Ok((hwnd, class_name)) => {
            let _ = sender.send(Ok(hwnd.0 as isize));
            run_message_loop();
            // SAFETY: the window belongs to this thread and the loop has ended.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            // SAFETY: the class was registered on this thread for this window.
            unsafe {
                let _ = UnregisterClassW(PCWSTR::from_raw(class_name.as_ptr()), None);
            }
        }
        Err(error) => {
            let _ = sender.send(Err(error));
        }
    }
}

fn create_border(
    area: Rect,
    bounds: Rect,
    dpi: DpiScale,
    theme: Theme,
) -> Result<(HWND, Vec<u16>), RecordBorderError> {
    // SAFETY: returns the module handle of this process without borrowing any memory.
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .map_err(|error| RecordBorderError::Window(error.to_string()))?;
    let class_name: Vec<u16> = CLASS_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let class = WNDCLASSEXW {
        cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(0),
        style: Default::default(),
        lpfnWndProc: Some(border_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: HINSTANCE(instance.0),
        hIcon: Default::default(),
        hCursor: Default::default(),
        hbrBackground: Default::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
        hIconSm: Default::default(),
    };
    // SAFETY: the class description and its string buffer outlive the call.
    let _ = unsafe { RegisterClassExW(&class) };
    let geometry = border_geometry(area, bounds, dpi);
    let width = geometry.rect.width.get().max(1);
    let height = geometry.rect.height.get().max(1);
    // SAFETY: the class name buffer, module handle and physical coordinates are valid.
    let hwnd = unsafe {
        CreateWindowExW(
            window_ex_style(),
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::null(),
            window_style(),
            geometry.rect.x.get(),
            geometry.rect.y.get(),
            width,
            height,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
    }
    .map_err(|error| RecordBorderError::Window(error.to_string()))?;
    let prepared = (|| -> Result<(), RecordBorderError> {
        // SAFETY: the window handle is live and owned by this thread.
        unsafe { SetWindowDisplayAffinity(hwnd, WINDOW_AFFINITY) }
            .map_err(|error| RecordBorderError::Affinity(error.to_string()))?;
        paint_border(
            hwnd,
            width,
            height,
            geometry.thickness,
            border_colour(theme),
        )?;
        // SAFETY: the window handle is live and owned by this thread.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        // SAFETY: the window handle is live and owned by this thread.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    Ok((hwnd, class_name))
}

fn paint_border(
    hwnd: HWND,
    width: i32,
    height: i32,
    thickness: i32,
    colour: [f32; 4],
) -> Result<(), RecordBorderError> {
    let header = BITMAPINFOHEADER {
        biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        bmiColors: [Default::default()],
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: returns the screen device context without transferring ownership.
    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        return Err(RecordBorderError::Paint(
            "No screen device context.".to_string(),
        ));
    }
    // SAFETY: the screen device context is live.
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };
    if memory.is_invalid() {
        // SAFETY: releases the device context obtained above.
        unsafe {
            let _ = ReleaseDC(None, screen);
        }
        return Err(RecordBorderError::Paint(
            "No memory device context.".to_string(),
        ));
    }
    // SAFETY: the info structure is live and the bits pointer is a valid out-parameter.
    let bitmap =
        unsafe { CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0) };
    let bitmap = match bitmap {
        Ok(bitmap) => bitmap,
        Err(error) => {
            // SAFETY: releases the device contexts obtained above.
            unsafe {
                let _ = DeleteDC(memory);
                let _ = ReleaseDC(None, screen);
            }
            return Err(RecordBorderError::Paint(error.to_string()));
        }
    };
    // SAFETY: the memory device context and bitmap are live.
    let previous = unsafe { SelectObject(memory, HGDIOBJ::from(bitmap)) };
    if bits.is_null() {
        // SAFETY: restores the device context and releases every handle created above.
        unsafe {
            let _ = SelectObject(memory, previous);
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(None, screen);
        }
        return Err(RecordBorderError::Paint(
            "The border bitmap has no pixels.".to_string(),
        ));
    }
    let pixel = pixel(colour);
    let length = usize::try_from(width.max(1))
        .unwrap_or(1)
        .saturating_mul(usize::try_from(height.max(1)).unwrap_or(1))
        .saturating_mul(4);
    // SAFETY: the DIB section allocated exactly four bytes per pixel for this size.
    let buffer = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), length) };
    write_frame(buffer, width, height, thickness, pixel);
    let target = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    // SAFETY: every handle and structure passed into the layered update is live for the call.
    let updated = unsafe {
        UpdateLayeredWindow(
            hwnd,
            Some(screen),
            None,
            Some(&size),
            Some(memory),
            Some(&target),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    };
    // SAFETY: restores the device context and releases every handle created above.
    unsafe {
        let _ = SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
    }
    updated.map_err(|error| RecordBorderError::Paint(error.to_string()))
}

fn pixel(colour: [f32; 4]) -> [u8; 4] {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let alpha = colour[3].clamp(0.0, 1.0);
    [
        channel(colour[2] * alpha),
        channel(colour[1] * alpha),
        channel(colour[0] * alpha),
        channel(alpha),
    ]
}

fn write_frame(buffer: &mut [u8], width: i32, height: i32, thickness: i32, pixel: [u8; 4]) {
    let columns = usize::try_from(width.max(0)).unwrap_or(0);
    let rows = usize::try_from(height.max(0)).unwrap_or(0);
    let edge = usize::try_from(thickness.max(0))
        .unwrap_or(0)
        .min(columns)
        .min(rows);
    if columns == 0 || rows == 0 || edge == 0 {
        return;
    }
    for row in 0..rows {
        let horizontal = row < edge || row >= rows - edge;
        let start = row * columns * 4;
        if horizontal {
            for index in 0..columns {
                let offset = start + index * 4;
                if offset + 4 <= buffer.len() {
                    buffer[offset..offset + 4].copy_from_slice(&pixel);
                }
            }
            continue;
        }
        for column in 0..columns {
            if column < edge || column >= columns - edge {
                let offset = start + column * 4;
                if offset + 4 <= buffer.len() {
                    buffer[offset..offset + 4].copy_from_slice(&pixel);
                }
            }
        }
    }
}

fn run_message_loop() {
    let mut message = MSG::default();
    loop {
        // SAFETY: the message buffer is valid and the window filter is empty.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        // SAFETY: standard message dispatch for windows owned by this thread.
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn border_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            // SAFETY: validates the paint region for the live window.
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            return LRESULT(0);
        }
        WM_CLOSE => {
            // SAFETY: destroys the window owned by this thread.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            // SAFETY: posts the quit message for this thread's message loop.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: unhandled messages use the default window procedure.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Self {
        // SAFETY: initialises COM for the border thread with a single threaded apartment.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self {
            initialized: result.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    #[test]
    fn border_rect_expands_by_the_frame_width_on_every_side() {
        let area = rect(100, 200, 400, 300);
        assert_eq!(border_rect(area), rect(98, 198, 404, 304));
    }

    #[test]
    fn border_rect_keeps_negative_monitor_origins() {
        let area = rect(-1000, 100, 400, 300);
        assert_eq!(border_rect(area), rect(-1002, 98, 404, 304));
        assert!(border_rect(area).contains(point(-1000, 100)));
    }

    #[test]
    fn border_geometry_clamps_to_the_monitor_bounds() {
        let bounds = rect(0, 0, 1920, 1080);
        let geometry = border_geometry(rect(1900, 100, 20, 100), bounds, DpiScale::ONE);
        assert_eq!(geometry.rect.right(), Px(1920));
        assert_eq!(geometry.rect.top(), Px(98));
        assert_eq!(geometry.rect.bottom(), Px(202));
    }

    #[test]
    fn border_geometry_clamps_a_negative_origin_monitor() {
        let bounds = rect(-1920, -200, 1920, 1080);
        let geometry = border_geometry(rect(-1920, -200, 200, 100), bounds, DpiScale::ONE);
        assert_eq!(geometry.rect.left(), Px(-1920));
        assert_eq!(geometry.rect.top(), Px(-200));
    }

    #[test]
    fn the_frame_stays_two_pixels_at_every_dpi() {
        for percent in [100, 125, 150, 200] {
            let dpi = DpiScale::from_percent(percent);
            assert_eq!(border_width(dpi.scale()), 2);
            let geometry = border_geometry(rect(10, 20, 300, 200), rect(0, 0, 1920, 1080), dpi);
            assert_eq!(geometry.thickness, 2);
            assert_eq!(geometry.rect.width, Px(304));
            assert_eq!(geometry.rect.height, Px(204));
        }
    }

    #[test]
    fn the_frame_uses_the_annotation_red_token() {
        assert_eq!(pixel(border_colour(Theme::Dark)), [0x4d, 0x48, 0xe5, 0xff]);
        assert_eq!(pixel(border_colour(Theme::Light)), [0x4d, 0x48, 0xe5, 0xff]);
    }

    #[test]
    fn the_frame_is_written_on_the_window_edges_only() {
        let width = 10_i32;
        let height = 8_i32;
        let mut buffer = vec![0_u8; (width * height * 4) as usize];
        write_frame(&mut buffer, width, height, 2, [1, 2, 3, 4]);
        assert_eq!(&buffer[0..4], &[1, 2, 3, 4]);
        assert_eq!(
            &buffer[(width as usize * 4 - 4)..(width as usize * 4)],
            &[1, 2, 3, 4]
        );
        let middle = (4 * width as usize + 5) * 4;
        assert_eq!(&buffer[middle..middle + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn the_frame_thickness_cannot_exceed_the_window() {
        let width = 4_i32;
        let height = 4_i32;
        let mut buffer = vec![0_u8; (width * height * 4) as usize];
        write_frame(&mut buffer, width, height, 40, [9, 9, 9, 9]);
        assert!(buffer.chunks_exact(4).all(|pixel| pixel == [9, 9, 9, 9]));
    }

    #[test]
    fn the_border_window_is_click_through_layered_and_top_most() {
        let style = window_ex_style();
        assert_ne!(style.0 & WS_EX_TRANSPARENT.0, 0);
        assert_ne!(style.0 & WS_EX_LAYERED.0, 0);
        assert_ne!(style.0 & WS_EX_TOPMOST.0, 0);
        assert_ne!(style.0 & WS_EX_TOOLWINDOW.0, 0);
        assert_ne!(style.0 & WS_EX_NOACTIVATE.0, 0);
        assert_eq!(window_style(), WS_POPUP);
        assert_eq!(WINDOW_AFFINITY, WDA_EXCLUDEFROMCAPTURE);
    }

    #[test]
    fn an_empty_area_is_rejected() {
        let monitor = Monitor {
            id: 1,
            name: String::from("Test"),
            bounds: rect(0, 0, 1920, 1080),
            work_area: rect(0, 0, 1920, 1040),
            dpi: DpiScale::ONE,
            primary: true,
        };
        assert_eq!(
            RecordBorder::show(Rect::ZERO, monitor, Theme::Dark).err(),
            Some(RecordBorderError::Empty)
        );
    }

    #[test]
    fn a_small_pointer_movement_is_still_a_click() {
        assert!(is_click(point(10, 10), point(12, 8)));
        assert!(is_click(point(10, 10), point(10, 10)));
        assert!(!is_click(point(10, 10), point(13, 10)));
        assert!(!is_click(point(10, 10), point(10, 13)));
    }

    #[test]
    fn a_region_target_has_no_window() {
        let target = resolve_record_target(rect(10, 20, 300, 200), 4, None);
        assert_eq!(target.rect, rect(10, 20, 300, 200));
        assert_eq!(target.monitor, 4);
        assert!(!target.is_window());
        assert!(target.window.is_none());
    }

    #[test]
    fn a_window_target_carries_the_handle_and_title() {
        let window = RecordWindow::new(0x1234, "Editor");
        let target = resolve_record_target(rect(0, 0, 800, 600), 2, Some(window.clone()));
        assert_eq!(target.monitor, 2);
        assert!(target.is_window());
        assert_eq!(target.window, Some(window));
        assert_eq!(
            target.window.as_ref().map(|window| window.handle),
            Some(0x1234)
        );
        assert_eq!(
            target.window.as_ref().map(|window| window.title.clone()),
            Some(String::from("Editor"))
        );
    }

    #[test]
    fn cancelling_the_picker_reports_cancelled() {
        assert_eq!(
            RecordPickError::from(OverlayError::Cancelled),
            RecordPickError::Cancelled
        );
        assert_eq!(
            RecordPickError::from(OverlayError::NoMonitors),
            RecordPickError::NoMonitors
        );
    }

    #[test]
    fn a_desktop_with_no_monitors_is_rejected_without_a_window() {
        let request = RecordPickRequest::new(FrozenDesktop {
            monitors: Vec::new(),
        });
        assert_eq!(
            pick_target(request).err(),
            Some(RecordPickError::NoMonitors)
        );
    }

    #[test]
    #[ignore = "desktop"]
    fn showing_and_closing_the_border_removes_the_window() {
        let monitor = Monitor {
            id: 1,
            name: String::from("Test"),
            bounds: rect(0, 0, 1920, 1080),
            work_area: rect(0, 0, 1920, 1040),
            dpi: DpiScale::ONE,
            primary: true,
        };
        let Ok(mut handle) = RecordBorder::show(rect(100, 100, 400, 300), monitor, Theme::Dark)
        else {
            return;
        };
        assert!(handle.is_visible());
        handle.close();
        assert!(!handle.is_visible());
    }
}
