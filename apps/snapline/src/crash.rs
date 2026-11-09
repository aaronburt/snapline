use std::any::Any;
use std::fmt::Write as _;
use std::io;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
};

const EXIT_CODE: i32 = 101;

static INSTALLED: OnceLock<()> = OnceLock::new();

pub fn crash_directory_in(local_app_data: &Path) -> PathBuf {
    local_app_data.join("SnapLine").join("crashes")
}

pub fn crash_directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| crash_directory_in(&root))
}

pub fn report_path_in(directory: &Path, unix_seconds: u64) -> PathBuf {
    directory.join(format!("{unix_seconds}.txt"))
}

pub fn write_report(directory: &Path, unix_seconds: u64, text: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let path = report_path_in(directory, unix_seconds);
    std::fs::write(&path, text)?;
    Ok(path)
}

pub fn install() {
    if INSTALLED.set(()).is_err() {
        return;
    }
    let directory = crash_directory();
    let interactive = !test_mode();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        run(info, directory.as_deref(), interactive);
    }));
}

#[cfg(test)]
fn install_in(directory: &Path, interactive: bool) {
    let directory = directory.to_path_buf();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        run(info, Some(&directory), interactive);
    }));
}

fn test_mode() -> bool {
    matches!(std::env::var("SNAPLINE_TEST"), Ok(value) if value == "1")
}

fn run(info: &PanicHookInfo<'_>, directory: Option<&Path>, interactive: bool) {
    let now = unix_seconds();
    let text = report_text(info.payload(), info.location(), now);
    let Some(directory) = directory else {
        eprintln!(
            "Snapline could not save a crash report because LOCALAPPDATA is not set.\n{text}"
        );
        if interactive {
            std::process::exit(EXIT_CODE);
        }
        return;
    };
    match write_report(directory, now, &text) {
        Ok(path) => {
            eprintln!("Snapline saved a crash report to {}", path.display());
            if interactive {
                show_message(&path);
                std::process::exit(EXIT_CODE);
            }
        }
        Err(error) => {
            eprintln!("Snapline could not save a crash report: {error}\n{text}");
            if interactive {
                std::process::exit(EXIT_CODE);
            }
        }
    }
}

fn report_text(
    payload: &(dyn Any + Send),
    location: Option<&std::panic::Location<'_>>,
    unix_seconds: u64,
) -> String {
    let mut text = String::with_capacity(4096);
    let _ = writeln!(text, "Snapline crash report");
    let _ = writeln!(text, "Version: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(text, "Windows build: {}", windows_build());
    let _ = writeln!(text, "Time (Unix seconds): {unix_seconds}");
    let _ = writeln!(text, "Message: {}", panic_message(payload));
    match location {
        Some(location) => {
            let _ = writeln!(
                text,
                "Location: {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }
        None => {
            let _ = writeln!(text, "Location: unknown");
        }
    }
    let _ = writeln!(
        text,
        "Backtrace:\n{}",
        std::backtrace::Backtrace::force_capture()
    );
    text
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_owned()
    }
}

fn windows_build() -> String {
    let Some(build) = registry_string(
        w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
        w!("CurrentBuildNumber"),
    ) else {
        return "unknown".to_owned();
    };
    match registry_u32(
        w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
        w!("UBR"),
    ) {
        Some(revision) => format!("{build}.{revision}"),
        None => build,
    }
}

fn registry_string(subkey: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 128];
    let mut bytes = u32::try_from(buffer.len() * std::mem::size_of::<u16>()).ok()?;
    // SAFETY: the buffer is a live, writable array of 128 u16 units and bytes is its size in bytes.
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if result.is_err() {
        return None;
    }
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

fn registry_u32(subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0_u32;
    let mut bytes = u32::try_from(std::mem::size_of::<u32>()).ok()?;
    // SAFETY: the pointer and size describe one live u32 that the call writes in full.
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut data).cast()),
            Some(&mut bytes),
        )
    };
    result.is_ok().then_some(data)
}

