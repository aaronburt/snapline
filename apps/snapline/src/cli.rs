use snapline_core::secret::SecretKey;
use snapline_services::uninstall::{self, UninstallError, UninstallHost};
use snapline_store::io::Paths;
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredEnumerateW, CredFree, CREDENTIALW, CRED_TYPE_GENERIC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_YESNO, MESSAGEBOX_STYLE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetWindow {
    Panel,
    Main,
}

impl BudgetWindow {
    #[must_use]
    pub fn from_argument(value: &str) -> Option<Self> {
        match value {
            "panel" => Some(Self::Panel),
            "main" => Some(Self::Main),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub budget_run: bool,
    pub budget_open: Option<BudgetWindow>,
    pub action: Option<String>,
    pub autostart: bool,
    pub uninstall_cleanup: bool,
}

impl Args {
    #[must_use]
    pub fn from_env() -> Self {
        Self::parse(std::env::args().skip(1))
    }

    #[must_use]
    pub fn parse<I, S>(arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let values: Vec<String> = arguments
            .into_iter()
            .map(|value| value.as_ref().to_string())
            .collect();
        let mut args = Self::default();
        let mut index = 0;
        while index < values.len() {
            match values[index].as_str() {
                "--budget-run" => args.budget_run = true,
                "--budget-open" => {
                    index += 1;
                    if let Some(value) = values.get(index) {
                        args.budget_open = BudgetWindow::from_argument(value);
                    }
                }
                "--action" => {
                    index += 1;
                    if let Some(value) = values.get(index) {
                        args.action = Some(value.clone());
                    }
                }
                "--autostart" => args.autostart = true,
                "--uninstall-cleanup" => args.uninstall_cleanup = true,
                _ => {}
            }
            index += 1;
        }
        args
    }

    #[must_use]
    pub fn parse_forwarded(argv: &[String]) -> Self {
        let skip = if argv.first().is_some_and(|first| !first.starts_with('-')) {
            1
        } else {
            0
        };
        Self::parse(argv.iter().skip(skip))
    }
}

pub fn run_uninstall_cleanup() -> i32 {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("Snapline could not find its folders: {error}");
            return 1;
        }
    };
    let report = uninstall::cleanup(&WindowsUninstallHost, &paths);
    if report.succeeded() {
        println!("{}", report.summary());
        0
    } else {
        eprintln!("{}", report.summary());
        1
    }
}

struct WindowsUninstallHost;

impl UninstallHost for WindowsUninstallHost {
    fn remove_run_value(&self) -> Result<(), UninstallError> {
        snapline_win::autostart::disable().map_err(host_error)
    }

    fn restore_prtsc(&self, previous: Option<u32>) -> Result<(), UninstallError> {
        snapline_win::prtsc::restore(previous).map_err(host_error)
    }

    fn remove_app_id(&self) -> Result<(), UninstallError> {
        snapline_win::notify::remove_app_id().map_err(host_error)
    }

    fn list_credential_targets(&self) -> Result<Vec<String>, UninstallError> {
        credential_targets()
    }

    fn delete_credential_target(&self, target: &str) -> Result<(), UninstallError> {
        delete_credential_target(target)
    }

    fn confirm(&self, message: &str, default_yes: bool) -> Result<bool, UninstallError> {
        let message: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
        let title: Vec<u16> = "Snapline"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut style: MESSAGEBOX_STYLE = MB_YESNO | MB_ICONQUESTION;
        if !default_yes {
            style |= MB_DEFBUTTON2;
        }
        // SAFETY: both strings are live null-terminated UTF-16 buffers that outlive the call.
        let result = unsafe {
            MessageBoxW(
                None,
                PCWSTR(message.as_ptr()),
                PCWSTR(title.as_ptr()),
                style,
            )
        };
        Ok(result == IDYES)
    }
}

fn host_error(error: impl std::fmt::Display) -> UninstallError {
    UninstallError::new(error.to_string())
}

