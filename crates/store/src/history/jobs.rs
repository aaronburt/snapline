use rusqlite::{params, OptionalExtension, Row};

use super::{CaptureId, HistoryError, HistoryRepo};

pub type JobId = i64;

const ERROR_LIMIT: usize = 300;

const SELECT_JOB: &str = "
SELECT id, capture_id, revision, operation_id, dest_id, config_fingerprint, run_id,
       batch_id, batch_index, batch_size, file_name, content_type, file_path, file_len,
       file_modified_ms, staged, state, attempts, next_retry_at, owner, url, page_url,
       deletion_url, error, created_at, updated_at, finished_at
FROM upload_job
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    WaitingForHistory,
    Queued,
    Uploading,
    WaitingToRetry,
    Paused,
    NeedsAttention,
    OutcomeUnknown,
    Complete,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WaitingForHistory => "waiting_history",
            Self::Queued => "queued",
            Self::Uploading => "uploading",
            Self::WaitingToRetry => "waiting_retry",
            Self::Paused => "paused",
            Self::NeedsAttention => "needs_attention",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Result<Self, HistoryError> {
        match value {
            "waiting_history" => Ok(Self::WaitingForHistory),
            "queued" => Ok(Self::Queued),
            "uploading" => Ok(Self::Uploading),
            "waiting_retry" => Ok(Self::WaitingToRetry),
            "paused" => Ok(Self::Paused),
            "needs_attention" => Ok(Self::NeedsAttention),
            "outcome_unknown" => Ok(Self::OutcomeUnknown),
            "complete" => Ok(Self::Complete),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(HistoryError::UnknownValue {
                field: "upload_job.state",
                value: value.to_owned(),
            }),
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Cancelled)
    }

    pub fn active(self) -> bool {
        matches!(
            self,
            Self::WaitingForHistory
                | Self::Queued
                | Self::Uploading
                | Self::WaitingToRetry
                | Self::Paused
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobFile {
    pub name: String,
    pub content_type: String,
    pub path: String,
    pub len: i64,
    pub modified_ms: i64,
    pub staged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobBatch {
    pub id: String,
    pub index: u32,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub capture_id: Option<CaptureId>,
    pub revision: Option<i64>,
    pub operation_id: Option<String>,
    pub dest_id: String,
    pub config_fingerprint: String,
    pub run_id: Option<String>,
    pub batch: Option<JobBatch>,
    pub file: JobFile,
    pub state: JobState,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: JobId,
    pub capture_id: Option<CaptureId>,
    pub revision: Option<i64>,
    pub operation_id: Option<String>,
    pub dest_id: String,
    pub config_fingerprint: String,
    pub run_id: Option<String>,
    pub batch: Option<JobBatch>,
    pub file: JobFile,
    pub state: JobState,
    pub attempts: i64,
    pub next_retry_at: Option<i64>,
    pub owner: Option<String>,
    pub url: Option<String>,
    pub page_url: Option<String>,
    pub deletion_url: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResult {
    pub url: String,
    pub page_url: Option<String>,
    pub deletion_url: Option<String>,
}

fn clip(error: &str) -> String {
    error.chars().take(ERROR_LIMIT).collect()
}

fn read_job(row: &Row<'_>) -> rusqlite::Result<Result<Job, HistoryError>> {
    let state: String = row.get(16)?;
    let batch = match (
        row.get::<_, Option<String>>(7)?,
        row.get::<_, Option<u32>>(8)?,
        row.get::<_, Option<u32>>(9)?,
    ) {
        (Some(id), Some(index), Some(size)) => Some(JobBatch { id, index, size }),
        _ => None,
    };
    let job = Job {
        id: row.get(0)?,
        capture_id: row.get(1)?,
        revision: row.get(2)?,
        operation_id: row.get(3)?,
        dest_id: row.get(4)?,
        config_fingerprint: row.get(5)?,
        run_id: row.get(6)?,
        batch,
        file: JobFile {
            name: row.get(10)?,
            content_type: row.get(11)?,
            path: row.get(12)?,
            len: row.get(13)?,
            modified_ms: row.get(14)?,
            staged: row.get::<_, i64>(15)? != 0,
        },
        attempts: row.get(17)?,
        next_retry_at: row.get(18)?,
        owner: row.get(19)?,
        url: row.get(20)?,
        page_url: row.get(21)?,
        deletion_url: row.get(22)?,
        error: row.get(23)?,
        created_at: row.get(24)?,
        updated_at: row.get(25)?,
        finished_at: row.get(26)?,
        state: JobState::Queued,
    };
    Ok(JobState::parse(&state).map(|state| Job { state, ..job }))
}

fn collect(
    statement: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> Result<Vec<Job>, HistoryError> {
    let rows = statement.query_map(params, read_job)?;
    let mut jobs = Vec::new();
    for row in rows {
        jobs.push(row??);
    }
    Ok(jobs)
}

impl HistoryRepo {
    pub fn create_job(&mut self, job: NewJob) -> Result<Job, HistoryError> {
        let transaction = self.conn.transaction()?;
        transaction.execute(
            "INSERT INTO upload_job (capture_id, revision, operation_id, dest_id, config_fingerprint, run_id, batch_id, batch_index, batch_size, file_name, content_type, file_path, file_len, file_modified_ms, staged, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
            params![
                job.capture_id,
                job.revision,
                job.operation_id,
                job.dest_id,
                job.config_fingerprint,
                job.run_id,
                job.batch.as_ref().map(|batch| batch.id.clone()),
                job.batch.as_ref().map(|batch| batch.index),
                job.batch.as_ref().map(|batch| batch.size),
                job.file.name,
                job.file.content_type,
                job.file.path,
                job.file.len,
                job.file.modified_ms,
                i64::from(job.file.staged),
                job.state.as_str(),
                job.created_at,
            ],
        )?;
        let id = transaction.last_insert_rowid();
        let created =
            transaction.query_row(&format!("{SELECT_JOB} WHERE id = ?1"), [id], read_job)??;
        transaction.commit()?;
        Ok(created)
    }

    pub fn job(&self, id: JobId) -> Result<Option<Job>, HistoryError> {
        self.conn
            .query_row(&format!("{SELECT_JOB} WHERE id = ?1"), [id], read_job)
            .optional()?
            .transpose()
    }

    pub fn jobs_in(&self, states: &[JobState]) -> Result<Vec<Job>, HistoryError> {
        let mut statement = self
            .conn
            .prepare(&format!("{SELECT_JOB} ORDER BY id ASC"))?;
        let mut jobs = collect(&mut statement, [])?;
        jobs.retain(|job| states.contains(&job.state));
        Ok(jobs)
    }

    pub fn jobs_for_capture(&self, capture_id: CaptureId) -> Result<Vec<Job>, HistoryError> {
        let mut statement = self.conn.prepare(&format!(
            "{SELECT_JOB} WHERE capture_id = ?1 ORDER BY id DESC"
        ))?;
        collect(&mut statement, [capture_id])
    }

    pub fn claim_job(
        &mut self,
        id: JobId,
        owner: &str,
        now: i64,
    ) -> Result<Option<Job>, HistoryError> {
        let transaction = self.conn.transaction()?;
        let changed = transaction.execute(
            "UPDATE upload_job SET state = 'uploading', owner = ?2, attempts = attempts + 1, next_retry_at = NULL, error = NULL, updated_at = ?3 WHERE id = ?1 AND state IN ('queued', 'waiting_retry')",
            params![id, owner, now],
        )?;
        let claimed = if changed == 1 {
            transaction
                .query_row(&format!("{SELECT_JOB} WHERE id = ?1"), [id], read_job)
                .optional()?
                .transpose()?
        } else {
            None
        };
        transaction.commit()?;
        Ok(claimed)
    }

    pub fn complete_job(
        &mut self,
        id: JobId,
        owner: &str,
        result: &JobResult,
        now: i64,
    ) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = 'complete', url = ?3, page_url = ?4, deletion_url = ?5, error = NULL, next_retry_at = NULL, owner = NULL, updated_at = ?6, finished_at = ?6 WHERE id = ?1 AND state = 'uploading' AND owner = ?2",
            params![id, owner, result.url, result.page_url, result.deletion_url, now],
        )?;
        Ok(changed == 1)
    }

    pub fn schedule_retry(
        &mut self,
        id: JobId,
        owner: &str,
        next_retry_at: i64,
        error: &str,
        now: i64,
    ) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = 'waiting_retry', next_retry_at = ?3, error = ?4, owner = NULL, updated_at = ?5 WHERE id = ?1 AND state = 'uploading' AND owner = ?2",
            params![id, owner, next_retry_at, clip(error), now],
        )?;
        Ok(changed == 1)
    }

    pub fn stop_job(
        &mut self,
        id: JobId,
        state: JobState,
        error: &str,
        now: i64,
    ) -> Result<bool, HistoryError> {
        if !matches!(state, JobState::NeedsAttention | JobState::OutcomeUnknown) {
            return Err(HistoryError::UnknownValue {
                field: "upload_job.stop_state",
                value: state.as_str().to_owned(),
            });
        }
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = ?2, error = ?3, next_retry_at = NULL, owner = NULL, updated_at = ?4 WHERE id = ?1 AND state IN ('waiting_history', 'queued', 'uploading', 'waiting_retry', 'paused')",
            params![id, state.as_str(), clip(error), now],
        )?;
        Ok(changed == 1)
    }

    pub fn pause_job(&mut self, id: JobId, now: i64) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = 'paused', updated_at = ?2 WHERE id = ?1 AND state IN ('queued', 'waiting_retry')",
            params![id, now],
        )?;
        Ok(changed == 1)
    }

    pub fn resume_job(&mut self, id: JobId, now: i64) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = CASE WHEN next_retry_at IS NOT NULL AND next_retry_at > ?2 THEN 'waiting_retry' ELSE 'queued' END, updated_at = ?2 WHERE id = ?1 AND state = 'paused'",
            params![id, now],
        )?;
        Ok(changed == 1)
    }

    pub fn cancel_job(&mut self, id: JobId, now: i64) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = 'cancelled', next_retry_at = NULL, owner = NULL, updated_at = ?2, finished_at = ?2 WHERE id = ?1 AND state NOT IN ('complete', 'cancelled')",
            params![id, now],
        )?;
        Ok(changed == 1)
    }

    pub fn retry_job(&mut self, id: JobId, now: i64) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = 'queued', attempts = 0, next_retry_at = NULL, owner = NULL, error = NULL, finished_at = NULL, updated_at = ?2 WHERE id = ?1 AND state IN ('needs_attention', 'outcome_unknown', 'cancelled')",
            params![id, now],
        )?;
        Ok(changed == 1)
    }

    pub fn requeue_interrupted(&mut self, id: JobId, now: i64) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET state = 'queued', owner = NULL, updated_at = ?2 WHERE id = ?1 AND state = 'uploading'",
            params![id, now],
        )?;
        Ok(changed == 1)
    }

    pub fn interrupted_jobs(&self, current_owner: &str) -> Result<Vec<Job>, HistoryError> {
        let mut statement = self.conn.prepare(&format!(
            "{SELECT_JOB} WHERE state = 'uploading' AND (owner IS NULL OR owner <> ?1) ORDER BY id ASC"
        ))?;
        collect(&mut statement, [current_owner])
    }

    pub fn attach_operation(
        &mut self,
        operation_id: &str,
        capture_id: CaptureId,
        revision: i64,
        file: &JobFile,
        now: i64,
    ) -> Result<usize, HistoryError> {
        Ok(self.conn.execute(
            "UPDATE upload_job SET capture_id = ?2, revision = ?3, state = 'queued', file_path = ?4, file_len = ?5, file_modified_ms = ?6, updated_at = ?7 WHERE operation_id = ?1 AND state = 'waiting_history'",
            params![
                operation_id,
                capture_id,
                revision,
                file.path,
                file.len,
                file.modified_ms,
                now
            ],
        )?)
    }

    pub fn queue_paused(&self) -> Result<bool, HistoryError> {
        Ok(self.conn.query_row(
            "SELECT paused FROM upload_queue_state WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    pub fn set_queue_paused(&mut self, paused: bool) -> Result<(), HistoryError> {
        self.conn.execute(
            "UPDATE upload_queue_state SET paused = ?1 WHERE id = 1",
            [i64::from(paused)],
        )?;
        Ok(())
    }

    pub fn rebind_job(
        &mut self,
        id: JobId,
        fingerprint: &str,
        now: i64,
    ) -> Result<bool, HistoryError> {
        let changed = self.conn.execute(
            "UPDATE upload_job SET config_fingerprint = ?2, updated_at = ?3 WHERE id = ?1 AND state NOT IN ('complete', 'uploading')",
            params![id, fingerprint, now],
        )?;
        Ok(changed == 1)
    }

    pub fn prune_jobs(&mut self, cutoff: i64) -> Result<Vec<Job>, HistoryError> {
        let transaction = self.conn.transaction()?;
        let pruned = {
            let mut statement = transaction.prepare(&format!(
                "{SELECT_JOB} WHERE state IN ('complete', 'cancelled') AND finished_at IS NOT NULL AND finished_at < ?1 ORDER BY id ASC"
            ))?;
            collect(&mut statement, [cutoff])?
        };
        transaction.execute(
            "DELETE FROM upload_job WHERE state IN ('complete', 'cancelled') AND finished_at IS NOT NULL AND finished_at < ?1",
            [cutoff],
        )?;
        transaction.commit()?;
        Ok(pruned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{CaptureKind, NewCapture};
    use tempfile::tempdir;

    const NOW: i64 = 1_760_000_000_000;
    const OWNER: &str = "run-1";

    fn repo() -> (tempfile::TempDir, HistoryRepo) {
        let directory = tempdir().expect("temp directory");
        let repo = HistoryRepo::open_write(&directory.path().join("history.db")).expect("open");
        (directory, repo)
    }

    fn capture(repo: &mut HistoryRepo) -> CaptureId {
        repo.record(NewCapture {
            kind: CaptureKind::Image,
            path: "C:\\captures\\a.png".to_owned(),
            bytes: 10,
            width: None,
            height: None,
            duration_ms: None,
            source_app: None,
            created_at: NOW,
            ocr_text: None,
        })
        .expect("capture")
    }

    fn new_job(capture_id: Option<CaptureId>, state: JobState) -> NewJob {
        NewJob {
            capture_id,
            revision: capture_id.map(|_| 1),
            operation_id: None,
            dest_id: "dest-1".to_owned(),
            config_fingerprint: "fp-1".to_owned(),
            run_id: None,
            batch: None,
            file: JobFile {
                name: "a.png".to_owned(),
                content_type: "image/png".to_owned(),
                path: "C:\\captures\\a.png".to_owned(),
                len: 10,
                modified_ms: NOW,
                staged: false,
            },
            state,
            created_at: NOW,
        }
    }

    fn result() -> JobResult {
        JobResult {
            url: "https://example.test/a.png".to_owned(),
            page_url: Some("https://example.test/a".to_owned()),
            deletion_url: Some("https://example.test/del".to_owned()),
        }
    }

    #[test]
    fn a_created_job_round_trips_and_survives_reopening() {
        let (directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let mut input = new_job(Some(capture_id), JobState::Queued);
        input.run_id = Some("run".to_owned());
        input.batch = Some(JobBatch {
            id: "b-1".to_owned(),
            index: 1,
            size: 3,
        });
        let created = repo.create_job(input).expect("create");
        assert_eq!(created.state, JobState::Queued);
        assert_eq!(created.attempts, 0);
        assert_eq!(created.capture_id, Some(capture_id));
        assert_eq!(created.revision, Some(1));
        assert_eq!(created.batch.as_ref().map(|batch| batch.size), Some(3));
        drop(repo);
        let reopened =
            HistoryRepo::open_write(&directory.path().join("history.db")).expect("reopen");
        assert_eq!(reopened.job(created.id).expect("read"), Some(created));
    }

    #[test]
    fn only_one_claim_wins_and_attempts_count() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        let first = repo.claim_job(job.id, OWNER, NOW + 1).expect("claim");
        assert_eq!(first.as_ref().map(|job| job.attempts), Some(1));
        assert_eq!(first.map(|job| job.state), Some(JobState::Uploading));
        assert_eq!(
            repo.claim_job(job.id, "run-2", NOW + 2).expect("claim"),
            None
        );
    }

    #[test]
    fn a_job_waiting_for_history_cannot_be_claimed() {
        let (_directory, mut repo) = repo();
        let mut input = new_job(None, JobState::WaitingForHistory);
        input.operation_id = Some("op-1".to_owned());
        let job = repo.create_job(input).expect("create");
        assert_eq!(repo.claim_job(job.id, OWNER, NOW).expect("claim"), None);
        let capture_id = capture(&mut repo);
        let mut moved = job.file.clone();
        moved.path = "moved.png".to_owned();
        moved.len = 99;
        assert_eq!(
            repo.attach_operation("op-1", capture_id, 2, &moved, NOW + 1)
                .expect("attach"),
            1
        );
        let attached = repo.job(job.id).expect("read").expect("job");
        assert_eq!(attached.file.path, moved.path);
        assert_eq!(attached.file.len, 99);
        assert_eq!(attached.state, JobState::Queued);
        assert_eq!(attached.capture_id, Some(capture_id));
        assert_eq!(attached.revision, Some(2));
        assert!(repo
            .claim_job(job.id, OWNER, NOW + 2)
            .expect("claim")
            .is_some());
    }

    #[test]
    fn completion_requires_the_claiming_owner_and_is_final() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        repo.claim_job(job.id, OWNER, NOW).expect("claim");
        assert!(!repo
            .complete_job(job.id, "someone-else", &result(), NOW + 1)
            .expect("complete"));
        assert!(repo
            .complete_job(job.id, OWNER, &result(), NOW + 2)
            .expect("complete"));
        let done = repo.job(job.id).expect("read").expect("job");
        assert_eq!(done.state, JobState::Complete);
        assert_eq!(done.url.as_deref(), Some("https://example.test/a.png"));
        assert_eq!(done.finished_at, Some(NOW + 2));
        assert!(!repo.cancel_job(job.id, NOW + 3).expect("cancel"));
        assert!(!repo.retry_job(job.id, NOW + 3).expect("retry"));
        assert!(!repo.pause_job(job.id, NOW + 3).expect("pause"));
        assert!(!repo
            .stop_job(job.id, JobState::NeedsAttention, "x", NOW + 3)
            .expect("stop"));
        assert!(!repo.requeue_interrupted(job.id, NOW + 3).expect("requeue"));
        assert_eq!(
            repo.job(job.id).expect("read").expect("job").state,
            JobState::Complete
        );
    }

    #[test]
    fn a_retry_keeps_its_deadline_and_attempt_count() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        repo.claim_job(job.id, OWNER, NOW).expect("claim");
        assert!(repo
            .schedule_retry(
                job.id,
                OWNER,
                NOW + 8_000,
                "The connection failed.",
                NOW + 1
            )
            .expect("schedule"));
        let waiting = repo.job(job.id).expect("read").expect("job");
        assert_eq!(waiting.state, JobState::WaitingToRetry);
        assert_eq!(waiting.next_retry_at, Some(NOW + 8_000));
        assert_eq!(waiting.attempts, 1);
        assert_eq!(waiting.owner, None);
        let again = repo
            .claim_job(job.id, OWNER, NOW + 9_000)
            .expect("claim")
            .expect("claimed");
        assert_eq!(again.attempts, 2);
        assert_eq!(again.next_retry_at, None);
    }

    #[test]
    fn pause_and_resume_respect_the_retry_deadline() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        assert!(repo.pause_job(job.id, NOW).expect("pause"));
        assert_eq!(repo.claim_job(job.id, OWNER, NOW).expect("claim"), None);
        assert!(repo.resume_job(job.id, NOW + 1).expect("resume"));
        assert_eq!(
            repo.job(job.id).expect("read").expect("job").state,
            JobState::Queued
        );
        repo.claim_job(job.id, OWNER, NOW + 2).expect("claim");
        repo.schedule_retry(job.id, OWNER, NOW + 60_000, "e", NOW + 3)
            .expect("schedule");
        assert!(repo.pause_job(job.id, NOW + 4).expect("pause"));
        assert!(repo.resume_job(job.id, NOW + 5).expect("resume"));
        assert_eq!(
            repo.job(job.id).expect("read").expect("job").state,
            JobState::WaitingToRetry
        );
        assert!(repo.pause_job(job.id, NOW + 6).expect("pause"));
        assert!(repo.resume_job(job.id, NOW + 70_000).expect("resume"));
        assert_eq!(
            repo.job(job.id).expect("read").expect("job").state,
            JobState::Queued
        );
    }

    #[test]
    fn cancelled_stays_cancelled_until_an_explicit_retry() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        assert!(repo.cancel_job(job.id, NOW).expect("cancel"));
        assert_eq!(repo.claim_job(job.id, OWNER, NOW + 1).expect("claim"), None);
        assert!(repo.retry_job(job.id, NOW + 2).expect("retry"));
        let retried = repo.job(job.id).expect("read").expect("job");
        assert_eq!(retried.state, JobState::Queued);
        assert_eq!(retried.attempts, 0);
        assert_eq!(retried.finished_at, None);
    }

    #[test]
    fn stopped_jobs_keep_a_clipped_error_and_need_an_explicit_retry() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        repo.claim_job(job.id, OWNER, NOW).expect("claim");
        let long = "x".repeat(1000);
        assert!(repo
            .stop_job(job.id, JobState::OutcomeUnknown, &long, NOW + 1)
            .expect("stop"));
        let stopped = repo.job(job.id).expect("read").expect("job");
        assert_eq!(stopped.state, JobState::OutcomeUnknown);
        assert_eq!(stopped.error.map(|error| error.chars().count()), Some(300));
        assert_eq!(repo.claim_job(job.id, OWNER, NOW + 2).expect("claim"), None);
        assert!(repo.retry_job(job.id, NOW + 3).expect("retry"));
        assert!(repo
            .stop_job(job.id, JobState::Cancelled, "x", NOW + 4)
            .is_err());
    }

    #[test]
    fn interrupted_jobs_are_those_claimed_by_another_run() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let first = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        let second = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        repo.claim_job(first.id, "old-run", NOW).expect("claim");
        repo.claim_job(second.id, OWNER, NOW).expect("claim");
        let stale = repo.interrupted_jobs(OWNER).expect("interrupted");
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, first.id);
        assert!(!repo
            .complete_job(first.id, OWNER, &result(), NOW + 1)
            .expect("complete"));
        assert!(repo
            .requeue_interrupted(first.id, NOW + 2)
            .expect("requeue"));
        assert_eq!(
            repo.job(first.id).expect("read").expect("job").state,
            JobState::Queued
        );
        assert!(repo
            .interrupted_jobs(OWNER)
            .expect("interrupted")
            .is_empty());
    }

    #[test]
    fn the_queue_pause_flag_persists_across_reopening() {
        let (directory, mut repo) = repo();
        assert!(!repo.queue_paused().expect("read"));
        repo.set_queue_paused(true).expect("pause");
        drop(repo);
        let mut reopened =
            HistoryRepo::open_write(&directory.path().join("history.db")).expect("reopen");
        assert!(reopened.queue_paused().expect("read"));
        reopened.set_queue_paused(false).expect("resume");
        assert!(!reopened.queue_paused().expect("read"));
    }

    #[test]
    fn rebinding_changes_the_fingerprint_only_while_the_job_is_not_running_or_complete() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        assert!(repo.rebind_job(job.id, "fp-2", NOW + 1).expect("rebind"));
        assert_eq!(
            repo.job(job.id)
                .expect("read")
                .expect("job")
                .config_fingerprint,
            "fp-2"
        );
        repo.claim_job(job.id, OWNER, NOW + 2).expect("claim");
        assert!(!repo.rebind_job(job.id, "fp-3", NOW + 3).expect("rebind"));
    }

    #[test]
    fn jobs_survive_their_capture_being_deleted() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let job = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        repo.delete(capture_id).expect("delete");
        let kept = repo.job(job.id).expect("read").expect("job");
        assert_eq!(kept.capture_id, None);
        assert_eq!(kept.state, JobState::Queued);
    }

    #[test]
    fn listing_filters_by_state_and_prune_removes_only_old_finished_jobs() {
        let (_directory, mut repo) = repo();
        let capture_id = capture(&mut repo);
        let done = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        let waiting = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        let cancelled = repo
            .create_job(new_job(Some(capture_id), JobState::Queued))
            .expect("create");
        repo.claim_job(done.id, OWNER, NOW).expect("claim");
        repo.complete_job(done.id, OWNER, &result(), NOW + 10)
            .expect("complete");
        repo.cancel_job(cancelled.id, NOW + 20).expect("cancel");
        let active = repo
            .jobs_in(&[JobState::Queued, JobState::Uploading])
            .expect("list");
        assert_eq!(
            active.iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![waiting.id]
        );
        let pruned = repo.prune_jobs(NOW + 15).expect("prune");
        assert_eq!(
            pruned.iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![done.id]
        );
        assert!(repo.job(done.id).expect("read").is_none());
        assert!(repo.job(cancelled.id).expect("read").is_some());
        assert!(repo.job(waiting.id).expect("read").is_some());
        assert_eq!(repo.jobs_for_capture(capture_id).expect("list").len(), 2);
    }
}
