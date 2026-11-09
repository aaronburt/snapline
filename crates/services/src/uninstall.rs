use std::path::{Path, PathBuf};

use snapline_core::secret::SecretKey;
use snapline_store::io::{self, Paths};
use snapline_store::settings as store_settings;
use thiserror::Error;

pub const CREDENTIALS_QUESTION: &str =
    "Delete Snapline's saved credentials from Credential Manager?";
pub const DATA_QUESTION: &str = "Also delete Snapline's settings and history?";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct UninstallError(String);

impl UninstallError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

pub trait UninstallHost: Send + Sync {
    fn remove_run_value(&self) -> Result<(), UninstallError>;
    fn restore_prtsc(&self, previous: Option<u32>) -> Result<(), UninstallError>;
    fn remove_app_id(&self) -> Result<(), UninstallError>;
    fn list_credential_targets(&self) -> Result<Vec<String>, UninstallError>;
    fn delete_credential_target(&self, target: &str) -> Result<(), UninstallError>;
    fn confirm(&self, message: &str, default_yes: bool) -> Result<bool, UninstallError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallReport {
    pub run_value_removed: bool,
    pub prtsc_restored: bool,
    pub app_id_removed: bool,
    pub credential_targets_found: usize,
    pub credentials_deleted: usize,
    pub credentials_kept: usize,
    pub data_deleted: bool,
    pub removed_paths: Vec<PathBuf>,
    pub failures: Vec<String>,
}

impl UninstallReport {
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut text = String::from("Snapline uninstall cleanup finished.");
        if self.run_value_removed {
            text.push_str(" The start-at-sign-in entry was removed.");
        }
        if self.prtsc_restored {
            text.push_str(" The PrtSc setting was restored.");
        }
        if self.app_id_removed {
            text.push_str(" The AppUserModelID registration was removed.");
        }
        match self.credentials_deleted {
            0 if self.credential_targets_found == 0 => {
                text.push_str(" No saved credentials were found.");
            }
            0 => text.push_str(&format!(
                " {} saved {} {} kept.",
                self.credentials_kept,
                credential_word(self.credentials_kept),
                if self.credentials_kept == 1 {
                    "was"
                } else {
                    "were"
                }
            )),
            deleted => text.push_str(&format!(
                " {deleted} saved {} {} deleted.",
                credential_word(deleted),
                if deleted == 1 { "was" } else { "were" }
            )),
        }
        if self.data_deleted {
            text.push_str(" Settings, history, thumbnails and logs were deleted.");
        } else {
            text.push_str(" Settings, history, thumbnails and logs were kept.");
        }
        text.push_str(" Your captures were kept.");
        if self.failures.is_empty() {
            text.push_str(" No errors.");
        } else {
            text.push_str(&format!(
                " Some steps failed: {}",
                self.failures.join(" ")
            ));
        }
        text
    }
}

fn credential_word(count: usize) -> &'static str {
    if count == 1 {
        "credential"
    } else {
        "credentials"
    }
}

