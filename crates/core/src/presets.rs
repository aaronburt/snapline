use crate::chord::Chord;
use crate::link_format::LinkFormat;
use crate::workflow::{
    CaptureKind, RecordKind, Step, Workflow, DEFAULT_WORKFLOW_ID, DEFAULT_WORKFLOW_NAME,
};

pub const ANNOTATE_AND_SHARE_ID: &str = "annotate-and-share";
pub const BUG_REPORT_CLIP_ID: &str = "bug-report-clip";
pub const GRAB_TEXT_ID: &str = "grab-text";
pub const SAVE_QUIETLY_ID: &str = "save-quietly";

pub fn default_workflow() -> Workflow {
    let mut workflow = Workflow::new(DEFAULT_WORKFLOW_ID, DEFAULT_WORKFLOW_NAME);
    workflow.steps = vec![
        Step::CopyImage,
        Step::Save,
        Step::Upload { dest: None },
        Step::CopyLink {
            format: LinkFormat::Direct,
        },
        Step::PlaySound,
    ];
    workflow
}

pub fn presets() -> Vec<Workflow> {
    vec![
        default_workflow(),
        preset(
            ANNOTATE_AND_SHARE_ID,
            "Annotate and share",
            Some("Ctrl+Shift+A"),
            true,
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::Annotate,
                Step::Upload { dest: None },
                Step::CopyLink {
                    format: LinkFormat::Markdown,
                },
            ],
        ),
        preset(
            BUG_REPORT_CLIP_ID,
            "Bug report clip",
            Some("Ctrl+Alt+R"),
            true,
            vec![
                Step::Record {
                    kind: RecordKind::Video,
                },
                Step::ConvertToGif,
                Step::Upload { dest: None },
                Step::CopyLink {
                    format: LinkFormat::Direct,
                },
            ],
        ),
        preset(
            GRAB_TEXT_ID,
            "Grab text",
            Some("Ctrl+Alt+T"),
            false,
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::ExtractText,
                Step::CopyText,
            ],
        ),
        preset(
            SAVE_QUIETLY_ID,
            "Save quietly",
            Some("Ctrl+Alt+PrtSc"),
            false,
            vec![
                Step::Capture {
                    kind: CaptureKind::Screen,
                },
                Step::Save,
            ],
        ),
    ]
}

pub fn initial_workflows(existing: &[Workflow]) -> Vec<Workflow> {
    if existing.is_empty() {
        presets()
    } else {
        existing.to_vec()
    }
}

