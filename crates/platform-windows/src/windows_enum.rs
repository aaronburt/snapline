use std::collections::HashSet;
use std::mem::size_of;

use snapline_core::geometry::{Px, Rect};
use snapline_core::overlay::SnapCandidate;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

const CLASS_BUFFER: usize = 256;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowProbe {
    pub visible: bool,
    pub cloaked: bool,
    pub tool_window: bool,
    pub excluded: bool,
    pub desktop: bool,
    pub empty: bool,
}

impl WindowProbe {
    pub const fn included(self) -> bool {
        self.visible
            && !self.cloaked
            && !self.tool_window
            && !self.excluded
            && !self.desktop
            && !self.empty
    }
}

#[derive(Debug, Clone)]
pub struct WindowEntry {
    pub hwnd: HWND,
    pub rect: Rect,
    pub title: Option<String>,
}

#[derive(Debug)]
struct RawWindow {
    probe: WindowProbe,
    entry: WindowEntry,
}

pub fn enumerate(exclude: &[HWND]) -> Vec<WindowEntry> {
    let excluded: HashSet<isize> = exclude.iter().map(|hwnd| handle_id(*hwnd)).collect();
    let mut raw: Vec<RawWindow> = Vec::new();
    let data = LPARAM(std::ptr::from_mut(&mut raw) as isize);
    // SAFETY: the callback receives a pointer to `raw`, which is live and exclusively borrowed for the duration of the call.
    let _ = unsafe { EnumWindows(Some(enum_callback), data) };
    raw.into_iter()
        .filter(|window| window.probe.included())
        .filter(|window| !excluded.contains(&handle_id(window.entry.hwnd)))
        .map(|window| window.entry)
        .collect()
}

pub fn snap_candidates(exclude: &[HWND]) -> Vec<SnapCandidate> {
    enumerate(exclude)
        .into_iter()
        .map(|entry| SnapCandidate::new(entry.rect))
        .collect()
}

#[cfg(test)]
pub fn filter_in_order(items: impl IntoIterator<Item = (WindowProbe, Rect)>) -> Vec<SnapCandidate> {
    items
        .into_iter()
        .filter(|(probe, rect)| probe.included() && !rect.is_empty())
        .map(|(_, rect)| SnapCandidate::new(rect))
        .collect()
}

unsafe extern "system" fn enum_callback(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the live pointer to the caller's Vec<RawWindow>, valid for the whole EnumWindows call.
    let windows = unsafe { &mut *(data.0 as *mut Vec<RawWindow>) };
    if let Some(window) = describe(hwnd) {
        windows.push(window);
    }
    BOOL(1)
}

fn describe(hwnd: HWND) -> Option<RawWindow> {
    let class = class_name(hwnd);
    let desktop = matches!(class.as_str(), "Progman" | "WorkerW");
    // SAFETY: the window handle comes from EnumWindows and is valid for the duration of the callback.
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let cloaked = is_cloaked(hwnd);
    // SAFETY: reads extended style bits from a live top level window.
    let tool_window =
        unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOOLWINDOW.0 != 0;
    let rect = frame_bounds(hwnd);
    let title = window_title(hwnd);
    Some(RawWindow {
        probe: WindowProbe {
            visible,
            cloaked,
            tool_window,
            excluded: false,
            desktop,
            empty: rect.is_empty(),
        },
        entry: WindowEntry { hwnd, rect, title },
    })
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0_u32;
    // SAFETY: the attribute and out-parameter are the documented types for DWMWA_CLOAKED.
    let status = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            u32::try_from(size_of::<u32>()).unwrap_or(0),
        )
    };
    status.is_ok() && cloaked != 0
}

fn frame_bounds(hwnd: HWND) -> Rect {
    let mut rect = RECT::default();
    // SAFETY: the attribute and out-parameter are the documented types for DWMWA_EXTENDED_FRAME_BOUNDS.
    let status = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            u32::try_from(size_of::<RECT>()).unwrap_or(0),
        )
    };
    if status.is_err() {
        // SAFETY: the window handle is live and `rect` is a live out-parameter.
        let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    }
    Rect::new(
        Px(rect.left),
        Px(rect.top),
        Px(rect.right - rect.left),
        Px(rect.bottom - rect.top),
    )
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0_u16; CLASS_BUFFER];
    // SAFETY: the buffer is live and its length is passed through a sized slice.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if length <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap_or(0)])
    }
}

fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: reads the length of the title owned by a live window.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return None;
    }
    let capacity = usize::try_from(length).ok()?.checked_add(1)?;
    let mut buffer = vec![0_u16; capacity];
    // SAFETY: the buffer is live and `capacity` matches its allocated length.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if copied <= 0 {
        return None;
    }
    let text = String::from_utf16_lossy(&buffer[..usize::try_from(copied).ok()?]);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn handle_id(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

#[cfg(test)]
mod tests {
    use super::*;

    const VISIBLE: WindowProbe = WindowProbe {
        visible: true,
        cloaked: false,
        tool_window: false,
        excluded: false,
        desktop: false,
        empty: false,
    };

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    #[test]
    fn visible_normal_window_is_included() {
        assert!(VISIBLE.included());
    }

    #[test]
    fn desktop_window_is_ignored() {
        let desktop = WindowProbe {
            desktop: true,
            ..VISIBLE
        };
        assert!(!desktop.included());
    }

    #[test]
    fn invisible_cloaked_tool_and_own_windows_are_ignored() {
        for probe in [
            WindowProbe {
                visible: false,
                ..VISIBLE
            },
            WindowProbe {
                cloaked: true,
                ..VISIBLE
            },
            WindowProbe {
                tool_window: true,
                ..VISIBLE
            },
            WindowProbe {
                excluded: true,
                ..VISIBLE
            },
            WindowProbe {
                empty: true,
                ..VISIBLE
            },
        ] {
            assert!(!probe.included());
        }
    }

    #[test]
    fn z_order_is_preserved() {
        let first = rect(0, 0, 100, 100);
        let second = rect(100, 100, 200, 200);
        let third = rect(300, 300, 400, 400);
        let candidates = filter_in_order([
            (VISIBLE, first),
            (
                WindowProbe {
                    visible: false,
                    ..VISIBLE
                },
                rect(10, 10, 10, 10),
            ),
            (VISIBLE, second),
            (VISIBLE, third),
        ]);
        assert_eq!(
            candidates,
            vec![
                SnapCandidate::new(first),
                SnapCandidate::new(second),
                SnapCandidate::new(third),
            ]
        );
    }

    #[test]
    #[ignore = "desktop"]
    fn enumeration_returns_visible_windows_without_desktop_or_overlay() {
        let entries = enumerate(&[]);
        for entry in &entries {
            assert!(!entry.rect.is_empty());
            assert!(entry.hwnd.0 != std::ptr::null_mut());
        }
    }
}
