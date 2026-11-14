use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use snapline_store::history::{HistoryRepo, Job, JobBatch, JobFile, JobResult, JobState, NewJob};
use snapline_upload::error::UploadError;
use snapline_upload::queue::{
    Change, Handle, Journal, JournalError, Registered, RestoreStart, RestoredJob, Status,
    UploadFile, UploadId, UploadJob,
};

use crate::uploads::{DestinationSource, TEST_RUN_ID};
use crate::{ServiceError, Services};

pub const INTERRUPTED_MESSAGE: &str =
    "Snapline closed while this upload was running. The file may have reached the destination.";
pub const DESTINATION_REMOVED_MESSAGE: &str =
    "The upload destination was removed. Choose a destination and retry.";
pub const DESTINATION_CHANGED_MESSAGE: &str =
    "The destination settings changed after this upload was queued. Retry to upload with the new settings.";
pub const FILE_MISSING_MESSAGE: &str =
    "The file for this upload is no longer available. Add it again to upload it.";

const FINISHED_RETENTION_MS: i64 = 30 * 86_400_000;

static NEXT_STAGED: AtomicU64 = AtomicU64::new(1);

pub fn run_owner() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

pub fn fingerprint_for(config: &impl serde::Serialize) -> String {
    let text = serde_json::to_string(config).unwrap_or_default();
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub struct StoreJournal {
    services: Services,
    destinations: Arc<dyn DestinationSource>,
    owner: String,
}

impl StoreJournal {
    pub fn new(
        services: &Services,
        destinations: Arc<dyn DestinationSource>,
        owner: String,
    ) -> Self {
        Self {
            services: services.clone(),
            destinations,
            owner,
        }
    }

    fn repo(&self) -> Result<HistoryRepo, JournalError> {
        let paths = self.services.paths().map_err(|_| JournalError)?;
        HistoryRepo::open_write(&paths.history_file).map_err(|_| JournalError)
    }

    fn staging_dir(&self) -> Result<PathBuf, JournalError> {
        Ok(self
            .services
            .paths()
            .map_err(|_| JournalError)?
            .upload_staging_dir)
    }

    fn stage(&self, file: &UploadFile) -> Result<(PathBuf, i64, i64), JournalError> {
        let directory = self.staging_dir()?;
        std::fs::create_dir_all(&directory).map_err(|_| JournalError)?;
        let name = format!(
            "{}-{}-{}",
            std::process::id(),
            NEXT_STAGED.fetch_add(1, Ordering::Relaxed),
            sanitise_file_name(&file.file_name)
        );
        let target = directory.join(name);
        let result = match &file.body {
            snapline_upload::queue::UploadBody::Bytes(bytes) => {
                std::fs::write(&target, bytes.as_slice())
            }
            snapline_upload::queue::UploadBody::Path { .. } => {
                let mut reader = file.open().map_err(|_| JournalError)?;
                std::fs::File::create(&target).and_then(|mut output| {
                    std::io::copy(&mut reader, &mut output)?;
                    output.sync_all()
                })
            }
        };
        if result.is_err() {
            let _ = std::fs::remove_file(&target);
            return Err(JournalError);
        }
        let metadata = std::fs::metadata(&target).map_err(|_| JournalError)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
            .ok_or(JournalError)?;
        let len = i64::try_from(metadata.len()).map_err(|_| JournalError)?;
        Ok((target, len, modified))
    }

    fn remove_staged(&self, job: &Job) {
        if job.file.staged {
            let _ = std::fs::remove_file(&job.file.path);
        }
    }
}

fn sanitise_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    cleaned.chars().take(80).collect()
}

fn job_id(id: UploadId) -> i64 {
    i64::try_from(id.get()).unwrap_or(i64::MAX)
}

impl Journal for StoreJournal {
    fn register(&self, job: &UploadJob) -> Result<Option<Registered>, JournalError> {
        if job.run_id.as_deref() == Some(TEST_RUN_ID) || self.services.paths().is_err() {
            return Ok(None);
        }
        let mut repo = self.repo()?;
        let (capture_id, revision, owned) = match job.capture_id {
            Some(capture_id) => match repo.get(capture_id).map_err(|_| JournalError)? {
                Some(capture) => {
                    let revision = repo
                        .current_revision(capture_id)
                        .map_err(|_| JournalError)?
                        .map(|revision| revision.revision);
                    (Some(capture_id), revision, capture.owned)
                }
                None => (None, None, false),
            },
            None => (None, None, false),
        };
        let (file, staged_file) = if owned {
            (
                JobFile {
                    name: job.file.file_name.clone(),
                    content_type: job.file.content_type.clone(),
                    path: job.file.path.to_string_lossy().into_owned(),
                    len: i64::try_from(job.file.len).map_err(|_| JournalError)?,
                    modified_ms: modified_of(&job.file)?,
                    staged: false,
                },
                None,
            )
        } else {
            let (path, len, modified_ms) = self.stage(&job.file)?;
            let staged = UploadFile::from_path(
                path.clone(),
                job.file.file_name.clone(),
                job.file.content_type.clone(),
                u64::try_from(len).map_err(|_| JournalError)?,
                modified_ms,
            );
            (
                JobFile {
                    name: job.file.file_name.clone(),
                    content_type: job.file.content_type.clone(),
                    path: path.to_string_lossy().into_owned(),
                    len,
                    modified_ms,
                    staged: true,
                },
                Some(staged),
            )
        };
        let staged_path = file.staged.then(|| PathBuf::from(&file.path));
        let created = repo.create_job(NewJob {
            capture_id,
            revision,
            operation_id: None,
            dest_id: job.dest.clone(),
            config_fingerprint: self.destinations.fingerprint(&job.dest).unwrap_or_default(),
            run_id: job.run_id.clone(),
            batch: job.link_batch.as_ref().map(|batch| JobBatch {
                id: batch.id.clone(),
                index: batch.index,
                size: batch.size,
            }),
            file,
            state: JobState::Queued,
            created_at: now_ms(),
        });
        match created {
            Ok(created) => Ok(Some(Registered {
                id: UploadId::new(u64::try_from(created.id).map_err(|_| JournalError)?),
                file: staged_file,
            })),
            Err(_) => {
                if let Some(path) = staged_path {
                    let _ = std::fs::remove_file(path);
                }
                Err(JournalError)
            }
        }
    }

