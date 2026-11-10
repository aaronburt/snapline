use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use super::HistoryError;

pub const CURRENT_VERSION: u32 = 4;
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_init.sql"),
    include_str!("../../migrations/0002_upload_etag.sql"),
    include_str!("../../migrations/0003_capture_sequence.sql"),
    include_str!("../../migrations/0004_files.sql"),
];
static MIGRATION_LOCK: Mutex<()> = Mutex::new(());

pub fn open_write(path: &Path) -> Result<Connection, HistoryError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(HistoryError::Folder)?;
        }
    }
    let mut connection = Connection::open(path)?;
    configure(&connection)?;
    let mode: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(HistoryError::JournalMode(mode));
    }
    migrate(&mut connection)?;
    Ok(connection)
}

pub fn open_read(path: &Path) -> Result<Connection, HistoryError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&connection)?;
    let found = schema_version(&connection)?;
    if found > CURRENT_VERSION {
        return Err(HistoryError::FutureSchema {
            found,
            supported: CURRENT_VERSION,
        });
    }
    if found < CURRENT_VERSION {
        return Err(HistoryError::StaleSchema {
            found,
            required: CURRENT_VERSION,
        });
    }
    Ok(connection)
}

pub fn schema_version(connection: &Connection) -> Result<u32, HistoryError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    u32::try_from(version).map_err(|_| HistoryError::InvalidSchemaVersion(version))
}

