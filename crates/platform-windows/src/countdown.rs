use std::mem::size_of;
use std::time::{Duration, Instant};

use thiserror::Error;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
    RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetMonitorInfoW, InvalidateRect, MonitorFromPoint, SelectObject, SetBkMode, SetTextColor,
    UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    UnregisterClassW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDC_ARROW, LWA_ALPHA, MSG,
    SW_SHOWNOACTIVATE, WM_DESTROY, WM_ERASEBKGND, WM_NCCREATE, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const WINDOW_CLASS: PCWSTR = w!("SnaplineCountdown");
const TIMER_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 100;
const LOGICAL_SIZE: f32 = 88.0;
const LOGICAL_MARGIN: f32 = 24.0;
const DEFAULT_DPI: u32 = 96;
const ALPHA: u8 = 235;

#[derive(Debug, Error)]
pub enum CountdownError {
    #[error("The countdown could not be shown.")]
    Show,
    #[error("The countdown could not be shown: {0}")]
    Win32(String),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Countdown;

impl Countdown {
    pub const fn new() -> Self {
        Self
    }

    pub fn run(&self, seconds: u8) -> Result<(), CountdownError> {
        if seconds == 0 {
            return Ok(());
        }
        let class = Class::register()?;
        let geometry = Geometry::around_cursor()?;
        // SAFETY: the class was registered above, the geometry is inside the work area and the instance outlives the window.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT,
                WINDOW_CLASS,
                PCWSTR::null(),
                WS_POPUP,
                geometry.x,
                geometry.y,
                geometry.size,
                geometry.size,
                None,
                None,
                Some(class.instance),
                None,
            )
        }
        .map_err(failed)?;
        let window = Window(hwnd);
        // SAFETY: the window is live and its layered attributes are set before it is shown.
        unsafe {
            let _ = SetLayeredWindowAttributes(window.0, COLORREF(0), ALPHA, LWA_ALPHA);
        }
        let mut state = Box::new(State {
            deadline: Instant::now() + Duration::from_secs(u64::from(seconds)),
            displayed: u64::from(seconds),
        });
        // SAFETY: the state is boxed for the whole message loop and only read by this window's procedure.
        unsafe {
            SetWindowLongPtrW(
                window.0,
                GWLP_USERDATA,
                std::ptr::from_mut(&mut *state) as isize,
            );
        }
        // SAFETY: the window and its state are live; showing without activation keeps input with the user's app.
        unsafe {
            let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(window.0);
            let _ = SetTimer(Some(window.0), TIMER_ID, TIMER_INTERVAL_MS, None);
        }
        pump_messages();
        // SAFETY: the message loop has ended, so no window procedure can observe the state after this point.
        unsafe {
            SetWindowLongPtrW(window.0, GWLP_USERDATA, 0);
        }
        drop(state);
        drop(window);
        drop(class);
        Ok(())
    }
}

fn failed(error: windows::core::Error) -> CountdownError {
    CountdownError::Win32(error.to_string())
}

fn pump_messages() {
    let mut message = MSG::default();
    loop {
        // SAFETY: the message pointer refers to a live MSG and no window filter is used.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 <= 0 {
            break;
        }
        // SAFETY: the message came from GetMessageW and is dispatched to its own window procedure.
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

struct Class {
    instance: HINSTANCE,
    registered: bool,
}

impl Class {
    fn register() -> Result<Self, CountdownError> {
        // SAFETY: no module name is requested, so this returns the handle of the running executable.
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(failed)?;
        let instance = HINSTANCE(module.0);
        // SAFETY: loading the standard arrow cursor has no preconditions.
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        // SAFETY: the class pointer refers to a live WNDCLASSW for the duration of the call.
        let registered = unsafe { RegisterClassW(&class) };
        if registered == 0 {
            // SAFETY: GetLastError reads the calling thread's last error with no arguments.
            let error = unsafe { GetLastError() };
            if error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(CountdownError::Show);
            }
            return Ok(Self {
                instance,
                registered: false,
            });
        }
        Ok(Self {
            instance,
            registered: true,
        })
    }
}

impl Drop for Class {
    fn drop(&mut self) {
        if !self.registered {
            return;
        }
        // SAFETY: the class was registered by this value and no window of it remains after the message loop.
        unsafe {
            let _ = UnregisterClassW(WINDOW_CLASS, Some(self.instance));
        }
    }
}

struct Window(HWND);