    fn start(&self, id: UploadId) -> Result<bool, JournalError> {
        let mut repo = self.repo()?;
        Ok(repo
            .claim_job(job_id(id), &self.owner, now_ms())
            .map_err(|_| JournalError)?
            .is_some())
    }

    fn record(&self, id: UploadId, change: &Change<'_>) -> Result<(), JournalError> {
        let mut repo = self.repo()?;
        let id = job_id(id);
        let now = now_ms();
        let applied = match change {
            Change::Retrying { retry_in, error } => {
                let due = now.saturating_add(i64::try_from(retry_in.as_millis()).unwrap_or(0));
                repo.schedule_retry(id, &self.owner, due, &error.to_string(), now)
            }
            Change::Done(uploaded) => {
                let applied = repo.complete_job(
                    id,
                    &self.owner,
                    &JobResult {
                        url: uploaded.link.clone(),
                        page_url: uploaded.page_link.clone(),
                        deletion_url: uploaded.deletion_link.as_ref().map(|link| link.url.clone()),
                    },
                    now,
                );
                if let Ok(true) = applied {
                    if let Ok(Some(job)) = repo.job(id) {
                        self.remove_staged(&job);
                    }
                }
                applied.map(|_| true)
            }
            Change::Failed(error) => {
                repo.stop_job(id, JobState::NeedsAttention, &error.to_string(), now)
            }
            Change::OutcomeUnknown(error) => {
                repo.stop_job(id, JobState::OutcomeUnknown, &error.to_string(), now)
            }
            Change::Cancelled => repo.cancel_job(id, now),
            Change::Paused => repo.pause_job(id, now),
            Change::Resumed => repo.resume_job(id, now),
            Change::Requeued => {
                if let Ok(Some(job)) = repo.job(id) {
                    if let Some(fingerprint) = self.destinations.fingerprint(&job.dest_id) {
                        let _ = repo.rebind_job(id, &fingerprint, now);
                    }
                }
                repo.retry_job(id, now)
            }
        };
        match (applied, change) {
            (Ok(true), _) => Ok(()),
            (
                Ok(false),
                Change::Retrying { .. }
                | Change::Done(_)
                | Change::Failed(_)
                | Change::OutcomeUnknown(_),
            ) => Ok(()),
            _ => Err(JournalError),
        }
    }
}

