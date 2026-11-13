use time::OffsetDateTime;

pub type Pairs = &'static [(&'static str, &'static str)];

pub const IMAGE_FORMAT: Pairs = &[("png", "PNG"), ("jpeg", "JPEG"), ("webp", "WebP")];
pub const LINK_FORMAT: Pairs = &[
    ("direct", "Direct link"),
    ("markdown", "Markdown"),
    ("html", "HTML <img>"),
    ("bbcode", "BBCode"),
    ("page", "Page link"),
];
pub const CAPTURE_DELAY: Pairs = &[("off", "Off"), ("1s", "1 s"), ("3s", "3 s"), ("5s", "5 s")];
pub const MAGNIFIER_ZOOM: Pairs = &[("2x", "2×"), ("4x", "4×"), ("8x", "8×")];
pub const MONITOR: Pairs = &[
    ("all_monitors", "All monitors"),
    ("monitor_under_cursor", "Monitor under cursor"),
    ("primary_monitor", "Primary monitor"),
];
pub const VIDEO_FORMAT: Pairs = &[
    ("mp4_h264", "MP4 · H.264"),
    ("mp4_h265_hevc", "MP4 · H.265 (HEVC)"),
    ("webm_vp9", "WebM · VP9"),
    ("mkv_h264", "MKV · H.264"),
];
pub const FRAME_RATE: Pairs = &[("fps24", "24"), ("fps30", "30"), ("fps60", "60")];
pub const QUALITY: Pairs = &[
    ("high", "High (recommended)"),
    ("balanced", "Balanced"),
    ("small_file", "Small file"),
    ("lossless", "Lossless"),
];
pub const GIF_FRAME_RATE: Pairs = &[
    ("fps10", "10"),
    ("fps15", "15"),
    ("fps20", "20"),
    ("fps30", "30"),
];
pub const GIF_WIDTH: Pairs = &[
    ("original", "Original size"),
    ("px1280", "1280 px wide"),
    ("px800", "800 px wide"),
    ("px480", "480 px wide"),
];

pub fn subfolder_pairs() -> Vec<(&'static str, String)> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let date = now.date();
    vec![
        (
            "by_month",
            format!("By month ({:04}-{:02})", date.year(), date.month() as u8),
        ),
        (
            "by_day",
            format!(
                "By day ({:04}-{:02}-{:02})",
                date.year(),
                date.month() as u8,
                date.day()
            ),
        ),
        ("by_app", "By app (chrome)".to_string()),
        ("none", "No subfolders".to_string()),
    ]
}

pub fn label(pairs: &[(&str, impl AsRef<str>)], value: &str, fallback: &str) -> String {
    pairs
        .iter()
        .find(|(stored, _)| *stored == value)
        .map(|(_, label)| label.as_ref().to_string())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn value(pairs: &[(&str, impl AsRef<str>)], label_text: &str, fallback: &str) -> String {
    pairs
        .iter()
        .find(|(_, shown)| shown.as_ref() == label_text)
        .map(|(stored, _)| (*stored).to_string())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn labels(pairs: &[(&str, impl AsRef<str>)]) -> Vec<String> {
    pairs
        .iter()
        .map(|(_, shown)| shown.as_ref().to_string())
        .collect()
}
