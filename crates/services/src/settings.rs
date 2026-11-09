use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use ts_rs::TS;

use crate::dto::settings::{SettingsDto, SettingsReq, SettingsResp};
use crate::{ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "SettingsAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct SettingsService {
    services: Services,
}

impl SettingsService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }

    pub fn handle(&self, req: SettingsReq) -> Result<SettingsResp, ServiceError> {
        match req {
            SettingsReq::Ping => Ok(SettingsResp::Pong),
            SettingsReq::Get => {
                let settings = self.services.settings_snapshot()?;
                Ok(SettingsResp::Settings {
                    settings: SettingsDto::from(&settings),
                })
            }
            SettingsReq::Update { patch } => {
                let settings = self.services.update_settings(&patch)?;
                Ok(SettingsResp::Updated {
                    settings: SettingsDto::from(&settings),
                })
            }
            SettingsReq::PreviewName { pattern, format } => Ok(SettingsResp::PreviewName {
                name: preview_name(&pattern, &format),
            }),
            SettingsReq::BrowseFolder { current } => {
                let path = self.services.pick_folder(current.as_deref())?;
                Ok(SettingsResp::BrowseFolder { path })
            }
        }
    }
}

pub fn preview_name(pattern: &str, format: &str) -> String {
    snapline_core::naming::preview(pattern, name_format(format))
}

fn name_format(format: &str) -> snapline_core::naming::NameFormat {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => snapline_core::naming::NameFormat::Jpeg,
        "webp" => snapline_core::naming::NameFormat::WebP,
        _ => snapline_core::naming::NameFormat::Png,
    }
}

