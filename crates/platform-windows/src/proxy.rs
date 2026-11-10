use thiserror::Error;
use windows::core::{s, w, BOOL, PWSTR};
use windows::Win32::Foundation::{FreeLibrary, GlobalFree, HGLOBAL, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemProxy {
    pub enabled: bool,
    pub auto_detect: bool,
    pub server: Option<String>,
    pub bypass: Vec<String>,
    pub auto_config_url: Option<String>,
}

impl SystemProxy {
    pub fn http_url(&self) -> Option<String> {
        self.server.as_deref().and_then(parse_proxy_url)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProxyError {
    #[error("Windows could not read the system proxy settings.")]
    Unavailable,
}

#[repr(C)]
struct IeProxyConfig {
    auto_detect: BOOL,
    auto_config_url: PWSTR,
    server: PWSTR,
    bypass: PWSTR,
}

type IeProxyConfigFn = unsafe extern "system" fn(*mut IeProxyConfig) -> BOOL;

pub fn system_proxy() -> Result<SystemProxy, ProxyError> {
    // SAFETY: winhttp.dll is a static null-terminated library name.
    let module = unsafe { LoadLibraryW(w!("winhttp.dll")) }.map_err(|_| ProxyError::Unavailable)?;
    let result = read_proxy_config(module);
    // SAFETY: the handle came from LoadLibraryW and is released exactly once here.
    unsafe {
        let _ = FreeLibrary(module);
    }
    result
}

fn read_proxy_config(module: HMODULE) -> Result<SystemProxy, ProxyError> {
    // SAFETY: the module handle is live and the symbol name is a static null-terminated string.
    let address = unsafe { GetProcAddress(module, s!("WinHttpGetIEProxyConfigForCurrentUser")) }
        .ok_or(ProxyError::Unavailable)?;
    // SAFETY: Windows documents this export with exactly this signature.
    let function: IeProxyConfigFn = unsafe { std::mem::transmute(address) };
    let mut raw = IeProxyConfig {
        auto_detect: BOOL(0),
        auto_config_url: PWSTR::null(),
        server: PWSTR::null(),
        bypass: PWSTR::null(),
    };
    // SAFETY: the output pointer is valid for the call and Windows owns every returned string.
    let succeeded = unsafe { function(&mut raw) };
    if !succeeded.as_bool() {
        return Err(ProxyError::Unavailable);
    }
    let server = take_string(raw.server);
    let bypass = parse_bypass(&take_string(raw.bypass).unwrap_or_default());
    Ok(SystemProxy {
        enabled: server
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        auto_detect: raw.auto_detect.as_bool(),
        server,
        bypass,
        auto_config_url: take_string(raw.auto_config_url),
    })
}

fn take_string(value: PWSTR) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: Windows returned a null-terminated wide string for this field.
    let text = unsafe { value.to_string().ok() };
    // SAFETY: the pointer is a global allocation owned by the returned proxy configuration.
    unsafe {
        let _ = GlobalFree(Some(HGLOBAL(value.0.cast())));
    }
    text
}

pub fn parse_proxy_url(server: &str) -> Option<String> {
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    let mut fallback = None;
    for entry in server.split(';') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match entry.split_once('=') {
            None => return Some(with_scheme("http", entry)),
            Some((scheme, address)) if scheme.trim().eq_ignore_ascii_case("http") => {
                return Some(with_scheme("http", address));
            }
            Some((scheme, address)) if scheme.trim().eq_ignore_ascii_case("https") => {
                fallback = Some(with_scheme("https", address));
            }
            Some(_) => {}
        }
    }
    fallback
}

pub fn parse_bypass(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

fn with_scheme(scheme: &str, address: &str) -> String {
    let address = address.trim();
    if address.contains("://") {
        address.to_owned()
    } else {
        format!("{scheme}://{address}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_server_gets_the_http_scheme() {
        assert_eq!(
            parse_proxy_url("proxy.local:8080").as_deref(),
            Some("http://proxy.local:8080")
        );
    }

    #[test]
    fn a_protocol_map_prefers_http() {
        assert_eq!(
            parse_proxy_url("ftp=proxy.local:21;http=proxy.local:8080;https=proxy.local:8443")
                .as_deref(),
            Some("http://proxy.local:8080")
        );
    }

    #[test]
    fn a_protocol_map_falls_back_to_https() {
        assert_eq!(
            parse_proxy_url("https=proxy.local:8443").as_deref(),
            Some("https://proxy.local:8443")
        );
    }

    #[test]
    fn an_existing_scheme_is_kept() {
        assert_eq!(
            parse_proxy_url("http://proxy.local:8080").as_deref(),
            Some("http://proxy.local:8080")
        );
    }

    #[test]
    fn an_empty_server_has_no_url() {
        assert_eq!(parse_proxy_url("   "), None);
    }

    #[test]
    fn bypass_entries_are_split_and_trimmed() {
        assert_eq!(
            parse_bypass("<local>; *.corp.example ;10.0.0.0/8;"),
            vec![
                "<local>".to_owned(),
                "*.corp.example".to_owned(),
                "10.0.0.0/8".to_owned()
            ]
        );
    }

    #[test]
    fn an_empty_bypass_list_is_empty() {
        assert!(parse_bypass("").is_empty());
    }

    #[test]
    #[ignore = "desktop"]
    fn the_current_user_proxy_can_be_read() {
        let proxy = system_proxy().expect("read");
        let expected = if proxy.enabled {
            proxy.server.as_deref().and_then(parse_proxy_url)
        } else {
            None
        };
        assert_eq!(proxy.http_url(), expected);
    }
}
