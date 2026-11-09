#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LinkFormat {
    #[default]
    Direct,
    Markdown,
    Html,
    Bbcode,
    Page,
}

impl LinkFormat {
    #[must_use]
    pub fn format(self, url: &str, file_name: &str, page_url: Option<&str>) -> String {
        match self {
            Self::Direct => url.to_owned(),
            Self::Markdown => format!("![{file_name}]({url})"),
            Self::Html => format!(
                "<img src=\"{}\" alt=\"{}\">",
                escape_attribute(url),
                escape_attribute(file_name)
            ),
            Self::Bbcode => format!("[img]{url}[/img]"),
            Self::Page => page_url
                .filter(|page| !page.is_empty())
                .unwrap_or(url)
                .to_owned(),
        }
    }
}

fn escape_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://i.example.com/2026/11/capture.png";
    const NAME: &str = "capture.png";
    const PAGE: &str = "https://example.com/gallery/42";

    #[test]
    fn direct_link_is_the_url() {
        assert_eq!(
            LinkFormat::Direct.format(URL, NAME, Some(PAGE)),
            "https://i.example.com/2026/11/capture.png"
        );
    }

    #[test]
    fn direct_link_has_no_angle_brackets() {
        let link = LinkFormat::Direct.format(URL, NAME, None);
        assert!(!link.starts_with('<'));
        assert!(!link.ends_with('>'));
    }

    #[test]
    fn markdown_link() {
        assert_eq!(
            LinkFormat::Markdown.format(URL, NAME, None),
            "![capture.png](https://i.example.com/2026/11/capture.png)"
        );
    }

    #[test]
    fn html_link() {
        assert_eq!(
            LinkFormat::Html.format(URL, NAME, None),
            "<img src=\"https://i.example.com/2026/11/capture.png\" alt=\"capture.png\">"
        );
    }

    #[test]
    fn bbcode_link() {
        assert_eq!(
            LinkFormat::Bbcode.format(URL, NAME, None),
            "[img]https://i.example.com/2026/11/capture.png[/img]"
        );
    }

    #[test]
    fn page_link_uses_the_host_page() {
        assert_eq!(LinkFormat::Page.format(URL, NAME, Some(PAGE)), PAGE);
    }

    #[test]
    fn page_link_falls_back_to_the_direct_link() {
        assert_eq!(LinkFormat::Page.format(URL, NAME, None), URL);
    }

    #[test]
    fn page_link_falls_back_when_the_page_is_empty() {
        assert_eq!(LinkFormat::Page.format(URL, NAME, Some("")), URL);
    }

    #[test]
    fn html_escapes_attribute_values() {
        let url = "https://example.com/a?x=1&y=\"two\"<z>";
        let name = "a & b <c>.png";
        assert_eq!(
            LinkFormat::Html.format(url, name, None),
            "<img src=\"https://example.com/a?x=1&amp;y=&quot;two&quot;&lt;z&gt;\" alt=\"a &amp; b &lt;c&gt;.png\">"
        );
    }

    #[test]
    fn html_keeps_utf8() {
        assert_eq!(
            LinkFormat::Html.format("https://example.com/é", "Capture — café.png", None),
            "<img src=\"https://example.com/é\" alt=\"Capture — café.png\">"
        );
    }

    #[test]
    fn other_formats_keep_utf8_and_do_not_escape() {
        assert_eq!(
            LinkFormat::Markdown.format("https://example.com/é", "café.png", None),
            "![café.png](https://example.com/é)"
        );
        assert_eq!(
            LinkFormat::Bbcode.format("https://example.com/é", "café.png", None),
            "[img]https://example.com/é[/img]"
        );
    }

    #[test]
    fn default_is_direct() {
        assert_eq!(LinkFormat::default(), LinkFormat::Direct);
    }
}