fn preset(id: &str, name: &str, hotkey: Option<&str>, enabled: bool, steps: Vec<Step>) -> Workflow {
    let mut workflow = Workflow::new(id, name);
    workflow.hotkey = hotkey.and_then(|text| Chord::parse(text).ok());
    workflow.enabled = enabled;
    workflow.steps = steps;
    workflow
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::actions::{self, ActionId, WORKFLOW_PREFIX};
    use crate::hotkey_plan::{self, Owner};
    use crate::workflow::{CaptureKind, RecordKind, Step};

    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::from_str(text).expect("valid chord")
    }

    fn action_defaults() -> Vec<(Owner, Chord)> {
        actions::CATALOGUE
            .iter()
            .filter_map(|definition| {
                let text = definition.default_chord?;
                Some((
                    Owner::from(ActionId::new(definition.id)),
                    Chord::from_str(text).ok()?,
                ))
            })
            .collect()
    }

    fn workflow_entries(workflows: &[Workflow]) -> Vec<(Owner, Chord)> {
        workflows
            .iter()
            .filter(|workflow| workflow.enabled)
            .filter_map(|workflow| {
                let hotkey = workflow.hotkey?;
                Some((
                    Owner::new(format!("{WORKFLOW_PREFIX}{}", workflow.id)),
                    hotkey,
                ))
            })
            .collect()
    }

    #[test]
    fn the_five_presets_have_the_design_names() {
        let names: Vec<String> = presets()
            .into_iter()
            .map(|workflow| workflow.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "Screenshot to link",
                "Annotate and share",
                "Bug report clip",
                "Grab text",
                "Save quietly",
            ]
        );
    }

    #[test]
    fn the_five_presets_have_the_design_ids() {
        let ids: Vec<String> = presets()
            .into_iter()
            .map(|workflow| workflow.id.to_string())
            .collect();
        assert_eq!(
            ids,
            vec![
                DEFAULT_WORKFLOW_ID.to_owned(),
                ANNOTATE_AND_SHARE_ID.to_owned(),
                BUG_REPORT_CLIP_ID.to_owned(),
                GRAB_TEXT_ID.to_owned(),
                SAVE_QUIETLY_ID.to_owned(),
            ]
        );
    }

    #[test]
    fn the_presets_have_the_design_enabled_states() {
        let enabled: Vec<bool> = presets()
            .into_iter()
            .map(|workflow| workflow.enabled)
            .collect();
        assert_eq!(enabled, vec![true, true, true, false, false]);
    }

    #[test]
    fn the_presets_have_the_design_chords() {
        let hotkeys: Vec<Option<String>> = presets()
            .into_iter()
            .map(|workflow| workflow.hotkey.map(|hotkey| hotkey.to_string()))
            .collect();
        assert_eq!(
            hotkeys,
            vec![
                None,
                Some("Ctrl+Shift+A".to_owned()),
                Some("Ctrl+Alt+R".to_owned()),
                Some("Ctrl+Alt+T".to_owned()),
                Some("Ctrl+Alt+PrtSc".to_owned()),
            ]
        );
    }

    #[test]
    fn every_preset_chord_parses_and_round_trips() {
        for text in ["Ctrl+Shift+A", "Ctrl+Alt+R", "Ctrl+Alt+T", "Ctrl+Alt+PrtSc"] {
            let parsed = Chord::from_str(text).expect("valid chord");
            assert_eq!(parsed.to_string(), text);
            assert_eq!(Chord::from_str(&parsed.to_string()), Ok(parsed));
        }
    }

    #[test]
    fn the_default_preset_is_screenshot_to_link_without_a_chord() {
        let preset = presets().remove(0);
        assert!(preset.is_default());
        assert_eq!(preset.name, "Screenshot to link");
        assert_eq!(preset.hotkey, None);
        assert!(preset.enabled);
    }

    #[test]
    fn the_default_workflow_matches_the_general_page_steps() {
        let workflow = default_workflow();
        assert_eq!(workflow, presets().remove(0));
        assert_eq!(workflow.id.as_str(), "default");
        assert_eq!(workflow.name, "Screenshot to link");
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
    fn annotate_and_share_uses_the_design_steps() {
        let workflow = presets().remove(1);
        assert_eq!(
            workflow.steps,
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::Annotate,
                Step::Upload { dest: None },
                Step::CopyLink {
                    format: LinkFormat::Markdown,
                },
            ]
        );
        assert_eq!(workflow.hotkey, Some(chord("Ctrl+Shift+A")));
        assert!(workflow.enabled);
    }

    #[test]
    fn bug_report_clip_uses_the_design_steps_and_upload_default() {
        let workflow = presets().remove(2);
        assert_eq!(
            workflow.steps,
            vec![
                Step::Record {
                    kind: RecordKind::Video,
                },
                Step::ConvertToGif,
                Step::Upload { dest: None },
                Step::CopyLink {
                    format: LinkFormat::Direct,
                },
            ]
        );
        assert_eq!(workflow.hotkey, Some(chord("Ctrl+Alt+R")));
        assert!(workflow.enabled);
    }

    #[test]
    fn grab_text_uses_the_design_steps() {
        let workflow = presets().remove(3);
        assert_eq!(
            workflow.steps,
            vec![
                Step::Capture {
                    kind: CaptureKind::Region,
                },
                Step::ExtractText,
                Step::CopyText,
            ]
        );
        assert_eq!(workflow.hotkey, Some(chord("Ctrl+Alt+T")));
        assert!(!workflow.enabled);
    }

    #[test]
    fn save_quietly_uses_the_design_steps() {
        let workflow = presets().remove(4);
        assert_eq!(
            workflow.steps,
            vec![
                Step::Capture {
                    kind: CaptureKind::Screen,
                },
                Step::Save,
            ]
        );
        assert_eq!(workflow.hotkey, Some(chord("Ctrl+Alt+PrtSc")));
        assert!(!workflow.enabled);
    }

    #[test]
    fn the_presets_other_than_the_default_are_valid() {
        for workflow in presets().into_iter().skip(1) {
            assert!(workflow.is_valid(), "{}", workflow.name);
        }
    }

    #[test]
    fn the_full_default_set_with_every_workflow_enabled_has_no_internal_clash() {
        let mut workflows = presets();
        for workflow in &mut workflows {
            workflow.enabled = true;
        }
        let mut entries = action_defaults();
        entries.extend(workflow_entries(&workflows));

        let planned = hotkey_plan::plan(&entries);
        assert!(planned.blocked.is_empty(), "{:?}", planned.blocked);

        for definition in actions::CATALOGUE {
            if let Some(text) = definition.default_chord {
                assert!(planned.is_bound(&chord(text)), "{}", definition.id);
            }
        }
        for workflow in &workflows {
            if let Some(hotkey) = workflow.hotkey {
                assert!(planned.is_bound(&hotkey), "{}", workflow.name);
            }
        }
    }

    #[test]
    fn initial_workflows_creates_the_five_presets_on_a_fresh_install() {
        assert_eq!(initial_workflows(&[]), presets());
    }

    #[test]
    fn initial_workflows_does_not_overwrite_existing_workflows() {
        let mut existing = Workflow::new("mine", "My workflow");
        existing.steps = vec![Step::Capture {
            kind: CaptureKind::Window,
        }];
        let result = initial_workflows(std::slice::from_ref(&existing));
        assert_eq!(result, vec![existing]);
    }
}