pub fn cleanup(host: &dyn UninstallHost, paths: &Paths) -> UninstallReport {
    let mut report = UninstallReport::default();
    let previous_prtsc = match store_settings::load(&paths.settings_file) {
        Ok(outcome) if outcome.notice.is_none() => {
            Some(outcome.settings.hotkeys.restore.print_screen_key_for_snipping_enabled)
        }
        Ok(_) => {
            report.failures.push(
                "Snapline could not read its settings, so the PrtSc setting was left alone."
                    .to_owned(),
            );
            None
        }
        Err(error) => {
            report.failures.push(format!(
                "Snapline could not read its settings, so the PrtSc setting was left alone: {error}"
            ));
            None
        }
    };

    if let Err(error) = host.remove_run_value() {
        report.failures.push(error.to_string());
    } else {
        report.run_value_removed = true;
    }

    if let Some(previous) = previous_prtsc {
        if let Err(error) = host.restore_prtsc(previous) {
            report.failures.push(error.to_string());
        } else {
            report.prtsc_restored = true;
        }
    }

    if let Err(error) = host.remove_app_id() {
        report.failures.push(error.to_string());
    } else {
        report.app_id_removed = true;
    }

    delete_credentials(host, &mut report);

    let delete_data = match host.confirm(DATA_QUESTION, false) {
        Ok(answer) => answer,
        Err(error) => {
            report.failures.push(error.to_string());
            false
        }
    };
    if delete_data {
        report.data_deleted = true;
        remove_file(&mut report, &paths.settings_file);
        remove_file(&mut report, &io::temp_path(&paths.settings_file));
        remove_file(&mut report, &paths.history_file);
        remove_file(&mut report, &sidecar(&paths.history_file, "-wal"));
        remove_file(&mut report, &sidecar(&paths.history_file, "-shm"));
        remove_directory(&mut report, &paths.thumbs_dir);
        remove_directory(&mut report, &paths.logs_dir);
    }

    report
}

fn delete_credentials(host: &dyn UninstallHost, report: &mut UninstallReport) {
    let targets = match host.list_credential_targets() {
        Ok(targets) => targets,
        Err(error) => {
            report.failures.push(error.to_string());
            return;
        }
    };
    let targets: Vec<String> = targets
        .into_iter()
        .filter(|target| target.starts_with(SecretKey::TARGET_PREFIX))
        .collect();
    report.credential_targets_found = targets.len();
    if targets.is_empty() {
        return;
    }
    let confirmed = match host.confirm(CREDENTIALS_QUESTION, false) {
        Ok(answer) => answer,
        Err(error) => {
            report.failures.push(error.to_string());
            report.credentials_kept = targets.len();
            return;
        }
    };
    if !confirmed {
        report.credentials_kept = targets.len();
        return;
    }
    for target in targets {
        match host.delete_credential_target(&target) {
            Ok(()) => report.credentials_deleted += 1,
            Err(error) => {
                report.credentials_kept += 1;
                report.failures.push(error.to_string());
            }
        }
    }
}

fn remove_file(report: &mut UninstallReport, path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => report.removed_paths.push(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report.failures.push(format!(
            "Snapline could not delete {}: {error}",
            path.display()
        )),
    }
}

