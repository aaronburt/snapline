use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

const APP_DIR: &str = "apps/snapline";
const CONFIG: &str = "apps/snapline/tauri.conf.json";
const EXE: &str = "snapline.exe";
const RELEASE_EXE: &str = "target/release/snapline.exe";
const NSIS_DIR: &str = "target/release/bundle/nsis";
const PORTABLE_DIR: &str = "target/portable";
const MARKER: &str = "portable";
const RESOURCES: [(&str, &str); 4] = [
    ("third_party/ffmpeg/ffmpeg.exe", "ffmpeg.exe"),
    ("LICENSE", "LICENSE"),
    ("third_party/ffmpeg/LICENSE", "FFmpeg-LICENSE.txt"),
    ("third_party/ffmpeg/SOURCE.txt", "SOURCE.txt"),
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    product_name: String,
    version: String,
}

pub fn run(root: &Path) -> Result<()> {
    check_sources(root)?;
    crate::ui(root)?;
    build_installer(root)?;
    let config = read_config(root)?;
    let installer = installer_path(root, &config)?;
    println!("package: installer {}", installer.display());
    let zip = portable_zip(root, &config)?;
    println!("package: portable zip {}", zip.display());
    Ok(())
}

fn read_config(root: &Path) -> Result<Config> {
    let path = root.join(CONFIG);
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("{} is not a usable Tauri config", path.display()))
}

fn check_sources(root: &Path) -> Result<()> {
    for (source, _) in RESOURCES {
        let path = root.join(source);
        if path.is_file() {
            continue;
        }
        if source.starts_with("third_party/ffmpeg/") {
            bail!(
                "package: {} is missing; run `cargo xtask fetch-ffmpeg` first",
                path.display()
            );
        }
        bail!("package: {} is missing", path.display());
    }
    Ok(())
}

fn build_installer(root: &Path) -> Result<()> {
    let directory = root.join(APP_DIR);
    let local_bin = root.join("ui/node_modules/.bin");
    if !local_bin.join("tauri").is_file() && !local_bin.join("tauri.cmd").is_file() {
        bail!(
            "package: the Tauri CLI is missing from {}; run `cargo xtask ui` first",
            local_bin.display()
        );
    }
    println!("> npx tauri build --bundles nsis");
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "npx", "tauri", "build", "--bundles", "nsis"]);
        command
    } else {
        let mut command = Command::new("npx");
        command.args(["tauri", "build", "--bundles", "nsis"]);
        command
    };
    command.env("PATH", cli_path(&local_bin)?);
    let status = command
        .current_dir(&directory)
        .status()
        .with_context(|| format!("could not start the Tauri CLI in {}", directory.display()))?;
    if !status.success() {
        bail!("package: the Tauri CLI failed with {status}");
    }
    Ok(())
}

fn cli_path(local_bin: &Path) -> Result<OsString> {
    let current = env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(local_bin.to_path_buf()).chain(env::split_paths(&current));
    env::join_paths(paths).context("cannot add ui/node_modules/.bin to PATH")
}

