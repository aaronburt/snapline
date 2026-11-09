use serde::{Deserialize, Serialize};

pub use snapline_core::workflow::Workflow;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflows {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<Workflow>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

pub fn default_workflow() -> Workflow {
    snapline_core::presets::default_workflow()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use snapline_core::chord::Chord;
    use snapline_core::ids::DestId;

    use super::*;

    #[test]
    fn the_default_workflow_uses_the_general_default_steps() {
        let workflow = default_workflow();
        assert_eq!(workflow.id.as_str(), DEFAULT_WORKFLOW_ID);
        assert_eq!(workflow.name, DEFAULT_WORKFLOW_NAME);
        assert!(workflow.enabled);
        assert_eq!(workflow.hotkey, None);
        assert_eq!(
            workflow.steps,
            vec![
                Step::CopyImage,
                Step::Save,
                Step::Upload { dest: None },
                Step::CopyLink {
                    format: LinkFormat::Direct,
                },
                Step::PlaySound,
            ]
        );
    }

    #[test]
    fn workflows_round_trip_through_toml() {
        let mut first = Workflow::new("annotate-and-share", "Annotate and share");
        first.hotkey = Some(Chord::from_str("Ctrl+Shift+A").expect("chord"));
        first.steps = vec![
            Step::Capture {
                kind: snapline_core::workflow::CaptureKind::Region,
            },
            Step::Annotate,
            Step::Upload {
                dest: Some(DestId::new("imgur")),
            },
            Step::CopyLink {
                format: LinkFormat::Markdown,
            },
        ];
        let mut second = Workflow::new("save-quietly", "Save quietly");
        second.enabled = false;
        second.steps = vec![Step::Save];
        let mut workflows = Workflows {
            list: vec![first, second],
            extra: toml::Table::new(),
        };
        workflows
            .extra
            .insert("unknown_workflows".to_owned(), toml::Value::Integer(7));

        let text = toml::to_string_pretty(&workflows).expect("serialise");
        let loaded: Workflows = toml::from_str(&text).expect("deserialise");
        assert_eq!(loaded, workflows);
    }

    #[test]
    fn workflow_chords_round_trip_as_display_strings() {
        let mut workflow = Workflow::new("bug-report", "Bug report clip");
        workflow.hotkey = Some(Chord::from_str("Ctrl+Alt+R").expect("chord"));
        let workflows = Workflows {
            list: vec![workflow],
            extra: toml::Table::new(),
        };

        let text = toml::to_string_pretty(&workflows).expect("serialise");
        assert!(text.contains("hotkey = \"Ctrl+Alt+R\""));
        let loaded: Workflows = toml::from_str(&text).expect("deserialise");
        assert_eq!(
            loaded.list[0]
                .hotkey
                .map(|chord| chord.to_string())
                .as_deref(),
            Some("Ctrl+Alt+R")
        );
    }

    #[test]
    fn disabled_workflows_persist() {
        let mut workflow = Workflow::new("grab-text", "Grab text");
        workflow.enabled = false;
        let workflows = Workflows {
            list: vec![workflow],
            extra: toml::Table::new(),
        };

        let text = toml::to_string_pretty(&workflows).expect("serialise");
        let loaded: Workflows = toml::from_str(&text).expect("deserialise");
        assert_eq!(loaded.list.len(), 1);
        assert!(!loaded.list[0].enabled);
        assert_eq!(loaded.list[0].name, "Grab text");
    }

    #[test]
    fn unknown_workflow_keys_are_preserved() {
        let text = concat!(
            "unknown_workflows = \"kept\"\n",
            "list = [{ id = \"other\", name = \"Other\", enabled = false }]\n",
        );
        let workflows: Workflows = toml::from_str(text).expect("deserialise");
        assert_eq!(workflows.list.len(), 1);
        assert_eq!(
            workflows.extra.get("unknown_workflows"),
            Some(&toml::Value::String("kept".to_owned()))
        );

        let saved = toml::to_string_pretty(&workflows).expect("serialise");
        assert!(saved.contains("unknown_workflows"));
    }

    #[test]
    fn workflow_names_keep_their_text() {
        let workflows = Workflows {
            list: vec![default_workflow()],
            extra: toml::Table::new(),
        };
        let text = toml::to_string_pretty(&workflows).expect("serialise");
        let loaded: Workflows = toml::from_str(&text).expect("deserialise");
        assert_eq!(loaded.list[0].name, "Screenshot to link");
        assert!(loaded.list[0].is_default());
    }
}