fn modified_of(file: &UploadFile) -> Result<i64, JournalError> {
    match &file.body {
        snapline_upload::queue::UploadBody::Path { modified_ms, .. } => Ok(*modified_ms),
        snapline_upload::queue::UploadBody::Bytes(_) => Ok(now_ms()),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    pub resumed: usize,
    pub waiting: usize,
    pub paused: usize,
    pub needs_attention: usize,
    pub outcome_unknown: usize,
}

struct Missing;

impl snapline_upload::queue::Uploader for Missing {
    fn upload(
        &self,
        _file: &UploadFile,
        _secrets: &dyn snapline_upload::queue::SecretStore,
    ) -> Result<snapline_upload::queue::Uploaded, UploadError> {
        Err(UploadError::permanent(DESTINATION_REMOVED_MESSAGE))
    }

    fn delete(
        &self,
        _link: &snapline_upload::queue::DeletionLink,
        _secrets: &dyn snapline_upload::queue::SecretStore,
    ) -> Result<(), UploadError> {
        Err(UploadError::permanent(DESTINATION_REMOVED_MESSAGE))
    }
}

fn upload_file(job: &Job) -> Result<UploadFile, ServiceError> {
    Ok(UploadFile::from_path(
        job.file.path.clone(),
        job.file.name.clone(),
        job.file.content_type.clone(),
        u64::try_from(job.file.len)
            .map_err(|_| ServiceError::Invalid("The upload file size was not valid.".to_owned()))?,
        job.file.modified_ms,
    ))
}

fn restored_stopped(
    job: &Job,
    uploader: Arc<dyn snapline_upload::queue::Uploader>,
    status: Status,
    error: UploadError,
) -> Option<RestoredJob> {
    let file = upload_file(job).ok()?;
    let mut upload = UploadJob::new(uploader, file, job.dest_id.clone(), None);
    if let Some(capture_id) = job.capture_id {
        upload = upload.with_capture_id(capture_id);
    }
    Some(RestoredJob {
        id: UploadId::new(u64::try_from(job.id).ok()?),
        job: upload,
        attempts: u32::try_from(job.attempts).unwrap_or(u32::MAX),
        start: RestoreStart::Stopped {
            status,
            error: Some(error),
        },
    })
}

fn restore_live(
    repo: &mut HistoryRepo,
    job: &Job,
    uploader: Option<Arc<dyn snapline_upload::queue::Uploader>>,
    destinations: &dyn DestinationSource,
    now: i64,
    summary: &mut Recovery,
) -> Result<Option<RestoredJob>, ServiceError> {
    let io = |error: snapline_store::history::HistoryError| ServiceError::Io(error.to_string());
    let problem = match &uploader {
        None => Some(DESTINATION_REMOVED_MESSAGE),
        Some(_)
            if destinations
                .fingerprint(&job.dest_id)
                .is_some_and(|current| current != job.config_fingerprint) =>
        {
            Some(DESTINATION_CHANGED_MESSAGE)
        }
        Some(_) if !Path::new(&job.file.path).is_file() => Some(FILE_MISSING_MESSAGE),
        Some(_) => None,
    };
    if let Some(message) = problem {
        repo.stop_job(job.id, JobState::NeedsAttention, message, now)
            .map_err(io)?;
        summary.needs_attention += 1;
        let uploader = uploader.unwrap_or_else(|| Arc::new(Missing));
        return Ok(restored_stopped(
            job,
            uploader,
            Status::Failed,
            UploadError::Permanent(message.to_owned()),
        ));
    }
    let Some(uploader) = uploader else {
        return Ok(None);
    };
    let Ok(file) = upload_file(job) else {
        return Ok(None);
    };
    let Ok(id) = u64::try_from(job.id) else {
        return Ok(None);
    };
    let mut upload = UploadJob::new(uploader, file, job.dest_id.clone(), None);
    if let Some(capture_id) = job.capture_id {
        upload = upload.with_capture_id(capture_id);
    }
    let until_retry = job
        .next_retry_at
        .map(|due| Duration::from_millis(u64::try_from((due - now).max(0)).unwrap_or(0)));
    let start = match job.state {
        JobState::Paused => {
            summary.paused += 1;
            RestoreStart::Paused {
                retry_in: until_retry,
            }
        }
        JobState::WaitingToRetry => match until_retry {
            Some(delay) if !delay.is_zero() => {
                summary.waiting += 1;
                RestoreStart::After(delay)
            }
            _ => {
                summary.resumed += 1;
                RestoreStart::Queued
            }
        },
        _ => {
            summary.resumed += 1;
            RestoreStart::Queued
        }
    };
    Ok(Some(RestoredJob {
        id: UploadId::new(id),
        job: upload,
        attempts: u32::try_from(job.attempts).unwrap_or(u32::MAX),
        start,
    }))
}

pub fn hold_for_history(
    services: &Services,
    destinations: &dyn DestinationSource,
    operation_id: &str,
    path: &Path,
    dest: &str,
) -> Result<u64, ServiceError> {
    if operation_id.trim().is_empty() {
        return Err(ServiceError::Invalid("The save was not valid.".to_owned()));
    }
    if dest.trim().is_empty() {
        return Err(ServiceError::Invalid(
            "Choose an upload destination.".to_owned(),
        ));
    }
    if destinations.uploader(dest).is_none() {
        return Err(ServiceError::Invalid(
            "The upload destination was not found.".to_owned(),
        ));
    }
    let io = |error: snapline_store::history::HistoryError| ServiceError::Io(error.to_string());
    let metadata = std::fs::metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .ok_or(ServiceError::NotFound)?;
    let file = JobFile {
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("capture")
            .to_owned(),
        content_type: snapline_core::media::content_type(path).to_owned(),
        path: path.to_string_lossy().into_owned(),
        len: i64::try_from(metadata.len())
            .map_err(|_| ServiceError::Invalid("The file size was not valid.".to_owned()))?,
        modified_ms: modified_ms(&metadata),
        staged: false,
    };
    let paths = services.paths()?;
    let mut repo = HistoryRepo::open_write(&paths.history_file).map_err(io)?;
    let created = repo
        .create_job(NewJob {
            capture_id: None,
            revision: None,
            operation_id: Some(operation_id.to_owned()),
            dest_id: dest.to_owned(),
            config_fingerprint: destinations.fingerprint(dest).unwrap_or_default(),
            run_id: None,
            batch: None,
            file,
            state: JobState::WaitingForHistory,
            created_at: now_ms(),
        })
        .map_err(io)?;
    u64::try_from(created.id).map_err(|_| ServiceError::Internal("The job id was invalid.".into()))
}

fn modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default()
}

fn attach_ready(repo: &mut HistoryRepo, now: i64) -> Result<(Vec<i64>, usize), ServiceError> {
    let io = |error: snapline_store::history::HistoryError| ServiceError::Io(error.to_string());
    let mut attached = Vec::new();
    let mut remaining = 0;
    for job in repo.jobs_in(&[JobState::WaitingForHistory]).map_err(io)? {
        let registered = match job.operation_id.as_deref() {
            Some(operation) => repo.capture_for_operation(operation).map_err(io)?,
            None => None,
        };
        let (Some(operation), Some(capture_id)) = (job.operation_id.as_deref(), registered) else {
            remaining += 1;
            continue;
        };
        let Some(capture) = repo.get(capture_id).map_err(io)? else {
            remaining += 1;
            continue;
        };
        let revision = repo
            .current_revision(capture_id)
            .map_err(io)?
            .map_or(1, |revision| revision.revision);
        let mut file = job.file.clone();
        file.path = capture.path.clone();
        if let Ok(metadata) = std::fs::metadata(&capture.path) {
            if let Ok(len) = i64::try_from(metadata.len()) {
                file.len = len;
            }
            file.modified_ms = modified_ms(&metadata);
        }
        if repo
            .attach_operation(operation, capture_id, revision, &file, now)
            .map_err(io)?
            > 0
        {
            attached.push(job.id);
        }
    }
    Ok((attached, remaining))
}

