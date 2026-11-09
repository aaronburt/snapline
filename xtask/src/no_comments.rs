use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    ".git",
    "third_party",
    "design/v2",
    "ui/src/lib/ipc/bindings",
    "apps/snapline/icons/menu",
    "ui/src/styles/tokens.css",
    "crates/overlay-windows/src/tokens.rs",
];

const EXTENSIONS: &[&str] = &["rs", "ts", "svelte", "css", "html", "toml", "sql"];

pub fn run(root: &Path) -> Result<()> {
    let mut problems = Vec::new();
    visit(root, root, &mut problems)?;
    if problems.is_empty() {
        println!("no-comments: clean");
        return Ok(());
    }
    for problem in &problems {
        eprintln!("{problem}");
    }
    anyhow::bail!("no-comments: {} problem(s) found", problems.len())
}

fn visit(root: &Path, dir: &Path, problems: &mut Vec<String>) -> Result<()> {
    for entry in
        fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            if SKIP_DIRS.iter().any(|skip| relative == *skip || relative.starts_with(&format!("{skip}/"))) {
                continue;
            }
            visit(root, &path, problems)?;
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !EXTENSIONS.contains(&extension) {
            continue;
        }
        if SKIP_DIRS.contains(&relative.as_str()) {
            continue;
        }
        let source = fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        scan_file(&relative, extension, &source, problems);
    }
    Ok(())
}

fn scan_file(name: &str, extension: &str, source: &str, problems: &mut Vec<String>) {
    let comments = match extension {
        "rs" | "ts" => scan_code(source, false),
        "svelte" | "html" => scan_markup(source),
        "css" => scan_css(source),
        "toml" => scan_toml(source),
        "sql" => scan_sql(source),
        _ => Vec::new(),
    };
    for comment in comments {
        if allowed_comment(source, &comment) {
            continue;
        }
        problems.push(format!(
            "{name}:{}:{}: comment: {}",
            comment.line,
            comment.column,
            comment.text.replace('\n', " ")
        ));
    }
}

#[derive(Debug, Clone)]
struct Comment {
    line: usize,
    column: usize,
    text: String,
    start_line: usize,
}

fn allowed_comment(source: &str, comment: &Comment) -> bool {
    if !comment.text.starts_with("// SAFETY:") {
        return false;
    }
    let lines: Vec<&str> = source.lines().collect();
    if comment.start_line == 0 {
        return false;
    }
    for line in lines.iter().skip(comment.start_line) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return trimmed.contains("unsafe");
    }
    false
}

fn position(source: &str, index: usize) -> (usize, usize) {
    let prefix = &source[..index];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rfind('\n')
        .map_or(prefix.len() + 1, |offset| prefix.len() - offset);
    (line, column)
}

fn comment(source: &str, start: usize, end: usize) -> Comment {
    let (line, column) = position(source, start);
    Comment {
        line,
        column,
        text: source[start..end].trim().to_string(),
        start_line: line,
    }
}

fn scan_code(source: &str, markup: bool) -> Vec<Comment> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let current = bytes[index];
        if current == b'"' || current == b'\'' || current == b'`' {
            index = skip_string(bytes, index, current);
            continue;
        }
        if !markup && current == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            let end = source[index..]
                .find('\n')
                .map_or(bytes.len(), |offset| index + offset);
            comments.push(comment(source, index, end));
            index = end;
            continue;
        }
        if current == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            let end = source[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |offset| index + 2 + offset + 2);
            comments.push(comment(source, index, end));
            index = end;
            continue;
        }
        index += 1;
    }
    comments
}

fn skip_string(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    index
}

fn scan_markup(source: &str) -> Vec<Comment> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if index + 3 < bytes.len() && &bytes[index..index + 4] == b"<!--" {
            let end = source[index + 4..]
                .find("-->")
                .map_or(bytes.len(), |offset| index + 4 + offset + 3);
            comments.push(comment(source, index, end));
            index = end;
            continue;
        }
        if bytes[index] == b'<' {
            if let Some(end) = script_end(source, index) {
                let body_start = source[index..]
                    .find('>')
                    .map_or(index, |offset| index + offset + 1);
                if body_start < end {
                    let inner = &source[body_start..end];
                    for mut found in scan_code(inner, false) {
                        found.line += position(source, body_start).0 - 1;
                        found.start_line = found.line;
                        comments.push(found);
                    }
                }
                index = end;
                continue;
            }
            if let Some(end) = style_end(source, index) {
                let body_start = source[index..]
                    .find('>')
                    .map_or(index, |offset| index + offset + 1);
                if body_start < end {
                    let inner = &source[body_start..end];
                    for mut found in scan_css(inner) {
                        found.line += position(source, body_start).0 - 1;
                        found.start_line = found.line;
                        comments.push(found);
                    }
                }
                index = end;
                continue;
            }
        }
        index += 1;
    }
    comments
}

