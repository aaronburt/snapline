use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

const KEEP: usize = 7;
const FILE_PREFIX: &str = "snapline.";
const FILE_SUFFIX: &str = ".log";

static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub fn log_directory_in(local_app_data: &Path) -> PathBuf {
    local_app_data.join("SnapLine").join("logs")
}

pub fn log_directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| log_directory_in(&root))
}

pub fn prune_logs(directory: &Path, keep: usize) -> io::Result<usize> {
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(FILE_PREFIX)
            && name.ends_with(FILE_SUFFIX)
            && entry.file_type()?.is_file()
        {
            files.push(entry.path());
        }
    }
    if files.len() <= keep {
        return Ok(0);
    }
    files.sort();
    let excess = files.len() - keep;
    let mut removed = 0;
    for path in files.into_iter().take(excess) {
        fs::remove_file(path)?;
        removed += 1;
    }
    Ok(removed)
}

pub fn init() {
    let Some(directory) = log_directory() else {
        eprintln!(
            "Snapline could not start file logging because LOCALAPPDATA is not set, so logs go to the console."
        );
        init_console();
        return;
    };
    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!(
            "Snapline could not create the log folder at {}, so logs go to the console: {error}",
            directory.display()
        );
        init_console();
        return;
    }
    if let Err(error) = prune_logs(&directory, KEEP) {
        eprintln!("Snapline could not tidy the old log files: {error}");
    }
    let appender = match RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("snapline")
        .filename_suffix("log")
        .max_log_files(KEEP)
        .build(&directory)
    {
        Ok(appender) => appender,
        Err(error) => {
            eprintln!(
                "Snapline could not open a log file in {}, so logs go to the console: {error}",
                directory.display()
            );
            init_console();
            return;
        }
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = GUARD.set(guard);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_ansi(false)
        .with_writer(writer)
        .finish();
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        eprintln!("Snapline could not install the log writer, so logs go to the console.");
    }
}

fn init_console() {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_writer(io::stderr)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("snapline-log-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        if let Err(error) = fs::create_dir_all(&path) {
            panic!("could not make a temporary folder: {error}");
        }
        path
    }

    #[test]
    fn log_directory_is_under_local_app_data() {
        let root = PathBuf::from("C:\\Users\\tester\\AppData\\Local");
        assert_eq!(log_directory_in(&root), root.join("SnapLine").join("logs"));
    }

    #[test]
    fn pruning_keeps_the_seven_newest_daily_files() {
        let directory = temporary_directory("prune");
        for day in 1..=10_u32 {
            let name = format!("snapline.2026-11-{day:02}.log");
            if let Err(error) = fs::write(directory.join(name), b"test") {
                panic!("could not write a log file: {error}");
            }
        }
        if let Err(error) = fs::write(directory.join("notes.txt"), b"test") {
            panic!("could not write an unrelated file: {error}");
        }

        match prune_logs(&directory, KEEP) {
            Ok(removed) => assert_eq!(removed, 3),
            Err(error) => panic!("could not prune the log files: {error}"),
        }

        let remaining = match fs::read_dir(&directory) {
            Ok(entries) => entries.count(),
            Err(error) => panic!("could not list the temporary folder: {error}"),
        };
        assert_eq!(remaining, 8);
        for day in 1..=3_u32 {
            assert!(!directory
                .join(format!("snapline.2026-11-{day:02}.log"))
                .exists());
        }
        for day in 4..=10_u32 {
            assert!(directory
                .join(format!("snapline.2026-11-{day:02}.log"))
                .exists());
        }
        assert!(directory.join("notes.txt").exists());

        match prune_logs(&directory, KEEP) {
            Ok(removed) => assert_eq!(removed, 0),
            Err(error) => panic!("could not prune the log files a second time: {error}"),
        }

        let _ = fs::remove_dir_all(&directory);
    }
}
