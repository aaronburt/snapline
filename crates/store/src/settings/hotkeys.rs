use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const DEFAULT_BINDINGS: &[(&str, &str)] = &[
    ("capture_region", "PrtSc"),
    ("capture_window", "Alt+PrtSc"),
    ("capture_screen", "Ctrl+PrtSc"),
    ("capture_scrolling", "Ctrl+Alt+S"),
    ("capture_last_region", "Ctrl+Shift+R"),
    ("record_video", "Shift+PrtSc"),
    ("record_gif", "Ctrl+Shift+G"),
    ("tool_ocr", "Ctrl+Shift+T"),
    ("tool_colour", "Ctrl+Shift+C"),
    ("tool_ruler", "Ctrl+Shift+M"),
    ("upload_clipboard", "Ctrl+Shift+V"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub bindings: BTreeMap<String, String>,
    pub take_over_prtsc: bool,
    pub warn_about_clashes: bool,
    #[serde(skip_serializing_if = "Restore::is_empty")]
    pub restore: Restore,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            bindings: default_bindings(),
            take_over_prtsc: true,
            warn_about_clashes: true,
            restore: Restore::default(),
            extra: toml::Table::new(),
        }
    }
}

pub fn default_bindings() -> BTreeMap<String, String> {
    DEFAULT_BINDINGS
        .iter()
        .map(|(action, chord)| ((*action).to_owned(), (*chord).to_owned()))
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Restore {
    pub print_screen_key_for_snipping_enabled: Option<u32>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Restore {
    pub fn is_empty(&self) -> bool {
        self.print_screen_key_for_snipping_enabled.is_none() && self.extra.is_empty()
    }
}
