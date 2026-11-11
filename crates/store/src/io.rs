use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub exe_dir: PathBuf,
    pub data_dir: PathBuf,
    pub local_dir: PathBuf,
    pub settings_file: PathBuf,
    pub history_file: PathBuf,
    pub thumbs_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub crashes_dir: PathBuf,
    pub webview2_dir: PathBuf,
    pub captures_dir: PathBuf,
    pub portable: bool,
}

impl Paths {
    pub fn discover() -> Result<Self, PathError> {
        let exe = env::current_exe().map_err(PathError::CurrentExe)?;
        let exe_dir = exe.parent().ok_or(PathError::NoExeDirectory)?;
        let app_data = environment_path("APPDATA")?;
        let local_app_data = environment_path("LOCALAPPDATA")?;
        let user_profile = environment_path("USERPROFILE")?;
        let portable = exe_dir.join("portable").is_file();
        Ok(Self::resolve(
            exe_dir,
            &app_data,
            &local_app_data,
            &user_profile,
            portable,
        ))
    }

    pub fn resolve(
        exe_dir: &Path,
        app_data: &Path,
        local_app_data: &Path,
        user_profile: &Path,
        portable: bool,
    ) -> Self {
        let data_dir = if portable {
            exe_dir.join("data")
        } else {
            app_data.join("SnapLine")
        };
        let local_dir = local_app_data.join("SnapLine");
        let settings_file = data_dir.join("settings.toml");
        let history_file = if portable {
            data_dir.join("history.db")
        } else {
            local_dir.join("history.db")
        };
        let thumbs_dir = if portable {
            data_dir.join("thumbs")
        } else {
            local_dir.join("thumbs")
        };
        let logs_dir = if portable {
            data_dir.join("logs")
        } else {
            local_dir.join("logs")
        };
        Self {
            exe_dir: exe_dir.to_path_buf(),
            data_dir,
            local_dir: local_dir.clone(),
            settings_file,
            history_file,
            thumbs_dir,
            logs_dir,
            crashes_dir: local_dir.join("crashes"),
            webview2_dir: local_dir.join("WebView2"),
            captures_dir: user_profile.join("Pictures").join("Snapline"),
            portable,
        }
    }
}

#[derive(Debug, Error)]
pub enum PathError {
    #[error("could not locate the Snapline executable: {0}")]
    CurrentExe(#[source] io::Error),
    #[error("the Snapline executable has no containing folder")]
    NoExeDirectory,
    #[error("the {0} environment variable is not set")]
    MissingEnvironment(&'static str),
}

fn environment_path(name: &'static str) -> Result<PathBuf, PathError> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or(PathError::MissingEnvironment(name))
}

pub fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |file| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn write_atomic_with(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temp_path(path);
    let written = std::fs::File::create(&temporary).and_then(|mut file| fill(&mut file));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

pub fn bundled_ffmpeg(exe_dir: &Path) -> PathBuf {
    exe_dir.join("ffmpeg.exe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_failed_write_removes_the_temporary_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        let target = directory.path().join("capture.png");
        let error = write_atomic_with(&target, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("disk full"))
        })
        .expect_err("write must fail");
        assert_eq!(error.to_string(), "disk full");
        assert!(!temp_path(&target).exists());
        assert!(!target.exists());
    }

    #[test]
    fn a_successful_write_leaves_only_the_target() {
        let directory = tempfile::tempdir().expect("temp dir");
        let target = directory.path().join("capture.png");
        write_atomic(&target, b"image").expect("write");
        assert_eq!(std::fs::read(&target).expect("read"), b"image");
        assert!(!temp_path(&target).exists());
    }

    #[test]
    fn portable_mode_moves_settings_history_thumbs_and_logs() {
        let exe = Path::new("C:\\Apps\\Snapline");
        let paths = Paths::resolve(
            exe,
            Path::new("C:\\Users\\Aaron\\AppData\\Roaming"),
            Path::new("C:\\Users\\Aaron\\AppData\\Local"),
            Path::new("C:\\Users\\Aaron"),
            true,
        );
        assert_eq!(paths.settings_file, exe.join("data").join("settings.toml"));
        assert_eq!(paths.history_file, exe.join("data").join("history.db"));
        assert_eq!(paths.thumbs_dir, exe.join("data").join("thumbs"));
        assert_eq!(paths.logs_dir, exe.join("data").join("logs"));
        assert!(paths.portable);
    }

    #[test]
    fn installed_mode_uses_appdata_and_localappdata() {
        let paths = Paths::resolve(
            Path::new("C:\\Program Files\\Snapline"),
            Path::new("C:\\Users\\Aaron\\AppData\\Roaming"),
            Path::new("C:\\Users\\Aaron\\AppData\\Local"),
            Path::new("C:\\Users\\Aaron"),
            false,
        );
        assert_eq!(
            paths.settings_file,
            Path::new("C:\\Users\\Aaron\\AppData\\Roaming\\SnapLine\\settings.toml")
        );
        assert_eq!(
            paths.history_file,
            Path::new("C:\\Users\\Aaron\\AppData\\Local\\SnapLine\\history.db")
        );
        assert_eq!(
            paths.webview2_dir,
            Path::new("C:\\Users\\Aaron\\AppData\\Local\\SnapLine\\WebView2")
        );
        assert_eq!(
            paths.captures_dir,
            Path::new("C:\\Users\\Aaron\\Pictures\\Snapline")
        );
    }

    #[test]
    fn temp_path_appends_the_suffix() {
        assert_eq!(
            temp_path(Path::new("settings.toml")),
            PathBuf::from("settings.toml.tmp")
        );
    }
}
