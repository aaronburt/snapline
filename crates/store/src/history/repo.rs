use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row};
use time::{Duration, OffsetDateTime, PrimitiveDateTime, Time};

use super::{
    capture_name, BorrowedFile, Capture, CaptureId, CaptureKind, Cursor, DeletedCapture,
    HistoryError, HistoryFilter, HistoryPage, HistoryStats, NewCapture, Origin, UploadRecord,
    UploadState, UploadUpdate,
};

const PAGE_SIZE: usize = 60;

const CAPTURE_SELECT: &str = "
SELECT c.id, c.kind, c.origin, c.owned, c.path, c.bytes, c.source_modified_ms,
       c.width, c.height, c.duration_ms, c.source_app, c.created_at, c.ocr_text,
       u.id, u.capture_id, u.dest_id, u.state, u.url, u.deletion_url, u.error, u.attempts,
       u.updated_at, u.etag, u.sent, u.total
FROM capture c
LEFT JOIN upload u ON u.id = (
    SELECT u2.id
    FROM upload u2
    WHERE u2.capture_id = c.id
    ORDER BY u2.updated_at DESC, u2.id DESC
    LIMIT 1
)
";

const PAGE_WHERE: &str = "
WHERE (
    ?1 = 0
    OR (?1 = 1 AND c.kind = 'image')
    OR (?1 = 2 AND c.kind = 'video')
    OR (?1 = 3 AND c.kind = 'gif')
    OR (?1 = 4 AND EXISTS (
        SELECT 1 FROM upload u3 WHERE u3.capture_id = c.id AND u3.state = 'done'
    ))
    OR (?1 = 5 AND NOT EXISTS (
        SELECT 1 FROM upload u4 WHERE u4.capture_id = c.id
    ))
    OR (?1 = 6 AND c.kind IN ('text','file'))
)
AND (
    ?2 IS NULL
    OR c.created_at < ?2
    OR (c.created_at = ?2 AND c.id < ?3)
)
";

const PAGE_ORDER: &str = "
ORDER BY c.created_at DESC, c.id DESC
LIMIT 61
";

const PAGE_SEARCH: &str = "
AND c.id IN (SELECT rowid FROM capture_fts WHERE capture_fts MATCH ?4)
";

const INSERT_CAPTURE: &str = "
INSERT INTO capture (kind, path, bytes, width, height, duration_ms, source_app, created_at, ocr_text, id)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
";

const INSERT_FTS: &str = "
INSERT INTO capture_fts (rowid, name, source_app, ocr_text) VALUES (?1, ?2, ?3, ?4)
";

const DELETE_FTS: &str = "
INSERT INTO capture_fts (capture_fts, rowid, name, source_app, ocr_text)
VALUES ('delete', ?1, ?2, ?3, ?4)
";

const SELECT_UPLOAD: &str = "
SELECT id, capture_id, dest_id, state, url, deletion_url, error, attempts, updated_at, etag, sent, total
FROM upload
WHERE id = ?1
";

pub struct HistoryRepo {
    conn: Connection,
}

impl HistoryRepo {
    pub fn open_write(path: &Path) -> Result<Self, HistoryError> {
        Ok(Self {
            conn: super::migrate::open_write(path)?,
        })
    }

    pub fn open_read(path: &Path) -> Result<Self, HistoryError> {
        Ok(Self {
            conn: super::migrate::open_read(path)?,
        })
    }

