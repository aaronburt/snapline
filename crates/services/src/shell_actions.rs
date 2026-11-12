use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use snapline_core::ports::Notice;
use snapline_store::io::Paths;
use snapline_store::settings::Settings;

use crate::{saver, uploads, ServiceError, Services};

pub const COPY_LAST_LINK: &str = "upload_copy_last_link";
pub const OPEN_SAVE_FOLDER: &str = "app_open_save_folder";

pub trait ShellHost: Send + Sync {
    fn copy_text(&self, text: &str) -> Result<(), ServiceError>;
    fn open_folder(&self, path: &Path) -> Result<(), ServiceError>;
}

static HOST: OnceLock<Arc<dyn ShellHost>> = OnceLock::new();

pub fn install_host(host: Arc<dyn ShellHost>) -> Result<(), ServiceError> {
    HOST.set(host)
        .map_err(|_| ServiceError::Internal("The shell actions are already installed.".to_owned()))
}

pub fn installed() -> Result<Arc<dyn ShellHost>, ServiceError> {
    HOST.get().cloned().ok_or(ServiceError::NotYet)
}

/// Copies the newest upload link. Returns whether a link was copied (false when there is none).
pub fn copy_last_link(services: &Services, host: &dyn ShellHost) -> Result<bool, ServiceError> {
    let notifications = services
        .settings_snapshot()
        .map(|settings| settings.general.notifications)
        .unwrap_or(true);
    let Some(link) = uploads::last_link(services)?.filter(|link| !link.trim().is_empty()) else {
        notify(
            services,
            true,
            "No link to copy",
            "Nothing has been uploaded yet.",
        );
        return Ok(false);
    };
    host.copy_text(&link)?;
    notify(
        services,
        notifications,
        "Link copied",
        "The last upload link is on the clipboard.",
    );
    Ok(true)
}

pub fn open_save_folder(services: &Services, host: &dyn ShellHost) -> Result<(), ServiceError> {
    let settings = services.settings_snapshot()?;
    let paths = services.paths()?;
    let folder = save_folder_path(&settings, &paths);
    std::fs::create_dir_all(&folder).map_err(|error| {
        ServiceError::Io(format!(
            "The save folder {} could not be created: {error}",
            folder.display()
        ))
    })?;
    host.open_folder(&folder)
}

pub fn save_folder_path(settings: &Settings, paths: &Paths) -> PathBuf {
    let profile = paths
        .captures_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""));
    PathBuf::from(saver::expand_profile(
        &settings.general.save_folder,
        profile,
    ))
}

pub fn run(action: &str, services: &Services) -> Result<(), ServiceError> {
    match action {
        COPY_LAST_LINK => copy_last_link(services, installed()?.as_ref()).map(|_| ()),
        OPEN_SAVE_FOLDER => open_save_folder(services, installed()?.as_ref()),
        _ => Err(ServiceError::NotYet),
    }
}

