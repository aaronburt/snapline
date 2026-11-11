use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionId(String);

impl ActionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for ActionId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ActionId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub id: &'static str,
    pub label: &'static str,
    pub default_chord: Option<&'static str>,
    pub group: &'static str,
    pub available_from_milestone: u8,
}

pub const WORKFLOW_PREFIX: &str = "workflow:";

pub const CATALOGUE: &[ActionDef] = &[
    ActionDef {
        id: "capture_region",
        label: "Capture region",
        default_chord: Some("PrtSc"),
        group: "capture",
        available_from_milestone: 3,
    },
    ActionDef {
        id: "capture_window",
        label: "Capture window",
        default_chord: Some("Alt+PrtSc"),
        group: "capture",
        available_from_milestone: 3,
    },
    ActionDef {
        id: "capture_screen",
        label: "Capture full screen",
        default_chord: Some("Ctrl+PrtSc"),
        group: "capture",
        available_from_milestone: 3,
    },
    ActionDef {
        id: "capture_last_region",
        label: "Last region again",
        default_chord: Some("Ctrl+Shift+R"),
        group: "capture",
        available_from_milestone: 3,
    },
    ActionDef {
        id: "record_video",
        label: "Record video",
        default_chord: Some("Shift+PrtSc"),
        group: "recording",
        available_from_milestone: 8,
    },
    ActionDef {
        id: "record_gif",
        label: "Record GIF",
        default_chord: Some("Ctrl+Shift+G"),
        group: "recording",
        available_from_milestone: 8,
    },
    ActionDef {
        id: "record_window",
        label: "Record active window",
        default_chord: None,
        group: "recording",
        available_from_milestone: 8,
    },
    ActionDef {
        id: "record_stop",
        label: "Stop recording",
        default_chord: None,
        group: "recording",
        available_from_milestone: 8,
    },
    ActionDef {
        id: "record_cancel",
        label: "Cancel recording",
        default_chord: None,
        group: "recording",
        available_from_milestone: 8,
    },
    ActionDef {
        id: "tool_ocr",
        label: "Text capture (OCR)",
        default_chord: Some("Ctrl+Shift+T"),
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "tool_colour",
        label: "Colour picker",
        default_chord: Some("Ctrl+Shift+C"),
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "tool_ruler",
        label: "Screen ruler",
        default_chord: Some("Ctrl+Shift+M"),
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "tool_qr",
        label: "QR code",
        default_chord: None,
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "tool_editor",
        label: "Image editor",
        default_chord: None,
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "tool_pin",
        label: "Pin to screen",
        default_chord: None,
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "tool_combine",
        label: "Image combiner",
        default_chord: None,
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "tool_convert",
        label: "Video converter",
        default_chord: None,
        group: "tools",
        available_from_milestone: 9,
    },
    ActionDef {
        id: "upload_clipboard",
        label: "Upload from clipboard",
        default_chord: Some("Ctrl+Shift+V"),
        group: "uploads",
        available_from_milestone: 6,
    },
    ActionDef {
        id: "upload_file",
        label: "Upload file",
        default_chord: None,
        group: "uploads",
        available_from_milestone: 1,
    },
    ActionDef {
        id: "upload_copy_last_link",
        label: "Copy last link",
        default_chord: None,
        group: "uploads",
        available_from_milestone: 6,
    },
    ActionDef {
        id: "app_open",
        label: "Open Snapline",
        default_chord: None,
        group: "app",
        available_from_milestone: 1,
    },
    ActionDef {
        id: "app_settings",
        label: "Settings",
        default_chord: None,
        group: "app",
        available_from_milestone: 1,
    },
    ActionDef {
        id: "app_open_save_folder",
        label: "Open save folder",
        default_chord: None,
        group: "app",
        available_from_milestone: 1,
    },
    ActionDef {
        id: "app_pause_hotkeys",
        label: "Pause hotkeys",
        default_chord: None,
        group: "app",
        available_from_milestone: 2,
    },
    ActionDef {
        id: "app_exit",
        label: "Exit",
        default_chord: None,
        group: "app",
        available_from_milestone: 0,
    },
];

pub fn find(id: &ActionId) -> Option<&'static ActionDef> {
    CATALOGUE
        .iter()
        .find(|definition| definition.id == id.as_str())
}

pub fn is_workflow(id: &ActionId) -> bool {
    id.as_str()
        .strip_prefix(WORKFLOW_PREFIX)
        .is_some_and(|rest| !rest.is_empty())
}

pub fn available(id: &ActionId, current_milestone: u8) -> bool {
    if is_workflow(id) {
        return current_milestone >= 7;
    }
    find(id).is_some_and(|definition| definition.available_from_milestone <= current_milestone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_ids_are_unique() {
        let mut ids: Vec<&str> = CATALOGUE.iter().map(|definition| definition.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOGUE.len());
    }

    #[test]
    fn default_chords_match_the_plan() {
        let defaults = [
            ("capture_region", Some("PrtSc")),
            ("capture_window", Some("Alt+PrtSc")),
            ("capture_screen", Some("Ctrl+PrtSc")),
            ("capture_last_region", Some("Ctrl+Shift+R")),
            ("record_video", Some("Shift+PrtSc")),
            ("record_gif", Some("Ctrl+Shift+G")),
            ("tool_ocr", Some("Ctrl+Shift+T")),
            ("tool_colour", Some("Ctrl+Shift+C")),
            ("tool_ruler", Some("Ctrl+Shift+M")),
            ("upload_clipboard", Some("Ctrl+Shift+V")),
        ];
        for (id, chord) in defaults {
            let definition = find(&ActionId::new(id)).expect("catalogue action");
            assert_eq!(definition.default_chord, chord, "{id}");
        }
    }

    #[test]
    fn workflow_ids_are_available_from_milestone_seven() {
        let id = ActionId::new("workflow:default");
        assert!(is_workflow(&id));
        assert!(!available(&id, 6));
        assert!(available(&id, 7));
        assert!(!is_workflow(&ActionId::new("workflow:")));
    }
}
