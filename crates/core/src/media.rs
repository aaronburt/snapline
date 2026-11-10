#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Gif,
    Text,
    File,
}

pub fn classify(file_name: &str, first_bytes: &[u8]) -> MediaKind {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "avif" => {
            return MediaKind::Image;
        }
        "gif" => return MediaKind::Gif,
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "wmv" | "mpeg" | "mpg" => {
            return MediaKind::Video;
        }
        "txt" | "md" | "rst" | "csv" | "tsv" | "json" | "xml" | "html" | "htm" | "css" | "js"
        | "ts" | "jsx" | "tsx" | "rs" | "py" | "go" | "java" | "c" | "h" | "cpp" | "hpp"
        | "toml" | "yaml" | "yml" | "ini" | "cfg" | "log" | "sh" | "bat" | "ps1" => {
            return MediaKind::Text
        }
        _ => {}
    }
    if first_bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || first_bytes.starts_with(&[0xff, 0xd8, 0xff])
        || first_bytes.starts_with(b"BM")
        || first_bytes.starts_with(b"GIF87a")
        || first_bytes.starts_with(b"GIF89a")
        || (first_bytes.len() >= 12
            && first_bytes.starts_with(b"RIFF")
            && first_bytes.get(8..12) == Some(b"WEBP"))
    {
        if first_bytes.starts_with(b"GIF87a") || first_bytes.starts_with(b"GIF89a") {
            return MediaKind::Gif;
        }
        return MediaKind::Image;
    }
    if (first_bytes.len() >= 8 && first_bytes.get(4..8) == Some(b"ftyp"))
        || first_bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
        || (first_bytes.len() >= 12
            && first_bytes.starts_with(b"RIFF")
            && first_bytes.get(8..12) == Some(b"AVI "))
    {
        return MediaKind::Video;
    }
    MediaKind::File
}

pub fn content_type(path: impl AsRef<std::path::Path>) -> &'static str {
    let extension = path
        .as_ref()
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "mpeg" | "mpg" => "video/mpeg",
        "txt" | "log" | "rs" | "py" | "go" | "java" | "c" | "h" | "cpp" | "hpp" | "sh" | "bat"
        | "ps1" => "text/plain",
        "md" | "rst" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "jsx" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml",
        "ini" | "cfg" => "text/plain",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_supported_extensions_without_relying_on_file_contents() {
        let cases = [
            ("capture.PNG", MediaKind::Image),
            ("capture.jpeg", MediaKind::Image),
            ("capture.gif", MediaKind::Gif),
            ("capture.MKV", MediaKind::Video),
            ("capture.json", MediaKind::Text),
            ("capture.unknown", MediaKind::File),
            ("extensionless", MediaKind::File),
        ];

        for (name, expected) in cases {
            assert_eq!(classify(name, &[]), expected, "{name}");
        }
    }

    #[test]
    fn recognizes_media_signatures_when_the_extension_is_unknown() {
        let cases: [(&[u8], MediaKind); 10] = [
            (b"\x89PNG\r\n\x1a\nmore", MediaKind::Image),
            (&[0xff, 0xd8, 0xff, 0], MediaKind::Image),
            (b"BMdata", MediaKind::Image),
            (b"GIF87a", MediaKind::Gif),
            (b"GIF89a", MediaKind::Gif),
            (b"RIFF1234WEBP", MediaKind::Image),
            (b"0000ftyp", MediaKind::Video),
            (&[0x1a, 0x45, 0xdf, 0xa3], MediaKind::Video),
            (b"RIFF1234AVI ", MediaKind::Video),
            (b"RIFF1234WEB", MediaKind::File),
        ];

        for (signature, expected) in cases {
            assert_eq!(classify("capture.unknown", signature), expected);
        }
    }

    #[test]
    fn a_known_extension_takes_precedence_over_a_conflicting_signature() {
        assert_eq!(classify("notes.txt", b"\x89PNG\r\n\x1a\n"), MediaKind::Text);
        assert_eq!(classify("picture.jpg", b"GIF89a"), MediaKind::Image);
    }

    #[test]
    fn maps_common_file_extensions_to_content_types() {
        let cases = [
            ("capture.PNG", "image/png"),
            ("capture.jpeg", "image/jpeg"),
            ("capture.webp", "image/webp"),
            ("capture.gif", "image/gif"),
            ("capture.mp4", "video/mp4"),
            ("capture.mov", "video/quicktime"),
            ("capture.md", "text/markdown"),
            ("capture.json", "application/json"),
            ("capture.pdf", "application/pdf"),
            ("capture.unknown", "application/octet-stream"),
        ];

        for (path, expected) in cases {
            assert_eq!(content_type(path), expected, "{path}");
        }
    }
}