fn notify(services: &Services, enabled: bool, title: &str, body: &str) {
    if !enabled {
        return;
    }
    if let Some(notifier) = services.notifier() {
        notifier.toast(Notice::new(title, body));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use snapline_core::ports::Notifier;

    use super::*;

    #[derive(Default)]
    struct FakeHost {
        copied: Mutex<Vec<String>>,
        opened: Mutex<Vec<PathBuf>>,
    }

    impl ShellHost for FakeHost {
        fn copy_text(&self, text: &str) -> Result<(), ServiceError> {
            self.copied.lock().expect("copied").push(text.to_owned());
            Ok(())
        }

        fn open_folder(&self, path: &Path) -> Result<(), ServiceError> {
            self.opened.lock().expect("opened").push(path.to_path_buf());
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeNotifier {
        notices: Mutex<Vec<Notice>>,
    }

    impl Notifier for FakeNotifier {
        fn toast(&self, notice: Notice) {
            self.notices.lock().expect("notices").push(notice);
        }
    }

    fn test_paths(directory: &Path) -> Paths {
        Paths::resolve(
            &directory.join("exe"),
            &directory.join("Roaming"),
            &directory.join("Local"),
            &directory.join("Profile"),
            false,
        )
    }

    fn services_in(directory: &Path) -> (Services, Arc<FakeNotifier>) {
        let notifier = Arc::new(FakeNotifier::default());
        let services = Services::with_paths(test_paths(directory))
            .expect("services")
            .with_notifier(notifier.clone());
        (services, notifier)
    }

    #[test]
    fn both_actions_are_in_the_catalogue_and_route_to_a_handler() {
        for id in [COPY_LAST_LINK, OPEN_SAVE_FOLDER] {
            let action = snapline_core::actions::ActionId::new(id);
            assert!(snapline_core::actions::find(&action).is_some(), "{id}");
            assert!(crate::actions::target(&action).is_some(), "{id}");
        }
    }

    #[test]
    fn copying_with_no_upload_tells_the_user_and_copies_nothing() {
        let directory = tempfile::tempdir().expect("temp");
        let (services, notifier) = services_in(directory.path());
        let host = FakeHost::default();
        assert!(!copy_last_link(&services, &host).expect("copy"));
        assert!(host.copied.lock().expect("copied").is_empty());
        let notices = notifier.notices.lock().expect("notices");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "No link to copy");
    }

    #[test]
    fn copying_puts_the_last_link_on_the_clipboard() {
        let directory = tempfile::tempdir().expect("temp");
        let (services, notifier) = services_in(directory.path());
        uploads::seed_done_upload(&services, "a.png", "https://example.test/a.png", 10);
        let host = FakeHost::default();
        assert!(copy_last_link(&services, &host).expect("copy"));
        assert_eq!(
            *host.copied.lock().expect("copied"),
            vec!["https://example.test/a.png".to_owned()]
        );
        assert_eq!(
            notifier.notices.lock().expect("notices")[0].title,
            "Link copied"
        );
    }

    #[test]
    fn copying_after_the_history_is_deleted_reports_no_link() {
        let directory = tempfile::tempdir().expect("temp");
        let (services, notifier) = services_in(directory.path());
        let id = uploads::seed_done_upload(&services, "a.png", "https://example.test/a.png", 10);
        let path = services.paths().expect("paths").history_file;
        snapline_store::history::HistoryRepo::open_write(&path)
            .expect("history")
            .delete(id)
            .expect("delete");
        let host = FakeHost::default();
        copy_last_link(&services, &host).expect("copy");
        assert!(host.copied.lock().expect("copied").is_empty());
        assert_eq!(
            notifier.notices.lock().expect("notices")[0].title,
            "No link to copy"
        );
    }

    #[test]
    fn the_save_folder_is_created_when_missing_and_opened() {
        let directory = tempfile::tempdir().expect("temp");
        let (services, _) = services_in(directory.path());
        let target = directory.path().join("shots").join("deep");
        services
            .update_settings(&serde_json::json!({
                "general": { "save_folder": target.to_string_lossy() }
            }))
            .expect("settings");
        let host = FakeHost::default();
        open_save_folder(&services, &host).expect("open");
        assert!(target.is_dir());
        assert_eq!(*host.opened.lock().expect("opened"), vec![target]);
    }

    #[test]
    fn profile_tokens_in_the_save_folder_are_expanded() {
        let paths = test_paths(Path::new("scratch"));
        let mut settings = Settings::default();
        settings.general.save_folder = "%USERPROFILE%\\Pictures\\Snapline".to_owned();
        let folder = save_folder_path(&settings, &paths);
        assert!(!folder.to_string_lossy().contains("%USERPROFILE%"));
        assert!(folder.to_string_lossy().ends_with("Pictures\\Snapline"));
    }

    #[test]
    fn unknown_ids_are_not_handled_here() {
        let services = Services::new();
        assert_eq!(run("nonsense", &services), Err(ServiceError::NotYet));
    }
}
