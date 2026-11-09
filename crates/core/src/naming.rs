use rand::RngExt;
use time::macros::datetime;
use time::OffsetDateTime;

pub const DEFAULT_PATTERN: &str = "{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}";
pub const WINDOW_TITLE_LIMIT: usize = 80;
pub const RANDOM_LEN: usize = 4;
pub const COUNTER_DIGITS: usize = 4;
pub const COUNTER_MAX: u32 = 9999;
pub const DESKTOP_APP: &str = "Desktop";
pub const SAMPLE_APP: &str = "chrome";
pub const SAMPLE_WINDOW: &str = "Pull-request-42";
pub const SAMPLE_RAND: &str = "k3f9";
pub const SAMPLE_COUNTER: u32 = 42;
pub const SAMPLE_WIDTH: u32 = 1280;
pub const SAMPLE_HEIGHT: u32 = 720;

const RANDOM_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const INVALID_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameFormat {
    Png,
    Jpeg,
    WebP,
}

impl NameFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameContext {
    pub app: Option<String>,
    pub window: String,
    pub timestamp: OffsetDateTime,
    pub counter: u32,
    pub width: u32,
    pub height: u32,
}

pub trait TokenSource {
    fn random_chars(&self) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadRandom;

impl TokenSource for ThreadRandom {
    fn random_chars(&self) -> String {
        let mut rng = rand::rng();
        (0..RANDOM_LEN)
            .map(|_| char::from(RANDOM_ALPHABET[rng.random_range(0..RANDOM_ALPHABET.len())]))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedRandom(String);

impl FixedRandom {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}

impl TokenSource for FixedRandom {
    fn random_chars(&self) -> String {
        self.0.clone()
    }
}

pub fn app_name(process: &str) -> String {
    if process.to_ascii_lowercase().ends_with(".exe") {
        process[..process.len() - 4].to_string()
    } else {
        process.to_string()
    }
}

pub fn app_label(process: Option<&str>) -> String {
    match process {
        Some(value) if !value.is_empty() => app_name(value),
        _ => DESKTOP_APP.to_string(),
    }
}

pub fn window_title(title: &str) -> String {
    title
        .chars()
        .map(|character| {
            if INVALID_CHARS.contains(&character) || character.is_control() {
                '-'
            } else {
                character
            }
        })
        .take(WINDOW_TITLE_LIMIT)
        .collect()
}

pub fn expand(pattern: &str, context: &NameContext, source: &dyn TokenSource) -> String {
    let mut output = String::with_capacity(pattern.len() + 16);
    let mut index = 0;
    while index < pattern.len() {
        let rest = &pattern[index..];
        if let Some(open) = rest.find('{') {
            output.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            if let Some(close) = after.find('}') {
                let token = &after[..close];
                match token_value(token, context, source) {
                    Some(value) => output.push_str(&value),
                    None => {
                        output.push('{');
                        output.push_str(token);
                        output.push('}');
                    }
                }
                index += open + close + 2;
            } else {
                output.push_str(rest);
                index = pattern.len();
            }
        } else {
            output.push_str(rest);
            index = pattern.len();
        }
    }
    output
}

fn token_value(token: &str, context: &NameContext, source: &dyn TokenSource) -> Option<String> {
    let timestamp = context.timestamp;
    let value = match token {
        "app" => app_label(context.app.as_deref()),
        "window" => window_title(&context.window),
        "yyyy" => {
            let year = timestamp.year();
            format!("{year:04}")
        }
        "MM" => {
            let month = timestamp.month() as u8;
            format!("{month:02}")
        }
        "dd" => {
            let day = timestamp.day();
            format!("{day:02}")
        }
        "HH" => {
            let hour = timestamp.hour();
            format!("{hour:02}")
        }
        "mm" => {
            let minute = timestamp.minute();
            format!("{minute:02}")
        }
        "ss" => {
            let second = timestamp.second();
            format!("{second:02}")
        }
        "rand" => source.random_chars(),
        "n" => format_counter(context.counter),
        "w" => context.width.to_string(),
        "h" => context.height.to_string(),
        _ => return None,
    };
    Some(value)
}

pub fn next_counter(current: u32) -> u32 {
    if current >= COUNTER_MAX {
        0
    } else {
        current + 1
    }
}

pub fn format_counter(value: u32) -> String {
    let wrapped = value % (COUNTER_MAX + 1);
    format!("{wrapped:04}")
}

pub fn sanitise_file_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|character| {
            if INVALID_CHARS.contains(&character) || character.is_control() {
                '-'
            } else {
                character
            }
        })
        .collect();
    let trimmed = replaced.trim_end_matches([' ', '.']);
    let (stem, extension) = split_extension(trimmed);
    let stem = protect_reserved(stem);
    format!("{stem}{extension}")
}

fn protect_reserved(stem: &str) -> String {
    let (head, tail) = match stem.split_once('.') {
        Some((head, tail)) => (head, Some(tail)),
        None => (stem, None),
    };
    let mut owned = head.to_string();
    if is_reserved(&owned) {
        owned.push('_');
    }
    match tail {
        Some(tail) => format!("{owned}.{tail}"),
        None => owned,
    }
}

fn is_reserved(stem: &str) -> bool {
    let upper = stem.trim_end().to_ascii_uppercase();
    RESERVED_NAMES.contains(&upper.as_str())
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    }
}