pub fn release_ready(
    services: &Services,
    queue: &Handle,
    destinations: &dyn DestinationSource,
) -> Result<bool, ServiceError> {
    let paths = services.paths()?;
    if !paths.history_file.is_file() {
        return Ok(false);
    }
    let io = |error: snapline_store::history::HistoryError| ServiceError::Io(error.to_string());
    let mut repo = HistoryRepo::open_write(&paths.history_file).map_err(io)?;
    let now = now_ms();
    let (attached, remaining) = attach_ready(&mut repo, now)?;
    let mut summary = Recovery::default();
    let mut restored = Vec::new();
    for id in attached {
        let Some(job) = repo.job(id).map_err(io)? else {
            continue;
        };
        let uploader = destinations.uploader(&job.dest_id);
        if let Some(entry) =
            restore_live(&mut repo, &job, uploader, destinations, now, &mut summary)?
        {
            restored.push(entry);
        }
    }
    queue
        .restore(restored)
        .map_err(|error| ServiceError::Internal(error.to_string()))?;
    Ok(remaining > 0)
}

pub fn recover(
    services: &Services,
    queue: &Handle,
    destinations: &dyn DestinationSource,
    owner: &str,
) -> Result<Recovery, ServiceError> {
    let paths = services.paths()?;
    if !paths.history_file.is_file() {
        return Ok(Recovery::default());
    }
    let mut repo = HistoryRepo::open_write(&paths.history_file)
        .map_err(|error| ServiceError::Io(error.to_string()))?;
    let io = |error: snapline_store::history::HistoryError| ServiceError::Io(error.to_string());
    let now = now_ms();
    let mut summary = Recovery::default();
    let mut restored = Vec::new();

    queue
        .set_paused(repo.queue_paused().map_err(io)?)
        .map_err(|error| ServiceError::Internal(error.to_string()))?;

    attach_ready(&mut repo, now)?;

    for job in repo.interrupted_jobs(owner).map_err(io)? {
        let uploader = destinations.uploader(&job.dest_id);
        let safe = uploader
            .as_ref()
            .is_some_and(|uploader| uploader.idempotent())
            && destinations
                .fingerprint(&job.dest_id)
                .is_some_and(|current| current == job.config_fingerprint);
        if safe {
            repo.requeue_interrupted(job.id, now).map_err(io)?;
        } else {
            repo.stop_job(job.id, JobState::OutcomeUnknown, INTERRUPTED_MESSAGE, now)
                .map_err(io)?;
        }
    }

    let stopped_states = [JobState::NeedsAttention, JobState::OutcomeUnknown];
    let live_states = [JobState::Queued, JobState::WaitingToRetry, JobState::Paused];
    for job in repo
        .jobs_in(&[live_states.as_slice(), stopped_states.as_slice()].concat())
        .map_err(io)?
    {
        let uploader = destinations.uploader(&job.dest_id);
        let live = live_states.contains(&job.state);
        if !live {
            let (status, error) = if job.state == JobState::OutcomeUnknown {
                (
                    Status::OutcomeUnknown,
                    UploadError::Uncertain(
                        job.error
                            .clone()
                            .unwrap_or_else(|| INTERRUPTED_MESSAGE.to_owned()),
                    ),
                )
            } else {
                (
                    Status::Failed,
                    UploadError::Permanent(
                        job.error
                            .clone()
                            .unwrap_or_else(|| FILE_MISSING_MESSAGE.to_owned()),
                    ),
                )
            };
            if job.state == JobState::OutcomeUnknown {
                summary.outcome_unknown += 1;
            } else {
                summary.needs_attention += 1;
            }
            let uploader = uploader.unwrap_or_else(|| Arc::new(Missing));
            if let Some(entry) = restored_stopped(&job, uploader, status, error) {
                restored.push(entry);
            }
            continue;
        }
        if let Some(entry) =
            restore_live(&mut repo, &job, uploader, destinations, now, &mut summary)?
        {
            restored.push(entry);
        }
    }

    let cutoff = now.saturating_sub(FINISHED_RETENTION_MS);
    for pruned in repo.prune_jobs(cutoff).map_err(io)? {
        if pruned.file.staged {
            let _ = std::fs::remove_file(&pruned.file.path);
        }
    }

    queue
        .restore(restored)
        .map_err(|error| ServiceError::Internal(error.to_string()))?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use snapline_core::ports::{SecretError as CoreSecretError, SecretStore};
    use snapline_core::secret::{Secret, SecretKey};
    use snapline_store::history::{CaptureKind, NewCapture};
    use snapline_store::io::Paths;
    use snapline_upload::queue::{
        DeletionLink, RetryPolicy, SecretStore as QueueSecrets, Uploaded, Uploader,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::dto::upload::{UploadReq, UploadResp, UploadStatus};
    use crate::uploads::{FileSource, UploaderThread};

    struct NoSecrets;

    impl SecretStore for NoSecrets {
        fn get(&self, _key: &SecretKey) -> Result<Option<Secret>, CoreSecretError> {
            Ok(None)
        }

        fn put(&self, _key: &SecretKey, _value: &Secret) -> Result<(), CoreSecretError> {
            Ok(())
        }

        fn delete(&self, _key: &SecretKey) -> Result<(), CoreSecretError> {
            Ok(())
        }
    }

    struct NoFiles;

    impl FileSource for NoFiles {
        fn file(&self, _capture_id: i64) -> Result<Option<UploadFile>, ServiceError> {
            Ok(None)
        }
    }

    struct Fake {
        calls: Mutex<Vec<String>>,
        results: Mutex<VecDeque<Result<Uploaded, UploadError>>>,
        idempotent: bool,
    }

    impl Fake {
        fn new(idempotent: bool, results: Vec<Result<Uploaded, UploadError>>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                results: Mutex::new(results.into()),
                idempotent,
            })
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl Uploader for Fake {
        fn upload(
            &self,
            file: &UploadFile,
            _secrets: &dyn QueueSecrets,
        ) -> Result<Uploaded, UploadError> {
            self.calls
                .lock()
                .expect("calls")
                .push(file.file_name.clone());
            self.results
                .lock()
                .expect("results")
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(Uploaded::new(format!(
                        "https://example.test/{}",
                        file.file_name
                    )))
                })
        }

        fn idempotent(&self) -> bool {
            self.idempotent
        }

        fn delete(
            &self,
            _link: &DeletionLink,
            _secrets: &dyn QueueSecrets,
        ) -> Result<(), UploadError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Destinations {
        entries: Mutex<HashMap<String, (Arc<Fake>, String)>>,
    }

    impl Destinations {
        fn set(&self, id: &str, uploader: Arc<Fake>, fingerprint: &str) {
            self.entries
                .lock()
                .expect("entries")
                .insert(id.to_owned(), (uploader, fingerprint.to_owned()));
        }

        fn remove(&self, id: &str) {
            self.entries.lock().expect("entries").remove(id);
        }
    }

    impl DestinationSource for Destinations {
        fn uploader(&self, dest: &str) -> Option<Arc<dyn Uploader>> {
            self.entries
                .lock()
                .expect("entries")
                .get(dest)
                .map(|(uploader, _)| Arc::clone(uploader) as Arc<dyn Uploader>)
        }

        fn fingerprint(&self, dest: &str) -> Option<String> {
            self.entries
                .lock()
                .expect("entries")
                .get(dest)
                .map(|(_, fingerprint)| fingerprint.clone())
        }
    }

    struct Fixture {
        directory: TempDir,
        services: Services,
        destinations: Arc<Destinations>,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("temp directory");
            let root = directory.path();
            let paths = Paths::resolve(
                &root.join("app"),
                &root.join("roaming"),
                &root.join("local"),
                &root.join("profile"),
                false,
            );
            let services = Services::with_paths(paths).expect("services");
            Self {
                directory,
                services,
                destinations: Arc::new(Destinations::default()),
            }
        }

        fn start(&self, retries: u32) -> UploaderThread {
            UploaderThread::start(
                &self.services,
                Arc::new(NoSecrets),
                RetryPolicy::with_backoff(retries, vec![Duration::from_millis(30)]),
                Arc::new(NoFiles),
                self.destinations.clone(),
            )
            .expect("uploader")
        }

        fn repo(&self) -> HistoryRepo {
            HistoryRepo::open_write(&self.services.paths().expect("paths").history_file)
                .expect("history")
        }

        fn external_file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let folder = self.directory.path().join("external");
            std::fs::create_dir_all(&folder).expect("folder");
            let path = folder.join(name);
            std::fs::write(&path, bytes).expect("file");
            path
        }

        fn job_input(&self, dest: &str, path: &Path, fingerprint: &str, state: JobState) -> NewJob {
            let metadata = std::fs::metadata(path).expect("metadata");
            NewJob {
                capture_id: None,
                revision: None,
                operation_id: None,
                dest_id: dest.to_owned(),
                config_fingerprint: fingerprint.to_owned(),
                run_id: None,
                batch: None,
                file: JobFile {
                    name: path
                        .file_name()
                        .expect("name")
                        .to_string_lossy()
                        .into_owned(),
                    content_type: "image/png".to_owned(),
                    path: path.to_string_lossy().into_owned(),
                    len: i64::try_from(metadata.len()).expect("len"),
                    modified_ms: i64::try_from(
                        metadata
                            .modified()
                            .expect("modified")
                            .duration_since(UNIX_EPOCH)
                            .expect("epoch")
                            .as_millis(),
                    )
                    .expect("ms"),
                    staged: false,
                },
                state,
                created_at: now_ms(),
            }
        }
    }

    fn states(thread: &UploaderThread) -> Vec<(u64, UploadStatus, u32)> {
        match thread.service().handle(UploadReq::List).expect("list") {
            UploadResp::States(states) => states
                .into_iter()
                .map(|state| (state.id, state.status, state.attempts))
                .collect(),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn wait_for(thread: &UploaderThread, id: u64, status: UploadStatus) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if states(thread)
                .iter()
                .any(|(found, current, _)| *found == id && *current == status)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("upload {id} never reached {status:?}: {:?}", states(thread));
    }

    fn enqueue_external(
        fixture: &Fixture,
        thread: &UploaderThread,
        dest: &str,
        path: &Path,
    ) -> u64 {
        let uploader = fixture
            .destinations
            .uploader(dest)
            .expect("registered destination");
        let file = UploadFile::read(path, "image/png").expect("file");
        thread.enqueue(uploader, file, dest, None).expect("enqueue")
    }

    #[test]
    fn an_external_file_is_copied_into_managed_storage_before_the_job_is_accepted() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let source = fixture.external_file("shot.png", b"pixels");
        let thread = fixture.start(0);
        let id = enqueue_external(&fixture, &thread, "dest", &source);
        let job = fixture
            .repo()
            .job(i64::try_from(id).expect("id"))
            .expect("read")
            .expect("job");
        assert!(job.file.staged || job.state == JobState::Complete);
        wait_for(&thread, id, UploadStatus::Done);
        assert_eq!(fake.calls(), vec!["shot.png".to_owned()]);
        assert!(source.is_file());
        assert_eq!(std::fs::read(&source).expect("source"), b"pixels");
        let done = fixture
            .repo()
            .job(i64::try_from(id).expect("id"))
            .expect("read")
            .expect("job");
        assert_eq!(done.state, JobState::Complete);
        assert_eq!(done.attempts, 1);
        assert_eq!(done.url.as_deref(), Some("https://example.test/shot.png"));
        assert!(!Path::new(&done.file.path).exists());
        assert_eq!(done.config_fingerprint, "fp");
    }

    #[test]
    fn deleting_the_source_after_acceptance_does_not_break_the_upload() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let source = fixture.external_file("gone.png", b"pixels");
        let thread = fixture.start(0);
        thread.handle().set_paused(true).expect("hold");
        let id = enqueue_external(&fixture, &thread, "dest", &source);
        std::fs::remove_file(&source).expect("remove source");
        thread.handle().set_paused(false).expect("release");
        wait_for(&thread, id, UploadStatus::Done);
        assert_eq!(fake.calls(), vec!["gone.png".to_owned()]);
    }

    #[test]
    fn a_safe_retry_survives_a_restart_and_keeps_its_attempt_count() {
        let fixture = Fixture::new();
        let flaky = Fake::new(
            false,
            vec![Err(UploadError::Retryable(
                "The connection failed.".to_owned(),
            ))],
        );
        fixture.destinations.set("dest", flaky.clone(), "fp");
        let source = fixture.external_file("retry.png", b"pixels");
        let thread = fixture.start(3);
        let handle = thread.handle();
        handle.set_paused(true).expect("hold");
        let id = enqueue_external(&fixture, &thread, "dest", &source);
        let numeric = i64::try_from(id).expect("id");
        {
            let mut repo = fixture.repo();
            let job = repo.claim_job(numeric, "old-run", now_ms()).expect("claim");
            assert!(job.is_some());
            repo.schedule_retry(
                numeric,
                "old-run",
                now_ms() + 60,
                "The connection failed.",
                now_ms(),
            )
            .expect("schedule");
        }
        drop(thread);

        let next = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", next.clone(), "fp");
        let restarted = fixture.start(3);
        let summary = restarted.recover().expect("recover");
        assert_eq!(summary.waiting + summary.resumed, 1);
        wait_for(&restarted, id, UploadStatus::Done);
        let done = fixture.repo().job(numeric).expect("read").expect("job");
        assert_eq!(done.state, JobState::Complete);
        assert_eq!(done.attempts, 2);
        assert_eq!(next.calls().len(), 1);
    }

    #[test]
    fn a_completed_job_is_never_repeated_after_a_restart() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let source = fixture.external_file("once.png", b"pixels");
        let thread = fixture.start(0);
        let id = enqueue_external(&fixture, &thread, "dest", &source);
        wait_for(&thread, id, UploadStatus::Done);
        drop(thread);
        let restarted = fixture.start(0);
        let summary = restarted.recover().expect("recover");
        assert_eq!(summary, Recovery::default());
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(fake.calls().len(), 1);
        assert!(states(&restarted).is_empty());
    }

    #[test]
    fn an_interrupted_upload_is_outcome_unknown_unless_the_destination_is_idempotent() {
        let fixture = Fixture::new();
        let cautious = Fake::new(false, Vec::new());
        let keyed = Fake::new(true, Vec::new());
        fixture.destinations.set("cautious", cautious.clone(), "fp");
        fixture.destinations.set("keyed", keyed.clone(), "fp");
        let first = fixture.external_file("a.png", b"a");
        let second = fixture.external_file("b.png", b"b");
        let (cautious_id, keyed_id) = {
            let mut repo = fixture.repo();
            let one = repo
                .create_job(fixture.job_input("cautious", &first, "fp", JobState::Queued))
                .expect("job");
            let two = repo
                .create_job(fixture.job_input("keyed", &second, "fp", JobState::Queued))
                .expect("job");
            repo.claim_job(one.id, "old-run", now_ms()).expect("claim");
            repo.claim_job(two.id, "old-run", now_ms()).expect("claim");
            (one.id, two.id)
        };
        let thread = fixture.start(0);
        let summary = thread.recover().expect("recover");
        assert_eq!(summary.outcome_unknown, 1);
        wait_for(
            &thread,
            u64::try_from(keyed_id).expect("id"),
            UploadStatus::Done,
        );
        wait_for(
            &thread,
            u64::try_from(cautious_id).expect("id"),
            UploadStatus::OutcomeUnknown,
        );
        assert!(cautious.calls().is_empty());
        assert_eq!(keyed.calls(), vec!["b.png".to_owned()]);
        let stopped = fixture.repo().job(cautious_id).expect("read").expect("job");
        assert_eq!(stopped.state, JobState::OutcomeUnknown);
        assert!(stopped
            .error
            .as_deref()
            .is_some_and(|error| error.contains("may have reached")));
        drop(thread);
        let again = fixture.start(0);
        let summary = again.recover().expect("recover");
        assert_eq!(summary.outcome_unknown, 1);
        assert!(cautious.calls().is_empty());
        let unknown = states(&again)
            .into_iter()
            .find(|(id, _, _)| *id == u64::try_from(cautious_id).expect("id"));
        assert_eq!(
            unknown.map(|entry| entry.1),
            Some(UploadStatus::OutcomeUnknown)
        );
        again
            .service()
            .handle(UploadReq::Retry {
                id: u64::try_from(cautious_id).expect("id"),
            })
            .expect("explicit retry");
        wait_for(
            &again,
            u64::try_from(cautious_id).expect("id"),
            UploadStatus::Done,
        );
        assert_eq!(cautious.calls().len(), 1);
    }

    #[test]
    fn pauses_and_cancellations_survive_a_restart() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let held = fixture.external_file("held.png", b"h");
        let dropped = fixture.external_file("dropped.png", b"d");
        let thread = fixture.start(0);
        thread.handle().set_paused(true).expect("hold");
        let held_id = enqueue_external(&fixture, &thread, "dest", &held);
        let dropped_id = enqueue_external(&fixture, &thread, "dest", &dropped);
        thread
            .service()
            .handle(UploadReq::Pause { id: held_id })
            .expect("pause");
        thread
            .service()
            .handle(UploadReq::Cancel { id: dropped_id })
            .expect("cancel");
        drop(thread);

        let restarted = fixture.start(0);
        restarted.recover().expect("recover");
        std::thread::sleep(Duration::from_millis(150));
        assert!(fake.calls().is_empty());
        let current = states(&restarted);
        assert_eq!(
            current
                .iter()
                .find(|(id, _, _)| *id == held_id)
                .map(|entry| entry.1),
            Some(UploadStatus::Paused)
        );
        assert!(!current.iter().any(|(id, _, _)| *id == dropped_id));
        let cancelled = fixture
            .repo()
            .job(i64::try_from(dropped_id).expect("id"))
            .expect("read")
            .expect("job");
        assert_eq!(cancelled.state, JobState::Cancelled);
        restarted
            .service()
            .handle(UploadReq::Resume { id: held_id })
            .expect("resume");
        restarted
            .service()
            .handle(UploadReq::SetQueuePaused { paused: false })
            .expect("release");
        wait_for(&restarted, held_id, UploadStatus::Done);
        assert_eq!(fake.calls(), vec!["held.png".to_owned()]);
    }

    #[test]
    fn the_queue_pause_flag_is_remembered_across_restarts() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let source = fixture.external_file("later.png", b"l");
        let thread = fixture.start(0);
        thread
            .service()
            .handle(UploadReq::SetQueuePaused { paused: true })
            .expect("pause");
        let id = enqueue_external(&fixture, &thread, "dest", &source);
        drop(thread);

        let restarted = fixture.start(0);
        restarted.recover().expect("recover");
        assert_eq!(
            restarted
                .service()
                .handle(UploadReq::QueueState)
                .expect("state"),
            UploadResp::QueuePaused { paused: true }
        );
        std::thread::sleep(Duration::from_millis(150));
        assert!(fake.calls().is_empty());
        restarted
            .service()
            .handle(UploadReq::SetQueuePaused { paused: false })
            .expect("release");
        wait_for(&restarted, id, UploadStatus::Done);
    }

    #[test]
    fn a_removed_or_changed_destination_needs_attention_and_is_never_redirected() {
        let fixture = Fixture::new();
        let old = Fake::new(false, Vec::new());
        fixture.destinations.set("removed", old.clone(), "fp");
        fixture.destinations.set("changed", old.clone(), "fp-old");
        let a = fixture.external_file("a.png", b"a");
        let b = fixture.external_file("b.png", b"b");
        let (removed_id, changed_id) = {
            let mut repo = fixture.repo();
            let one = repo
                .create_job(fixture.job_input("removed", &a, "fp", JobState::Queued))
                .expect("job");
            let two = repo
                .create_job(fixture.job_input("changed", &b, "fp-old", JobState::Queued))
                .expect("job");
            (one.id, two.id)
        };
        fixture.destinations.remove("removed");
        let replacement = Fake::new(false, Vec::new());
        fixture
            .destinations
            .set("changed", replacement.clone(), "fp-new");
        let thread = fixture.start(0);
        let summary = thread.recover().expect("recover");
        assert_eq!(summary.needs_attention, 2);
        std::thread::sleep(Duration::from_millis(150));
        assert!(old.calls().is_empty());
        assert!(replacement.calls().is_empty());
        for id in [removed_id, changed_id] {
            let job = fixture.repo().job(id).expect("read").expect("job");
            assert_eq!(job.state, JobState::NeedsAttention);
        }
        let changed = fixture.repo().job(changed_id).expect("read").expect("job");
        assert_eq!(changed.error.as_deref(), Some(DESTINATION_CHANGED_MESSAGE));
        let changed_id = u64::try_from(changed_id).expect("id");
        thread
            .service()
            .handle(UploadReq::Retry { id: changed_id })
            .expect("explicit retry");
        wait_for(&thread, changed_id, UploadStatus::Done);
        assert_eq!(replacement.calls(), vec!["b.png".to_owned()]);
        let rebound = fixture
            .repo()
            .job(i64::try_from(changed_id).expect("id"))
            .expect("read")
            .expect("job");
        assert_eq!(rebound.config_fingerprint, "fp-new");
    }

    #[test]
    fn a_missing_staged_file_needs_attention() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let source = fixture.external_file("lost.png", b"x");
        let id = {
            let mut repo = fixture.repo();
            let job = repo
                .create_job(fixture.job_input("dest", &source, "fp", JobState::Queued))
                .expect("job");
            job.id
        };
        std::fs::remove_file(&source).expect("remove");
        let thread = fixture.start(0);
        let summary = thread.recover().expect("recover");
        assert_eq!(summary.needs_attention, 1);
        let job = fixture.repo().job(id).expect("read").expect("job");
        assert_eq!(job.state, JobState::NeedsAttention);
        assert_eq!(job.error.as_deref(), Some(FILE_MISSING_MESSAGE));
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn an_upload_that_may_have_reached_the_server_stops_for_an_explicit_retry() {
        let fixture = Fixture::new();
        let fake = Fake::new(
            false,
            vec![Err(UploadError::Uncertain(
                "The request timed out.".to_owned(),
            ))],
        );
        fixture.destinations.set("dest", fake.clone(), "fp");
        let source = fixture.external_file("maybe.png", b"x");
        let thread = fixture.start(5);
        let id = enqueue_external(&fixture, &thread, "dest", &source);
        wait_for(&thread, id, UploadStatus::OutcomeUnknown);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(fake.calls().len(), 1);
        let job = fixture
            .repo()
            .job(i64::try_from(id).expect("id"))
            .expect("read")
            .expect("job");
        assert_eq!(job.state, JobState::OutcomeUnknown);
        assert!(Path::new(&job.file.path).is_file());
        drop(thread);
        let restarted = fixture.start(5);
        restarted.recover().expect("recover");
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(fake.calls().len(), 1);
    }

    #[test]
    fn owned_captures_are_uploaded_in_place_and_bound_to_their_revision() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let path = fixture.external_file("owned.png", b"pixels");
        let capture_id = {
            let mut repo = fixture.repo();
            repo.record(NewCapture {
                kind: CaptureKind::Image,
                path: path.to_string_lossy().into_owned(),
                bytes: 6,
                width: None,
                height: None,
                duration_ms: None,
                source_app: None,
                created_at: now_ms(),
                ocr_text: None,
            })
            .expect("capture")
        };
        let thread = fixture.start(0);
        let uploader = fixture.destinations.uploader("dest").expect("uploader");
        let file = UploadFile::read(&path, "image/png").expect("file");
        let id = thread
            .handle()
            .enqueue(
                snapline_upload::queue::UploadJob::new(uploader, file, "dest", None)
                    .with_capture_id(capture_id),
            )
            .expect("enqueue")
            .get();
        wait_for(&thread, id, UploadStatus::Done);
        let job = fixture
            .repo()
            .job(i64::try_from(id).expect("id"))
            .expect("read")
            .expect("job");
        assert_eq!(job.capture_id, Some(capture_id));
        assert_eq!(job.revision, Some(1));
        assert!(path.is_file());
    }

    fn capture_input(path: &Path) -> NewCapture {
        NewCapture {
            kind: CaptureKind::Image,
            path: path.to_string_lossy().into_owned(),
            bytes: 6,
            width: None,
            height: None,
            duration_ms: None,
            source_app: None,
            created_at: now_ms(),
            ocr_text: None,
        }
    }

    fn hold(thread: &UploaderThread, operation: &str, path: &Path) -> u64 {
        match thread
            .service()
            .handle(UploadReq::HoldForHistory {
                operation_id: operation.to_owned(),
                path: path.to_string_lossy().into_owned(),
                dest: "dest".to_owned(),
            })
            .expect("hold")
        {
            UploadResp::Accepted { id } => id,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn an_upload_held_for_history_waits_and_then_uploads_once_the_capture_exists() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let path = fixture.external_file("pending.png", b"pixels");
        let thread = fixture.start(0);
        let id = hold(&thread, "op-1", &path);
        std::thread::sleep(Duration::from_millis(200));
        assert!(fake.calls().is_empty());
        assert!(states(&thread).is_empty());
        let held = fixture
            .repo()
            .job(i64::try_from(id).expect("id"))
            .expect("read")
            .expect("job");
        assert_eq!(held.state, JobState::WaitingForHistory);
        assert_eq!(held.capture_id, None);
        let capture_id = fixture
            .repo()
            .record_operation("op-1", capture_input(&path))
            .expect("register")
            .id;
        wait_for(&thread, id, UploadStatus::Done);
        assert_eq!(fake.calls(), vec!["pending.png".to_owned()]);
        let done = fixture
            .repo()
            .job(i64::try_from(id).expect("id"))
            .expect("read")
            .expect("job");
        assert_eq!(done.capture_id, Some(capture_id));
        assert_eq!(done.revision, Some(1));
        assert_eq!(done.state, JobState::Complete);
        assert!(!done.file.staged);
        assert!(path.is_file());
    }

    #[test]
    fn a_held_upload_survives_a_restart_and_starts_when_history_catches_up() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let path = fixture.external_file("later.png", b"pixels");
        let first = fixture.start(0);
        let id = hold(&first, "op-2", &path);
        first.join().expect("join");
        let second = fixture.start(0);
        let summary = second.recover().expect("recover");
        assert_eq!(summary.resumed, 0);
        std::thread::sleep(Duration::from_millis(150));
        assert!(fake.calls().is_empty());
        fixture
            .repo()
            .record_operation("op-2", capture_input(&path))
            .expect("register");
        wait_for(&second, id, UploadStatus::Done);
        assert_eq!(fake.calls(), vec!["later.png".to_owned()]);
    }

    #[test]
    fn recovery_starts_a_held_upload_whose_capture_was_registered_while_closed() {
        let fixture = Fixture::new();
        let fake = Fake::new(false, Vec::new());
        fixture.destinations.set("dest", fake.clone(), "fp");
        let path = fixture.external_file("closed.png", b"pixels");
        let first = fixture.start(0);
        let id = hold(&first, "op-3", &path);
        first.join().expect("join");
        fixture
            .repo()
            .record_operation("op-3", capture_input(&path))
            .expect("register");
        let second = fixture.start(0);
        let summary = second.recover().expect("recover");
        assert_eq!(summary.resumed, 1);
        wait_for(&second, id, UploadStatus::Done);
    }

    #[test]
    fn a_held_upload_is_rejected_for_a_missing_file_or_destination() {
        let fixture = Fixture::new();
        fixture
            .destinations
            .set("dest", Fake::new(false, Vec::new()), "fp");
        let path = fixture.external_file("here.png", b"x");
        let thread = fixture.start(0);
        let service = thread.service();
        assert_eq!(
            service
                .handle(UploadReq::HoldForHistory {
                    operation_id: "op".to_owned(),
                    path: fixture
                        .directory
                        .path()
                        .join("gone.png")
                        .to_string_lossy()
                        .into_owned(),
                    dest: "dest".to_owned(),
                })
                .expect_err("missing file"),
            ServiceError::NotFound
        );
        assert!(matches!(
            service
                .handle(UploadReq::HoldForHistory {
                    operation_id: "op".to_owned(),
                    path: path.to_string_lossy().into_owned(),
                    dest: "missing".to_owned(),
                })
                .expect_err("missing destination"),
            ServiceError::Invalid(_)
        ));
        assert!(fixture
            .repo()
            .jobs_in(&[JobState::WaitingForHistory])
            .expect("jobs")
            .is_empty());
    }
}