fn script_end(source: &str, index: usize) -> Option<usize> {
    let rest = source.get(index..)?;
    if !rest.to_ascii_lowercase().starts_with("<script") {
        return None;
    }
    rest.to_ascii_lowercase()
        .find("</script>")
        .map(|offset| index + offset)
}

fn style_end(source: &str, index: usize) -> Option<usize> {
    let rest = source.get(index..)?;
    if !rest.to_ascii_lowercase().starts_with("<style") {
        return None;
    }
    rest.to_ascii_lowercase()
        .find("</style>")
        .map(|offset| index + offset)
}

fn scan_css(source: &str) -> Vec<Comment> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' || bytes[index] == b'\'' {
            index = skip_string(bytes, index, bytes[index]);
            continue;
        }
        if bytes[index] == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            let end = source[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |offset| index + 2 + offset + 2);
            comments.push(comment(source, index, end));
            index = end;
            continue;
        }
        index += 1;
    }
    comments
}

fn scan_toml(source: &str) -> Vec<Comment> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' || bytes[index] == b'\'' {
            index = skip_string(bytes, index, bytes[index]);
            continue;
        }
        if bytes[index] == b'#' {
            let end = source[index..]
                .find('\n')
                .map_or(bytes.len(), |offset| index + offset);
            comments.push(comment(source, index, end));
            index = end;
            continue;
        }
        index += 1;
    }
    comments
}

fn scan_sql(source: &str) -> Vec<Comment> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            index = skip_string(bytes, index, b'\'');
            continue;
        }
        if bytes[index] == b'-' && index + 1 < bytes.len() && bytes[index + 1] == b'-' {
            let end = source[index..]
                .find('\n')
                .map_or(bytes.len(), |offset| index + offset);
            comments.push(comment(source, index, end));
            index = end;
            continue;
        }
        if bytes[index] == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            let end = source[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |offset| index + 2 + offset + 2);
            comments.push(comment(source, index, end));
            index = end;
            continue;
        }
        index += 1;
    }
    comments
}

pub fn root() -> Result<PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask has no parent directory")?
        .to_path_buf())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn found(name: &str, extension: &str, source: &str) -> Vec<String> {
        let mut problems = Vec::new();
        scan_file(name, extension, source, &mut problems);
        problems
    }

    #[test]
    fn flags_line_comments() {
        assert_eq!(found("a.rs", "rs", "fn main() {}\n// x\n").len(), 1);
    }

    #[test]
    fn flags_block_comments() {
        assert_eq!(found("a.rs", "rs", "/* x */\n").len(), 1);
    }

    #[test]
    fn flags_html_comments() {
        assert_eq!(found("a.html", "html", "<!-- x -->\n").len(), 1);
    }

    #[test]
    fn flags_toml_comments() {
        assert_eq!(found("a.toml", "toml", "# x\n").len(), 1);
    }

    #[test]
    fn flags_sql_comments() {
        assert_eq!(found("a.sql", "sql", "-- x\n").len(), 1);
    }

    #[test]
    fn ignores_strings() {
        assert!(found("a.rs", "rs", "let x = \"// not a comment\";\n").is_empty());
    }

    #[test]
    fn ignores_template_literals() {
        assert!(found(
            "a.ts",
            "ts",
            "const x = `// not a comment ${1 /* no */}`;\n"
        )
        .is_empty());
    }

    #[test]
    fn allows_safety_above_unsafe() {
        assert!(found(
            "a.rs",
            "rs",
            "fn f() {\n    // SAFETY: the pointer is valid\n    unsafe { call(); }\n}\n"
        )
        .is_empty());
    }

    #[test]
    fn rejects_safety_not_above_unsafe() {
        assert_eq!(
            found("a.rs", "rs", "// SAFETY: nope\nfn f() {}\n").len(),
            1
        );
    }

    #[test]
    fn flags_svelte_script_comments() {
        assert_eq!(
            found(
                "a.svelte",
                "svelte",
                "<script lang=\"ts\">\n// x\n</script>\n<p>hi</p>\n"
            )
            .len(),
            1
        );
    }
}
