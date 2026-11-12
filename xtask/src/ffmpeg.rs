use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::config::IpFamily;

const FFMPEG: &str = "ffmpeg.exe";
const LICENCE: &str = "LICENSE";
const SOURCE: &str = "SOURCE.txt";
const STAMP: &str = "archive.sha256";
const OUTPUT: &str = "third_party/ffmpeg";
const CHUNK: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
struct LockFile {
    url: String,
    version: String,
    released: String,
    sha256: String,
    archive: String,
    build_page: String,
    source_url: String,
    licence: String,
    licence_url: String,
}

struct Scratch {
    path: PathBuf,
    directory: bool,
}

impl Scratch {
    fn file(path: PathBuf) -> Self {
        Self {
            path,
            directory: false,
        }
    }

    fn directory(path: PathBuf) -> Self {
        let _ = fs::remove_dir_all(&path);
        Self {
            path,
            directory: true,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.directory {
            let _ = fs::remove_dir_all(&self.path);
        } else {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn run(root: &Path) -> Result<()> {
    let lock = read_lock(root)?;
    let output = root.join(OUTPUT);
    if installed(&output, &lock.sha256) {
        println!(
            "fetch-ffmpeg: FFmpeg {} is already installed in {}",
            lock.version,
            output.display()
        );
        return Ok(());
    }
    println!(
        "fetch-ffmpeg: downloading FFmpeg {} from {}",
        lock.version, lock.url
    );
    let download =
        Scratch::file(env::temp_dir().join(format!("snapline-ffmpeg-{}.zip", std::process::id())));
    let measured = download_archive(&lock.url, download.path())?;
    if !verify_digest(&measured, &lock.sha256) {
        bail!(
            "the archive downloaded from {} has SHA-256 {measured}, but {} expects {}; nothing was installed",
            lock.url,
            lock_file(root).display(),
            lock.sha256
        );
    }
    println!("fetch-ffmpeg: verified the SHA-256 of the archive");
    let staging = Scratch::directory(
        env::temp_dir().join(format!("snapline-ffmpeg-unpack-{}", std::process::id())),
    );
    extract(&lock, download.path(), staging.path())?;
    install(&output, staging.path(), &lock)?;
    println!(
        "fetch-ffmpeg: installed FFmpeg {} into {}",
        lock.version,
        output.display()
    );
    Ok(())
}

fn lock_file(root: &Path) -> PathBuf {
    root.join("xtask").join("ffmpeg.lock")
}

fn read_lock(root: &Path) -> Result<LockFile> {
    let path = lock_file(root);
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    parse_lock(&text).with_context(|| format!("{} is not a usable FFmpeg lock", path.display()))
}

fn parse_lock(text: &str) -> Result<LockFile> {
    let lock: LockFile = toml::from_str(text)?;
    if lock.version.trim().is_empty() {
        bail!("the lock file has no version");
    }
    if !lock.url.starts_with("https://") {
        bail!("the lock file URL is not HTTPS");
    }
    if !lock.source_url.starts_with("https://") {
        bail!("the lock file source URL is not HTTPS");
    }
    if !is_sha256(&lock.sha256) {
        bail!("the lock file SHA-256 is not 64 lower-case hexadecimal digits");
    }
    if lock.archive.trim().is_empty() {
        bail!("the lock file has no archive folder name");
    }
    Ok(lock)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn verify_digest(measured: &str, expected: &str) -> bool {
    measured == expected
}

fn installed(directory: &Path, sha256: &str) -> bool {
    for name in [FFMPEG, LICENCE, SOURCE, STAMP] {
        if !directory.join(name).is_file() {
            return false;
        }
    }
    fs::read_to_string(directory.join(STAMP)).is_ok_and(|text| text.trim() == sha256)
}

fn download_archive(url: &str, path: &Path) -> Result<String> {
    let ipv4 = retrieve(url, path, IpFamily::Ipv4Only);
    if let Err(error) = &ipv4 {
        println!("fetch-ffmpeg: the IPv4 download failed ({error}), so trying any address instead");
    }
    ipv4.or_else(|_| {
        retrieve(url, path, IpFamily::Any).with_context(|| format!("could not download {url}"))
    })
}

fn retrieve(url: &str, path: &Path, family: IpFamily) -> Result<String> {
    let config = ureq::Agent::config_builder().ip_family(family).build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent
        .get(url)
        .call()
        .with_context(|| format!("could not download {url}"))?;
    let mut reader = response.body_mut().as_reader();
    let file = File::create(path).with_context(|| format!("cannot write {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("could not read the download from {url}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .with_context(|| format!("cannot write {}", path.display()))?;
        total += read as u64;
    }
    writer
        .flush()
        .with_context(|| format!("cannot write {}", path.display()))?;
    println!("fetch-ffmpeg: downloaded {total} bytes");
    Ok(hex(hasher.finalize().as_slice()))
}

fn tar_path() -> Result<PathBuf> {
    let system_root = env::var_os("SystemRoot")
        .or_else(|| env::var_os("windir"))
        .context("SystemRoot is not set, so the Windows archive tool cannot be found")?;
    let tar = PathBuf::from(system_root).join("System32").join("tar.exe");
    if !tar.is_file() {
        bail!(
            "{} is missing, so the FFmpeg archive cannot be unpacked",
            tar.display()
        );
    }
    Ok(tar)
}

fn archive_members(archive: &str) -> Vec<String> {
    vec![
        format!("{archive}/bin/{FFMPEG}"),
        format!("{archive}/{LICENCE}"),
    ]
}

fn extract(lock: &LockFile, archive: &Path, staging: &Path) -> Result<()> {
    let tar = tar_path()?;
    fs::create_dir_all(staging).with_context(|| format!("cannot create {}", staging.display()))?;
    println!("fetch-ffmpeg: unpacking the archive with {}", tar.display());
    let status = Command::new(&tar)
        .arg("-x")
        .arg("-f")
        .arg(archive)
        .arg("-C")
        .arg(staging)
        .args(archive_members(&lock.archive))
        .status()
        .with_context(|| format!("could not start {}", tar.display()))?;
    if !status.success() {
        bail!(
            "{} could not unpack {} ({status})",
            tar.display(),
            archive.display()
        );
    }
    Ok(())
}

fn source_text(lock: &LockFile) -> String {
    format!(
        "FFmpeg {} for Windows x64, bundled with Snapline as a separate program.\n\
         \n\
         Archive: {}\n\
         Archive SHA-256: {}\n\
         Released: {}\n\
         Licence: {} ({})\n\
         Source code: {}\n\
         Build page: {}\n",
        lock.version,
        lock.url,
        lock.sha256,
        lock.released,
        lock.licence,
        lock.licence_url,
        lock.source_url,
        lock.build_page
    )
}

fn install(output: &Path, staging: &Path, lock: &LockFile) -> Result<()> {
    let unpacked = staging.join(&lock.archive);
    let exe = unpacked.join("bin").join(FFMPEG);
    let licence = unpacked.join(LICENCE);
    if !exe.is_file() {
        bail!("the archive did not contain {}", exe.display());
    }
    if !licence.is_file() {
        bail!("the archive did not contain {}", licence.display());
    }
    fs::create_dir_all(output).with_context(|| format!("cannot create {}", output.display()))?;
    let installed_exe = output.join(FFMPEG);
    fs::copy(&exe, &installed_exe)
        .with_context(|| format!("cannot write {}", installed_exe.display()))?;
    let installed_licence = output.join(LICENCE);
    fs::copy(&licence, &installed_licence)
        .with_context(|| format!("cannot write {}", installed_licence.display()))?;
    let source = output.join(SOURCE);
    fs::write(&source, source_text(lock))
        .with_context(|| format!("cannot write {}", source.display()))?;
    let stamp = output.join(STAMP);
    fs::write(&stamp, format!("{}\n", lock.sha256))
        .with_context(|| format!("cannot write {}", stamp.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::env;
    use std::fs;

    use super::{
        archive_members, hex, installed, is_sha256, parse_lock, source_text, verify_digest,
        Scratch, FFMPEG, LICENCE, SOURCE, STAMP,
    };
    use sha2::{Digest, Sha256};

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn lock_text() -> &'static str {
        include_str!("../ffmpeg.lock")
    }

    fn scratch(name: &str) -> Scratch {
        Scratch::directory(env::temp_dir().join(format!(
            "snapline-ffmpeg-test-{}-{name}",
            std::process::id()
        )))
    }

    #[test]
    fn lock_file_parses() {
        let lock = parse_lock(lock_text()).expect("the lock file should parse");
        assert!(!lock.version.is_empty());
        assert_eq!(
            lock.url,
            format!(
                "https://github.com/GyanD/codexffmpeg/releases/download/{}/{}.zip",
                lock.version, lock.archive
            )
        );
        assert!(lock
            .source_url
            .starts_with("https://github.com/FFmpeg/FFmpeg/"));
        assert!(is_sha256(&lock.sha256));
        assert!(!lock.archive.is_empty());
    }

    #[test]
    fn sha256_needs_sixty_four_lower_case_digits() {
        assert!(is_sha256(EMPTY_SHA256));
        assert!(!is_sha256(&EMPTY_SHA256.to_uppercase()));
        assert!(!is_sha256(&EMPTY_SHA256[..63]));
        assert!(!is_sha256(&format!("{EMPTY_SHA256}0")));
        assert!(!is_sha256(
            "g3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
    }

    #[test]
    fn empty_digest_matches_the_known_vector() {
        assert_eq!(hex(Sha256::digest(b"").as_slice()), EMPTY_SHA256);
        assert!(verify_digest(
            &hex(Sha256::digest(b"").as_slice()),
            EMPTY_SHA256
        ));
        assert!(!verify_digest("x", EMPTY_SHA256));
    }

    #[test]
    fn hex_pads_each_byte() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn expected_archive_layout() {
        assert_eq!(
            archive_members("ffmpeg-test_build"),
            vec![
                "ffmpeg-test_build/bin/ffmpeg.exe".to_string(),
                "ffmpeg-test_build/LICENSE".to_string(),
            ]
        );
    }

    #[test]
    fn install_needs_every_file_and_the_matching_stamp() {
        let directory = scratch("install");
        fs::create_dir_all(directory.path()).unwrap();
        assert!(!installed(directory.path(), EMPTY_SHA256));
        for name in [FFMPEG, LICENCE, SOURCE] {
            fs::write(directory.path().join(name), b"x").unwrap();
        }
        assert!(!installed(directory.path(), EMPTY_SHA256));
        fs::write(directory.path().join(STAMP), format!("{EMPTY_SHA256}\n")).unwrap();
        assert!(installed(directory.path(), EMPTY_SHA256));
        fs::write(directory.path().join(STAMP), "00").unwrap();
        assert!(!installed(directory.path(), EMPTY_SHA256));
        fs::write(directory.path().join(STAMP), format!("{EMPTY_SHA256}\n")).unwrap();
        fs::remove_file(directory.path().join(SOURCE)).unwrap();
        assert!(!installed(directory.path(), EMPTY_SHA256));
    }

    #[test]
    fn source_file_names_the_pin() {
        let lock = parse_lock(lock_text()).unwrap();
        let text = source_text(&lock);
        assert!(text.contains(&lock.version));
        assert!(text.contains(&lock.url));
        assert!(text.contains(&lock.sha256));
        assert!(text.contains(&lock.source_url));
        assert!(text.contains(&lock.licence));
    }
}
