use std::path::{Path, PathBuf};

use thiserror::Error;
use windows::core::{HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_CANCELLED, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, IShellItem, IShellItemArray,
    SHCreateItemFromParsingName, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
    FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

#[derive(Debug, Error)]
pub enum DialogError {
    #[error("The folder picker could not be opened.")]
    Open(#[source] windows::core::Error),
    #[error("The chosen folder could not be read.")]
    Read(#[source] windows::core::Error),
    #[error("The file picker could not be opened.")]
    FilesOpen(#[source] windows::core::Error),
    #[error("The selected files could not be read.")]
    FilesRead(#[source] windows::core::Error),
    #[error("The save dialog could not be opened.")]
    SaveOpen(#[source] windows::core::Error),
    #[error("The chosen save path could not be read.")]
    SaveRead(#[source] windows::core::Error),
    #[error("The chosen file could not be written.")]
    SaveWrite(#[source] std::io::Error),
    #[error("Choose an existing FFmpeg executable.")]
    InvalidFfmpeg,
}

pub fn pick_files() -> Result<Vec<PathBuf>, DialogError> {
    let _com = Com::start().map_err(DialogError::FilesOpen)?;
    let dialog: IFileOpenDialog =
        // SAFETY: FileOpenDialog is a registered COM class and the returned interface owns a reference.
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(DialogError::FilesOpen)?;
    // SAFETY: the interface is live for the duration of the call.
    let options = unsafe { dialog.GetOptions() }.map_err(DialogError::FilesOpen)?;
    let options = options | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST;
    // SAFETY: the interface is live and `options` is a valid FILEOPENDIALOGOPTIONS value.
    unsafe { dialog.SetOptions(options) }.map_err(DialogError::FilesOpen)?;
    // SAFETY: the title string outlives the call and the interface is live.
    unsafe { dialog.SetTitle(&HSTRING::from("Choose files to upload")) }
        .map_err(DialogError::FilesOpen)?;
    // SAFETY: the dialog is shown with no owner window, so no parent handle is borrowed.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(DialogError::FilesOpen(error)),
    }
    let results: IShellItemArray =
        // SAFETY: the dialog completed, so GetResults returns a live shell item array.
        unsafe { dialog.GetResults() }.map_err(DialogError::FilesRead)?;
    // SAFETY: the shell item array is live for the duration of the call.
    let count = unsafe { results.GetCount() }.map_err(DialogError::FilesRead)?;
    let mut paths = Vec::new();
    for index in 0..count {
        // SAFETY: `index` is below `count` reported by the live shell item array.
        let item = unsafe { results.GetItemAt(index) }.map_err(DialogError::FilesRead)?;
        // SAFETY: the shell item is live and the returned display name is freed below.
        let name =
            unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(DialogError::FilesRead)?;
        // SAFETY: `name` is a CoTaskMem UTF-16 string returned by GetDisplayName.
        let value = unsafe { name.to_string() };
        // SAFETY: `name` was allocated by GetDisplayName and is freed exactly once.
        unsafe { CoTaskMemFree(Some(name.0.cast())) };
        let value = value.map_err(|error| DialogError::FilesRead(error.into()))?;
        paths.push(PathBuf::from(value));
    }
    Ok(paths)
}

pub fn pick_ffmpeg() -> Result<Option<PathBuf>, DialogError> {
    let _com = Com::start().map_err(DialogError::FilesOpen)?;
    let dialog: IFileOpenDialog =
        // SAFETY: FileOpenDialog is a registered COM class and the returned interface owns a reference.
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(DialogError::FilesOpen)?;
    // SAFETY: the interface is live for the duration of the call.
    let options = unsafe { dialog.GetOptions() }.map_err(DialogError::FilesOpen)?;
    // SAFETY: the interface is live and options is a valid FILEOPENDIALOGOPTIONS value.
    unsafe { dialog.SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST) }
        .map_err(DialogError::FilesOpen)?;
    // SAFETY: the title string outlives the call and the interface is live.
    unsafe { dialog.SetTitle(&HSTRING::from("Choose the FFmpeg executable")) }
        .map_err(DialogError::FilesOpen)?;
    // SAFETY: the dialog is shown with no owner window, so no parent handle is borrowed.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return Ok(None);
        }
        Err(error) => return Err(DialogError::FilesOpen(error)),
    }
    // SAFETY: the dialog completed, so GetResult returns a live shell item.
    let item = unsafe { dialog.GetResult() }.map_err(DialogError::FilesRead)?;
    // SAFETY: the shell item is live and the returned display name is freed below.
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(DialogError::FilesRead)?;
    // SAFETY: `name` is a CoTaskMem UTF-16 string returned by GetDisplayName.
    let value = unsafe { name.to_string() };
    // SAFETY: `name` was allocated by GetDisplayName and is freed exactly once.
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    let value = value.map_err(|error| DialogError::FilesRead(error.into()))?;
    let path = PathBuf::from(value);
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err(DialogError::InvalidFfmpeg);
    }
    Ok(Some(path))
}

pub fn pick_sxcu() -> Result<Option<PathBuf>, DialogError> {
    let _com = Com::start().map_err(DialogError::FilesOpen)?;
    let dialog: IFileOpenDialog =
        // SAFETY: FileOpenDialog is a registered COM class and the returned interface owns a reference.
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(DialogError::FilesOpen)?;
    // SAFETY: the interface is live for the duration of the call.
    let options = unsafe { dialog.GetOptions() }.map_err(DialogError::FilesOpen)?;
    // SAFETY: the interface is live and options is a valid FILEOPENDIALOGOPTIONS value.
    unsafe { dialog.SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST) }
        .map_err(DialogError::FilesOpen)?;
    // SAFETY: the title string outlives the call and the interface is live.
    unsafe { dialog.SetTitle(&HSTRING::from("Choose a ShareX .sxcu file")) }
        .map_err(DialogError::FilesOpen)?;
    // SAFETY: the dialog is shown with no owner window, so no parent handle is borrowed.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
        Err(error) => return Err(DialogError::FilesOpen(error)),
    }
    // SAFETY: the dialog completed, so GetResult returns a live shell item.
    let item = unsafe { dialog.GetResult() }.map_err(DialogError::FilesRead)?;
    // SAFETY: the shell item is live and the returned display name is freed below.
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(DialogError::FilesRead)?;
    // SAFETY: `name` is a CoTaskMem UTF-16 string returned by GetDisplayName.
    let value = unsafe { name.to_string() };
    // SAFETY: `name` was allocated by GetDisplayName and is freed exactly once.
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    Ok(Some(PathBuf::from(
        value.map_err(|error| DialogError::FilesRead(error.into()))?,
    )))
}

fn save_file(
    title: &str,
    filter_name: &str,
    filter_spec: &str,
    default_ext: &str,
    suggested_name: &str,
) -> Result<Option<PathBuf>, DialogError> {
    let _com = Com::start().map_err(DialogError::SaveOpen)?;
    let dialog: IFileSaveDialog =
        // SAFETY: FileSaveDialog is a registered COM class and the returned interface owns a reference.
        unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(DialogError::SaveOpen)?;
    // SAFETY: the interface is live for the duration of the call.
    let options = unsafe { dialog.GetOptions() }.map_err(DialogError::SaveOpen)?;
    let options = options | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_OVERWRITEPROMPT;
    // SAFETY: the interface is live and `options` is a valid FILEOPENDIALOGOPTIONS value.
    unsafe { dialog.SetOptions(options) }.map_err(DialogError::SaveOpen)?;
    let filter_name = HSTRING::from(filter_name);
    let filter_spec = HSTRING::from(filter_spec);
    let filters = [COMDLG_FILTERSPEC {
        pszName: PCWSTR(filter_name.as_ptr()),
        pszSpec: PCWSTR(filter_spec.as_ptr()),
    }];
    // SAFETY: `filters` and both backing HSTRINGs outlive the call on the live interface.
    unsafe { dialog.SetFileTypes(&filters) }.map_err(DialogError::SaveOpen)?;
    // SAFETY: the extension string outlives the call and the interface is live.
    unsafe { dialog.SetDefaultExtension(&HSTRING::from(default_ext)) }
        .map_err(DialogError::SaveOpen)?;
    // SAFETY: the suggested file name string outlives the call and the interface is live.
    unsafe { dialog.SetFileName(&HSTRING::from(suggested_name)) }.map_err(DialogError::SaveOpen)?;
    // SAFETY: the title string outlives the call and the interface is live.
    unsafe { dialog.SetTitle(&HSTRING::from(title)) }.map_err(DialogError::SaveOpen)?;
    // SAFETY: the dialog is shown with no owner window, so no parent handle is borrowed.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
        Err(error) => return Err(DialogError::SaveOpen(error)),
    }
    // SAFETY: the dialog completed, so GetResult returns a live shell item.
    let item = unsafe { dialog.GetResult() }.map_err(DialogError::SaveRead)?;
    // SAFETY: the shell item is live and the returned display name is freed below.
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(DialogError::SaveRead)?;
    // SAFETY: `name` is a CoTaskMem UTF-16 string returned by GetDisplayName.
    let value = unsafe { name.to_string() };
    // SAFETY: `name` was allocated by GetDisplayName and is freed exactly once.
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    Ok(Some(PathBuf::from(
        value.map_err(|error| DialogError::SaveRead(error.into()))?,
    )))
}

