use std::ffi::c_void;

use snapline_core::geometry::{Point, Rect};
use snapline_core::ids::WindowId;
use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, GWL_EXSTYLE,
    WS_EX_TRANSPARENT,
};

use crate::windows_enum::enumerate;

const PATH_BUFFER: usize = 1024;
const FRAME_HOST: &str = "ApplicationFrameHost.exe";

pub fn app_window_at(point: Point) -> Option<WindowId> {
    // SAFETY: reads the id of the current process and has no preconditions.
    let own = unsafe { GetCurrentProcessId() };
    let entries = enumerate(&[]).into_iter().map(|entry| TopWindow {
        rect: entry.rect,
        click_through: is_click_through(entry.hwnd),
        own: process_id(entry.hwnd) == Some(own),
        hwnd: entry.hwnd,
    });
    topmost_at(entries, point).and_then(window_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturableWindow {
    pub id: WindowId,
    pub title: String,
}

pub fn capturable_windows() -> Vec<CapturableWindow> {
    // SAFETY: reads the id of the current process and has no preconditions.
    let own = unsafe { GetCurrentProcessId() };
    enumerate(&[])
        .into_iter()
        .filter_map(|entry| {
            let title = entry.title?;
            // SAFETY: the handle came from the enumeration moments ago and a stale one only returns false.
            let minimised = unsafe { IsIconic(entry.hwnd) }.as_bool();
            if !listable(process_id(entry.hwnd) == Some(own), minimised, &title) {
                return None;
            }
            window_id(entry.hwnd).map(|id| CapturableWindow { id, title })
        })
        .collect()
}

fn listable(own: bool, minimised: bool, title: &str) -> bool {
    !own && !minimised && !title.trim().is_empty()
}

pub fn process_name(id: WindowId) -> Option<String> {
    let hwnd = HWND(id.get() as usize as *mut c_void);
    let name = image_name(process_id(hwnd)?)?;
    if name.eq_ignore_ascii_case(FRAME_HOST) {
        if let Some(hosted) = hosted_process(hwnd).and_then(image_name) {
            return Some(hosted);
        }
    }
    Some(name)
}

struct TopWindow {
    rect: Rect,
    click_through: bool,
    own: bool,
    hwnd: HWND,
}

fn topmost_at(windows: impl IntoIterator<Item = TopWindow>, point: Point) -> Option<HWND> {
    windows
        .into_iter()
        .find(|window| !window.click_through && !window.own && window.rect.contains(point))
        .map(|window| window.hwnd)
}

fn window_id(hwnd: HWND) -> Option<WindowId> {
    if hwnd.0.is_null() {
        None
    } else {
        Some(WindowId::new(hwnd.0 as usize as u64))
    }
}

fn is_click_through(hwnd: HWND) -> bool {
    // SAFETY: reads extended style bits and a stale handle only returns zero.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    style as u32 & WS_EX_TRANSPARENT.0 != 0
}

fn process_id(hwnd: HWND) -> Option<u32> {
    let mut process = 0_u32;
    // SAFETY: the out pointer is a live local and a stale handle only leaves it at zero.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    (process != 0).then_some(process)
}

fn hosted_process(frame: HWND) -> Option<u32> {
    let host = process_id(frame)?;
    // SAFETY: the class name is a static string and a stale parent only makes the search fail.
    let child =
        unsafe { FindWindowExW(Some(frame), None, w!("Windows.UI.Core.CoreWindow"), None) }.ok()?;
    process_id(child).filter(|process| *process != host)
}

fn image_name(process: u32) -> Option<String> {
    // SAFETY: opens the process for a limited query and the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) }.ok()?;
    let mut buffer = [0_u16; PATH_BUFFER];
    let mut length = u32::try_from(buffer.len()).unwrap_or(0);
    // SAFETY: the buffer and length describe the same writable memory and the handle is live.
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    // SAFETY: closes the handle opened above exactly once.
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.ok()?;
    let length = usize::try_from(length).ok()?;
    file_name(&String::from_utf16_lossy(buffer.get(..length)?))
}

fn file_name(path: &str) -> Option<String> {
    path.rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use snapline_core::geometry::Px;

    use super::*;

    fn window(id: usize, rect: Rect) -> TopWindow {
        TopWindow {
            rect,
            click_through: false,
            own: false,
            hwnd: HWND(id as *mut c_void),
        }
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    fn found(windows: Vec<TopWindow>, at: Point) -> Option<usize> {
        topmost_at(windows, at).map(|hwnd| hwnd.0 as usize)
    }

    #[test]
    fn the_first_window_in_z_order_containing_the_point_wins() {
        let windows = vec![
            window(1, rect(500, 500, 100, 100)),
            window(2, rect(0, 0, 400, 400)),
            window(3, rect(0, 0, 800, 800)),
        ];
        assert_eq!(found(windows, point(50, 50)), Some(2));
    }

    #[test]
    fn own_minimised_and_untitled_windows_are_not_listed() {
        assert!(listable(false, false, "Notes"));
        assert!(!listable(true, false, "Snapline"));
        assert!(!listable(false, true, "Notes"));
        assert!(!listable(false, false, "   "));
    }

    #[test]
    fn click_through_and_own_windows_are_skipped() {
        let overlay = TopWindow {
            click_through: true,
            ..window(1, rect(0, 0, 800, 800))
        };
        let own = TopWindow {
            own: true,
            ..window(2, rect(0, 0, 800, 800))
        };
        let windows = vec![overlay, own, window(3, rect(0, 0, 400, 400))];
        assert_eq!(found(windows, point(50, 50)), Some(3));
    }

    #[test]
    fn a_point_over_no_window_finds_nothing() {
        let windows = vec![window(1, rect(0, 0, 100, 100))];
        assert_eq!(found(windows, point(500, 500)), None);
    }

    #[test]
    fn the_file_name_is_the_last_path_segment() {
        assert_eq!(
            file_name("C:\\Program Files\\Mozilla Firefox\\firefox.exe").as_deref(),
            Some("firefox.exe")
        );
        assert_eq!(file_name("chrome.exe").as_deref(), Some("chrome.exe"));
        assert_eq!(file_name("C:\\folder\\"), None);
    }
}
