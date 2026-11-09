use std::path::PathBuf;

use thiserror::Error;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;

#[derive(Debug, Error)]
pub enum VideoDialogError {
    #[error("The video picker could not be opened.")]
    Open,
    #[error("The selected video could not be read.")]
    Read,
}

struct Apartment(bool);

impl Apartment {
    fn start() -> Result<Self, VideoDialogError> {
        // SAFETY: this initialises COM on the current thread and the guard uninitialises it exactly once.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            Ok(Self(true))
        } else if result == windows::Win32::Foundation::RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(VideoDialogError::Open)
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: this matches the successful CoInitializeEx in Apartment::start on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

pub fn pick_video() -> Result<Option<PathBuf>, VideoDialogError> {
    let _apartment = Apartment::start()?;
    let dialog: IFileOpenDialog =
        // SAFETY: FileOpenDialog is a registered COM class and the returned interface owns a reference.
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| VideoDialogError::Open)?;
    // SAFETY: the interface is live for the duration of the call.
    let options = unsafe { dialog.GetOptions() }.map_err(|_| VideoDialogError::Open)?;
    let options = options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST;
    // SAFETY: the interface is live and options is a valid FILEOPENDIALOGOPTIONS value.
    unsafe { dialog.SetOptions(options) }.map_err(|_| VideoDialogError::Open)?;
    let filter_name = HSTRING::from("Videos");
    let filter_spec = HSTRING::from("*.mp4;*.webm;*.mkv;*.mov;*.avi;*.gif");
    let filters = [COMDLG_FILTERSPEC {
        pszName: PCWSTR(filter_name.as_ptr()),
        pszSpec: PCWSTR(filter_spec.as_ptr()),
    }];
    // SAFETY: the filter array and strings outlive the call and the interface is live.
    unsafe { dialog.SetFileTypes(&filters) }.map_err(|_| VideoDialogError::Open)?;
    // SAFETY: the dialog is shown with no owner window, so no parent handle is borrowed.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return Ok(None);
        }
        Err(_) => return Err(VideoDialogError::Open),
    }
    let result =
        // SAFETY: the interface is live and the result belongs to this dialog.
        unsafe { dialog.GetResult() }.map_err(|_| VideoDialogError::Read)?;
    let name =
        // SAFETY: the shell item is live and the returned string is freed below.
        unsafe { result.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(|_| VideoDialogError::Read)?;
    // SAFETY: name is a CoTaskMem string returned by the shell and freed exactly once here.
    let value = unsafe { name.to_string() }.unwrap_or_default();
    // SAFETY: name is the pointer returned by GetDisplayName and is owned by this call.
    unsafe { CoTaskMemFree(Some(name.0 as *const _ as *mut _)) };
    Ok(Some(PathBuf::from(value)))
}