pub fn resolve_collision<F>(name: &str, exists: F) -> String
where
    F: Fn(&str) -> bool,
{
    if !exists(name) {
        return name.to_string();
    }
    let (stem, extension) = split_extension(name);
    let mut index = 2;
    loop {
        let candidate = format!("{stem} ({index}){extension}");
        if !exists(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

pub fn file_name(
    pattern: &str,
    context: &NameContext,
    source: &dyn TokenSource,
    format: NameFormat,
) -> String {
    let stem = expand(pattern, context, source);
    let extension = format.extension();
    sanitise_file_name(&format!("{stem}.{extension}"))
}

pub fn sample_context() -> NameContext {
    NameContext {
        app: Some(SAMPLE_APP.to_string()),
        window: SAMPLE_WINDOW.to_string(),
        timestamp: datetime!(2026-11-08 14:31:07 UTC),
        counter: SAMPLE_COUNTER,
        width: SAMPLE_WIDTH,
        height: SAMPLE_HEIGHT,
    }
}

pub fn preview(pattern: &str, format: NameFormat) -> String {
    file_name(
        pattern,
        &sample_context(),
        &FixedRandom::new(SAMPLE_RAND),
        format,
    )
}

pub fn sample_preview(format: NameFormat) -> String {
    preview(DEFAULT_PATTERN, format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn context() -> NameContext {
        NameContext {
            app: Some("chrome.exe".to_string()),
            window: "Pull-request-42".to_string(),
            timestamp: datetime!(2026-11-08 14:31:07 UTC),
            counter: 42,
            width: 1280,
            height: 720,
        }
    }

    fn random() -> FixedRandom {
        FixedRandom::new("k3f9")
    }

    #[test]
    fn every_token_expands() {
        let pattern = "{app}|{window}|{yyyy}|{MM}|{dd}|{HH}|{mm}|{ss}|{rand}|{n}|{w}|{h}";
        let rendered = expand(pattern, &context(), &random());
        assert_eq!(
            rendered,
            "chrome|Pull-request-42|2026|11|08|14|31|07|k3f9|0042|1280|720"
        );
    }

    #[test]
    fn app_token_drops_the_exe_suffix() {
        assert_eq!(app_name("chrome.exe"), "chrome");
        assert_eq!(app_name("chrome.EXE"), "chrome");
        assert_eq!(app_name("chrome"), "chrome");
        assert_eq!(app_label(Some("Code.exe")), "Code");
    }

    #[test]
    fn app_token_falls_back_to_desktop() {
        assert_eq!(app_label(None), "Desktop");
        assert_eq!(app_label(Some("")), "Desktop");
    }

    #[test]
    fn literal_text_and_tokens_work_in_any_order() {
        let rendered = expand("shot-{w}x{h}-{app}-{n}", &context(), &random());
        assert_eq!(rendered, "shot-1280x720-chrome-0042");
        assert_eq!(expand("no tokens", &context(), &random()), "no tokens");
        assert_eq!(
            expand("{h}{w}{app}{n}", &context(), &random()),
            "7201280chrome0042"
        );
    }

    #[test]
    fn window_title_replaces_invalid_characters() {
        assert_eq!(window_title("a<b>c:d\"e/f\\g|h?i*j"), "a-b-c-d-e-f-g-h-i-j");
        assert_eq!(window_title("tab\there\nline"), "tab-here-line");
        assert_eq!(window_title("plain title"), "plain title");
    }

    #[test]
    fn window_title_is_trimmed_to_eighty_characters() {
        let long = "x".repeat(100);
        let trimmed = window_title(&long);
        assert_eq!(trimmed, "x".repeat(80));
        assert_eq!(trimmed.chars().count(), WINDOW_TITLE_LIMIT);
        let rendered = expand("{window}", &context_with_window(&long), &random());
        assert_eq!(rendered.chars().count(), 80);
    }

    fn context_with_window(window: &str) -> NameContext {
        let mut value = context();
        value.window = window.to_string();
        value
    }

    #[test]
    fn unknown_braces_stay_literal() {
        assert_eq!(
            expand("{app}-{unknown}-{app}", &context(), &random()),
            "chrome-{unknown}-chrome"
        );
        assert_eq!(expand("{app", &context(), &random()), "{app");
        assert_eq!(expand("{app}-{", &context(), &random()), "chrome-{");
        assert_eq!(expand("}{app}{}", &context(), &random()), "}chrome{}");
        assert_eq!(expand("a{wat}b", &context(), &random()), "a{wat}b");
    }

    #[test]
    fn counter_increments_and_wraps() {
        assert_eq!(next_counter(0), 1);
        assert_eq!(next_counter(41), 42);
        assert_eq!(next_counter(9998), 9999);
        assert_eq!(next_counter(9999), 0);
        assert_eq!(next_counter(12_000), 0);
    }

    #[test]
    fn counter_is_four_digits() {
        assert_eq!(format_counter(0), "0000");
        assert_eq!(format_counter(7), "0007");
        assert_eq!(format_counter(42), "0042");
        assert_eq!(format_counter(9999), "9999");
    }

    #[test]
    fn collision_appends_a_number_before_the_extension() {
        let taken = ["shot.png", "shot (2).png"];
        let resolved = resolve_collision("shot.png", |name| taken.contains(&name));
        assert_eq!(resolved, "shot (3).png");

        let resolved = resolve_collision("shot.png", |name| name == "shot.png");
        assert_eq!(resolved, "shot (2).png");

        let resolved = resolve_collision("shot.png", |_| false);
        assert_eq!(resolved, "shot.png");
    }

    #[test]
    fn collision_keeps_names_without_an_extension_intact() {
        let resolved = resolve_collision("shot", |name| name == "shot");
        assert_eq!(resolved, "shot (2)");

        let taken = ["a.b.png", "a.b (2).png"];
        let resolved = resolve_collision("a.b.png", |name| taken.contains(&name));
        assert_eq!(resolved, "a.b (3).png");
    }

    #[test]
    fn collision_handles_a_leading_dot_name() {
        let resolved = resolve_collision(".hidden", |name| name == ".hidden");
        assert_eq!(resolved, ".hidden (2)");
    }

    #[test]
    fn sanitise_replaces_invalid_characters() {
        assert_eq!(sanitise_file_name("a:b*c?.png"), "a-b-c-.png");
        assert_eq!(sanitise_file_name("a<b>c.png"), "a-b-c.png");
        assert_eq!(sanitise_file_name("a|b\tc.png"), "a-b-c.png");
        assert_eq!(sanitise_file_name("clean_name.png"), "clean_name.png");
    }

    #[test]
    fn sanitise_trims_trailing_dots_and_spaces() {
        assert_eq!(sanitise_file_name("shot.png "), "shot.png");
        assert_eq!(sanitise_file_name("shot."), "shot");
        assert_eq!(sanitise_file_name("shot..."), "shot");
    }

    #[test]
    fn sanitise_protects_reserved_device_names() {
        for reserved in [
            "CON", "con", "PRN", "AUX", "NUL", "COM1", "com9", "LPT1", "LPT9",
        ] {
            let name = format!("{reserved}.png");
            let expected = format!("{reserved}_.png");
            assert_eq!(sanitise_file_name(&name), expected, "{reserved}");
            assert_eq!(sanitise_file_name(reserved), format!("{reserved}_"));
        }
        assert_eq!(sanitise_file_name("console.png"), "console.png");
        assert_eq!(sanitise_file_name("COM10.png"), "COM10.png");
    }

    #[test]
    fn sanitise_checks_the_name_before_the_first_dot() {
        assert_eq!(sanitise_file_name("NUL.tar.gz"), "NUL_.tar.gz");
        assert_eq!(sanitise_file_name("aux.notes.txt"), "aux_.notes.txt");
        assert_eq!(sanitise_file_name("com1.backup.png"), "com1_.backup.png");
    }

    #[test]
    fn sanitise_never_strips_the_extension() {
        assert_eq!(sanitise_file_name("name.jpg"), "name.jpg");
        assert_eq!(sanitise_file_name("name.webp"), "name.webp");
        assert_eq!(sanitise_file_name("name.tar.gz"), "name.tar.gz");
    }

    #[test]
    fn sample_preview_matches_the_design() {
        assert_eq!(
            sample_preview(NameFormat::Png),
            "chrome_2026-11-08_14-31-07.png"
        );
        assert_eq!(
            sample_preview(NameFormat::Jpeg),
            "chrome_2026-11-08_14-31-07.jpg"
        );
        assert_eq!(
            sample_preview(NameFormat::WebP),
            "chrome_2026-11-08_14-31-07.webp"
        );
        assert_eq!(
            preview(DEFAULT_PATTERN, NameFormat::Png),
            sample_preview(NameFormat::Png)
        );
    }

    #[test]
    fn preview_fills_a_custom_pattern_with_the_sample_values() {
        let rendered = preview("{window}_{rand}_{n}_{w}x{h}", NameFormat::Png);
        assert_eq!(rendered, "Pull-request-42_k3f9_0042_1280x720.png");
        let rendered = preview("{app}-{yyyy}-{MM}-{dd}-{HH}-{mm}-{ss}", NameFormat::Png);
        assert_eq!(rendered, "chrome-2026-11-08-14-31-07.png");
    }

    #[test]
    fn file_name_sanitises_the_rendered_pattern() {
        let mut value = context();
        value.window = "a/b:c*d".to_string();
        let rendered = file_name("{window}", &value, &random(), NameFormat::Png);
        assert_eq!(rendered, "a-b-c-d.png");
    }

    #[test]
    fn file_name_uses_the_chosen_extension() {
        let rendered = file_name("{app}_{n}", &context(), &random(), NameFormat::WebP);
        assert_eq!(rendered, "chrome_0042.webp");
    }
}
