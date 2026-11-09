use std::mem::size_of;

use thiserror::Error;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_VALUE_TYPE,
};

const KEY_PATH: PCWSTR = w!("Control Panel\\Keyboard");
const VALUE_NAME: PCWSTR = w!("PrintScreenKeyForSnippingEnabled");

pub const SIGN_OUT_HINT: &str = "Sign out and back in for Windows to release the PrtSc key.";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PrtscError {
    #[error("Snapline could not read the PrtSc key setting.")]
    Read,
    #[error("Snapline could not take over the PrtSc key.")]
    Write,
    #[error("Snapline could not restore the PrtSc key setting.")]
    Restore,
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

fn open() -> Result<Key, PrtscError> {
    let mut key = HKEY::default();
    // SAFETY: the key path is a static null-terminated string and the out pointer is valid.
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            KEY_PATH,
            Some(0),
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut key,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(Key(key))
    } else {
        Err(PrtscError::Read)
    }
}

pub fn current() -> Result<Option<u32>, PrtscError> {
    let key = open()?;
    read(&key)
}

pub fn enable() -> Result<Option<u32>, PrtscError> {
    let key = open()?;
    let previous = read(&key)?;
    write(&key, 0)?;
    Ok(previous)
}

pub fn disable() -> Result<(), PrtscError> {
    let key = open()?;
    remove(&key)
}

pub fn restore(previous: Option<u32>) -> Result<(), PrtscError> {
    let key = open()?;
    match previous {
        Some(value) => write(&key, value),
        None => remove(&key),
    }
}

fn read(key: &Key) -> Result<Option<u32>, PrtscError> {
    let mut kind = REG_VALUE_TYPE::default();
    let mut bytes = [0_u8; size_of::<u32>()];
    let mut length = u32::try_from(bytes.len()).map_err(|_| PrtscError::Read)?;
    // SAFETY: the type pointer, byte buffer and length all describe live values for the call.
    let result = unsafe {
        RegQueryValueExW(
            key.0,
            VALUE_NAME,
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut length),
        )
    };
    match result {
        ERROR_SUCCESS if kind == REG_DWORD && length == 4 => Ok(Some(u32::from_le_bytes(bytes))),
        ERROR_SUCCESS => Err(PrtscError::Read),
        ERROR_FILE_NOT_FOUND => Ok(None),
        _ => Err(PrtscError::Read),
    }
}

fn write(key: &Key, value: u32) -> Result<(), PrtscError> {
    let bytes = value.to_le_bytes();
    // SAFETY: the value name is static and the byte slice contains a live four-byte DWORD.
    let result = unsafe { RegSetValueExW(key.0, VALUE_NAME, Some(0), REG_DWORD, Some(&bytes)) };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(PrtscError::Write)
    }
}

fn remove(key: &Key) -> Result<(), PrtscError> {
    // SAFETY: the value name is static and the key handle is live.
    let result: WIN32_ERROR = unsafe { RegDeleteValueW(key.0, VALUE_NAME) };
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(PrtscError::Restore)
    }
}

pub struct ScopedOverride {
    previous: Option<u32>,
}

impl ScopedOverride {
    pub fn take_over() -> Result<Self, PrtscError> {
        let previous = enable()?;
        Ok(Self { previous })
    }

    pub fn previous(&self) -> Option<u32> {
        self.previous
    }
}

impl Drop for ScopedOverride {
    fn drop(&mut self) {
        let _ = restore(self.previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_out_hint_is_exact() {
        assert_eq!(
            SIGN_OUT_HINT,
            "Sign out and back in for Windows to release the PrtSc key."
        );
    }

    #[test]
    #[ignore = "desktop"]
    fn taking_over_and_restoring_returns_the_same_value() {
        let before = current().expect("current");
        let guard = ScopedOverride::take_over().expect("take over");
        assert_eq!(guard.previous(), before);
        assert_eq!(current().expect("current after takeover"), Some(0));
        drop(guard);
        assert_eq!(current().expect("current after restore"), before);
    }
}