    pub fn record(&mut self, capture: NewCapture) -> Result<CaptureId, HistoryError> {
        let transaction = self.conn.transaction()?;
        let id: CaptureId = transaction.query_row(
            "UPDATE capture_sequence SET last_id = last_id + 1 RETURNING last_id",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            INSERT_CAPTURE,
            params![
                capture.kind.as_str(),
                capture.path,
                capture.bytes,
                capture.width,
                capture.height,
                capture.duration_ms,
                capture.source_app,
                capture.created_at,
                capture.ocr_text,
                id,
            ],
        )?;
        transaction.execute(
            INSERT_FTS,
            params![
                id,
                capture_name(&capture.path),
                capture.source_app,
                capture.ocr_text,
            ],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn record_borrowed_files(
        &mut self,
        files: &[BorrowedFile],
    ) -> Result<Vec<CaptureId>, HistoryError> {
        let transaction = self.conn.transaction()?;
        let mut ids = Vec::with_capacity(files.len());
        for file in files {
            let id: CaptureId = transaction.query_row(
                "UPDATE capture_sequence SET last_id = last_id + 1 RETURNING last_id",
                [],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT INTO capture (kind, origin, owned, path, bytes, source_modified_ms, created_at, id) VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7)",
                params![file.kind.as_str(), file.origin.as_str(), file.path, file.bytes, file.source_modified_ms, file.created_at, id],
            )?;
            transaction.execute(
                INSERT_FTS,
                params![
                    id,
                    capture_name(&file.path),
                    Option::<String>::None,
                    Option::<String>::None
                ],
            )?;
            ids.push(id);
        }
        transaction.commit()?;
        Ok(ids)
    }

    pub fn page(
        &self,
        filter: HistoryFilter,
        cursor: Option<Cursor>,
    ) -> Result<HistoryPage, HistoryError> {
        self.query_page(filter, cursor, None)
    }

    pub fn get(&self, id: CaptureId) -> Result<Option<Capture>, HistoryError> {
        let sql = format!("{CAPTURE_SELECT} WHERE c.id = ?1");
        let mut statement = self.conn.prepare(&sql)?;
        let mut rows = statement.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(capture_from_row(row)?)),
            None => Ok(None),
        }
    }

    pub fn id_for_path(&self, path: &str) -> Result<Option<CaptureId>, HistoryError> {
        self.conn
            .query_row(
                "SELECT id FROM capture WHERE path = ?1 ORDER BY id DESC LIMIT 1",
                [path],
                |row| row.get(0),
            )
            .optional()
            .map_err(HistoryError::from)
    }

    pub fn stats(&self) -> Result<HistoryStats, HistoryError> {
        let (count, bytes): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM capture",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let now = OffsetDateTime::now_local().map_err(|_| HistoryError::LocalTimeUnavailable)?;
        let start = week_start_millis(now);
        let uploaded_this_week: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT capture_id) FROM upload WHERE state = 'done' AND updated_at >= ?1",
            [start],
            |row| row.get(0),
        )?;
        Ok(HistoryStats {
            count,
            bytes,
            uploaded_this_week,
        })
    }

    pub fn latest_upload_url(&self) -> Result<Option<String>, HistoryError> {
        self.conn
            .query_row(
                "SELECT url FROM upload WHERE state = 'done' AND url IS NOT NULL AND url <> '' ORDER BY updated_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(HistoryError::from)
    }

    pub fn set_ocr_text(&mut self, id: CaptureId, text: &str) -> Result<(), HistoryError> {
        let transaction = self.conn.transaction()?;
        let row: Option<(String, Option<String>, Option<String>)> = transaction
            .query_row(
                "SELECT path, source_app, ocr_text FROM capture WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((path, source_app, old_text)) = row else {
            return Ok(());
        };
        transaction.execute(
            "UPDATE capture SET ocr_text = ?1 WHERE id = ?2",
            params![text, id],
        )?;
        transaction.execute(
            DELETE_FTS,
            params![id, capture_name(&path), source_app, old_text],
        )?;
        transaction.execute(
            INSERT_FTS,
            params![id, capture_name(&path), source_app, text],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn delete(&mut self, id: CaptureId) -> Result<Option<DeletedCapture>, HistoryError> {
        let transaction = self.conn.transaction()?;
        let found: Option<(String, bool, String, i64, Option<String>, Option<String>)> = transaction
            .query_row(
                "SELECT kind, owned, path, created_at, source_app, ocr_text FROM capture WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, owned, path, created_at, source_app, ocr_text)) = found else {
            return Ok(None);
        };
        let kind = CaptureKind::parse(&kind)?;
        transaction.execute(
            DELETE_FTS,
            params![id, capture_name(&path), source_app, ocr_text],
        )?;
        transaction.execute("DELETE FROM capture WHERE id = ?1", [id])?;
        transaction.commit()?;
        Ok(Some(DeletedCapture {
            id,
            kind,
            owned,
            path,
            created_at,
        }))
    }

    pub fn search(
        &self,
        text: &str,
        filter: HistoryFilter,
        cursor: Option<Cursor>,
    ) -> Result<HistoryPage, HistoryError> {
        let query = fts_query(text);
        self.query_page(filter, cursor, query.as_deref())
    }

    pub fn mark_uploads_interrupted(&self) -> Result<(), HistoryError> {
        self.conn.execute(
            "UPDATE upload SET state = 'failed', error = 'The upload was interrupted when Snapline closed.' WHERE state IN ('queued', 'uploading')",
            [],
        )?;
        Ok(())
    }

    pub fn expire_uploads(&self, cutoff: i64) -> Result<(), HistoryError> {
        self.conn.execute(
            "DELETE FROM upload WHERE updated_at < ?1 AND state IN ('done', 'failed', 'cancelled')",
            [cutoff],
        )?;
        Ok(())
    }

    pub fn set_upload(&mut self, update: UploadUpdate) -> Result<UploadRecord, HistoryError> {
        let transaction = self.conn.transaction()?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM upload WHERE capture_id = ?1 AND dest_id = ?2 ORDER BY updated_at DESC, id DESC LIMIT 1",
                params![update.capture_id, update.dest_id],
                |row| row.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => {
                transaction.execute(
                    "UPDATE upload SET state = ?1, url = ?2, deletion_url = ?3, error = ?4, attempts = ?5, updated_at = ?6, sent = ?7, total = ?8 WHERE id = ?9",
                    params![
                        update.state.as_str(),
                        update.url,
                        update.deletion_url,
                        update.error,
                        update.attempts,
                        update.updated_at,
                        update.sent,
                        update.total,
                        id,
                    ],
                )?;
                id
            }
            None => {
                transaction.execute(
                    "INSERT INTO upload (capture_id, dest_id, state, url, deletion_url, error, attempts, updated_at, sent, total) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        update.capture_id,
                        update.dest_id,
                        update.state.as_str(),
                        update.url,
                        update.deletion_url,
                        update.error,
                        update.attempts,
                        update.updated_at,
                        update.sent,
                        update.total,
                    ],
                )?;
                transaction.last_insert_rowid()
            }
        };
        let raw = transaction.query_row(SELECT_UPLOAD, [id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
            ))
        })?;
        let state = UploadState::parse(&raw.3)?;
        transaction.commit()?;
        Ok(UploadRecord {
            id: raw.0,
            capture_id: raw.1,
            dest_id: raw.2,
            state,
            url: raw.4,
            deletion_url: raw.5,
            etag: raw.9,
            error: raw.6,
            attempts: raw.7,
            updated_at: raw.8,
            sent: raw.10,
            total: raw.11,
        })
    }

    fn query_page(
        &self,
        filter: HistoryFilter,
        cursor: Option<Cursor>,
        search: Option<&str>,
    ) -> Result<HistoryPage, HistoryError> {
        let sql = match search {
            Some(_) => format!("{CAPTURE_SELECT}{PAGE_WHERE}{PAGE_SEARCH}{PAGE_ORDER}"),
            None => format!("{CAPTURE_SELECT}{PAGE_WHERE}{PAGE_ORDER}"),
        };
        let mut statement = self.conn.prepare(&sql)?;
        let mut rows = match search {
            Some(query) => statement.query(params![
                filter.code(),
                cursor.map(|value| value.created_at),
                cursor.map(|value| value.id),
                query,
            ])?,
            None => statement.query(params![
                filter.code(),
                cursor.map(|value| value.created_at),
                cursor.map(|value| value.id),
            ])?,
        };
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(capture_from_row(row)?);
        }
        let next = if items.len() > PAGE_SIZE {
            items.truncate(PAGE_SIZE);
            items.last().map(|capture| Cursor {
                created_at: capture.created_at,
                id: capture.id,
            })
        } else {
            None
        };
        Ok(HistoryPage { items, next })
    }
}

