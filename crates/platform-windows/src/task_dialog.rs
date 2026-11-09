use std::mem::size_of;

use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_POSITION_RELATIVE_TO_WINDOW, TDF_SIZE_TO_CONTENT,
};
use windows::Win32::UI::WindowsAndMessaging::IDCANCEL;

pub const UPLOAD_BUTTON: i32 = 1001;
pub const CANCEL_BUTTON: i32 = IDCANCEL.0;
pub const DEFAULT_BUTTON: i32 = CANCEL_BUTTON;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDialogChoice {
    Upload,
    Cancel,
}

#[derive(Debug, Error)]
pub enum TaskDialogError {
    #[error("The confirmation dialog could not be shown.")]
    Show(#[source] windows::core::Error),
    #[error("The confirmation dialog returned an unknown button.")]
    UnknownButton,
}

#[derive(Debug, Clone, Copy)]
pub struct TaskDialog {
    owner: Option<HWND>,
}

impl TaskDialog {
    pub const fn new(owner: Option<HWND>) -> Self {
        Self { owner }
    }

    pub const fn owner(&self) -> Option<HWND> {
        self.owner
    }

    pub fn confirm_upload(&self, message: &str) -> Result<TaskDialogChoice, TaskDialogError> {
        self.confirm(message, "Upload", "Cancel")
    }

    pub fn confirm(
        &self,
        message: &str,
        upload_label: &str,
        cancel_label: &str,
    ) -> Result<TaskDialogChoice, TaskDialogError> {
        let title = wide("Snapline");
        let message = wide(message);
        let upload = wide(upload_label);
        let cancel = wide(cancel_label);
        let buttons = [
            TASKDIALOG_BUTTON {
                nButtonID: UPLOAD_BUTTON,
                pszButtonText: PCWSTR(upload.as_ptr()),
            },
            TASKDIALOG_BUTTON {
                nButtonID: CANCEL_BUTTON,
                pszButtonText: PCWSTR(cancel.as_ptr()),
            },
        ];
        let mut flags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT;
        if self.owner.is_some() {
            flags |= TDF_POSITION_RELATIVE_TO_WINDOW;
        }
        let config = TASKDIALOGCONFIG {
            cbSize: u32::try_from(size_of::<TASKDIALOGCONFIG>()).unwrap_or(u32::MAX),
            hwndParent: self.owner.unwrap_or_default(),
            dwFlags: flags,
            pszWindowTitle: PCWSTR(title.as_ptr()),
            pszMainInstruction: PCWSTR(message.as_ptr()),
            cButtons: u32::try_from(buttons.len()).unwrap_or(u32::MAX),
            pButtons: buttons.as_ptr(),
            nDefaultButton: DEFAULT_BUTTON,
            ..Default::default()
        };
        let mut pressed = 0_i32;
        // SAFETY: every string and the button array stay alive for the duration of the call, and the config points only at them.
        unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) }
            .map_err(TaskDialogError::Show)?;
        match pressed {
            UPLOAD_BUTTON => Ok(TaskDialogChoice::Upload),
            CANCEL_BUTTON | 0 => Ok(TaskDialogChoice::Cancel),
            _ => Err(TaskDialogError::UnknownButton),
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_cancel_button_is_the_default() {
        assert_eq!(CANCEL_BUTTON, 2);
        assert_eq!(DEFAULT_BUTTON, CANCEL_BUTTON);
        assert_ne!(UPLOAD_BUTTON, CANCEL_BUTTON);
    }

    #[test]
    fn text_is_null_terminated_utf16() {
        let text = wide("Upload");
        assert_eq!(
            text,
            "Upload"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        );
        assert_eq!(text.last(), Some(&0));
        assert_eq!(wide("").len(), 1);
    }

    #[test]
    fn the_owner_is_optional() {
        assert_eq!(TaskDialog::new(None).owner(), None);
        assert_eq!(
            TaskDialog::new(Some(HWND(std::ptr::null_mut()))).owner(),
            Some(HWND(std::ptr::null_mut()))
        );
    }

    #[test]
    fn errors_use_uk_english() {
        assert_eq!(
            TaskDialogError::UnknownButton.to_string(),
            "The confirmation dialog returned an unknown button."
        );
    }

    #[test]
    #[ignore = "desktop"]
    fn the_upload_dialog_returns_a_choice() {
        let choice = TaskDialog::new(None)
            .confirm_upload("Upload the image on the clipboard to imgur?")
            .expect("dialog");
        assert!(matches!(
            choice,
            TaskDialogChoice::Upload | TaskDialogChoice::Cancel
        ));
    }
}
