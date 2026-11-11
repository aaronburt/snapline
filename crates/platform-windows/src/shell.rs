use std::path::Path;
use std::ptr;

use thiserror::Error;
use windows::core::{w, Error, HSTRING, PCWSTR};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TD_WARNING_ICON,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    FileOperation, IFileOperation, IFileOperationProgressSink, IShellItem,
    SHCreateItemFromParsingName, SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteW,
    FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CONFIRM_DELETE: i32 = 1;

#[derive(Debug, Error)]
pub enum ShellError {
    #[error("The file could not be opened.")]
    Open(#[source] Error),
    #[error("The folder could not be opened.")]
    Folder(#[source] Error),
    #[error("The link is not an http or https address.")]
    InvalidUrl,
    #[error("The link could not be opened.")]
    OpenUrl(#[source] Error),
    #[error("The file could not be moved to the Recycle Bin.")]
    Recycle(#[source] Error),
    #[error("The file was not moved to the Recycle Bin.")]
    Aborted,
    #[error("The confirmation could not be shown.")]
    Confirm(#[source] Error),
}

pub fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty())
        && !url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        && !url
            .chars()
            .any(|character| matches!(character, '"' | '\'' | '`' | '<' | '>'))
}

pub fn open_file(path: &Path) -> Result<(), ShellError> {
    launch(&HSTRING::from(path), ShellError::Open)
}

pub fn open_url(url: &str) -> Result<(), ShellError> {
    if !is_web_url(url) {
        return Err(ShellError::InvalidUrl);
    }
    launch(&HSTRING::from(url), ShellError::OpenUrl)
}

fn launch(target: &HSTRING, error: fn(Error) -> ShellError) -> Result<(), ShellError> {
    // SAFETY: the operation and target strings outlive the call and are null-terminated by HSTRING.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            target,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err(error(Error::from_thread()))
    } else {
        Ok(())
    }
}

pub fn show_in_folder(path: &Path) -> Result<(), ShellError> {
    let name = HSTRING::from(path);
    let mut item: *mut ITEMIDLIST = ptr::null_mut();
    // SAFETY: the name is a live null-terminated string and the output pointer is valid for one write.
    unsafe { SHParseDisplayName(&name, None::<&IBindCtx>, &mut item, 0, None) }
        .map_err(ShellError::Folder)?;
    if item.is_null() {
        return Err(ShellError::Folder(Error::from_thread()));
    }
    let result = unsafe { SHOpenFolderAndSelectItems(item, None, 0) };
    // SAFETY: SHParseDisplayName allocated this pidl with CoTaskMemAlloc and it is freed exactly once.
    unsafe { CoTaskMemFree(Some(item.cast())) };
    result.map_err(ShellError::Folder)
}

pub fn recycle(path: &Path) -> Result<(), ShellError> {
    let _com = Com::start().map_err(ShellError::Recycle)?;
    let name = HSTRING::from(path);
    // SAFETY: the name is a live null-terminated string and the returned shell item owns its reference.
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(&name, None::<&IBindCtx>) }
        .map_err(ShellError::Recycle)?;
    // SAFETY: FileOperation is a registered COM class and the returned interface owns a reference.
    let operation: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }
        .map_err(ShellError::Recycle)?;
    let flags =
        FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT;
    // SAFETY: the interface is live and the flags are a valid FILEOPERATION_FLAGS value.
    unsafe { operation.SetOperationFlags(flags) }.map_err(ShellError::Recycle)?;
    // SAFETY: both interfaces are live for the duration of the call and no progress sink is wanted.
    unsafe { operation.DeleteItem(&item, None::<&IFileOperationProgressSink>) }
        .map_err(ShellError::Recycle)?;
    // SAFETY: the interface is live and the queued delete was set up above.
    unsafe { operation.PerformOperations() }.map_err(ShellError::Recycle)?;
    // SAFETY: the interface is live and the operation has completed.
    let aborted = unsafe { operation.GetAnyOperationsAborted() }.map_err(ShellError::Recycle)?;
    if aborted.as_bool() {
        Err(ShellError::Aborted)
    } else {
        Ok(())
    }
}

pub fn confirm_delete(message: &str) -> Result<bool, ShellError> {
    let message = HSTRING::from(message);
    let title = HSTRING::from("Snapline");
    let instruction = HSTRING::from("Delete this capture?");
    let delete = HSTRING::from("Delete");
    let cancel = HSTRING::from("Cancel");
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: CONFIRM_DELETE,
            pszButtonText: PCWSTR(delete.as_ptr()),
        },
        TASKDIALOG_BUTTON {
            nButtonID: 2,
            pszButtonText: PCWSTR(cancel.as_ptr()),
        },
    ];
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_WARNING_ICON,
        },
        pszMainInstruction: PCWSTR(instruction.as_ptr()),
        pszContent: PCWSTR(message.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: 2,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        ..Default::default()
    };
    let mut pressed = 0_i32;
    // SAFETY: the configuration and all strings it points at are live for the duration of the call.
    unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) }
        .map_err(ShellError::Confirm)?;
    Ok(pressed == CONFIRM_DELETE)
}

struct Com(bool);

impl Com {
    fn start() -> Result<Self, Error> {
        // SAFETY: no pointer is passed and the matching CoUninitialize runs in Drop on this thread.
        let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if status.is_ok() {
            return Ok(Self(true));
        }
        if status == RPC_E_CHANGED_MODE {
            return Ok(Self(false));
        }
        Err(Error::from_hresult(status))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: this runs once for the successful CoInitializeEx in Com::start on the same thread.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_urls_are_accepted() {
        assert!(is_web_url("https://example.com/a.png"));
        assert!(is_web_url("http://example.com"));
        assert!(!is_web_url("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("ms-settings:privacy"));
        assert!(!is_web_url("https://example.com/a b.png"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url("http://example.com/\"quoted\""));
    }

    #[test]
    fn opening_a_rejected_url_never_reaches_the_shell() {
        assert!(matches!(
            open_url("javascript:alert(1)"),
            Err(ShellError::InvalidUrl)
        ));
    }

    #[test]
    fn error_messages_are_uk_english_and_hide_paths() {
        assert_eq!(
            ShellError::Recycle(Error::from_thread()).to_string(),
            "The file could not be moved to the Recycle Bin."
        );
        assert_eq!(
            ShellError::InvalidUrl.to_string(),
            "The link is not an http or https address."
        );
        assert_eq!(
            ShellError::Aborted.to_string(),
            "The file was not moved to the Recycle Bin."
        );
    }
}
