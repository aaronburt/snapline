use std::path::{Path, PathBuf};

use thiserror::Error;
use windows::core::{HRESULT, HSTRING};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, IShellItemArray, SHCreateItemFromParsingName,
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
    FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
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
    #[error("Choose an existing FFmpeg executable.")]
    InvalidFfmpeg,
}

pub fn pick_files() -> Result<Vec<PathBuf>, DialogError> {
    let _com = Com::start().map_err(DialogError::FilesOpen)?;
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(DialogError::FilesOpen)?;
    let options = unsafe { dialog.GetOptions() }.map_err(DialogError::FilesOpen)?;
    let options = options | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST;
    unsafe { dialog.SetOptions(options) }.map_err(DialogError::FilesOpen)?;
    unsafe { dialog.SetTitle(&HSTRING::from("Choose files to upload")) }
        .map_err(DialogError::FilesOpen)?;
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(DialogError::FilesOpen(error)),
    }
    let results: IShellItemArray =
        unsafe { dialog.GetResults() }.map_err(DialogError::FilesRead)?;
    let count = unsafe { results.GetCount() }.map_err(DialogError::FilesRead)?;
    let mut paths = Vec::new();
    for index in 0..count {
        let item = unsafe { results.GetItemAt(index) }.map_err(DialogError::FilesRead)?;
        let name =
            unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(DialogError::FilesRead)?;
        let value = unsafe { name.to_string() };
        unsafe { CoTaskMemFree(Some(name.0.cast())) };
        let value = value.map_err(|error| DialogError::FilesRead(error.into()))?;
        paths.push(PathBuf::from(value));
    }
    Ok(paths)
}

pub fn pick_ffmpeg() -> Result<Option<PathBuf>, DialogError> {
    let _com = Com::start().map_err(DialogError::FilesOpen)?;
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(DialogError::FilesOpen)?;
    let options = unsafe { dialog.GetOptions() }.map_err(DialogError::FilesOpen)?;
    unsafe { dialog.SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST) }
        .map_err(DialogError::FilesOpen)?;
    unsafe { dialog.SetTitle(&HSTRING::from("Choose the FFmpeg executable")) }
        .map_err(DialogError::FilesOpen)?;
    match unsafe { dialog.Show(None) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => {
            return Ok(None);
        }
        Err(error) => return Err(DialogError::FilesOpen(error)),
    }
    let item = unsafe { dialog.GetResult() }.map_err(DialogError::FilesRead)?;
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.map_err(DialogError::FilesRead)?;
    let value = unsafe { name.to_string() };
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

struct Com;

impl Com {
    fn start() -> Result<Self, windows::core::Error> {
        // SAFETY: no pointer is passed and the matching CoUninitialize runs in Drop on this thread.
        let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        status.ok().map(|()| Self)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: this runs once for the successful CoInitializeEx in Com::start on the same thread.
        unsafe { CoUninitialize() };
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
    let path = unsafe { name.to_string() }.map_err(|error| DialogError::Read(error.into()))?;
    // SAFETY: `name` came from CoTaskMemAlloc through GetDisplayName and is freed exactly once.
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    Ok(Some(PathBuf::from(path)))
}