pub fn week_start_millis(now: OffsetDateTime) -> i64 {
    let days = i64::from(now.weekday().number_days_from_monday());
    let monday = now.date() - Duration::days(days);
    let start = PrimitiveDateTime::new(monday, Time::MIDNIGHT).assume_offset(now.offset());
    start.unix_timestamp() * 1000
}

fn capture_from_row(row: &Row<'_>) -> Result<Capture, HistoryError> {
    let kind = CaptureKind::parse(&row.get::<_, String>(1)?)?;
    let origin = Origin::parse(&row.get::<_, String>(2)?)?;
    let upload_id: Option<i64> = row.get(13)?;
    let upload = match upload_id {
        Some(id) => {
            let state = UploadState::parse(&row.get::<_, String>(16)?)?;
            Some(UploadRecord {
                id,
                capture_id: row.get(14)?,
                dest_id: row.get(15)?,
                state,
                url: row.get(17)?,
                deletion_url: row.get(18)?,
                error: row.get(19)?,
                attempts: row.get(20)?,
                updated_at: row.get(21)?,
                etag: row.get(22)?,
                sent: row.get(23)?,
                total: row.get(24)?,
            })
        }
        None => None,
    };
    Ok(Capture {
        id: row.get(0)?,
        kind,
        origin,
        owned: row.get(3)?,
        path: row.get(4)?,
        bytes: row.get(5)?,
        source_modified_ms: row.get(6)?,
        width: row.get(7)?,
        height: row.get(8)?,
        duration_ms: row.get(9)?,
        source_app: row.get(10)?,
        created_at: row.get(11)?,
        ocr_text: row.get(12)?,
        upload,
    })
}

