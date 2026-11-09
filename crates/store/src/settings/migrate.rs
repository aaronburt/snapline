use thiserror::Error;

use snapline_core::presets::initial_workflows;
use snapline_core::workflow::Workflow;

pub const CURRENT_VERSION: u32 = 2;

type Step = fn(toml::Table) -> Result<toml::Table, MigrationError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MigrationError {
    #[error("settings version {0} is newer than this build supports")]
    FutureVersion(u32),
    #[error("the settings version is not a whole number")]
    InvalidVersion,
    #[error("no settings migration exists for version {0}")]
    MissingStep(u32),
    #[error("could not create the default workflow")]
    DefaultWorkflow,
}

pub fn version_of(table: &toml::Table) -> Result<u32, MigrationError> {
    match table.get("version") {
        None => Ok(CURRENT_VERSION),
        Some(toml::Value::Integer(value)) => {
            u32::try_from(*value).map_err(|_| MigrationError::InvalidVersion)
        }
        Some(_) => Err(MigrationError::InvalidVersion),
    }
}

pub fn migrate(mut table: toml::Table, from: u32) -> Result<toml::Table, MigrationError> {
    if from > CURRENT_VERSION {
        return Err(MigrationError::FutureVersion(from));
    }
    for version in (from + 1)..=CURRENT_VERSION {
        let step = step_for(version).ok_or(MigrationError::MissingStep(version))?;
        table = step(table)?;
    }
    table.insert(
        "version".to_owned(),
        toml::Value::Integer(i64::from(CURRENT_VERSION)),
    );
    Ok(table)
}

fn step_for(version: u32) -> Option<Step> {
    match version {
        2 => Some(add_default_workflow),
        _ => None,
    }
}

fn add_default_workflow(mut table: toml::Table) -> Result<toml::Table, MigrationError> {
    if !table.contains_key("workflows") {
        table.insert("workflows".to_owned(), toml::Value::Table(toml::Table::new()));
    }
    if let Some(toml::Value::Table(workflows)) = table.get_mut("workflows") {
        let existing: Vec<Workflow> = match workflows.get("list") {
            Some(toml::Value::Array(items)) if !items.is_empty() => {
                toml::Value::Array(items.clone())
                    .try_into()
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        if existing.is_empty() {
            let presets = initial_workflows(&existing);
            let values = presets
                .iter()
                .map(toml::Value::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| MigrationError::DefaultWorkflow)?;
            workflows.insert("list".to_owned(), toml::Value::Array(values));
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use snapline_core::workflow::{Step, DEFAULT_WORKFLOW_ID, DEFAULT_WORKFLOW_NAME};

    #[test]
    fn a_version_one_file_gains_the_default_workflow() {
        let mut table = toml::Table::new();
        table.insert("version".to_owned(), toml::Value::Integer(1));
        table.insert("marker".to_owned(), toml::Value::String("kept".to_owned()));
        let migrated = migrate(table, 1).expect("migration");
        assert_eq!(
            migrated.get("version"),
            Some(&toml::Value::Integer(i64::from(CURRENT_VERSION)))
        );
        assert_eq!(
            migrated.get("marker"),
            Some(&toml::Value::String("kept".to_owned()))
        );
        let Some(toml::Value::Table(workflows)) = migrated.get("workflows") else {
            panic!("workflows table");
        };
        let Some(toml::Value::Array(list)) = workflows.get("list") else {
            panic!("workflow list");
        };
        assert_eq!(list.len(), 5);
        let Some(toml::Value::Table(workflow)) = list.first() else {
            panic!("default workflow");
        };
        assert_eq!(
            workflow.get("id"),
            Some(&toml::Value::String(DEFAULT_WORKFLOW_ID.to_owned()))
        );
        assert_eq!(
            workflow.get("name"),
            Some(&toml::Value::String(DEFAULT_WORKFLOW_NAME.to_owned()))
        );
        assert_eq!(workflow.get("enabled"), Some(&toml::Value::Boolean(true)));
        assert!(workflow.get("hotkey").is_none());
        let loaded: crate::settings::Settings =
            crate::settings::Settings::deserialize(toml::Value::Table(migrated.clone()))
                .expect("settings");
        assert_eq!(
            loaded.workflows.list[0].steps,
            vec![
                Step::CopyImage,
                Step::Save,
                Step::Upload { dest: None },
                Step::CopyLink {
                    format: snapline_core::link_format::LinkFormat::Direct,
                },
                Step::PlaySound,
            ]
        );
    }

    #[test]
    fn a_version_one_file_with_workflows_is_left_alone() {
        let mut table = toml::Table::new();
        table.insert("version".to_owned(), toml::Value::Integer(1));
        let mut workflows = toml::Table::new();
        workflows.insert(
            "list".to_owned(),
            toml::Value::Array(vec![toml::Value::Table(toml::Table::new())]),
        );
        table.insert("workflows".to_owned(), toml::Value::Table(workflows));
        let migrated = migrate(table, 1).expect("migration");
        let Some(toml::Value::Table(workflows)) = migrated.get("workflows") else {
            panic!("workflows table");
        };
        let Some(toml::Value::Array(list)) = workflows.get("list") else {
            panic!("workflow list");
        };
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn a_version_two_file_keeps_an_empty_list() {
        let mut table = toml::Table::new();
        table.insert("version".to_owned(), toml::Value::Integer(2));
        let migrated = migrate(table, 2).expect("migration");
        assert!(migrated.get("workflows").is_none());
    }

    #[test]
    fn a_future_version_is_refused() {
        assert_eq!(
            migrate(toml::Table::new(), CURRENT_VERSION + 1),
            Err(MigrationError::FutureVersion(CURRENT_VERSION + 1))
        );
    }

    #[test]
    fn a_missing_version_reads_as_the_current_version() {
        assert_eq!(version_of(&toml::Table::new()), Ok(CURRENT_VERSION));
    }
}
