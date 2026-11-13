use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::SavedRecording;

pub const RECORDING_SAVED_TITLE: &str = "Recording saved";

pub fn recording_saved_notice(
    saved: &SavedRecording,
    thumbnail: Option<PathBuf>,
) -> snapline_core::ports::RichNotice {
    use crate::saver::{capture_tag, file_url};
    use snapline_core::ports::{NoticeButton, RichNotice};

    let video = file_url(&saved.path);
    let mut buttons = vec![NoticeButton::open("Open video", video.clone())];
    if let Some(folder) = saved.path.parent() {
        buttons.push(NoticeButton::open("Open folder", file_url(folder)));
    }
    RichNotice {
        title: RECORDING_SAVED_TITLE.to_owned(),
        lines: vec![
            saved.file_name.clone(),
            format!(
                "{}, {}",
                format_bytes(saved.bytes),
                format_duration(saved.duration_ms)
            ),
        ],
        image: thumbnail,
        open: Some(video),
        buttons,
        tag: saved.capture_id.map(capture_tag),
    }
}

pub fn recording_upload_failed_notice(
    saved: &SavedRecording,
    reason: &str,
    thumbnail: Option<PathBuf>,
) -> snapline_core::ports::RichNotice {
    let mut notice = recording_saved_notice(saved, thumbnail);
    notice.title = crate::uploads::UPLOAD_FAILED_TITLE.to_owned();
    notice.lines = vec![saved.file_name.clone(), reason.to_owned()];
    notice
}

fn format_bytes(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes >= 1_048_576.0 {
        format!("{:.1} MB", bytes / 1_048_576.0)
    } else {
        format!("{:.0} KB", (bytes / 1024.0).max(1.0))
    }
}

fn format_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

static LAST_REPORTED: Mutex<Option<(String, Instant)>> = Mutex::new(None);
const REPORTED_FOR: Duration = Duration::from_secs(10);

pub(super) fn mark_reported(message: &str) {
    let mut last = LAST_REPORTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *last = Some((message.to_owned(), Instant::now()));
}

/// True when a recording failure with this text was just shown to the user, so callers further up
/// (the action dispatcher, the workflow runner) do not show a second notification for it.
pub fn was_reported(text: &str) -> bool {
    let last = LAST_REPORTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    last.as_ref().is_some_and(|(message, at)| {
        at.elapsed() < REPORTED_FOR && !message.is_empty() && text.contains(message.as_str())
    })
}