pub fn migrate(connection: &mut Connection) -> Result<(), HistoryError> {
    let _guard = MIGRATION_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let found = schema_version(connection)?;
    if found > CURRENT_VERSION {
        return Err(HistoryError::FutureSchema {
            found,
            supported: CURRENT_VERSION,
        });
    }
    if found == CURRENT_VERSION {
        return Ok(());
    }
    backup(connection)?;
    if let Err(error) = connection.pragma_update(None, "foreign_keys", false) {
        let _ = connection.pragma_update(None, "foreign_keys", true);
        return Err(error.into());
    }
    let migration_result = (|| {
        let foreign_keys: i64 =
            connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        if foreign_keys != 0 {
            return Err(HistoryError::ForeignKeys);
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for version in (found + 1)..=CURRENT_VERSION {
            let migration = MIGRATIONS
                .get((version - 1) as usize)
                .copied()
                .ok_or(HistoryError::MissingMigration(version))?;
            transaction.execute_batch(migration)?;
            transaction.pragma_update(None, "user_version", version)?;
        }
        let violation = transaction
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()?;
        if violation.is_some() {
            return Err(HistoryError::ForeignKeys);
        }
        transaction.commit()?;
        Ok(())
    })();
    let restore_result = (|| {
        connection.pragma_update(None, "foreign_keys", true)?;
        let foreign_keys: i64 =
            connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        if foreign_keys != 1 {
            return Err(HistoryError::ForeignKeys);
        }
        Ok(())
    })();
    migration_result?;
    restore_result
}

fn backup(connection: &Connection) -> Result<(), HistoryError> {
    let database: String = connection.query_row("PRAGMA database_list", [], |row| row.get(2))?;
    if database.is_empty() {
        return Ok(());
    }
    let mut backup = std::ffi::OsString::from(database);
    backup.push(".bak");
    let backup = std::path::PathBuf::from(backup);
    if backup.exists() {
        std::fs::remove_file(&backup).map_err(HistoryError::Backup)?;
    }
    connection.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
    Ok(())
}

fn configure(connection: &Connection) -> Result<(), HistoryError> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn database() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("history.db");
        (directory, path)
    }

    #[test]
    fn an_empty_database_is_migrated_to_the_current_version() {
        let (_directory, path) = database();
        let connection = open_write(&path).expect("open");
        assert_eq!(
            schema_version(&connection).expect("version"),
            CURRENT_VERSION
        );
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('capture', 'upload')",
                [],
                |row| row.get(0),
            )
            .expect("count tables");
        assert_eq!(count, 2);
        let index: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('capture_created', 'upload_capture')",
                [],
                |row| row.get(0),
            )
            .expect("count indexes");
        assert_eq!(index, 2);
        let fts: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'capture_fts'",
                [],
                |row| row.get(0),
            )
            .expect("count fts");
        assert_eq!(fts, 1);
    }

    #[test]
    fn wal_and_foreign_keys_are_enabled() {
        let (_directory, path) = database();
        let connection = open_write(&path).expect("open");
        let mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal mode");
        assert_eq!(mode, "wal");
        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("foreign keys");
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn migration_two_adds_the_etag_column_and_keeps_existing_rows() {
        let (_directory, path) = database();
        let connection = Connection::open(&path).expect("open version one");
        connection
            .execute_batch(include_str!("../../migrations/0001_init.sql"))
            .expect("version one schema");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("version one");
        connection
            .execute(
                "INSERT INTO capture (id, kind, path, bytes, created_at) VALUES (1, 'image', 'C:\\shot.png', 12, 1)",
                [],
            )
            .expect("capture");
        connection
            .execute(
                "INSERT INTO upload (id, capture_id, dest_id, state, url, attempts, updated_at) VALUES (1, 1, 'imgur', 'done', 'https://example.com/shot.png', 1, 2)",
                [],
            )
            .expect("upload");
        drop(connection);

        let connection = open_write(&path).expect("migrate");
        assert_eq!(
            schema_version(&connection).expect("version"),
            CURRENT_VERSION
        );
        let (url, etag): (Option<String>, Option<String>) = connection
            .query_row("SELECT url, etag FROM upload WHERE id = 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("upload row");
        assert_eq!(url.as_deref(), Some("https://example.com/shot.png"));
        assert_eq!(etag, None);
        let captures: i64 = connection
            .query_row("SELECT COUNT(*) FROM capture", [], |row| row.get(0))
            .expect("capture count");
        assert_eq!(captures, 1);
    }

    #[test]
    fn a_future_schema_is_refused() {
        let (_directory, path) = database();
        let connection = open_write(&path).expect("open");
        connection
            .pragma_update(None, "user_version", CURRENT_VERSION + 1)
            .expect("bump version");
        drop(connection);
        assert!(matches!(
            open_write(&path),
            Err(HistoryError::FutureSchema { .. })
        ));
    }

    #[test]
    fn migration_four_preserves_uploads_and_backs_up_version_three() {
        let (_directory, path) = database();
        let connection = Connection::open(&path).expect("open version three");
        for migration in &MIGRATIONS[..3] {
            connection.execute_batch(migration).expect("seed schema");
        }
        connection
            .pragma_update(None, "user_version", 3)
            .expect("version");
        connection.execute_batch(
            "INSERT INTO capture (id, kind, path, bytes, created_at) VALUES (7, 'image', 'shot.png', 10, 1);
             INSERT INTO upload (id, capture_id, dest_id, state, updated_at) VALUES (9, 7, 'sftp', 'done', 2);
             INSERT INTO capture_fts (rowid, name) VALUES (7, 'shot.png');",
        ).expect("seed history");
        drop(connection);

        let upgraded = open_write(&path).expect("upgrade");
        let row: (String, bool, i64, i64) = upgraded
            .query_row(
                "SELECT capture.origin, capture.owned, upload.sent, upload.total
             FROM capture JOIN upload ON upload.capture_id = capture.id
             WHERE capture.id = 7 AND upload.id = 9",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("preserved upload");
        assert_eq!(row, ("capture".to_owned(), true, 0, 0));
        let indexed: i64 = upgraded
            .query_row(
                "SELECT rowid FROM capture_fts WHERE capture_fts MATCH 'shot'",
                [],
                |row| row.get(0),
            )
            .expect("preserved search index");
        assert_eq!(indexed, 7);

        let backup = Connection::open_with_flags(
            path.with_extension("db.bak"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open backup");
        assert_eq!(schema_version(&backup).expect("backup version"), 3);
        let uploads: i64 = backup
            .query_row("SELECT count(*) FROM upload", [], |row| row.get(0))
            .expect("backup uploads");
        assert_eq!(uploads, 1);
    }

    #[test]
    fn a_read_only_connection_opens_a_current_database() {
        let (_directory, path) = database();
        let connection = open_write(&path).expect("open");
        drop(connection);
        let read = open_read(&path).expect("read only open");
        assert_eq!(schema_version(&read).expect("version"), CURRENT_VERSION);
    }

    #[test]
    fn a_read_only_connection_refuses_a_stale_database() {
        let (_directory, path) = database();
        let connection = Connection::open(&path).expect("open");
        connection
            .execute_batch("CREATE TABLE marker (id INTEGER PRIMARY KEY)")
            .expect("create marker");
        drop(connection);
        assert!(matches!(
            open_read(&path),
            Err(HistoryError::StaleSchema { found: 0, .. })
        ));
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let (_directory, path) = database();
        let mut connection = open_write(&path).expect("open");
        migrate(&mut connection).expect("second migration");
        assert_eq!(
            schema_version(&connection).expect("version"),
            CURRENT_VERSION
        );
    }
}
