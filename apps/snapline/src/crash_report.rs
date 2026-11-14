//! Turns a saved crash report into a prefilled GitHub issue link.
//!
//! Nothing here sends anything. The caller opens the link in the browser and the
//! person reviews the issue and presses Submit. Personal paths and user names are
//! removed before the text goes into the link.

use std::fs;
use std::path::{Path, PathBuf};

const ISSUE_URL: &str = "https://github.com/aaronburt/snapline/issues/new";
const MAX_URL_LENGTH: usize = 7000;
const REPORT_EXTENSION: &str = "txt";
const REPORTED_EXTENSION: &str = "reported";
const LOG_LINES: usize = 40;
const USER_PLACEHOLDER: &str = "<user>";

/// The newest crash report that has not been offered to the person yet.
pub fn pending_report(directory: &Path) -> Option<PathBuf> {
    let mut newest: Option<(u64, PathBuf)> = None;
    for entry in fs::read_dir(directory).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some(REPORT_EXTENSION) {
            continue;
        }
        let Some(stamp) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        if newest.as_ref().map_or(true, |(best, _)| stamp > *best) {
            newest = Some((stamp, path));
        }
    }
    newest.map(|(_, path)| path)
}

/// Stops the report from being offered again. The file stays on disk.
pub fn mark_reported(path: &Path) -> std::io::Result<PathBuf> {
    let target = path.with_extension(format!("{REPORT_EXTENSION}.{REPORTED_EXTENSION}"));
    fs::rename(path, &target)?;
    Ok(target)
}

/// Marks every other waiting report as handled so only the newest is offered.
pub fn mark_older_reported(directory: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path != keep && path.extension().and_then(|value| value.to_str()) == Some(REPORT_EXTENSION)
        {
            let _ = mark_reported(&path);
        }
    }
}

/// The last lines of the newest log file, or an empty string when there is none.
pub fn recent_log_lines(log_directory: &Path) -> String {
    let Ok(entries) = fs::read_dir(log_directory) else {
        return String::new();
    };
    let newest = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("snapline.") && name.ends_with(".log"))
        })
        .max();
    let Some(path) = newest else {
        return String::new();
    };
    let Ok(bytes) = fs::read(path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(LOG_LINES);
    lines[start..].join("\n")
}

/// Removes the Windows user name from paths and from any other place it appears.
pub fn scrub(text: &str, user_name: Option<&str>) -> String {
    let mut result = scrub_user_folders(text);
    if let Some(name) = user_name.filter(|name| name.chars().count() >= 3) {
        result = replace_ignore_ascii_case(&result, name, USER_PLACEHOLDER);
    }
    result
}

fn scrub_user_folders(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = find_users_marker(&lower, cursor) {
        let (marker_end, _) = found;
        output.push_str(&text[cursor..marker_end]);
        let rest = &text[marker_end..];
        let name_length = rest
            .find(|character: char| {
                matches!(character, '\\' | '/' | '"' | '\'' | '<' | '>' | '|' | ':')
                    || character.is_whitespace()
            })
            .unwrap_or(rest.len());
        if name_length > 0 {
            output.push_str(USER_PLACEHOLDER);
        }
        cursor = marker_end + name_length;
    }
    output.push_str(&text[cursor..]);
    output
}

/// Finds the next `\users\` or `/users/` at or after `from`, returning where the user name starts.
fn find_users_marker(lower: &str, from: usize) -> Option<(usize, usize)> {
    let haystack = &lower[from..];
    let backslash = haystack.find("\\users\\");
    let slash = haystack.find("/users/");
    let start = match (backslash, slash) {
        (Some(left), Some(right)) => left.min(right),
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => return None,
    };
    let marker_end = from + start + "\\users\\".len();
    Some((marker_end, from + start))
}

fn replace_ignore_ascii_case(text: &str, needle: &str, replacement: &str) -> String {
    let lower_text = text.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(found) = lower_text[cursor..].find(&lower_needle) {
        let start = cursor + found;
        output.push_str(&text[cursor..start]);
        output.push_str(replacement);
        cursor = start + needle.len();
    }
    output.push_str(&text[cursor..]);
    output
}

