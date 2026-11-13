use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

const APP_MANIFEST: &str = "apps/snapline/Cargo.toml";
const WORKSPACE_MANIFEST: &str = "Cargo.toml";
const SCRIPT: &str = "apps/snapline/installer/snapline.nsi";
const PRODUCT: &str = "Snapline";
const EXE: &str = "snapline.exe";
const RELEASE_EXE: &str = "target/release/snapline.exe";
const NSIS_DIR: &str = "target/release/bundle/nsis";
const PORTABLE_DIR: &str = "target/portable";
const MARKER: &str = "portable";
const RESOURCES: [(&str, &str); 6] = [
    ("third_party/ffmpeg/ffmpeg.exe", "ffmpeg.exe"),
    ("LICENSE", "LICENSE"),
    ("third_party/ffmpeg/LICENSE", "FFmpeg-LICENSE.txt"),
    ("third_party/ffmpeg/SOURCE.txt", "SOURCE.txt"),
    ("assets/sounds/shutter.wav", "assets/sounds/shutter.wav"),
    ("assets/sounds/done.wav", "assets/sounds/done.wav"),
];

#[derive(Debug)]
struct Config {
    product_name: String,
    version: String,
}

pub fn run(root: &Path) -> Result<()> {
    check_sources(root)?;
    let config = read_config(root)?;
    build_release(root)?;
    build_installer(root, &config)?;
    let installer = installer_path(root, &config)?;
    println!("package: installer {}", installer.display());
    let zip = portable_zip(root, &config)?;
    println!("package: portable zip {}", zip.display());
    Ok(())
}

fn read_config(root: &Path) -> Result<Config> {
    let app = read_toml(&root.join(APP_MANIFEST))?;
    let version = match app
        .get("package")
        .and_then(|package| package.get("version"))
    {
        Some(toml::Value::String(version)) => version.clone(),
        _ => {
            let workspace = read_toml(&root.join(WORKSPACE_MANIFEST))?;
            workspace
                .get("workspace")
                .and_then(|table| table.get("package"))
                .and_then(|package| package.get("version"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .context("package: the workspace manifest has no package version")?
        }
    };
    Ok(Config {
        product_name: PRODUCT.to_owned(),
        version,
    })
}

fn read_toml(path: &Path) -> Result<toml::Value> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    text.parse::<toml::Value>()
        .with_context(|| format!("{} is not valid TOML", path.display()))
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

fn build_release(root: &Path) -> Result<()> {
    println!("> cargo build --release -p snapline");
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "snapline"])
        .current_dir(root)
        .status()
        .context("could not start cargo")?;
    if !status.success() {
        bail!("package: the release build failed with {status}");
    }
    Ok(())
}

fn makensis_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            candidates.push(directory.join("makensis.exe"));
            candidates.push(directory.join("makensis"));
        }
    }
    if let Some(base) = env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(base).join("tauri/NSIS/makensis.exe"));
    }
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(base) = env::var_os(variable) {
            candidates.push(PathBuf::from(base).join("NSIS/makensis.exe"));
        }
    }
    candidates
}

fn find_makensis() -> Result<PathBuf> {
    makensis_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            anyhow!(
                "package: makensis was not found; install NSIS with `winget install NSIS.NSIS` and run the build again"
            )
        })
}

fn nsis_arguments(root: &Path, config: &Config, output: &Path) -> Vec<OsString> {
    vec![
        OsString::from("/V2"),
        OsString::from(format!("/DROOT={}", root.display())),
        OsString::from(format!("/DOUTFILE={}", output.display())),
        OsString::from(format!("/DVERSION={}", config.version)),
        root.join(SCRIPT).into_os_string(),
    ]
}

