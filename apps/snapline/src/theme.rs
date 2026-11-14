use std::mem::size_of;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};

pub fn is_light() -> bool {
    snapline_win::theme::is_light()
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
