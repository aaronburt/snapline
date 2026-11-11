use std::ffi::c_void;
use std::mem::size_of;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::ports::Monitor;
use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW,
    Ellipse, EndPaint, FillRect, InvalidateRect, RoundRect, SelectObject, SetBkMode, SetTextColor,
    SetWindowRgn, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_CENTER, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_SEMIBOLD, HGDIOBJ, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetTimer, SetWindowDisplayAffinity, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    UnregisterClassW, CREATESTRUCTW, GWLP_USERDATA, IDC_ARROW, MA_NOACTIVATE, MSG,
    SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WINDOW_DISPLAY_AFFINITY, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_NCCREATE,
    WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::Theme;

const CLASS_NAME: &str = "SnaplineRecordControls";
const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 250;

pub const WINDOW_AFFINITY: WINDOW_DISPLAY_AFFINITY = WDA_EXCLUDEFROMCAPTURE;

const BAR_HEIGHT: f32 = 40.0;
const BAR_PADDING: f32 = 8.0;
const TIMER_WIDTH: f32 = 84.0;
const BUTTON_WIDTH: f32 = 84.0;
const BUTTON_HEIGHT: f32 = 28.0;
const BAR_GAP: f32 = 10.0;
const ITEM_GAP: f32 = 8.0;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RecordControlsError {
    #[error("The recorded area is empty.")]
    Empty,
    #[error("The recording controls thread could not be started.")]
    Thread,
    #[error("The recording controls window could not be created: {0}")]
    Window(String),
    #[error("The recording controls could not be excluded from capture: {0}")]
    Affinity(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlHit {
    Stop,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlLayout {
    pub size: (i32, i32),
    pub timer: Rect,
    pub pause: Option<Rect>,
    pub cancel: Rect,
    pub stop: Rect,
}

fn scaled(value: f32, scale: f32) -> i32 {
    (value * scale).round() as i32
}

fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
    Rect::new(Px(x), Px(y), Px(width), Px(height))
}

pub fn control_layout(scale: f32) -> ControlLayout {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let height = scaled(BAR_HEIGHT, scale);
    let padding = scaled(BAR_PADDING, scale);
    let gap = scaled(ITEM_GAP, scale);
    let timer_width = scaled(TIMER_WIDTH, scale);
    let button_width = scaled(BUTTON_WIDTH, scale);
    let button_height = scaled(BUTTON_HEIGHT, scale);
    let button_y = (height - button_height) / 2;
    let timer = rect(padding, 0, timer_width, height);
    let cancel = rect(
        padding + timer_width + gap,
        button_y,
        button_width,
        button_height,
    );
    let stop = rect(
        cancel.right().get() + gap,
        button_y,
        button_width,
        button_height,
    );
    ControlLayout {
        size: (stop.right().get() + padding, height),
        timer,
        pause: None,
        cancel,
        stop,
    }
}

pub fn control_hit(layout: &ControlLayout, point: Point) -> Option<ControlHit> {
    if layout.stop.contains(point) {
        Some(ControlHit::Stop)
    } else if layout.cancel.contains(point) {
        Some(ControlHit::Cancel)
    } else {
        None
    }
}

pub fn format_elapsed(milliseconds: u64) -> String {
    let total = milliseconds / 1000;
    let seconds = total % 60;
    let minutes = (total / 60) % 60;
    let hours = total / 3600;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn clamp_axis(start: i32, length: i32, low: i32, high: i32) -> i32 {
    let upper = (high - length).max(low);
    start.clamp(low, upper)
}

pub fn place_bar(region: Rect, size: (i32, i32), bounds: Rect, gap: i32) -> Rect {
    let (width, height) = size;
    let centred = region.x.get() + (region.width.get() - width) / 2;
    let x = clamp_axis(centred, width, bounds.left().get(), bounds.right().get());
    let below = region.bottom().get() + gap;
    let above = region.top().get() - gap - height;
    let y = if below + height <= bounds.bottom().get() {
        below
    } else if above >= bounds.top().get() {
        above
    } else {
        clamp_axis(
            region.bottom().get() - gap - height,
            height,
            bounds.top().get(),
            bounds.bottom().get(),
        )
    };
    rect(x, y, width, height)
}

pub fn bar_gap(scale: f32) -> i32 {
    scaled(BAR_GAP, scale)
}

pub const fn window_ex_style() -> WINDOW_EX_STYLE {
    WINDOW_EX_STYLE(WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0)
}

pub const fn window_style() -> WINDOW_STYLE {
    WS_POPUP
}

pub type StopCallback = Arc<dyn Fn() + Send + Sync>;

struct BarState {
    layout: ControlLayout,
    started: Instant,
    theme: Theme,
    scale: f32,
    stop: StopCallback,
    cancel: StopCallback,
}

pub struct RecordControls;

impl RecordControls {
    pub fn show(
        area: Rect,
        monitor: Monitor,
        theme: Theme,
        stop: StopCallback,
        cancel: StopCallback,
    ) -> Result<RecordControlsHandle, RecordControlsError> {
        if area.is_empty() {
            return Err(RecordControlsError::Empty);
        }
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(String::from("snapline-record-controls"))
            .spawn(move || run_controls(area, monitor, theme, stop, cancel, sender))
            .map_err(|_| RecordControlsError::Thread)?;
        match receiver.recv() {
            Ok(Ok(handle)) => Ok(RecordControlsHandle {
                hwnd: handle,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(RecordControlsError::Thread)
            }
        }
    }
}

pub struct RecordControlsHandle {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
}

impl RecordControlsHandle {
    pub const fn hwnd(&self) -> isize {
        self.hwnd
    }

    pub fn close(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        // SAFETY: posts a close request to the window owned by the controls thread.
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

impl Drop for RecordControlsHandle {
    fn drop(&mut self) {
        self.close();
    }
}

fn run_controls(
    area: Rect,
    monitor: Monitor,
    theme: Theme,
    stop: StopCallback,
    cancel: StopCallback,
    sender: mpsc::Sender<Result<isize, RecordControlsError>>,
) {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated controls thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let scale = monitor.dpi.scale();
    let layout = control_layout(scale);
    let placed = place_bar(area, layout.size, monitor.bounds, bar_gap(scale));
    let state = Box::new(BarState {
        layout,
        started: Instant::now(),
        theme,
        scale,
        stop,
        cancel,
    });
    let state_pointer = Box::into_raw(state);
    match create_controls(placed, state_pointer) {
        Ok((hwnd, class_name)) => {
            let _ = sender.send(Ok(hwnd.0 as isize));
            run_message_loop();
            // SAFETY: the timer and window belong to this thread and the loop has ended.
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
                let _ = DestroyWindow(hwnd);
                let _ = UnregisterClassW(PCWSTR::from_raw(class_name.as_ptr()), None);
            }
        }
        Err(error) => {
            let _ = sender.send(Err(error));
        }
    }
    // SAFETY: the pointer came from Box::into_raw above and the window no longer uses it.
    drop(unsafe { Box::from_raw(state_pointer) });
}

fn create_controls(
    placed: Rect,
    state: *mut BarState,
) -> Result<(HWND, Vec<u16>), RecordControlsError> {
    // SAFETY: returns the module handle of this process without borrowing any memory.
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .map_err(|error| RecordControlsError::Window(error.to_string()))?;
    let class_name: Vec<u16> = CLASS_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: loads a shared system cursor that needs no release.
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
    let class = WNDCLASSEXW {
        cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(0),
        style: Default::default(),
        lpfnWndProc: Some(controls_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: HINSTANCE(instance.0),
        hIcon: Default::default(),
        hCursor: cursor,
        hbrBackground: Default::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
        hIconSm: Default::default(),
    };
    // SAFETY: the class description and its string buffer outlive the call.
    let _ = unsafe { RegisterClassExW(&class) };
    let width = placed.width.get().max(1);
    let height = placed.height.get().max(1);
    // SAFETY: the class name, module handle and state pointer are valid for the window's life.
    let hwnd = unsafe {
        CreateWindowExW(
            window_ex_style(),
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::null(),
            window_style(),
            placed.x.get(),
            placed.y.get(),
            width,
            height,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            Some(state.cast::<c_void>().cast_const()),
        )
    }
    .map_err(|error| RecordControlsError::Window(error.to_string()))?;
    // SAFETY: the window handle is live and owned by this thread.
    if let Err(error) = unsafe { SetWindowDisplayAffinity(hwnd, WINDOW_AFFINITY) } {
        // SAFETY: the window handle is live and owned by this thread.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(RecordControlsError::Affinity(error.to_string()));
    }
    // SAFETY: the window handle is live; the region is owned by the window after the call.
    unsafe {
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, height, height);
        let _ = SetWindowRgn(hwnd, Some(region), true);
        let _ = SetTimer(Some(hwnd), TIMER_ID, TIMER_MS, None);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok((hwnd, class_name))
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

fn colour(value: [f32; 4]) -> COLORREF {
    let channel = |component: f32| (component.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(channel(value[0]) | (channel(value[1]) << 8) | (channel(value[2]) << 16))
}

fn to_win_rect(rect: Rect) -> RECT {
    RECT {
        left: rect.left().get(),
        top: rect.top().get(),
        right: rect.right().get(),
        bottom: rect.bottom().get(),
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn paint(hwnd: HWND, state: &BarState) {
    let palette = state.theme.palette();
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: begins painting the live window with a valid paint structure.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if hdc.is_invalid() {
        return;
    }
    let (width, height) = state.layout.size;
    let background = to_win_rect(rect(0, 0, width, height));
    // SAFETY: every GDI handle created here is released before the function returns.
    unsafe {
        let brush = CreateSolidBrush(colour(palette.card));
        FillRect(hdc, &background, brush);
        let _ = DeleteObject(HGDIOBJ::from(brush));

        let font = CreateFontW(
            -scaled(15.0, state.scale),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            FF_DONTCARE.0 as u32,
            PCWSTR::from_raw(wide("Segoe UI\0").as_ptr()),
        );
        let previous_font = SelectObject(hdc, HGDIOBJ::from(font));
        SetBkMode(hdc, TRANSPARENT);

        let dot = scaled(10.0, state.scale);
        let timer = state.layout.timer;
        let dot_top = (height - dot) / 2;
        let red = CreateSolidBrush(colour(palette.red));
        let previous_brush = SelectObject(hdc, HGDIOBJ::from(red));
        let _ = Ellipse(
            hdc,
            timer.left().get(),
            dot_top,
            timer.left().get() + dot,
            dot_top + dot,
        );

        SetTextColor(hdc, colour(palette.text));
        let mut label = wide(&format_elapsed(
            u64::try_from(state.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        ));
        let mut label_rect = to_win_rect(rect(
            timer.left().get() + dot + scaled(8.0, state.scale),
            0,
            timer.width.get() - dot - scaled(8.0, state.scale),
            height,
        ));
        DrawTextW(
            hdc,
            &mut label,
            &mut label_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        let cancel = state.layout.cancel;
        let neutral = CreateSolidBrush(colour(palette.card_alt));
        SelectObject(hdc, HGDIOBJ::from(neutral));
        let cancel_radius = scaled(8.0, state.scale);
        let _ = RoundRect(
            hdc,
            cancel.left().get(),
            cancel.top().get(),
            cancel.right().get(),
            cancel.bottom().get(),
            cancel_radius,
            cancel_radius,
        );
        SetTextColor(hdc, colour(palette.text));
        let mut cancel_label = wide("Cancel");
        let mut cancel_rect = to_win_rect(cancel);
        DrawTextW(
            hdc,
            &mut cancel_label,
            &mut cancel_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        let _ = DeleteObject(HGDIOBJ::from(neutral));

        let stop = state.layout.stop;
        let accent = CreateSolidBrush(colour(palette.red));
        SelectObject(hdc, HGDIOBJ::from(accent));
        SelectObject(hdc, HGDIOBJ::from(accent));
        let radius = scaled(8.0, state.scale);
        let _ = RoundRect(
            hdc,
            stop.left().get(),
            stop.top().get(),
            stop.right().get(),
            stop.bottom().get(),
            radius,
            radius,
        );
        SetTextColor(hdc, colour(palette.white));
        let mut stop_label = wide("Stop");
        let mut stop_rect = to_win_rect(stop);
        DrawTextW(
            hdc,
            &mut stop_label,
            &mut stop_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        SelectObject(hdc, previous_brush);
        SelectObject(hdc, previous_font);
        let _ = DeleteObject(HGDIOBJ::from(accent));
        let _ = DeleteObject(HGDIOBJ::from(red));
        let _ = DeleteObject(HGDIOBJ::from(font));
        let _ = EndPaint(hwnd, &paint);
    }
}

fn state_of<'a>(hwnd: HWND) -> Option<&'a BarState> {
    // SAFETY: the user data holds the state pointer set during creation, valid until the thread ends.
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const BarState;
    // SAFETY: a non-null pointer refers to the state owned by the controls thread.
    unsafe { pointer.as_ref() }
}

fn point_from(lparam: LPARAM) -> Point {
    let x = i32::from((lparam.0 & 0xffff) as u16 as i16);
    let y = i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16);
    Point::new(Px(x), Px(y))
}

unsafe extern "system" fn controls_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // SAFETY: lparam points at the creation structure for this message.
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            // SAFETY: stores the state pointer passed at creation in the window's user data.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            if let Some(state) = state_of(hwnd) {
                paint(hwnd, state);
            }
            return LRESULT(0);
        }
        WM_TIMER => {
            // SAFETY: invalidates the live window so the timer text redraws.
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            if let Some(state) = state_of(hwnd) {
                match control_hit(&state.layout, point_from(lparam)) {
                    Some(ControlHit::Stop) => (state.stop)(),
                    Some(ControlHit::Cancel) => (state.cancel)(),
                    None => {}
                }
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
        // SAFETY: initialises COM for the controls thread with a single threaded apartment.
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

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    const SIZE: (i32, i32) = (200, 40);

    #[test]
    fn elapsed_time_is_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(999), "0:00");
        assert_eq!(format_elapsed(7_400), "0:07");
        assert_eq!(format_elapsed(65_000), "1:05");
        assert_eq!(format_elapsed(59 * 60_000 + 59_000), "59:59");
    }

    #[test]
    fn elapsed_time_adds_hours_after_sixty_minutes() {
        assert_eq!(format_elapsed(3_600_000), "1:00:00");
        assert_eq!(format_elapsed(3_723_000), "1:02:03");
    }

    #[test]
    fn the_bar_sits_below_the_region_and_centred_when_there_is_room() {
        let bounds = rect(0, 0, 1920, 1080);
        let placed = place_bar(rect(400, 200, 600, 300), SIZE, bounds, 10);
        assert_eq!(placed, rect(600, 510, 200, 40));
    }

    #[test]
    fn the_bar_moves_above_when_the_region_touches_the_bottom() {
        let bounds = rect(0, 0, 1920, 1080);
        let placed = place_bar(rect(400, 700, 600, 380), SIZE, bounds, 10);
        assert_eq!(placed.y, Px(650));
        assert!(placed.bottom() <= Px(700));
    }

    #[test]
    fn the_bar_goes_inside_the_region_when_neither_side_has_room() {
        let bounds = rect(0, 0, 1920, 1080);
        let placed = place_bar(rect(0, 0, 1920, 1080), SIZE, bounds, 10);
        assert_eq!(placed.y, Px(1030));
        assert_eq!(placed.x, Px(860));
        assert!(placed.bottom() <= bounds.bottom());
    }

    #[test]
    fn the_bar_is_clamped_to_the_horizontal_monitor_edges() {
        let bounds = rect(0, 0, 1920, 1080);
        let left = place_bar(rect(0, 100, 40, 40), SIZE, bounds, 10);
        assert_eq!(left.x, Px(0));
        let right = place_bar(rect(1900, 100, 20, 40), SIZE, bounds, 10);
        assert_eq!(right.right(), Px(1920));
    }

    #[test]
    fn the_bar_follows_a_negative_origin_monitor() {
        let bounds = rect(-1920, -200, 1920, 1080);
        let placed = place_bar(rect(-1900, -180, 100, 100), SIZE, bounds, 10);
        assert_eq!(placed.x, Px(-1920));
        assert_eq!(placed.y, Px(-70));
        assert!(bounds.contains(point(placed.x.get(), placed.y.get())));
    }

    #[test]
    fn the_bar_never_leaves_a_monitor_smaller_than_itself() {
        let bounds = rect(0, 0, 100, 30);
        let placed = place_bar(rect(10, 5, 50, 20), SIZE, bounds, 10);
        assert_eq!(placed.x, Px(0));
        assert_eq!(placed.y, Px(0));
    }

    #[test]
    fn the_layout_scales_with_the_display() {
        let normal = control_layout(1.0);
        let large = control_layout(2.0);
        assert_eq!(large.size.0, normal.size.0 * 2);
        assert_eq!(large.size.1, normal.size.1 * 2);
        assert_eq!(large.stop.width.get(), normal.stop.width.get() * 2);
    }

    #[test]
    fn a_broken_scale_falls_back_to_one() {
        assert_eq!(control_layout(0.0), control_layout(1.0));
        assert_eq!(control_layout(f32::NAN), control_layout(1.0));
    }

    #[test]
    fn the_stop_button_fits_inside_the_bar() {
        let layout = control_layout(1.5);
        assert!(layout.stop.right().get() < layout.size.0);
        assert!(layout.stop.bottom().get() <= layout.size.1);
        assert!(layout.timer.right() <= layout.cancel.left());
        assert!(layout.cancel.right() <= layout.stop.left());
        assert!(layout.cancel.bottom().get() <= layout.size.1);
    }

    #[test]
    fn the_cancel_button_is_hit_separately_from_stop() {
        let layout = control_layout(1.0);
        assert_eq!(
            control_hit(&layout, layout.cancel.center()),
            Some(ControlHit::Cancel)
        );
        assert_ne!(
            control_hit(&layout, layout.cancel.center()),
            control_hit(&layout, layout.stop.center())
        );
    }

    #[test]
    fn only_the_stop_button_is_hit() {
        let layout = control_layout(1.0);
        let inside = layout.stop.center();
        assert_eq!(control_hit(&layout, inside), Some(ControlHit::Stop));
        assert_eq!(control_hit(&layout, point(2, 2)), None);
        assert_eq!(control_hit(&layout, layout.timer.center()), None);
    }

    #[test]
    fn pause_is_reserved_but_not_built() {
        assert_eq!(control_layout(1.0).pause, None);
    }

    #[test]
    fn the_bar_is_hidden_from_capture_and_never_takes_focus() {
        assert_eq!(WINDOW_AFFINITY, WDA_EXCLUDEFROMCAPTURE);
        let style = window_ex_style();
        assert_eq!(style.0 & WS_EX_NOACTIVATE.0, WS_EX_NOACTIVATE.0);
        assert_eq!(style.0 & WS_EX_TOPMOST.0, WS_EX_TOPMOST.0);
        assert_eq!(style.0 & WS_EX_TOOLWINDOW.0, WS_EX_TOOLWINDOW.0);
    }
}