fn credential_targets() -> Result<Vec<String>, UninstallError> {
    let filter_text = format!("{}*", SecretKey::TARGET_PREFIX);
    let filter: Vec<u16> = filter_text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut count = 0_u32;
    let mut entries: *mut *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: the filter is a live null-terminated UTF-16 buffer and both out pointers are valid.
    let status = unsafe { CredEnumerateW(PCWSTR(filter.as_ptr()), None, &mut count, &mut entries) };
    if let Err(error) = status {
        if error.code() == ERROR_NOT_FOUND.to_hresult() {
            return Ok(Vec::new());
        }
        return Err(host_error(error.message()));
    }
    let list = CredentialList(entries);
    let mut targets = Vec::new();
    for index in 0..count as usize {
        // SAFETY: CredEnumerateW returned `count` entries in this array and index is inside it.
        let entry = unsafe { *list.0.add(index) };
        if entry.is_null() {
            continue;
        }
        // SAFETY: entry is a live CREDENTIALW from CredEnumerateW.
        let name = unsafe { (*entry).TargetName };
        if name.is_null() {
            continue;
        }
        // SAFETY: the target name is a live null-terminated UTF-16 string owned by the array.
        if let Ok(target) = unsafe { name.to_string() } {
            if target.starts_with(SecretKey::TARGET_PREFIX) {
                targets.push(target);
            }
        }
    }
    Ok(targets)
}

fn delete_credential_target(target: &str) -> Result<(), UninstallError> {
    if !target.starts_with(SecretKey::TARGET_PREFIX) {
        return Err(UninstallError::new(
            "Snapline refused to delete a credential it does not own.",
        ));
    }
    let target: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the target is a live null-terminated UTF-16 buffer for the duration of the call.
    match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(error) => Err(host_error(error.message())),
    }
}

struct CredentialList(*mut *mut CREDENTIALW);

impl Drop for CredentialList {
    fn drop(&mut self) {
        if self.0.is_null() {
            return;
        }
        // SAFETY: CredEnumerateW allocated this array and it is freed exactly once here.
        unsafe { CredFree(self.0.cast()) };
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, BudgetWindow};

    fn parse(arguments: &[&str]) -> Args {
        Args::parse(arguments.iter().copied())
    }

    #[test]
    fn defaults_to_no_flags() {
        assert_eq!(parse(&[]), Args::default());
    }

    #[test]
    fn parses_budget_run() {
        assert!(parse(&["--budget-run"]).budget_run);
    }

    #[test]
    fn parses_budget_open_panel() {
        assert_eq!(
            parse(&["--budget-open", "panel"]).budget_open,
            Some(BudgetWindow::Panel)
        );
    }

    #[test]
    fn parses_budget_open_main() {
        assert_eq!(
            parse(&["--budget-open", "main"]).budget_open,
            Some(BudgetWindow::Main)
        );
    }

    #[test]
    fn ignores_an_unknown_budget_window() {
        let args = parse(&["--budget-open", "settings"]);
        assert_eq!(args.budget_open, None);
    }

    #[test]
    fn ignores_budget_open_without_a_value() {
        assert_eq!(parse(&["--budget-open"]).budget_open, None);
    }

    #[test]
    fn parses_action() {
        let args = parse(&["--action", "capture_region"]);
        assert_eq!(args.action.as_deref(), Some("capture_region"));
    }

    #[test]
    fn ignores_action_without_a_value() {
        assert_eq!(parse(&["--action"]).action, None);
    }

    #[test]
    fn parses_reserved_flags() {
        let args = parse(&["--autostart", "--uninstall-cleanup"]);
        assert!(args.autostart);
        assert!(args.uninstall_cleanup);
    }

    #[test]
    fn ignores_unknown_arguments() {
        let args = parse(&["--colour", "blue", "--budget-run", "extra"]);
        assert!(args.budget_run);
        assert_eq!(args.action, None);
        assert_eq!(args.budget_open, None);
        assert!(!args.autostart);
        assert!(!args.uninstall_cleanup);
    }

    #[test]
    fn parses_flags_in_any_order() {
        let args = parse(&[
            "--action",
            "app_open",
            "--budget-run",
            "--budget-open",
            "main",
        ]);
        assert!(args.budget_run);
        assert_eq!(args.action.as_deref(), Some("app_open"));
        assert_eq!(args.budget_open, Some(BudgetWindow::Main));
    }

    #[test]
    fn forwarded_arguments_skip_the_executable_path() {
        let argv = vec![
            "C:\\Program Files\\Snapline\\snapline.exe".to_string(),
            "--budget-open".to_string(),
            "panel".to_string(),
        ];
        assert_eq!(
            Args::parse_forwarded(&argv).budget_open,
            Some(BudgetWindow::Panel)
        );
    }

    #[test]
    fn forwarded_arguments_without_an_executable_path_still_parse() {
        let argv = vec!["--budget-open".to_string(), "main".to_string()];
        assert_eq!(
            Args::parse_forwarded(&argv).budget_open,
            Some(BudgetWindow::Main)
        );
    }
}
