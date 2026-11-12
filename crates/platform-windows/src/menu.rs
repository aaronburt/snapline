use thiserror::Error;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DestroyMenu, DestroyWindow, GetCursorPos,
    GetDesktopWindow, GetForegroundWindow, PostMessageW, SetForegroundWindow, TrackPopupMenuEx,
    MF_SEPARATOR, MF_STRING, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE,
    WM_NULL, WS_EX_TOOLWINDOW, WS_POPUP,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSourceChoice {
    File,
    Clipboard,
}

#[derive(Debug, Error)]
pub enum MenuError {
    #[error("The cursor position could not be read.")]
    Cursor,
    #[error("The menu could not be created.")]
    Create,
}

pub fn choose_pin_source() -> Result<Option<PinSourceChoice>, MenuError> {
    // SAFETY: creates a popup menu owned by this thread until DestroyMenu below.
    let menu = unsafe { CreatePopupMenu() }.map_err(|_| MenuError::Create)?;
    let file: Vec<u16> = "From file…"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let clipboard: Vec<u16> = "From clipboard"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: the menu is live and each label buffer outlives the call.
    let _ = unsafe { AppendMenuW(menu, MF_STRING, 1, PCWSTR::from_raw(file.as_ptr())) };
    // SAFETY: the menu is live and each label buffer outlives the call.
    let _ = unsafe { AppendMenuW(menu, MF_STRING, 2, PCWSTR::from_raw(clipboard.as_ptr())) };
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: the point is a valid writable POINT.
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        // SAFETY: destroys the menu created above exactly once.
        let _ = unsafe { DestroyMenu(menu) };
        return Err(MenuError::Cursor);
    }
    // SAFETY: returns the foreground window or a null handle.
    let mut owner = unsafe { GetForegroundWindow() };
    if owner.0.is_null() {
        // SAFETY: returns the desktop window handle.
        owner = unsafe { GetDesktopWindow() };
    }
    // SAFETY: the owner is a live window handle.
    let _ = unsafe { SetForegroundWindow(owner) };
    // SAFETY: the menu is live, the owner is valid and no result structure is supplied.
    let command = unsafe {
        TrackPopupMenuEx(
            menu,
            (TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY).0,
            point.x,
            point.y,
            owner,
            None,
        )
    };
    // SAFETY: destroys the menu created above exactly once.
    let _ = unsafe { DestroyMenu(menu) };
    Ok(choice_from_command(command.0 as u32 as usize))
}

pub fn choose_from(labels: &[String]) -> Result<Option<usize>, MenuError> {
    if labels.is_empty() {
        return Ok(None);
    }
    // SAFETY: creates a popup menu owned by this thread until DestroyMenu below.
    let menu = unsafe { CreatePopupMenu() }.map_err(|_| MenuError::Create)?;
    for (index, label) in labels.iter().enumerate() {
        let text: Vec<u16> = menu_text(label)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: the menu is live and the label buffer outlives the call.
        let _ = unsafe { AppendMenuW(menu, MF_STRING, index + 1, PCWSTR::from_raw(text.as_ptr())) };
        if index == 0 && labels.len() > 1 {
            // SAFETY: the menu is live and a separator needs no text.
            let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
        }
    }
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: the point is a valid writable POINT.
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        // SAFETY: destroys the menu created above exactly once.
        let _ = unsafe { DestroyMenu(menu) };
        return Err(MenuError::Cursor);
    }
    // SAFETY: creates a hidden zero-size window on this thread to own the menu.
    let helper = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
    }
    .ok();
    let owner = helper.unwrap_or_else(|| {
        // SAFETY: returns the foreground window or a null handle.
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            // SAFETY: returns the desktop window handle.
            unsafe { GetDesktopWindow() }
        } else {
            foreground
        }
    });
    // SAFETY: the owner is a live window handle.
    let _ = unsafe { SetForegroundWindow(owner) };
    // SAFETY: the menu is live, the owner is valid and no result structure is supplied.
    let command = unsafe {
        TrackPopupMenuEx(
            menu,
            (TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY).0,
            point.x,
            point.y,
            owner,
            None,
        )
    };
    // SAFETY: the documented follow-up to TrackPopupMenu, so the menu closes on an outside click
    // even when the owner was not made the foreground window.
    let _ = unsafe { PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0)) };
    if let Some(helper) = helper {
        destroy(helper);
    }
    // SAFETY: destroys the menu created above exactly once.
    let _ = unsafe { DestroyMenu(menu) };
    Ok(index_from_command(command.0 as u32 as usize, labels.len()))
}

fn destroy(window: HWND) {
    // SAFETY: the window was created on this thread and is destroyed exactly once.
    let _ = unsafe { DestroyWindow(window) };
}

pub fn menu_text(label: &str) -> String {
    label.replace('\0', "").replace('&', "&&")
}

pub fn index_from_command(command: usize, count: usize) -> Option<usize> {
    command.checked_sub(1).filter(|index| *index < count)
}

pub fn choice_from_command(command: usize) -> Option<PinSourceChoice> {
    match command {
        1 => Some(PinSourceChoice::File),
        2 => Some(PinSourceChoice::Clipboard),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_map_back_to_label_indexes() {
        assert_eq!(index_from_command(0, 3), None);
        assert_eq!(index_from_command(1, 3), Some(0));
        assert_eq!(index_from_command(3, 3), Some(2));
        assert_eq!(index_from_command(4, 3), None);
    }

    #[test]
    fn ampersands_are_escaped_so_they_are_not_mnemonics() {
        assert_eq!(menu_text("R&D \u{2014} notes"), "R&&D \u{2014} notes");
        assert_eq!(menu_text("a\0b"), "ab");
    }

    #[test]
    fn an_empty_label_list_shows_nothing() {
        assert!(matches!(choose_from(&[]), Ok(None)));
    }

    #[test]
    fn commands_map_to_the_two_sources() {
        assert_eq!(choice_from_command(1), Some(PinSourceChoice::File));
        assert_eq!(choice_from_command(2), Some(PinSourceChoice::Clipboard));
        assert_eq!(choice_from_command(0), None);
        assert_eq!(choice_from_command(99), None);
    }
}