pub fn run(_action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    Err(ServiceError::NotYet)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use snapline_store::io::Paths;
    use snapline_store::settings as store_settings;

    use super::*;
    use crate::FolderPicker;

    #[derive(Debug)]
    struct FakePicker {
        path: Option<PathBuf>,
    }

    impl FolderPicker for FakePicker {
        fn pick_folder(&self, _current: Option<&Path>) -> Result<Option<PathBuf>, ServiceError> {
            Ok(self.path.clone())
        }
    }

    fn paths(directory: &Path) -> Paths {
        Paths::resolve(
            directory,
            &directory.join("Roaming"),
            &directory.join("Local"),
            &directory.join("Profile"),
            false,
        )
    }

    fn services(directory: &Path) -> Services {
        Services::with_paths(paths(directory)).expect("services")
    }

    fn picker(path: Option<&str>) -> Arc<dyn FolderPicker> {
        Arc::new(FakePicker {
            path: path.map(PathBuf::from),
        })
    }

    fn dto_of(resp: SettingsResp) -> SettingsDto {
        match resp {
            SettingsResp::Settings { settings } | SettingsResp::Updated { settings } => settings,
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn get_returns_the_defaults() {
        let directory = tempfile::tempdir().expect("temp dir");
        let services = services(directory.path());
        let settings = dto_of(services.settings().handle(SettingsReq::Get).expect("get"));
        assert_eq!(settings.version, 1);
        assert!(settings.general.launch_at_signin);
        assert!(settings.general.start_minimised);
        assert_eq!(
            settings.general.save_folder,
            "%USERPROFILE%\\Pictures\\Snapline"
        );
        assert_eq!(
            settings.general.image_format,
            crate::dto::settings::ImageFormatDto::Png
        );
        assert_eq!(settings.capture.scroll_delay_ms, 300);
        assert_eq!(settings.uploads.images, "save_locally_only");
        assert!(settings.hotkeys.bindings.contains_key("capture_region"));
    }

    #[test]
    fn ping_round_trips() {
        let services = Services::new();
        assert_eq!(
            services.settings().handle(SettingsReq::Ping),
            Ok(SettingsResp::Pong)
        );
    }

    #[test]
    fn update_round_trips_a_nested_change_and_keeps_unknown_keys() {
        let directory = tempfile::tempdir().expect("temp dir");
        let paths = paths(directory.path());
        std::fs::create_dir_all(paths.settings_file.parent().expect("parent")).expect("folder");
        std::fs::write(
            &paths.settings_file,
            concat!(
                "version = 1\n",
                "unknown_top = \"kept\"\n",
                "[general]\n",
                "notifications = true\n",
                "unknown_general = 7\n",
                "[hotkeys]\n",
                "[hotkeys.bindings]\n",
                "capture_region = \"PrtSc\"\n",
                "capture_window = \"Alt+PrtSc\"\n",
            ),
        )
        .expect("write");
        let services = Services::with_paths(paths.clone()).expect("services");
        let patch = serde_json::json!({
            "general": { "notifications": false, "lossy_quality": 75 },
            "hotkeys": { "bindings": { "capture_region": "Ctrl+Shift+1" } },
            "capture": { "magnifier_zoom": "8x" }
        });
        let settings = dto_of(
            services
                .settings()
                .handle(SettingsReq::Update { patch })
                .expect("update"),
        );
        assert!(!settings.general.notifications);
        assert_eq!(settings.general.lossy_quality, 75);
        assert_eq!(
            settings.capture.magnifier_zoom,
            crate::dto::settings::MagnifierZoomDto::X8
        );
        assert_eq!(
            settings
                .hotkeys
                .bindings
                .get("capture_region")
                .map(String::as_str),
            Some("Ctrl+Shift+1")
        );
        assert_eq!(
            settings
                .hotkeys
                .bindings
                .get("capture_window")
                .map(String::as_str),
            Some("Alt+PrtSc")
        );
        let text = std::fs::read_to_string(&paths.settings_file).expect("read");
        assert!(text.contains("unknown_top"));
        assert!(text.contains("unknown_general"));
        let reloaded = store_settings::load_from(&paths).expect("reload");
        assert!(!reloaded.settings.general.notifications);
        assert_eq!(reloaded.settings.general.lossy_quality, 75);
    }

    #[test]
    fn update_rejects_a_patch_that_is_not_an_object() {
        let services = Services::new();
        let error = services
            .settings()
            .handle(SettingsReq::Update {
                patch: serde_json::json!([1, 2, 3]),
            })
            .expect_err("invalid");
        assert_eq!(error.code(), "invalid");
        assert_eq!(error.to_string(), "Settings changes must be an object.");
    }

    #[test]
    fn preview_name_appends_the_format_extension() {
        let services = Services::new();
        let resp = services
            .settings()
            .handle(SettingsReq::PreviewName {
                pattern: "{app}_{yyyy}-{MM}-{dd}".to_owned(),
                format: "png".to_owned(),
            })
            .expect("preview");
        assert_eq!(
            resp,
            SettingsResp::PreviewName {
                name: "chrome_2026-11-08.png".to_owned()
            }
        );
        assert_eq!(preview_name("{app}", "jpeg"), "chrome.jpg");
        assert_eq!(preview_name("{app}", "WebP"), "chrome.webp");
    }

    #[test]
    fn browse_folder_returns_the_picked_path() {
        let services = Services::new().with_folder_picker(picker(Some("C:\\Captures")));
        let resp = services
            .settings()
            .handle(SettingsReq::BrowseFolder {
                current: Some("%USERPROFILE%\\Pictures\\Snapline".to_owned()),
            })
            .expect("browse");
        assert_eq!(
            resp,
            SettingsResp::BrowseFolder {
                path: Some("C:\\Captures".to_owned())
            }
        );
    }

    #[test]
    fn browse_folder_returns_none_on_cancel() {
        let services = Services::new().with_folder_picker(picker(None));
        let resp = services
            .settings()
            .handle(SettingsReq::BrowseFolder { current: None })
            .expect("browse");
        assert_eq!(resp, SettingsResp::BrowseFolder { path: None });
    }

    #[test]
    fn a_bad_settings_file_is_noticed_without_its_contents() {
        let directory = tempfile::tempdir().expect("temp dir");
        let paths = paths(directory.path());
        std::fs::create_dir_all(paths.settings_file.parent().expect("parent")).expect("folder");
        std::fs::write(&paths.settings_file, "secret-value\n[general\n").expect("write");
        let services = Services::with_paths(paths).expect("services");
        let notice = services.settings_notice().expect("notice").expect("some");
        assert_eq!(notice.message, store_settings::LoadNotice::MESSAGE);
        assert!(!notice.message.contains("secret-value"));
        let settings = services.settings_snapshot().expect("snapshot");
        assert_eq!(
            settings.general.save_folder,
            "%USERPROFILE%\\Pictures\\Snapline"
        );
    }
}
