use std::path::Path;

use thiserror::Error;
use windows::core::w;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: windows::core::PCWSTR = w!("Snapline");
const REGISTRY_TARGET: &str = "Snapline";

#[derive(Debug, Error)]
pub enum AutostartError {
    #[error("Snapline could not read its start-at-sign-in setting.")]
    Read,
    #[error("Snapline could not save its start-at-sign-in setting.")]
    Write,
    #[error("Snapline could not remove its start-at-sign-in setting.")]
    Remove,
    #[error("Snapline could not find its executable path.")]
    CurrentExe,
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by RegOpenKeyExW and is closed exactly once here.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

pub fn command(exe: &Path) -> String {
    format!("\"{}\" --autostart", exe.display())
}

pub fn enable(exe: &Path) -> Result<(), AutostartError> {
    enable_value(exe, VALUE_NAME)
}

pub fn disable() -> Result<(), AutostartError> {
    disable_value(VALUE_NAME)
}

pub fn enabled() -> Result<bool, AutostartError> {
    query_value(VALUE_NAME)
}

pub fn enable_current() -> Result<(), AutostartError> {
    let exe = std::env::current_exe().map_err(|_| AutostartError::CurrentExe)?;
    enable(&exe)
}

fn open_key() -> Result<Key, AutostartError> {
    let mut key = HKEY::default();
    // SAFETY: the key path is a static null-terminated string and the out pointer is valid.
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            Some(0),
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut key,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(Key(key))
    } else {
        Err(AutostartError::Read)
    }
}

fn enable_value(exe: &Path, name: windows::core::PCWSTR) -> Result<(), AutostartError> {
    let key = open_key()?;
    let command = command(exe);
    let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    // SAFETY: the value name is static and the byte slice contains the live wide buffer.
    let result = unsafe { RegSetValueExW(key.0, name, Some(0), REG_SZ, Some(&bytes)) };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(AutostartError::Write)
    }
}

fn disable_value(name: windows::core::PCWSTR) -> Result<(), AutostartError> {
    let key = open_key()?;
    // SAFETY: the value name is a static null-terminated string and the key is live.
    let result = unsafe { RegDeleteValueW(key.0, name) };
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(AutostartError::Remove)
    }
}

fn query_value(name: windows::core::PCWSTR) -> Result<bool, AutostartError> {
    let key = open_key()?;
    let mut bytes = 0_u32;
    // SAFETY: a null data pointer with a zero length asks Windows for the required size.
    let result = unsafe { RegQueryValueExW(key.0, name, None, None, None, Some(&mut bytes)) };
    match result {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        _ => Err(AutostartError::Read),
    }
}

pub fn value_name() -> &'static str {
    REGISTRY_TARGET
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_quotes_the_executable() {
        let command = command(Path::new("C:\\Program Files\\Snapline\\snapline.exe"));
        assert_eq!(
            command,
            "\"C:\\Program Files\\Snapline\\snapline.exe\" --autostart"
        );
    }

    #[test]
    fn value_name_is_snapline() {
        assert_eq!(value_name(), "Snapline");
    }

    #[test]
    #[ignore = "desktop"]
    fn run_key_round_trip_with_a_test_value() {
        let name = w!("SnapLineTest:Autostart");
        let exe = Path::new("C:\\SnaplineTest\\snapline.exe");
        enable_value(exe, name).expect("enable");
        assert!(query_value(name).expect("query"));
        disable_value(name).expect("disable");
        assert!(!query_value(name).expect("query after disable"));
    }
}