fn show_message(path: &Path) {
    let text = format!(
        "Snapline stopped because of a problem. A report was saved to {}.",
        path.display()
    );
    let text: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let caption: Vec<u16> = "Snapline"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both buffers are null-terminated UTF-16 arrays that outlive the call.
    let _ = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("snapline-crash-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        if let Err(error) = std::fs::create_dir_all(&path) {
            panic!("could not make a temporary folder: {error}");
        }
        path
    }

    fn child_directory(process: u32) -> PathBuf {
        std::env::temp_dir().join(format!("snapline-crash-child-{process}"))
    }

    #[test]
    fn report_path_is_one_text_file_per_crash() {
        let root = PathBuf::from("C:\\Users\\tester\\AppData\\Local");
        let directory = crash_directory_in(&root);
        assert_eq!(directory, root.join("SnapLine").join("crashes"));
        assert_eq!(
            report_path_in(&directory, 1_700_000_000),
            directory.join("1700000000.txt")
        );
    }

    #[test]
    fn write_report_creates_the_folder_and_the_file() {
        let base = temporary_directory("write");
        let directory = base.join("nested");
        let path = match write_report(&directory, 1_700_000_000, "test report") {
            Ok(path) => path,
            Err(error) => panic!("could not write the report: {error}"),
        };
        assert!(path.exists());
        let saved = match std::fs::read_to_string(&path) {
            Ok(saved) => saved,
            Err(error) => panic!("could not read the report back: {error}"),
        };
        assert_eq!(saved, "test report");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn forced_panic_writes_a_report() {
        let directory = temporary_directory("panic");
        let payload = std::panic::catch_unwind(|| -> () {
            panic!("forced panic for the crash report test");
        });
        let Err(payload) = payload else {
            panic!("the forced panic should have unwound");
        };
        let text = report_text(payload.as_ref(), None, 1_700_000_000);
        let path = match write_report(&directory, 1_700_000_000, &text) {
            Ok(path) => path,
            Err(error) => panic!("could not write the report: {error}"),
        };
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("1700000000.txt")
        );
        let saved = match std::fs::read_to_string(&path) {
            Ok(saved) => saved,
            Err(error) => panic!("could not read the report back: {error}"),
        };
        assert!(saved.contains("forced panic for the crash report test"));
        assert!(saved.contains(env!("CARGO_PKG_VERSION")));
        assert!(saved.contains("Windows build:"));
        assert!(saved.contains("Backtrace"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn panic_child_process() {
        if std::env::var("SNAPLINE_CRASH_CHILD").is_err() {
            return;
        }
        let directory = child_directory(std::process::id());
        let _ = std::fs::remove_dir_all(&directory);
        install_in(&directory, false);
        panic!("forced panic in a child process");
    }

    #[test]
    fn child_process_panic_writes_a_report() {
        let Ok(executable) = std::env::current_exe() else {
            panic!("could not find the test binary");
        };
        let child = match std::process::Command::new(executable)
            .arg("crash::tests::panic_child_process")
            .arg("--exact")
            .arg("--nocapture")
            .env("SNAPLINE_CRASH_CHILD", "1")
            .env("SNAPLINE_TEST", "1")
            .spawn()
        {
            Ok(child) => child,
            Err(error) => panic!("could not start the child test process: {error}"),
        };
        let directory = child_directory(child.id());
        let mut child = child;
        if let Err(error) = child.wait() {
            panic!("could not wait for the child test process: {error}");
        }
        let mut reports: Vec<PathBuf> = Vec::new();
        match std::fs::read_dir(&directory) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    reports.push(entry.path());
                }
            }
            Err(error) => panic!("the child process wrote no report folder: {error}"),
        }
        assert_eq!(reports.len(), 1, "expected exactly one crash report");
        let saved = match std::fs::read_to_string(&reports[0]) {
            Ok(saved) => saved,
            Err(error) => panic!("could not read the report back: {error}"),
        };
        assert!(saved.contains("forced panic in a child process"));
        assert!(saved.contains("Windows build:"));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