pub fn percent_encode(text: &str) -> String {
    let mut output = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn headline(report: &str) -> String {
    let field = |name: &str| {
        report
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let message = field("Message:").unwrap_or("unknown");
    let version = field("Version:").unwrap_or("unknown");
    let message: String = message.chars().take(80).collect();
    format!("Crash in {version}: {message}")
}

fn body_for(report: &str) -> String {
    format!(
        "SnapLine crashed and saved this report. Personal paths and the user name were removed before this page opened. Please add anything that helps, such as what you were doing, then press Submit.\n\n```\n{report}\n```\n"
    )
}

/// A new-issue link with the report filled in. The report is cut to keep the link short enough.
pub fn issue_url(report: &str) -> String {
    let title = percent_encode(&headline(report));
    let mut kept: Vec<char> = report.chars().collect();
    loop {
        let shown: String = kept.iter().collect();
        let note = if kept.len() < report.chars().count() {
            "\n[trimmed: the full report is in the SnapLine crashes folder]"
        } else {
            ""
        };
        let url = format!(
            "{ISSUE_URL}?title={title}&body={}",
            percent_encode(&body_for(&format!("{shown}{note}")))
        );
        if url.len() <= MAX_URL_LENGTH || kept.len() < 200 {
            return url;
        }
        let next = kept.len() * 9 / 10;
        kept.truncate(next);
    }
}

/// Reads a saved report and builds the scrubbed issue link for it.
pub fn issue_url_for(path: &Path, user_name: Option<&str>) -> std::io::Result<String> {
    let report = fs::read_to_string(path)?;
    Ok(issue_url(&scrub(&report, user_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "snapline-crash-report-{}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        if let Err(error) = fs::create_dir_all(&path) {
            panic!("could not make a temporary folder: {error}");
        }
        path
    }

    #[test]
    fn scrub_hides_the_user_folder_in_both_slash_styles() {
        let text = "at C:\\Users\\Aaron\\.cargo\\registry\\x.rs:1 and C:/Users/Aaron/src/y.rs";
        let clean = scrub(text, None);
        assert!(!clean.contains("Aaron"));
        assert!(clean.contains("C:\\Users\\<user>\\.cargo"));
        assert!(clean.contains("C:/Users/<user>/src"));
    }

    #[test]
    fn scrub_hides_the_user_name_anywhere() {
        let clean = scrub("host AARONS-PC user aaronburt", Some("aaronburt"));
        assert_eq!(clean, "host AARONS-PC user <user>");
    }

    #[test]
    fn scrub_ignores_very_short_user_names() {
        assert_eq!(scrub("a b c", Some("a")), "a b c");
    }

    #[test]
    fn scrub_keeps_text_without_user_folders() {
        assert_eq!(scrub("plain text", None), "plain text");
    }

    #[test]
    fn percent_encode_escapes_everything_unsafe() {
        assert_eq!(percent_encode("a b&c#d\n"), "a%20b%26c%23d%0A");
        assert_eq!(percent_encode("é"), "%C3%A9");
    }

    #[test]
    fn issue_url_has_a_title_and_body() {
        let report = "Snapline crash report\nVersion: 0.1.0\nMessage: boom\nBacktrace:\nframe";
        let url = issue_url(report);
        assert!(url.starts_with("https://github.com/aaronburt/snapline/issues/new?title="));
        assert!(url.contains("Crash%20in%200.1.0%3A%20boom"));
        assert!(url.contains("&body="));
        assert!(!url.contains(' '));
    }

    #[test]
    fn issue_url_trims_long_reports() {
        let report = format!("Version: 0.1.0\nMessage: boom\n{}", "frame line\n".repeat(5000));
        let url = issue_url(&report);
        assert!(url.len() <= MAX_URL_LENGTH);
        assert!(url.contains("trimmed"));
    }

    #[test]
    fn pending_report_picks_the_newest_and_skips_reported_ones() {
        let directory = temporary_directory("pending");
        for stamp in ["100", "300", "200"] {
            let _ = fs::write(directory.join(format!("{stamp}.txt")), "x");
        }
        let _ = fs::write(directory.join("notes.txt"), "x");
        let newest = pending_report(&directory);
        assert_eq!(newest.as_deref(), Some(directory.join("300.txt").as_path()));
        let Some(newest) = newest else {
            panic!("a report should be waiting");
        };
        mark_older_reported(&directory, &newest);
        assert!(mark_reported(&newest).is_ok());
        assert_eq!(pending_report(&directory), None);
        assert!(directory.join("100.txt.reported").exists());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn recent_log_lines_returns_the_tail_of_the_newest_log() {
        let directory = temporary_directory("logs");
        let _ = fs::write(directory.join("snapline.2026-01-01.log"), "old\n");
        let lines: Vec<String> = (0..100).map(|index| format!("line {index}")).collect();
        let _ = fs::write(directory.join("snapline.2026-01-02.log"), lines.join("\n"));
        let tail = recent_log_lines(&directory);
        assert!(tail.ends_with("line 99"));
        assert!(!tail.contains("old"));
        assert_eq!(tail.lines().count(), LOG_LINES);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn recent_log_lines_is_empty_without_a_log_folder() {
        assert_eq!(recent_log_lines(Path::new("does-not-exist-anywhere")), "");
    }
}
