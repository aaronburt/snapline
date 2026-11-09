use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row};
use time::{Duration, OffsetDateTime, PrimitiveDateTime, Time};

use super::{
    capture_name, Capture, CaptureId, CaptureKind, Cursor, DeletedCapture, HistoryError,
    HistoryFilter, HistoryPage, HistoryStats, NewCapture, UploadRecord, UploadState, UploadUpdate,
};

const PAGE_SIZE: usize = 60;

const CAPTURE_SELECT: &str = "
SELECT c.id, c.kind, c.path, c.bytes, c.width, c.height, c.duration_ms, c.source_app,
       c.created_at, c.ocr_text,
       u.id, u.capture_id, u.dest_id, u.state, u.url, u.deletion_url, u.error, u.attempts,
       u.updated_at
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
INSERT INTO capture (kind, path, bytes, width, height, duration_ms, source_app, created_at, ocr_text)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
";

const INSERT_FTS: &str = "
INSERT INTO capture_fts (rowid, name, source_app, ocr_text) VALUES (?1, ?2, ?3, ?4)
";

const DELETE_FTS: &str = "
INSERT INTO capture_fts (capture_fts, rowid, name, source_app, ocr_text)
VALUES ('delete', ?1, ?2, ?3, ?4)
";

const SELECT_UPLOAD: &str = "
SELECT id, capture_id, dest_id, state, url, deletion_url, error, attempts, updated_at
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
            ],
        )?;
        let id = transaction.last_insert_rowid();
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
        let found: Option<(String, String, i64, Option<String>, Option<String>)> = transaction
            .query_row(
                "SELECT kind, path, created_at, source_app, ocr_text FROM capture WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, path, created_at, source_app, ocr_text)) = found else {
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
                    "UPDATE upload SET state = ?1, url = ?2, deletion_url = ?3, error = ?4, attempts = ?5, updated_at = ?6 WHERE id = ?7",
                    params![
                        update.state.as_str(),
                        update.url,
                        update.deletion_url,
                        update.error,
                        update.attempts,
                        update.updated_at,
                        id,
                    ],
                )?;
                id
            }
            None => {
                transaction.execute(
                    "INSERT INTO upload (capture_id, dest_id, state, url, deletion_url, error, attempts, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        update.capture_id,
                        update.dest_id,
                        update.state.as_str(),
                        update.url,
                        update.deletion_url,
                        update.error,
                        update.attempts,
                        update.updated_at,
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
            error: raw.6,
            attempts: raw.7,
            updated_at: raw.8,
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
    let upload_id: Option<i64> = row.get(10)?;
    let upload = match upload_id {
        Some(id) => {
            let state = UploadState::parse(&row.get::<_, String>(13)?)?;
            Some(UploadRecord {
                id,
                capture_id: row.get(11)?,
                dest_id: row.get(12)?,
                state,
                url: row.get(14)?,
                deletion_url: row.get(15)?,
                error: row.get(16)?,
                attempts: row.get(17)?,
                updated_at: row.get(18)?,
            })
        }
        None => None,
    };
    Ok(Capture {
        id: row.get(0)?,
        kind,
        path: row.get(2)?,
        bytes: row.get(3)?,
        width: row.get(4)?,
        height: row.get(5)?,
        duration_ms: row.get(6)?,
        source_app: row.get(7)?,
        created_at: row.get(8)?,
        ocr_text: row.get(9)?,
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
    use time::{Date, Month};
    use tempfile::tempdir;

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
            .record(capture(CaptureKind::Image, "C:\\Pictures\\two.png", BASE + 1))
            .expect("record");
        assert_eq!(second, 2);
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
            repo.page(HistoryFilter::All, None).expect("all").items.len(),
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
            repo.page(HistoryFilter::Gifs, None).expect("gifs").items.len(),
            1
        );
        let uploaded = repo
            .page(HistoryFilter::Uploaded, None)
            .expect("uploaded");
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
            .query_row("SELECT COUNT(*) FROM upload WHERE capture_id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("count uploads");
        assert_eq!(count, 1);
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
            .query_row("SELECT COUNT(*) FROM upload WHERE capture_id = ?1", [id], |row| {
                row.get(0)
            })
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
    fn deleting_frees_the_fts_terms_for_another_row() {
        let (_directory, mut repo) = repo();
        let first = repo
            .record(capture(CaptureKind::Image, "C:\\Pictures\\one.png", BASE))
            .expect("record");
        repo.delete(first).expect("delete");
        repo.record(capture(CaptureKind::Image, "C:\\Pictures\\two.png", BASE + 1))
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
            repo.record(capture(
                CaptureKind::Image,
                "C:\\Pictures\\kept.png",
                BASE,
            ))
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