fn build_installer(root: &Path, config: &Config) -> Result<()> {
    let directory = root.join(NSIS_DIR);
    fs::create_dir_all(&directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;
    let output = directory.join(installer_name(config));
    let makensis = find_makensis()?;
    let arguments = nsis_arguments(root, config, &output);
    println!("> {} {}", makensis.display(), SCRIPT);
    let status = Command::new(&makensis)
        .args(&arguments)
        .current_dir(root)
        .status()
        .with_context(|| format!("could not start {}", makensis.display()))?;
    if !status.success() {
        bail!("package: makensis failed with {status}");
    }
    Ok(())
}

fn installer_name(config: &Config) -> String {
    format!("{}_{}_x64-setup.exe", config.product_name, config.version)
}

fn installer_path(root: &Path, config: &Config) -> Result<PathBuf> {
    let directory = root.join(NSIS_DIR);
    if !directory.is_dir() {
        bail!(
            "package: the NSIS bundle directory {} is missing; makensis did not produce it",
            directory.display()
        );
    }
    let expected = directory.join(installer_name(config));
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
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
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

    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        check_sources, installer_path, nsis_arguments, read_config, stage_portable, Config, EXE,
        MARKER, NSIS_DIR, RELEASE_EXE, RESOURCES, SCRIPT,
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

    fn script() -> String {
        fs::read_to_string(root().join(SCRIPT)).unwrap()
    }

    #[test]
    fn the_installer_script_is_a_current_user_install_to_the_programs_folder() {
        let script = script();
        assert!(script.contains("RequestExecutionLevel user"));
        assert!(script.contains("InstallDir \"$LOCALAPPDATA\\Programs\\${PRODUCT}\""));
        assert!(script.contains("!define APP_ID \"io.github.aaronburt.snapline\""));
        assert!(script.contains("!define MUI_FINISHPAGE_RUN_TEXT \"Launch Snapline\""));
    }

    #[test]
    fn the_installer_script_installs_every_packaged_resource() {
        let script = script();
        for (source, target) in RESOURCES {
            let source = source.replace('/', "\\");
            assert!(script.contains(&source), "{source} is not installed");
            let file = target.rsplit('/').next().unwrap();
            assert!(script.contains(file), "{file} is not installed");
        }
        assert!(script.contains("${ROOT}\\target\\release\\${EXE}"));
    }

    #[test]
    fn the_app_version_comes_from_the_manifests() {
        let config = read_config(&root()).unwrap();
        assert_eq!(config.product_name, "Snapline");
        assert!(!config.version.is_empty());
    }

    #[test]
    fn packaged_resources_include_ffmpeg_and_both_licences() {
        let targets: Vec<&str> = RESOURCES.iter().map(|(_, target)| *target).collect();
        assert_eq!(
            targets,
            vec![
                "ffmpeg.exe",
                "LICENSE",
                "FFmpeg-LICENSE.txt",
                "SOURCE.txt",
                "assets/sounds/shutter.wav",
                "assets/sounds/done.wav"
            ]
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
        assert!(!staging.join("settings.toml").exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_uninstaller_runs_cleanup_without_blocking_the_uninstall() {
        let script = script();
        assert!(script.contains("Section \"Uninstall\""));
        assert!(script.contains("--uninstall-cleanup"));
        assert!(script.contains("ExecWait"));
        assert!(script.contains("--set-shortcut-app-id"));
    }

    #[test]
    fn makensis_gets_the_script_and_the_package_names() {
        let config = Config {
            product_name: "Snapline".to_string(),
            version: "0.1.0".to_string(),
        };
        let output = Path::new("out/Snapline_0.1.0_x64-setup.exe");
        let arguments = nsis_arguments(Path::new("repo"), &config, output);
        let text: Vec<String> = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert!(text.contains(&"/DVERSION=0.1.0".to_owned()));
        assert!(text
            .iter()
            .any(|argument| argument.starts_with("/DOUTFILE=")
                && argument.ends_with("x64-setup.exe")));
        assert!(text.last().unwrap().ends_with("snapline.nsi"));
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
