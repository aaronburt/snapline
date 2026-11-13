use std::mem::size_of;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
};

pub fn is_light() -> bool {
    snapline_win::theme::is_light()
}

pub fn is_windows_11() -> bool {
    snapline_win::theme::is_windows_11()
}

pub fn apply_hwnd_dark_mode(hwnd: HWND, light: bool) {
    let dark = i32::from(u8::from(!light));
    // SAFETY: hwnd is a live top-level native window handle and the pointer reads a live i32.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&dark).cast(),
            u32::try_from(size_of::<i32>()).unwrap_or(4),
        )
    };
}

pub fn apply_rounded_corners(hwnd: HWND) {
    if !is_windows_11() {
        return;
    }
    let preference = DWMWCP_ROUND;
    // SAFETY: hwnd is a live native window handle and the pointer reads a live preference value.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&preference).cast(),
            u32::try_from(size_of::<DWM_WINDOW_CORNER_PREFERENCE>()).unwrap_or(4),
        )
    };
}

pub fn bring_to_front(hwnd: HWND) {
    // SAFETY: the foreground window handle is only read, and the thread ids come from the system.
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let this_thread = GetCurrentThreadId();
        let attached = foreground_thread != 0
            && foreground_thread != this_thread
            && AttachThreadInput(foreground_thread, this_thread, true).as_bool();
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(foreground_thread, this_thread, false);
        }
    }
}