fn fts_query(text: &str) -> Option<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() {
            current.push(character);
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    if words.is_empty() {
        return None;
    }
    Some(
        words
            .iter()
            .map(|word| format!("\"{word}\"*"))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use time::{Date, Month};

    const BASE: i64 = 1_760_000_000_000;

    fn repo() -> (tempfile::TempDir, HistoryRepo) {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("history.db");
        let repo = HistoryRepo::open_write(&path).expect("open");
        (directory, repo)
    }

    fn capture(kind: CaptureKind, path: &str, created_at: i64) -> NewCapture {
        NewCapture {
            kind,
            path: path.to_owned(),
            bytes: 1024,
            width: Some(640),
            height: Some(480),
            duration_ms: None,
            source_app: Some("chrome.exe".to_owned()),
            created_at,
            ocr_text: None,
        }
    }

    fn upload(capture_id: CaptureId, state: UploadState, updated_at: i64) -> UploadUpdate {
        UploadUpdate {
            sent: 0,
            total: 0,
            capture_id,
            dest_id: "imgur".to_owned(),
            state,
            url: None,
            deletion_url: None,
            error: None,
            attempts: 1,
            updated_at,
        }
    }

    #[test]
    fn recording_returns_the_new_id() {
        let (_directory, mut repo) = repo();
        let id = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\one.png", BASE))
            .expect("record");
        assert_eq!(id, 1);
        let second = repo
            .record(capture(
                CaptureKind::Image,
                "C:\\Pictures\\two.png",
                BASE + 1,
            ))
            .expect("record");
        assert_eq!(second, 2);
    }

    #[test]
    fn deleting_the_latest_capture_does_not_reuse_its_id() {
        let (_directory, mut repo) = repo();
        let first = repo
            .record(capture(CaptureKind::Image, "old.png", 1))
            .expect("capture");
        repo.delete(first).expect("delete");
        let next = repo
            .record(capture(CaptureKind::Image, "new.png", 2))
            .expect("capture");
        assert!(next > first);
        assert!(repo
            .set_upload(upload(first, UploadState::Done, 3))
            .is_err());
        assert!(repo
            .get(next)
            .expect("get")
            .expect("capture")
            .upload
            .is_none());
    }

    #[test]
    fn a_page_holds_sixty_rows_in_descending_order() {
        let (_directory, mut repo) = repo();
        for index in 0..75 {
            repo.record(capture(
                CaptureKind::Image,
                &format!("C:\\Pictures\\{index}.png"),
                BASE + index,
            ))
            .expect("record");
        }
        let first = repo.page(HistoryFilter::All, None).expect("first page");
        assert_eq!(first.items.len(), 60);
        assert_eq!(first.items[0].created_at, BASE + 74);
        assert_eq!(first.items[59].created_at, BASE + 15);
        assert_eq!(
            first.next,
            Some(Cursor {
                created_at: BASE + 15,
                id: first.items[59].id,
            })
        );
        let second = repo
            .page(HistoryFilter::All, first.next)
            .expect("second page");
        assert_eq!(second.items.len(), 15);
        assert_eq!(second.items[0].created_at, BASE + 14);
        assert_eq!(second.items[14].created_at, BASE);
        assert_eq!(second.next, None);
    }

    #[test]
    fn cursor_pagination_with_equal_timestamps_has_no_duplicates() {
        let (_directory, mut repo) = repo();
        for index in 0..130 {
            repo.record(capture(
                CaptureKind::Image,
                &format!("C:\\Pictures\\{index}.png"),
                BASE,
            ))
            .expect("record");
        }
        let mut seen = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let page = repo.page(HistoryFilter::All, cursor).expect("page");
            assert!(page.items.len() <= PAGE_SIZE);
            seen.extend(page.items.iter().map(|capture| capture.id));
            pages += 1;
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(pages, 3);
        assert_eq!(seen.len(), 130);
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 130);
        assert_eq!(seen[0], 130);
        assert_eq!(seen[129], 1);
    }

    #[test]
    fn filters_match_kind_and_upload_state() {
        let (_directory, mut repo) = repo();
        let image = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
            .expect("record");
        let video = repo
            .record(capture(CaptureKind::Video, "C:\\Pictures\\b.mp4", BASE + 1))
            .expect("record");
        let gif = repo
            .record(capture(CaptureKind::Gif, "C:\\Pictures\\c.gif", BASE + 2))
            .expect("record");
        let queued = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\d.png", BASE + 3))
            .expect("record");
        repo.set_upload(upload(video, UploadState::Done, BASE + 4))
            .expect("done upload");
        repo.set_upload(upload(gif, UploadState::Failed, BASE + 5))
            .expect("failed upload");
        repo.set_upload(upload(queued, UploadState::Queued, BASE + 6))
            .expect("queued upload");
        assert_eq!(
            repo.page(HistoryFilter::All, None)
                .expect("all")
                .items
                .len(),
            4
        );
        assert_eq!(
            repo.page(HistoryFilter::Images, None)
                .expect("images")
                .items
                .len(),
            2
        );
        assert_eq!(
            repo.page(HistoryFilter::Videos, None)
                .expect("videos")
                .items
                .len(),
            1
        );
        assert_eq!(
            repo.page(HistoryFilter::Gifs, None)
                .expect("gifs")
                .items
                .len(),
            1
        );
        let uploaded = repo.page(HistoryFilter::Uploaded, None).expect("uploaded");
        assert_eq!(uploaded.items.len(), 1);
        assert_eq!(uploaded.items[0].id, video);
        let local = repo.page(HistoryFilter::LocalOnly, None).expect("local");
        assert_eq!(local.items.len(), 1);
        assert_eq!(local.items[0].id, image);
    }

    #[test]
    fn search_matches_prefixes_in_name_app_and_text() {
        let (_directory, mut repo) = repo();
        let mut first = capture(
            CaptureKind::Image,
            "C:\\Pictures\\Screenshot_2026-11-08_14-31-07.png",
            BASE,
        );
        first.ocr_text = Some("Error 0x80070005 access denied".to_owned());
        repo.record(first).expect("record");
        repo.record(capture(
            CaptureKind::Video,
            "C:\\Pictures\\Recording_2026-11-08.mp4",
            BASE + 1,
        ))
        .expect("record");
        assert_eq!(
            repo.search("screens", HistoryFilter::All, None)
                .expect("name")
                .items
                .len(),
            1
        );
        assert_eq!(
            repo.search("chro", HistoryFilter::All, None)
                .expect("app")
                .items
                .len(),
            2
        );
        assert_eq!(
            repo.search("erro", HistoryFilter::All, None)
                .expect("text")
                .items
                .len(),
            1
        );
        assert_eq!(
            repo.search("screens chro", HistoryFilter::All, None)
                .expect("both")
                .items
                .len(),
            1
        );
        assert_eq!(
            repo.search("screens zzz", HistoryFilter::All, None)
                .expect("none")
                .items
                .len(),
            0
        );
        let empty = repo.search("   ", HistoryFilter::All, None).expect("empty");
        assert_eq!(empty.items.len(), 2);
        assert_eq!(empty, repo.page(HistoryFilter::All, None).expect("page"));
    }

    #[test]
    fn get_returns_the_latest_upload() {
        let (_directory, mut repo) = repo();
        let id = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
            .expect("record");
        let mut older = upload(id, UploadState::Failed, BASE + 10);
        older.dest_id = "s3".to_owned();
        repo.set_upload(older).expect("older upload");
        let mut newer = upload(id, UploadState::Done, BASE + 20);
        newer.dest_id = "imgur".to_owned();
        newer.url = Some("https://example.com/a.png".to_owned());
        repo.set_upload(newer).expect("newer upload");
        let found = repo.get(id).expect("get").expect("capture");
        assert_eq!(found.name(), "a.png");
        let latest = found.upload.expect("upload");
        assert_eq!(latest.dest_id, "imgur");
        assert_eq!(latest.state, UploadState::Done);
        assert_eq!(latest.url.as_deref(), Some("https://example.com/a.png"));
        assert_eq!(repo.get(id + 1).expect("missing"), None);
    }

    #[test]
    fn set_upload_updates_one_row_per_capture_and_destination() {
        let (_directory, mut repo) = repo();
        let id = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
            .expect("record");
        repo.set_upload(upload(id, UploadState::Queued, BASE + 1))
            .expect("queued");
        let mut update = upload(id, UploadState::Done, BASE + 2);
        update.url = Some("https://example.com/a.png".to_owned());
        update.deletion_url = Some("https://example.com/delete/1".to_owned());
        update.attempts = 2;
        let record = repo.set_upload(update).expect("done");
        assert_eq!(record.state, UploadState::Done);
        assert_eq!(record.attempts, 2);
        assert_eq!(
            record.deletion_url.as_deref(),
            Some("https://example.com/delete/1")
        );
        let count: i64 = repo
            .conn
            .query_row(
                "SELECT COUNT(*) FROM upload WHERE capture_id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("count uploads");
        assert_eq!(count, 1);
    }

    fn upload_states(repo: &HistoryRepo) -> Vec<(String, String, Option<String>)> {
        let mut statement = repo
            .conn
            .prepare("SELECT dest_id, state, error FROM upload ORDER BY dest_id")
            .expect("prepare");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query");
        rows.map(|row| row.expect("row")).collect()
    }

    #[test]
    fn expiring_removes_only_finished_uploads_older_than_the_cutoff() {
        let (_directory, mut repo) = repo();
        let id = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
            .expect("record");
        let cutoff = BASE + 100;
        let rows = [
            ("old-done", UploadState::Done, cutoff - 1),
            ("old-failed", UploadState::Failed, cutoff - 1),
            ("old-cancelled", UploadState::Cancelled, cutoff - 1),
            ("old-queued", UploadState::Queued, cutoff - 1),
            ("old-uploading", UploadState::Uploading, cutoff - 1),
            ("edge-done", UploadState::Done, cutoff),
            ("new-done", UploadState::Done, cutoff + 1),
        ];
        for (dest, state, updated_at) in rows {
            let mut update = upload(id, state, updated_at);
            update.dest_id = dest.to_owned();
            repo.set_upload(update).expect("upload");
        }
        repo.expire_uploads(cutoff).expect("expire");
        let kept: Vec<String> = upload_states(&repo)
            .into_iter()
            .map(|(dest, _, _)| dest)
            .collect();
        assert_eq!(
            kept,
            vec!["edge-done", "new-done", "old-queued", "old-uploading"]
        );
        assert!(repo.get(id).expect("get").is_some());
    }

    #[test]
    fn interrupted_uploads_become_failed_and_finished_ones_are_untouched() {
        let (_directory, mut repo) = repo();
        let id = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
            .expect("record");
        for (dest, state) in [
            ("a-queued", UploadState::Queued),
            ("b-uploading", UploadState::Uploading),
            ("c-done", UploadState::Done),
            ("d-cancelled", UploadState::Cancelled),
        ] {
            let mut update = upload(id, state, BASE + 1);
            update.dest_id = dest.to_owned();
            repo.set_upload(update).expect("upload");
        }
        repo.mark_uploads_interrupted().expect("mark");
        let interrupted = Some("The upload was interrupted when Snapline closed.".to_owned());
        assert_eq!(
            upload_states(&repo),
            vec![
                (
                    "a-queued".to_owned(),
                    "failed".to_owned(),
                    interrupted.clone()
                ),
                ("b-uploading".to_owned(), "failed".to_owned(), interrupted),
                ("c-done".to_owned(), "done".to_owned(), None),
                ("d-cancelled".to_owned(), "cancelled".to_owned(), None),
            ]
        );
    }

    #[test]
    fn an_upload_reads_its_saved_etag() {
        let (_directory, mut repo) = repo();
        let id = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
            .expect("record");
        repo.set_upload(upload(id, UploadState::Done, BASE + 1))
            .expect("upload");
        repo.conn
            .execute(
                "UPDATE upload SET etag = ?1 WHERE capture_id = ?2",
                params!["\"9b2cf535f27731c974343645a3985328\"", id],
            )
            .expect("etag");
        let found = repo.get(id).expect("get").expect("capture");
        assert_eq!(
            found.upload.expect("upload").etag.as_deref(),
            Some("\"9b2cf535f27731c974343645a3985328\"")
        );
    }

    #[test]
    fn stats_count_captures_bytes_and_this_weeks_uploads() {
        let (_directory, mut repo) = repo();
        let start = week_start_millis(OffsetDateTime::now_local().expect("local time"));
        let first = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
            .expect("record");
        let second = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\b.png", BASE + 1))
            .expect("record");
        let third = repo
            .record(capture(CaptureKind::Gif, "C:\\Pictures\\c.gif", BASE + 2))
            .expect("record");
        repo.set_upload(upload(first, UploadState::Done, start - 1))
            .expect("last week");
        repo.set_upload(upload(second, UploadState::Done, start))
            .expect("monday");
        repo.set_upload(upload(third, UploadState::Done, start + 1))
            .expect("this week");
        let stats = repo.stats().expect("stats");
        assert_eq!(stats.count, 3);
        assert_eq!(stats.bytes, 3072);
        assert_eq!(stats.uploaded_this_week, 2);
    }

    #[test]
    fn the_week_starts_on_monday_at_local_midnight() {
        let offset = time::UtcOffset::from_hms(2, 0, 0).expect("offset");
        let wednesday = PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::November, 11).expect("date"),
            Time::from_hms(15, 30, 0).expect("time"),
        )
        .assume_offset(offset);
        let sunday = PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::November, 15).expect("date"),
            Time::from_hms(23, 59, 59).expect("time"),
        )
        .assume_offset(offset);
        let monday = PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::November, 9).expect("date"),
            Time::MIDNIGHT,
        )
        .assume_offset(offset);
        let expected = monday.unix_timestamp() * 1000;
        assert_eq!(week_start_millis(wednesday), expected);
        assert_eq!(week_start_millis(sunday), expected);
        assert_eq!(week_start_millis(monday), expected);
    }

    #[test]
    fn delete_removes_the_capture_uploads_and_search_row() {
        let (_directory, mut repo) = repo();
        let mut recorded = capture(
            CaptureKind::Image,
            "C:\\Pictures\\Screenshot_delete.png",
            BASE,
        );
        recorded.ocr_text = Some("hello world".to_owned());
        let id = repo.record(recorded).expect("record");
        let mut second = upload(id, UploadState::Failed, BASE + 1);
        second.dest_id = "s3".to_owned();
        repo.set_upload(upload(id, UploadState::Done, BASE + 2))
            .expect("upload");
        repo.set_upload(second).expect("second upload");
        assert_eq!(
            repo.search("hello", HistoryFilter::All, None)
                .expect("search")
                .items
                .len(),
            1
        );
        let deleted = repo.delete(id).expect("delete").expect("capture");
        assert_eq!(deleted.id, id);
        assert_eq!(deleted.path, "C:\\Pictures\\Screenshot_delete.png");
        assert_eq!(deleted.kind, CaptureKind::Image);
        assert_eq!(deleted.created_at, BASE);
        assert_eq!(repo.get(id).expect("get"), None);
        let uploads: i64 = repo
            .conn
            .query_row(
                "SELECT COUNT(*) FROM upload WHERE capture_id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("count uploads");
        assert_eq!(uploads, 0);
        assert_eq!(
            repo.search("hello", HistoryFilter::All, None)
                .expect("search")
                .items
                .len(),
            0
        );
        assert_eq!(repo.delete(id).expect("delete missing"), None);
    }

    #[test]
    fn the_latest_upload_url_follows_the_newest_finished_upload() {
        let (_directory, mut repo) = repo();
        assert_eq!(repo.latest_upload_url().expect("none"), None);
        let older = repo
            .record(capture(CaptureKind::Image, "older.png", BASE))
            .expect("record");
        let newer = repo
            .record(capture(CaptureKind::Image, "newer.png", BASE + 1))
            .expect("record");
        let mut first = upload(older, UploadState::Done, BASE + 2);
        first.url = Some("https://example.test/older".to_owned());
        repo.set_upload(first).expect("older upload");
        let mut second = upload(newer, UploadState::Done, BASE + 3);
        second.url = Some("https://example.test/newer".to_owned());
        repo.set_upload(second).expect("newer upload");
        let mut failed = upload(newer, UploadState::Failed, BASE + 4);
        failed.dest_id = "s3".to_owned();
        failed.url = Some("https://example.test/failed".to_owned());
        repo.set_upload(failed).expect("failed upload");
        assert_eq!(
            repo.latest_upload_url().expect("latest"),
            Some("https://example.test/newer".to_owned())
        );
        repo.delete(newer).expect("delete");
        assert_eq!(
            repo.latest_upload_url().expect("fallback"),
            Some("https://example.test/older".to_owned())
        );
        repo.delete(older).expect("delete");
        assert_eq!(repo.latest_upload_url().expect("cleared"), None);
    }

    #[test]
    fn deleting_frees_the_fts_terms_for_another_row() {
        let (_directory, mut repo) = repo();
        let first = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\one.png", BASE))
            .expect("record");
        repo.delete(first).expect("delete");
        repo.record(capture(
            CaptureKind::Image,
            "C:\\Pictures\\two.png",
            BASE + 1,
        ))
        .expect("record");
        assert_eq!(
            repo.search("two", HistoryFilter::All, None)
                .expect("search")
                .items
                .len(),
            1
        );
        assert_eq!(
            repo.search("one", HistoryFilter::All, None)
                .expect("search")
                .items
                .len(),
            0
        );
    }

    #[test]
    fn reopening_a_database_preserves_rows() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("history.db");
        {
            let mut repo = HistoryRepo::open_write(&path).expect("open");
            repo.record(capture(CaptureKind::Image, "C:\\Pictures\\kept.png", BASE))
                .expect("record");
            assert_eq!(repo.stats().expect("stats").count, 1);
        }
        {
            let repo = HistoryRepo::open_read(&path).expect("read only");
            let page = repo.page(HistoryFilter::All, None).expect("page");
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.items[0].name(), "kept.png");
        }
        let repo = HistoryRepo::open_write(&path).expect("reopen");
        assert_eq!(repo.stats().expect("stats").count, 1);
    }

    #[test]
    fn a_read_only_repository_refuses_writes() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("history.db");
        {
            let mut repo = HistoryRepo::open_write(&path).expect("open");
            repo.record(capture(CaptureKind::Image, "C:\\Pictures\\a.png", BASE))
                .expect("record");
        }
        let mut repo = HistoryRepo::open_read(&path).expect("read only");
        assert!(repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\b.png", BASE + 1))
            .is_err());
    }
}
