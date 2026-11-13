use anyhow::{bail, Result};
use snapline_services::dto::settings::LinkFormatDto;
use snapline_services::dto::workflow::{
    StepDto, WorkflowInfo, WorkflowInput, WorkflowReq, WorkflowResp,
};

use crate::backend::Backend;

pub const ROWS: &[(&str, &str)] = &[
    ("copy", "Copy image to clipboard"),
    ("edit", "Open in editor to annotate"),
    ("save", "Save to folder"),
    ("upload", "Upload to default destination"),
    ("link", "Copy link to clipboard"),
    ("sound", "Play a sound"),
];

pub struct Row {
    pub id: String,
    pub label: String,
    pub checked: bool,
    pub note: String,
}

fn matches_row(step: &StepDto, row_id: &str) -> bool {
    matches!(
        (row_id, step),
        ("copy", StepDto::CopyImage)
            | ("edit", StepDto::Annotate)
            | ("save", StepDto::Save)
            | ("upload", StepDto::Upload { .. })
            | ("link", StepDto::CopyLink { .. })
            | ("sound", StepDto::PlaySound)
    )
}

fn is_known_step(step: &StepDto) -> bool {
    matches!(
        step,
        StepDto::CopyImage
            | StepDto::Annotate
            | StepDto::Save
            | StepDto::Upload { .. }
            | StepDto::CopyLink { .. }
            | StepDto::PlaySound
    )
}

fn has_step(steps: &[StepDto], row_id: &str) -> bool {
    steps.iter().any(|step| matches_row(step, row_id))
}

pub fn build_rows(steps: &[StepDto], upload_note: &str) -> Vec<Row> {
    let canonical = |id: &str| ROWS.iter().position(|(row, _)| *row == id).unwrap_or(0);
    let mut order: Vec<&str> = ROWS
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| has_step(steps, id))
        .collect();
    for (id, _) in ROWS {
        if order.contains(id) {
            continue;
        }
        let at = order
            .iter()
            .position(|existing| canonical(existing) > canonical(id))
            .unwrap_or(order.len());
        order.insert(at, id);
    }
    order
        .into_iter()
        .map(|id| {
            let label = ROWS
                .iter()
                .find(|(row, _)| *row == id)
                .map(|(_, l)| *l)
                .unwrap_or("");
            Row {
                id: id.to_string(),
                label: label.to_string(),
                checked: has_step(steps, id),
                note: if id == "upload" {
                    upload_note.to_string()
                } else {
                    String::new()
                },
            }
        })
        .collect()
}

pub fn build_steps(rows: &[Row], current: &[StepDto], link_format: LinkFormatDto) -> Vec<StepDto> {
    let mut steps: Vec<StepDto> = rows
        .iter()
        .filter(|row| row.checked)
        .filter_map(|row| step_for(&row.id, current, link_format))
        .collect();
    for (index, step) in current.iter().enumerate() {
        if is_known_step(step) {
            continue;
        }
        let at = index.min(steps.len());
        steps.insert(at, step.clone());
    }
    steps
}

fn step_for(id: &str, current: &[StepDto], link_format: LinkFormatDto) -> Option<StepDto> {
    if let Some(existing) = current.iter().find(|step| matches_row(step, id)) {
        return Some(existing.clone());
    }
    Some(match id {
        "copy" => StepDto::CopyImage,
        "edit" => StepDto::Annotate,
        "save" => StepDto::Save,
        "upload" => StepDto::Upload { dest: None },
        "link" => StepDto::CopyLink {
            format: link_format,
        },
        "sound" => StepDto::PlaySound,
        _ => return None,
    })
}

pub fn load(backend: &Backend) -> Result<(WorkflowInfo, usize)> {
    let current = match backend.workflow_request(WorkflowReq::Get {
        id: "default".into(),
    })? {
        WorkflowResp::Workflow { workflow } => workflow,
        other => bail!("unexpected workflow response: {other:?}"),
    };
    let count = match backend.workflow_request(WorkflowReq::List)? {
        WorkflowResp::List { workflows, .. } => workflows.len(),
        other => bail!("unexpected workflow list response: {other:?}"),
    };
    Ok((current, count))
}

pub fn save(
    backend: &Backend,
    current: &WorkflowInfo,
    steps: Vec<StepDto>,
) -> Result<WorkflowInfo> {
    let input = WorkflowInput {
        name: current.name.clone(),
        hotkey: current.hotkey.clone(),
        enabled: current.enabled,
        steps,
    };
    match backend.workflow_request(WorkflowReq::Update {
        id: "default".into(),
        workflow: input,
    })? {
        WorkflowResp::Workflow { workflow } => Ok(workflow),
        other => bail!("unexpected workflow response: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_rows_orders_canonical_rows_and_marks_checked() {
        let steps = vec![
            StepDto::Upload {
                dest: Some("imgur".to_owned()),
            },
            StepDto::CopyImage,
        ];
        let rows = build_rows(&steps, "Imgur");
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["copy", "edit", "save", "upload", "link", "sound"]);
        assert!(rows[0].checked);
        assert!(!rows[1].checked);
        assert!(rows[3].checked);
        assert_eq!(rows[3].note, "Imgur");
        assert!(rows[0].note.is_empty());
    }

    #[test]
    fn build_steps_preserves_existing_upload_dest_and_unknown_steps() {
        let current = vec![
            StepDto::CopyImage,
            StepDto::Notify,
            StepDto::Upload {
                dest: Some("s3".to_owned()),
            },
        ];
        let mut rows = build_rows(&current, "Amazon S3");
        if let Some(link) = rows.iter_mut().find(|r| r.id == "link") {
            link.checked = true;
        }
        let next = build_steps(&rows, &current, LinkFormatDto::Markdown);
        assert_eq!(
            next,
            vec![
                StepDto::CopyImage,
                StepDto::Notify,
                StepDto::Upload {
                    dest: Some("s3".to_owned()),
                },
                StepDto::CopyLink {
                    format: LinkFormatDto::Markdown,
                },
            ]
        );
    }
}
