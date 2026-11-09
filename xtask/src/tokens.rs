use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct TokenFile {
    dark: Theme,
    light: Theme,
    annotation: Annotation,
    #[serde(rename = "type")]
    typography: Typography,
    radius: Radius,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Theme {
    bg: String,
    mica: String,
    layer: String,
    card: String,
    card_alt: String,
    stroke: String,
    stroke_hi: String,
    text: String,
    text2: String,
    text3: String,
    accent: String,
    on_accent: String,
    accent_soft: String,
    subtle: String,
    ok: String,
    ok_soft: String,
    err: String,
    err_soft: String,
    shadow: String,
    menu_layer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Annotation {
    red: String,
    orange: String,
    yellow: String,
    green: String,
    blue: String,
    white: String,
    red_highlight: String,
    orange_highlight: String,
    yellow_highlight: String,
    green_highlight: String,
    blue_highlight: String,
    white_highlight: String,
    dim: String,
    hint_pill: String,
    hint_text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Typography {
    sans: String,
    sans_fallback: String,
    mono: String,
    mono_fallback: String,
    base_size: String,
}

#[derive(Deserialize)]
struct Radius {
    sm: String,
    md: String,
    lg: String,
}

const THEME_CSS: [&str; 20] = [
    "bg",
    "mica",
    "layer",
    "card",
    "cardAlt",
    "stroke",
    "strokeHi",
    "text",
    "text2",
    "text3",
    "accent",
    "onAccent",
    "accentSoft",
    "subtle",
    "ok",
    "okSoft",
    "err",
    "errSoft",
    "shadow",
    "menuLayer",
];

const THEME_RUST: [&str; 20] = [
    "bg",
    "mica",
    "layer",
    "card",
    "card_alt",
    "stroke",
    "stroke_hi",
    "text",
    "text2",
    "text3",
    "accent",
    "on_accent",
    "accent_soft",
    "subtle",
    "ok",
    "ok_soft",
    "err",
    "err_soft",
    "shadow",
    "menu_layer",
];

const ANNOTATION_CSS: [&str; 15] = [
    "red",
    "orange",
    "yellow",
    "green",
    "blue",
    "white",
    "redHighlight",
    "orangeHighlight",
    "yellowHighlight",
    "greenHighlight",
    "blueHighlight",
    "whiteHighlight",
    "dim",
    "hintPill",
    "hintText",
];

const ANNOTATION_RUST: [&str; 15] = [
    "red",
    "orange",
    "yellow",
    "green",
    "blue",
    "white",
    "red_highlight",
    "orange_highlight",
    "yellow_highlight",
    "green_highlight",
    "blue_highlight",
    "white_highlight",
    "dim",
    "hint_pill",
    "hint_text",
];

fn theme_values(theme: &Theme) -> [&str; 20] {
    [
        &theme.bg,
        &theme.mica,
        &theme.layer,
        &theme.card,
        &theme.card_alt,
        &theme.stroke,
        &theme.stroke_hi,
        &theme.text,
        &theme.text2,
        &theme.text3,
        &theme.accent,
        &theme.on_accent,
        &theme.accent_soft,
        &theme.subtle,
        &theme.ok,
        &theme.ok_soft,
        &theme.err,
        &theme.err_soft,
        &theme.shadow,
        &theme.menu_layer,
    ]
}

fn annotation_values(annotation: &Annotation) -> [&str; 15] {
    [
        &annotation.red,
        &annotation.orange,
        &annotation.yellow,
        &annotation.green,
        &annotation.blue,
        &annotation.white,
        &annotation.red_highlight,
        &annotation.orange_highlight,
        &annotation.yellow_highlight,
        &annotation.green_highlight,
        &annotation.blue_highlight,
        &annotation.white_highlight,
        &annotation.dim,
        &annotation.hint_pill,
        &annotation.hint_text,
    ]
}

fn load(root: &Path) -> Result<TokenFile> {
    let path = root.join("design/tokens.toml");
    let source =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    toml::from_str(&source).with_context(|| format!("cannot parse {}", path.display()))
}

fn sans_stack(typography: &Typography) -> String {
    format!(
        "'{}', '{}', system-ui, sans-serif",
        typography.sans, typography.sans_fallback
    )
}

fn sans_fallback(typography: &Typography) -> String {
    format!("'{}', system-ui, sans-serif", typography.sans_fallback)
}

fn mono_stack(typography: &Typography) -> String {
    format!(
        "'{}', 'Cascadia Code', Consolas, ui-monospace, monospace",
        typography.mono
    )
}

fn mono_fallback(typography: &Typography) -> String {
    format!("{}, ui-monospace, monospace", typography.mono_fallback)
}

fn css(tokens: &TokenFile) -> String {
    let mut out = String::from(":root {\n  color-scheme: dark;\n");
    for (name, value) in THEME_CSS.iter().zip(theme_values(&tokens.dark)) {
        out.push_str(&format!("  --{name}: {value};\n"));
    }
    for (name, value) in ANNOTATION_CSS
        .iter()
        .zip(annotation_values(&tokens.annotation))
    {
        out.push_str(&format!("  --{name}: {value};\n"));
    }
    out.push_str(&format!(
        "  --type-sans: {};\n  --type-sans-fallback: {};\n  --type-mono: {};\n  --type-mono-fallback: {};\n  --type-base-size: {};\n  --radius-sm: {};\n  --radius-md: {};\n  --radius-lg: {};\n}}\n",
        sans_stack(&tokens.typography),
        sans_fallback(&tokens.typography),
        mono_stack(&tokens.typography),
        mono_fallback(&tokens.typography),
        tokens.typography.base_size,
        tokens.radius.sm,
        tokens.radius.md,
        tokens.radius.lg
    ));
    out.push_str("@media (prefers-color-scheme: light) {\n  :root {\n    color-scheme: light;\n");
    for (name, value) in THEME_CSS.iter().zip(theme_values(&tokens.light)) {
        out.push_str(&format!("    --{name}: {value};\n"));
    }
    out.push_str("  }\n}\n");
    out
}

fn colour(literal: &str) -> Result<[f32; 4]> {
    let value = literal.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() != 6 {
            anyhow::bail!("unsupported colour literal: {literal}");
        }
        let channel = |start: usize| -> Result<f32> {
            let parsed = u8::from_str_radix(&hex[start..start + 2], 16)
                .with_context(|| format!("unsupported colour literal: {literal}"))?;
            Ok(f32::from(parsed) / 255.0)
        };
        return Ok([channel(0)?, channel(2)?, channel(4)?, 1.0]);
    }
    if let Some(inner) = value
        .strip_prefix("rgba(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != 4 {
            anyhow::bail!("unsupported colour literal: {literal}");
        }
        let red: f32 = parts[0]
            .parse()
            .with_context(|| format!("unsupported colour literal: {literal}"))?;
        let green: f32 = parts[1]
            .parse()
            .with_context(|| format!("unsupported colour literal: {literal}"))?;
        let blue: f32 = parts[2]
            .parse()
            .with_context(|| format!("unsupported colour literal: {literal}"))?;
        let alpha: f32 = parts[3]
            .parse()
            .with_context(|| format!("unsupported colour literal: {literal}"))?;
        return Ok([red / 255.0, green / 255.0, blue / 255.0, alpha]);
    }
    anyhow::bail!("unsupported colour literal: {literal}")
}

fn array(literal: &str) -> Result<String> {
    let colour = colour(literal)?;
    Ok(format!(
        "[{:.6}, {:.6}, {:.6}, {:.6}]",
        colour[0], colour[1], colour[2], colour[3]
    ))
}

fn palette_literal(tokens: &TokenFile, dark: bool) -> Result<Vec<(String, String)>> {
    let theme = if dark { &tokens.dark } else { &tokens.light };
    let mut fields = Vec::new();
    for (name, value) in THEME_RUST.iter().zip(theme_values(theme)) {
        fields.push(((*name).to_string(), array(value)?));
    }
    for (name, value) in ANNOTATION_RUST
        .iter()
        .zip(annotation_values(&tokens.annotation))
    {
        fields.push(((*name).to_string(), array(value)?));
    }
    Ok(fields)
}

fn rust(tokens: &TokenFile) -> Result<String> {
    let mut out = String::from("pub struct Palette {\n");
    for name in THEME_RUST {
        out.push_str(&format!("    pub {name}: [f32; 4],\n"));
    }
    for name in ANNOTATION_RUST {
        out.push_str(&format!("    pub {name}: [f32; 4],\n"));
    }
    out.push_str("}\n\n");
    for (name, dark) in [("DARK", true), ("LIGHT", false)] {
        out.push_str(&format!("pub const {name}: Palette = Palette {{\n"));
        for (field, value) in palette_literal(tokens, dark)? {
            out.push_str(&format!("    {field}: {value},\n"));
        }
        out.push_str("};\n\n");
    }
    Ok(out)
}

fn css_path(root: &Path) -> std::path::PathBuf {
    root.join("ui/src/styles/tokens.css")
}

fn rust_path(root: &Path) -> std::path::PathBuf {
    root.join("crates/overlay-windows/src/tokens.rs")
}

fn compare(path: &Path, expected: &str) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(current) => Ok(current == expected),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

pub fn generate(root: &Path) -> Result<()> {
    let tokens = load(root)?;
    let css_target = css_path(root);
    let rust_target = rust_path(root);
    if let Some(parent) = css_target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    if let Some(parent) = rust_target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(&css_target, css(&tokens))
        .with_context(|| format!("cannot write {}", css_target.display()))?;
    fs::write(&rust_target, rust(&tokens)?)
        .with_context(|| format!("cannot write {}", rust_target.display()))?;
    println!(
        "tokens: wrote {} and {}",
        css_target.display(),
        rust_target.display()
    );
    Ok(())
}

pub fn check(root: &Path) -> Result<()> {
    let tokens = load(root)?;
    let css_target = css_path(root);
    let rust_target = rust_path(root);
    let mut stale = Vec::new();
    if !compare(&css_target, &css(&tokens))? {
        stale.push(css_target);
    }
    if !compare(&rust_target, &rust(&tokens)?)? {
        stale.push(rust_target);
    }
    if !stale.is_empty() {
        for path in &stale {
            eprintln!("tokens: {} is out of date", path.display());
        }
        anyhow::bail!("tokens are out of date; run cargo xtask tokens");
    }
    println!("tokens: up to date");
    Ok(())
}
