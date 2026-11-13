use std::mem::size_of;

use tauri::window::Color;
use tauri::{AppHandle, Manager, WebviewWindow};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};

const MAIN_LABEL: &str = "main";

pub fn is_light() -> bool {
    snapline_win::theme::is_light()
}

pub fn is_windows_11() -> bool {
    snapline_win::theme::is_windows_11()
}

pub fn has_mica() -> bool {
    is_windows_11()
}

pub const MICA_DARK: Color = Color(28, 28, 28, 255);
pub const MICA_LIGHT: Color = Color(238, 238, 238, 255);
pub const LAYER_DARK: Color = Color(39, 39, 39, 255);
pub const LAYER_LIGHT: Color = Color(249, 249, 249, 255);

pub fn mica_background(light: bool) -> Color {
    if light {
        MICA_LIGHT
    } else {
        MICA_DARK
    }
}

pub fn layer_background(light: bool) -> Color {
    if light {
        LAYER_LIGHT
    } else {
        LAYER_DARK
    }
}

pub fn apply_to_window(window: &WebviewWindow) {
    let light = is_light();
    if window.label() == MAIN_LABEL {
        if has_mica() {
            let effects = tauri::window::EffectsBuilder::new()
                .effect(tauri::window::Effect::Mica)
                .build();
            let _ = window.set_effects(effects);
        } else {
            let _ = window.set_background_color(Some(mica_background(light)));
        }
    } else {
        let _ = window.set_background_color(Some(layer_background(light)));
    }
    apply_dark_mode(window, light);
}

pub fn refresh_open_windows(app: &AppHandle) {
    for (_, window) in app.webview_windows() {
        apply_to_window(&window);
    }
}

fn apply_dark_mode(window: &WebviewWindow, light: bool) {
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    apply_hwnd_dark_mode(hwnd, light);
}

pub fn apply_hwnd_dark_mode(hwnd: windows::Win32::Foundation::HWND, light: bool) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backgrounds_match_the_design_tokens() {
        assert_eq!(mica_background(false), Color(28, 28, 28, 255));
        assert_eq!(mica_background(true), Color(238, 238, 238, 255));
        assert_eq!(layer_background(false), Color(39, 39, 39, 255));
        assert_eq!(layer_background(true), Color(249, 249, 249, 255));
    }
}