fn installer_path(root: &Path, config: &Config) -> Result<PathBuf> {
    let directory = root.join(NSIS_DIR);
    if !directory.is_dir() {
        bail!(
            "package: the NSIS bundle directory {} is missing; `npx tauri build --bundles nsis` did not produce it",
            directory.display()
        );
    }
    let expected = directory.join(format!(
        "{}_{}_x64-setup.exe",
        config.product_name, config.version
    ));
    if expected.is_file() {
        return Ok(expected);
    }
    let entries =
        fs::read_dir(&directory).with_context(|| format!("cannot read {}", directory.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let name = path.file_name().and_then(|value| value.to_str());
        if path.is_file() && name.is_some_and(|value| value.ends_with("-setup.exe")) {
            found.push(path);
        }
    }
    found.sort();
    found.pop().ok_or_else(|| {
        anyhow!(
            "package: no NSIS installer was produced in {} (expected {})",
            directory.display(),
            expected.display()
        )
    })
}

fn portable_zip(root: &Path, config: &Config) -> Result<PathBuf> {
    let parent = root.join(PORTABLE_DIR);
    let staging = parent.join(&config.product_name);
    stage_portable(root, &staging)?;
    let zip = parent.join(format!(
        "{}_{}_x64-portable.zip",
        config.product_name, config.version
    ));
    if zip.exists() {
        fs::remove_file(&zip).with_context(|| format!("cannot replace {}", zip.display()))?;
    }
    let tar = tar_path()?;
    println!(
        "> {} -a -c -f {} -C {} {}",
        tar.display(),
        zip.display(),
        parent.display(),
        config.product_name
    );
    let status = Command::new(&tar)
        .arg("-a")
        .arg("-c")
        .arg("-f")
        .arg(&zip)
        .arg("-C")
        .arg(&parent)
        .arg(&config.product_name)
        .status()
        .with_context(|| format!("could not start {}", tar.display()))?;
    if !status.success() {
        bail!(
            "package: {} could not create {}",
            tar.display(),
            zip.display()
        );
    }
    Ok(zip)
}

fn stage_portable(root: &Path, staging: &Path) -> Result<()> {
    let exe = root.join(RELEASE_EXE);
    if !exe.is_file() {
        bail!(
            "package: {} is missing; the release build did not produce it",
            exe.display()
        );
    }
    if staging.exists() {
        fs::remove_dir_all(staging)
            .with_context(|| format!("cannot clear {}", staging.display()))?;
    }
    fs::create_dir_all(staging).with_context(|| format!("cannot create {}", staging.display()))?;
    copy(&exe, &staging.join(EXE))?;
    for (source, target) in RESOURCES {
        copy(&root.join(source), &staging.join(target))?;
    }
    let marker = staging.join(MARKER);
    fs::write(&marker, b"").with_context(|| format!("cannot write {}", marker.display()))?;
    Ok(())
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_file() {
        bail!("package: {} is missing", source.display());
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "cannot copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn tar_path() -> Result<PathBuf> {
    let system_root = env::var_os("SystemRoot")
        .or_else(|| env::var_os("windir"))
        .context("SystemRoot is not set, so the Windows archive tool cannot be found")?;
    let tar = PathBuf::from(system_root).join("System32").join("tar.exe");
    if !tar.is_file() {
        bail!(
            "{} is missing, so the portable zip cannot be created",
            tar.display()
        );
    }
    Ok(tar)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeMap;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde_json::Value;

    use super::{
        check_sources, installer_path, stage_portable, Config, EXE, MARKER, NSIS_DIR, RELEASE_EXE,
        RESOURCES,
    };

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    fn scratch(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "snapline-package-test-{}-{name}",
            std::process::id()
        ))
    }

    fn config() -> Value {
        let path = root().join("apps/snapline/tauri.conf.json");
        let text = fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn bundle_config_is_a_current_user_nsis_installer() {
        let config = config();
        assert_eq!(config["productName"].as_str(), Some("Snapline"));
        assert_eq!(
            config["identifier"].as_str(),
            Some("io.github.aaronburt.snapline")
        );
        assert_eq!(config["bundle"]["active"].as_bool(), Some(true));
        assert_eq!(config["bundle"]["targets"], serde_json::json!(["nsis"]));
        let windows = &config["bundle"]["windows"];
        assert_eq!(
            windows["webviewInstallMode"]["type"].as_str(),
            Some("embedBootstrapper")
        );
        assert_eq!(windows["nsis"]["installMode"].as_str(), Some("currentUser"));
        assert_eq!(
            windows["nsis"]["installerHooks"].as_str(),
            Some("installer/hooks.nsh")
        );
    }

    #[test]
    fn bundle_resources_match_the_packaged_files() {
        let config = config();
        let resources: BTreeMap<String, String> =
            serde_json::from_value(config["bundle"]["resources"].clone()).unwrap();
        let expected: BTreeMap<String, String> = RESOURCES
            .iter()
            .map(|(source, target)| (format!("../../{source}"), (*target).to_string()))
            .collect();
        assert_eq!(resources, expected);
    }

    #[test]
    fn packaged_resources_include_ffmpeg_and_both_licences() {
        let targets: Vec<&str> = RESOURCES.iter().map(|(_, target)| *target).collect();
        assert_eq!(
            targets,
            vec!["ffmpeg.exe", "LICENSE", "FFmpeg-LICENSE.txt", "SOURCE.txt"]
        );
    }

    #[test]
    fn portable_staging_has_the_app_files_and_an_empty_marker() {
        let base = scratch("portable");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("target/release")).unwrap();
        fs::write(base.join(RELEASE_EXE), b"exe").unwrap();
        for (source, _) in RESOURCES {
            let path = base.join(source);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        let staging = base.join("target/portable/Snapline");
        stage_portable(&base, &staging).unwrap();
        assert!(staging.join(EXE).is_file());
        for (_, target) in RESOURCES {
            assert!(staging.join(target).is_file());
        }
        let marker = staging.join(MARKER);
        assert!(marker.is_file());
        assert_eq!(fs::metadata(&marker).unwrap().len(), 0);
        assert!(!staging.join("data").exists());
        assert!(!staging.join("WebView2").exists());
        assert!(!staging.join("settings.toml").exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn preuninstall_hook_runs_cleanup_without_blocking_the_uninstall() {
        let path = root().join("apps/snapline/installer/hooks.nsh");
        let hooks = fs::read_to_string(path).unwrap();
        assert!(hooks.contains("!macro NSIS_HOOK_PREUNINSTALL"));
        assert!(hooks.contains("!macroend"));
        assert!(hooks.contains("$UpdateMode <> 1"));
        assert!(hooks.contains("CheckIfAppIsRunning \"$INSTDIR\\snapline.exe\""));
        assert!(hooks.contains("\"$INSTDIR\\snapline.exe\" --uninstall-cleanup"));
        assert!(hooks.contains("ExecWait"));
        assert!(!hooks.contains("Abort"));
        assert!(hooks.contains("!define MUI_FINISHPAGE_RUN_TEXT \"Launch Snapline\""));
        assert!(hooks.contains("SetLnkAppUserModelId"));
        assert!(hooks.contains("$LOCALAPPDATA\\Programs\\${PRODUCTNAME}"));
    }

    #[test]
    fn missing_ffmpeg_names_the_fetch_command() {
        let base = scratch("missing");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let error = check_sources(&base).unwrap_err().to_string();
        assert!(error.contains("cargo xtask fetch-ffmpeg"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn installer_path_is_found_from_the_bundle_directory() {
        let base = scratch("installer");
        let _ = fs::remove_dir_all(&base);
        let directory = base.join(NSIS_DIR);
        fs::create_dir_all(&directory).unwrap();
        let expected = directory.join("Snapline_0.1.0_x64-setup.exe");
        fs::write(&expected, b"setup").unwrap();
        let config = Config {
            product_name: "Snapline".to_string(),
            version: "0.1.0".to_string(),
        };
        assert_eq!(installer_path(&base, &config).unwrap(), expected);
        fs::remove_dir_all(&base).unwrap();
    }
}