pub fn save_sxcu(suggested_name: &str, contents: &str) -> Result<Option<PathBuf>, DialogError> {
    let Some(path) = save_file(
        "Export ShareX .sxcu file",
        "ShareX custom uploader (*.sxcu)",
        "*.sxcu",
        "sxcu",
        suggested_name,
    )?
    else {
        return Ok(None);
    };
    std::fs::write(&path, contents).map_err(DialogError::SaveWrite)?;
    Ok(Some(path))
}

pub fn save_png(suggested_name: &str, bytes: &[u8]) -> Result<Option<PathBuf>, DialogError> {
    let Some(path) = save_file(
        "Save PNG image",
        "PNG image (*.png)",
        "*.png",
        "png",
        suggested_name,
    )?
    else {
        return Ok(None);
    };
    std::fs::write(&path, bytes).map_err(DialogError::SaveWrite)?;
    Ok(Some(path))
}

struct Com(bool);

impl Com {
    fn start() -> Result<Self, windows::core::Error> {
        // SAFETY: no pointer is passed and the matching CoUninitialize runs in Drop when this call owns the apartment.
        let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if status == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            status.ok().map(|()| Self(true))
        }
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

pub fn pick_folder(current: Option<&Path>) -> Result<Option<PathBuf>, DialogError> {
    let _com = Com::start().map_err(DialogError::Open)?;
    let dialog: IFileOpenDialog =
        // SAFETY: FileOpenDialog is a registered COM class and the returned interface owns a reference.
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(DialogError::Open)?;
    // SAFETY: the interface is live for the duration of the call.
    let options = unsafe { dialog.GetOptions() }.map_err(DialogError::Open)?;
    let options = options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
    // SAFETY: the interface is live and `options` is a valid FILEOPENDIALOGOPTIONS value.
    unsafe { dialog.SetOptions(options) }.map_err(DialogError::Open)?;
    // SAFETY: the title string outlives the call and the interface is live.
    unsafe { dialog.SetTitle(&HSTRING::from("Choose a folder")) }.map_err(DialogError::Open)?;
    if let Some(current) = current.filter(|path| path.is_dir()) {
        let name = HSTRING::from(current);
        // SAFETY: the shell item is created from a live path string and owns its own reference.
        if let Ok(folder) =
            unsafe { SHCreateItemFromParsingName::<_, Option<&IBindCtx>, IShellItem>(&name, None) }
        {
            // SAFETY: both interfaces are live for the duration of the call.
            let _ = unsafe { dialog.SetFolder(&folder) };
        }
    }
    // SAFETY: the dialog is shown with no owner window, so no parent handle is borrowed.
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
        Err(error) => return Err(DialogError::Open(error)),
    }
    // SAFETY: the dialog completed, so GetResult returns a live shell item.
    let item = unsafe { dialog.GetResult() }.map_err(DialogError::Read)?;
    // SAFETY: the shell item is live and the requested display name is a file-system path.
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(DialogError::Read)?;
    // SAFETY: GetDisplayName returns a task-allocated UTF-16 string owned by this call.
    let path = unsafe { name.to_string() };
    // SAFETY: `name` came from CoTaskMemAlloc through GetDisplayName and is freed exactly once.
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    let path = path.map_err(|error| DialogError::Read(error.into()))?;
    Ok(Some(PathBuf::from(path)))
}