impl Drop for Window {
    fn drop(&mut self) {
        if self.0 .0.is_null() {
            return;
        }
        // SAFETY: the window handle was created by this value and is destroyed at most once here.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

struct State {
    deadline: Instant,
    displayed: u64,
}

impl State {
    fn remaining(&self) -> u64 {
        let left = self.deadline.saturating_duration_since(Instant::now());
        let millis = left.as_millis().saturating_add(999) / 1000;
        u64::try_from(millis).unwrap_or(0)
    }
}

struct Geometry {
    x: i32,
    y: i32,
    size: i32,
}

impl Geometry {
    fn around_cursor() -> Result<Self, CountdownError> {
        let mut point = POINT::default();
        // SAFETY: `point` is a live out-parameter for the cursor position.
        unsafe { GetCursorPos(&mut point) }.map_err(failed)?;
        // SAFETY: the point came from GetCursorPos and the nearest monitor always exists on a desktop.
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: u32::try_from(size_of::<MONITORINFO>()).map_err(|_| CountdownError::Show)?,
            ..Default::default()
        };
        // SAFETY: `info` is a live MONITORINFO with a valid size and the monitor handle came from MonitorFromPoint.
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return Err(CountdownError::Show);
        }
        let mut dpi_x = DEFAULT_DPI;
        let mut dpi_y = DEFAULT_DPI;
        // SAFETY: both out-pointers refer to live u32 values and the monitor handle came from MonitorFromPoint.
        let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        let scale = if dpi_x == 0 {
            1.0
        } else {
            dpi_x as f32 / DEFAULT_DPI as f32
        };
        let size = (LOGICAL_SIZE * scale).round() as i32;
        let margin = (LOGICAL_MARGIN * scale).round() as i32;
        Ok(Self {
            x: info.rcWork.right - size - margin,
            y: info.rcWork.bottom - size - margin,
            size,
        })
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => LRESULT(1),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            // SAFETY: the user data was set to a live state before the first paint message.
            let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
            // SAFETY: the pointer came from a boxed State that outlives the window's messages.
            if let Some(state) = unsafe { pointer.as_mut() } {
                paint(hwnd, state.displayed);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            // SAFETY: the user data was set to a live state before the timer started.
            let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
            // SAFETY: the pointer came from a boxed State that outlives the window's messages.
            if let Some(state) = unsafe { pointer.as_mut() } {
                let remaining = state.remaining();
                if remaining == 0 {
                    // SAFETY: the timer belongs to this live window and destroying it ends the message loop.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                } else if remaining != state.displayed {
                    state.displayed = remaining;
                    // SAFETY: the window is live and the rectangle is the whole client area.
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, true);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: the timer belongs to this window and the thread's message loop is ended once.
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => {
            // SAFETY: the default handler accepts any window message.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

fn paint(hwnd: HWND, remaining: u64) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: the window is live and the paint structure is a valid out-parameter.
    let dc: HDC = unsafe { BeginPaint(hwnd, &mut paint) };
    let mut rect = RECT::default();
    // SAFETY: the window is live and the rectangle is a valid out-parameter.
    let _ = unsafe { GetClientRect(hwnd, &mut rect) };
    // SAFETY: creating a solid brush has no preconditions; it is deleted below.
    let brush = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
    // SAFETY: the device context is live and the brush was created above.
    unsafe {
        FillRect(dc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }
    let height = rect.bottom - rect.top;
    // SAFETY: the font is created with a live height and deleted after it is no longer selected.
    let font = unsafe {
        CreateFontW(
            -(height * 2 / 3),
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            u32::from(DEFAULT_PITCH.0),
            w!("Segoe UI Variable Display"),
        )
    };
    // SAFETY: the device context is live and the font was created above.
    let previous = unsafe { SelectObject(dc, font.into()) };
    // SAFETY: the device context is live and the font is selected for the following text call.
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00ff_ffff));
    }
    let mut text: Vec<u16> = remaining.to_string().encode_utf16().collect();
    let mut area = rect;
    // SAFETY: the device context and the text buffer are live, and the area is the whole client rectangle.
    unsafe {
        DrawTextW(
            dc,
            &mut text,
            &mut area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(dc, previous);
        let _ = DeleteObject(font.into());
        let _ = EndPaint(hwnd, &paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "desktop"]
    fn a_one_second_countdown_shows_and_returns() {
        Countdown::new().run(1).expect("countdown");
    }

    #[test]
    fn zero_seconds_returns_without_a_window() {
        assert!(Countdown::new().run(0).is_ok());
    }

    #[test]
    #[ignore = "desktop"]
    fn a_delayed_freeze_captures_every_monitor() {
        use snapline_core::ports::ScreenCapture;

        let backend = crate::capture::CaptureBackend::default();
        Countdown::new().run(1).expect("countdown");
        let frozen = backend.freeze_all().expect("freeze");
        assert_eq!(frozen.monitors.len(), backend.monitors().len());
    }
}