fn remove_directory(report: &mut UninstallReport, path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => report.removed_paths.push(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report.failures.push(format!(
            "Snapline could not delete {}: {error}",
            path.display()
        )),
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;
    use snapline_store::settings::Settings;
    use tempfile::TempDir;

    struct FakeState {
        run_removed: bool,
        prtsc_previous: Option<Option<u32>>,
        app_id_removed: bool,
        credential_targets: Vec<String>,
        deleted_credentials: Vec<String>,
        confirmations: Vec<(String, bool)>,
        credential_answer: bool,
        data_answer: bool,
        fail_run: bool,
    }

    impl Default for FakeState {
        fn default() -> Self {
            Self {
                run_removed: false,
                prtsc_previous: None,
                app_id_removed: false,
                credential_targets: Vec::new(),
                deleted_credentials: Vec::new(),
                confirmations: Vec::new(),
                credential_answer: false,
                data_answer: false,
                fail_run: false,
            }
        }
    }

    struct FakeHost {
        state: Mutex<FakeState>,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                state: Mutex::new(FakeState::default()),
            }
        }

        fn with_credentials(targets: &[&str]) -> Self {
            let host = Self::new();
            host.state_mut().credential_targets =
                targets.iter().map(|target| (*target).to_owned()).collect();
            host
        }

        fn state_mut(&self) -> std::sync::MutexGuard<'_, FakeState> {
            match self.state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            }
        }

        fn set_credential_answer(&self, answer: bool) {
            self.state_mut().credential_answer = answer;
        }

        fn set_data_answer(&self, answer: bool) {
            self.state_mut().data_answer = answer;
        }

        fn set_fail_run(&self, fail: bool) {
            self.state_mut().fail_run = fail;
        }

        fn clear_credentials(&self) {
            self.state_mut().credential_targets.clear();
        }
    }

    impl UninstallHost for FakeHost {
        fn remove_run_value(&self) -> Result<(), UninstallError> {
            let mut state = self.state_mut();
            if state.fail_run {
                return Err(UninstallError::new(
                    "Snapline could not remove its start-at-sign-in setting.",
                ));
            }
            state.run_removed = true;
            Ok(())
        }

        fn restore_prtsc(&self, previous: Option<u32>) -> Result<(), UninstallError> {
            self.state_mut().prtsc_previous = Some(previous);
            Ok(())
        }

        fn remove_app_id(&self) -> Result<(), UninstallError> {
            self.state_mut().app_id_removed = true;
            Ok(())
        }

        fn list_credential_targets(&self) -> Result<Vec<String>, UninstallError> {
            Ok(self.state_mut().credential_targets.clone())
        }

        fn delete_credential_target(&self, target: &str) -> Result<(), UninstallError> {
            self.state_mut().deleted_credentials.push(target.to_owned());
            Ok(())
        }

        fn confirm(&self, message: &str, default_yes: bool) -> Result<bool, UninstallError> {
            let mut state = self.state_mut();
            state
                .confirmations
                .push((message.to_owned(), default_yes));
            if message == DATA_QUESTION {
                Ok(state.data_answer)
            } else if message == CREDENTIALS_QUESTION {
                Ok(state.credential_answer)
            } else {
                Ok(false)
            }
        }
    }

    struct Fixture {
        _directory: TempDir,
        paths: Paths,
        host: FakeHost,
        capture: PathBuf,
    }

    fn fixture(restore: Option<Option<u32>>) -> Fixture {
        let directory = match tempfile::tempdir() {
            Ok(directory) => directory,
            Err(error) => panic!("temp dir: {error}"),
        };
        let root = directory.path();
        let paths = Paths::resolve(
            &root.join("exe"),
            &root.join("appdata"),
            &root.join("local"),
            &root.join("user"),
            false,
        );
        if let Some(previous) = restore {
            let mut settings = Settings::default();
            settings
                .hotkeys
                .restore
                .print_screen_key_for_snipping_enabled = previous;
            if let Err(error) = store_settings::save(&paths.settings_file, &settings) {
                panic!("write settings: {error}");
            }
        }
        create_file(&paths.history_file);
        create_file(&paths.thumbs_dir.join("1.webp"));
        create_file(&paths.logs_dir.join("snapline.log"));
        let capture = paths.captures_dir.join("capture.png");
        create_file(&capture);
        Fixture {
            _directory: directory,
            paths,
            host: FakeHost::new(),
            capture,
        }
    }

    fn create_file(path: &Path) {
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                panic!("create folder: {error}");
            }
        }
        if let Err(error) = std::fs::write(path, b"test") {
            panic!("write file: {error}");
        }
    }

    fn confirmations(host: &FakeHost) -> Vec<(String, bool)> {
        host.state_mut().confirmations.clone()
    }

    #[test]
    fn removes_the_run_value_and_the_app_id() {
        let fixture = fixture(Some(None));
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert!(fixture.host.state_mut().run_removed);
        assert!(fixture.host.state_mut().app_id_removed);
    }

    #[test]
    fn restores_a_previous_prtsc_value_exactly() {
        let fixture = fixture(Some(Some(1)));
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert_eq!(fixture.host.state_mut().prtsc_previous, Some(Some(1)));
    }

    #[test]
    fn restores_prtsc_absence_exactly() {
        let fixture = fixture(Some(None));
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert_eq!(fixture.host.state_mut().prtsc_previous, Some(None));
    }

    #[test]
    fn deletes_only_snapline_credentials_after_yes() {
        let mut fixture = fixture(Some(None));
        fixture.host = FakeHost::with_credentials(&[
            "SnapLine:imgur:token",
            "SnapLine:s3:secret",
            "OtherApplication:token",
        ]);
        fixture.host.set_credential_answer(true);
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert_eq!(
            fixture.host.state_mut().deleted_credentials,
            vec![
                "SnapLine:imgur:token".to_owned(),
                "SnapLine:s3:secret".to_owned()
            ]
        );
        assert_eq!(report.credentials_deleted, 2);
        assert_eq!(report.credentials_kept, 0);
    }

    #[test]
    fn keeps_credentials_after_no() {
        let mut fixture = fixture(Some(None));
        fixture.host =
            FakeHost::with_credentials(&["SnapLine:imgur:token", "SnapLine:s3:secret"]);
        fixture.host.set_credential_answer(false);
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert!(fixture.host.state_mut().deleted_credentials.is_empty());
        assert_eq!(report.credentials_deleted, 0);
        assert_eq!(report.credentials_kept, 2);
    }

    #[test]
    fn never_deletes_captures_when_data_is_deleted() {
        let fixture = fixture(Some(None));
        fixture.host.set_data_answer(true);
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert!(fixture.capture.exists());
        assert!(!report
            .removed_paths
            .iter()
            .any(|path| path.starts_with(&fixture.paths.captures_dir)));
    }

    #[test]
    fn deletes_settings_history_thumbs_and_logs_after_yes() {
        let fixture = fixture(Some(None));
        fixture.host.set_data_answer(true);
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert!(!fixture.paths.settings_file.exists());
        assert!(!fixture.paths.history_file.exists());
        assert!(!fixture.paths.thumbs_dir.exists());
        assert!(!fixture.paths.logs_dir.exists());
        assert!(report.data_deleted);
        assert!(fixture.capture.exists());
    }

    #[test]
    fn keeps_settings_history_thumbs_and_logs_after_no() {
        let fixture = fixture(Some(None));
        fixture.host.set_data_answer(false);
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert!(fixture.paths.settings_file.exists());
        assert!(fixture.paths.history_file.exists());
        assert!(fixture.paths.thumbs_dir.exists());
        assert!(fixture.paths.logs_dir.exists());
        assert!(!report.data_deleted);
        assert!(fixture.capture.exists());
    }

    #[test]
    fn asks_the_documented_question_with_no_as_the_default() {
        let fixture = fixture(Some(None));
        fixture.host.set_data_answer(false);
        let _ = cleanup(&fixture.host, &fixture.paths);
        let asked = confirmations(&fixture.host);
        assert!(asked
            .iter()
            .any(|(message, default_yes)| message == DATA_QUESTION && !*default_yes));
    }

    #[test]
    fn a_second_run_is_idempotent() {
        let fixture = fixture(Some(Some(1)));
        fixture.host.set_data_answer(true);
        let first = cleanup(&fixture.host, &fixture.paths);
        assert!(first.succeeded());
        fixture.host.clear_credentials();
        let second = cleanup(&fixture.host, &fixture.paths);
        assert!(second.succeeded());
        assert!(second.run_value_removed);
        assert!(second.prtsc_restored);
        assert!(second.app_id_removed);
        assert!(second.failures.is_empty());
    }

    #[test]
    fn reports_host_failures_and_continues() {
        let fixture = fixture(Some(None));
        fixture.host.set_fail_run(true);
        fixture.host.set_data_answer(true);
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(!report.succeeded());
        assert!(!report.run_value_removed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("start-at-sign-in")));
        assert!(fixture.host.state_mut().app_id_removed);
        assert!(report.data_deleted);
    }

    #[test]
    fn a_missing_settings_file_still_removes_the_prtsc_value() {
        let fixture = fixture(None);
        let report = cleanup(&fixture.host, &fixture.paths);
        assert!(report.succeeded());
        assert_eq!(fixture.host.state_mut().prtsc_previous, Some(None));
    }
}
