use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetDesktopWindow, GetForegroundWindow,
    SetForegroundWindow, TrackPopupMenuEx, MF_STRING, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
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
    fn commands_map_to_the_two_sources() {
        assert_eq!(choice_from_command(1), Some(PinSourceChoice::File));
        assert_eq!(choice_from_command(2), Some(PinSourceChoice::Clipboard));
        assert_eq!(choice_from_command(0), None);
        assert_eq!(choice_from_command(99), None);
    }
}
