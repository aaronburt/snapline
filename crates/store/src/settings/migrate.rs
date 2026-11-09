use thiserror::Error;

pub const CURRENT_VERSION: u32 = 1;

type Step = fn(toml::Table) -> Result<toml::Table, MigrationError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MigrationError {
    #[error("settings version {0} is newer than this build supports")]
    FutureVersion(u32),
    #[error("the settings version is not a whole number")]
    InvalidVersion,
    #[error("no settings migration exists for version {0}")]
    MissingStep(u32),
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
    for version in from..=CURRENT_VERSION {
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
        1 => Some(keep_version_one),
        _ => None,
    }
}

fn keep_version_one(table: toml::Table) -> Result<toml::Table, MigrationError> {
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_from_version_one_to_one_keeps_the_table() {
        let mut table = toml::Table::new();
        table.insert("version".to_owned(), toml::Value::Integer(1));
        table.insert(
            "marker".to_owned(),
            toml::Value::String("kept".to_owned()),
        );
        let migrated = migrate(table, 1).expect("migration");
        assert_eq!(
            migrated.get("version"),
            Some(&toml::Value::Integer(i64::from(CURRENT_VERSION)))
        );
        assert_eq!(
            migrated.get("marker"),
            Some(&toml::Value::String("kept".to_owned()))
        );
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
