use std::mem::size_of;

use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub const WINDOWS_11_BUILD: u32 = 22_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsBuild {
    pub build: u32,
}

impl WindowsBuild {
    pub fn current() -> Self {
        if let Some(build) = build_from_rtl() {
            return Self { build };
        }
        Self {
            build: build_from_registry().unwrap_or(0),
        }
    }

    pub fn is_windows_11(self) -> bool {
        is_windows_11_build(self.build)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn current() -> Self {
        if is_light() {
            Self::Light
        } else {
            Self::Dark
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, Self::Light)
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

pub fn is_light() -> bool {
    apps_use_light_theme().unwrap_or(false)
}

pub fn is_windows_11() -> bool {
    WindowsBuild::current().is_windows_11()
}

pub fn has_mica() -> bool {
    is_windows_11()
}

pub fn is_windows_11_build(build: u32) -> bool {
    build >= WINDOWS_11_BUILD
}

fn apps_use_light_theme() -> Option<bool> {
    let mut value = 0_u32;
    let mut size = u32::try_from(size_of::<u32>()).ok()?;
    // SAFETY: the value and size pointers describe a live u32 and its byte length for the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut value).cast()),
            Some(std::ptr::from_mut(&mut size)),
        )
    };
    status.is_ok().then_some(value != 0)
}

#[repr(C)]
struct RtlOsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform_id: u32,
    csd_version: [u16; 128],
}

type RtlGetVersion = unsafe extern "system" fn(*mut RtlOsVersionInfoW) -> i32;

fn build_from_rtl() -> Option<u32> {
    // SAFETY: ntdll.dll is loaded in every Windows process, so the handle stays valid for the process lifetime.
    let module = unsafe { GetModuleHandleW(w!("ntdll.dll")) }.ok()?;
    // SAFETY: the module handle comes from GetModuleHandleW and the symbol name is a static null-terminated string.
    let address = unsafe { GetProcAddress(module, s!("RtlGetVersion")) }?;
    // SAFETY: RtlGetVersion is exported by ntdll with this documented signature.
    let rtl_get_version = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, RtlGetVersion>(address)
    };
    let mut info = RtlOsVersionInfoW {
        size: u32::try_from(size_of::<RtlOsVersionInfoW>()).ok()?,
        major: 0,
        minor: 0,
        build: 0,
        platform_id: 0,
        csd_version: [0; 128],
    };
    // SAFETY: `info` is a live RTL_OSVERSIONINFOW with its size field set to the size of the struct.
    let status = unsafe { rtl_get_version(&mut info) };
    (status == 0).then_some(info.build)
}

fn build_from_registry() -> Option<u32> {
    let mut buffer = [0_u16; 64];
    let mut size = u32::try_from(buffer.len().checked_mul(size_of::<u16>())?).ok()?;
    // SAFETY: the buffer pointer and size describe a live UTF-16 array for the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(std::ptr::from_mut(&mut size)),
        )
    };
    if status.is_err() {
        return None;
    }
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    parse_build_number(&String::from_utf16_lossy(&buffer[..length]))
}

fn parse_build_number(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_numbers_parse_from_registry_text() {
        assert_eq!(parse_build_number("22631"), Some(22_631));
        assert_eq!(parse_build_number(" 26100\n"), Some(26_100));
        assert_eq!(parse_build_number(""), None);
        assert_eq!(parse_build_number("unknown"), None);
    }

    #[test]
    fn windows_11_starts_at_build_22000() {
        assert!(!is_windows_11_build(21_999));
        assert!(is_windows_11_build(22_000));
        assert!(is_windows_11_build(26_100));
    }

    #[test]
    fn a_build_reports_itself_as_windows_11() {
        assert!(!WindowsBuild { build: 19_045 }.is_windows_11());
        assert!(WindowsBuild { build: 22_631 }.is_windows_11());
    }

    #[test]
    fn theme_reports_its_own_mode() {
        assert!(Theme::Light.is_light());
        assert!(!Theme::Light.is_dark());
        assert!(Theme::Dark.is_dark());
        assert!(!Theme::Dark.is_light());
    }

    #[test]
    #[ignore = "desktop"]
    fn registry_reports_the_current_theme() {
        let _ = is_light();
        let _ = Theme::current();
    }

    #[test]
    #[ignore = "desktop"]
    fn rtl_get_version_reports_a_build() {
        assert!(WindowsBuild::current().build > 0);
        let _ = has_mica();
    }
}
