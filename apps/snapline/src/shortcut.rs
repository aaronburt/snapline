use std::path::Path;

use windows::core::{Interface, PCWSTR};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READWRITE,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

pub fn set_app_user_model_id(shortcut: &Path, id: &str) -> windows::core::Result<()> {
    let wide: Vec<u16> = shortcut
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let path = PCWSTR(wide.as_ptr());
    // SAFETY: COM was initialised on this thread; every pointer passed below outlives its call.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file: IPersistFile = link.cast()?;
        file.Load(path, STGM_READWRITE)?;
        let store: IPropertyStore = link.cast()?;
        store.SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(id))?;
        store.Commit()?;
        file.Save(path, true)?;
    }
    Ok(())
}
