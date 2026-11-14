use std::collections::{HashMap, VecDeque};
use std::io::{self, Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::error::UploadError;

pub const DEFAULT_BACKOFF: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(8),
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(300),
];
pub const MAX_BUFFERED_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UploadId(u64);

impl UploadId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadFile {
    pub path: PathBuf,
    pub file_name: String,
    pub content_type: String,
    pub len: u64,
    pub body: UploadBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadBody {
    Bytes(Arc<Vec<u8>>),
    Path { path: PathBuf, modified_ms: i64 },
}

impl UploadFile {
    pub fn new(
        path: impl Into<PathBuf>,
        file_name: impl Into<String>,
        content_type: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        let bytes = bytes.into();
        Self {
            path: path.into(),
            file_name: file_name.into(),
            content_type: content_type.into(),
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            body: UploadBody::Bytes(Arc::new(bytes)),
        }
    }

    pub fn from_path(
        path: impl Into<PathBuf>,
        file_name: impl Into<String>,
        content_type: impl Into<String>,
        len: u64,
        modified_ms: i64,
    ) -> Self {
        let path = path.into();
        Self {
            path: path.clone(),
            file_name: file_name.into(),
            content_type: content_type.into(),
            len,
            body: UploadBody::Path { path, modified_ms },
        }
    }

    pub fn read(
        path: impl Into<PathBuf>,
        content_type: impl Into<String>,
    ) -> Result<Self, std::io::Error> {
        let path = path.into();
        let metadata = std::fs::metadata(&path)?;
        let modified_ms = modified_ms(&metadata)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("upload")
            .to_owned();
        Ok(Self::from_path(
            path.clone(),
            file_name,
            content_type,
            metadata.len(),
            modified_ms,
        ))
    }

    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    pub fn open(&self) -> Result<Box<dyn Read + Send + '_>, UploadError> {
        match &self.body {
            UploadBody::Bytes(bytes) => Ok(Box::new(Cursor::new(bytes.as_slice()))),
            UploadBody::Path {
                path,
                modified_ms: expected,
            } => {
                let file = std::fs::File::open(path).map_err(|_| {
                    UploadError::permanent(
                        "The file is no longer available. Add it again before uploading.",
                    )
                })?;
                let metadata = file
                    .metadata()
                    .map_err(|_| UploadError::permanent("The file could not be checked."))?;
                if !metadata.is_file()
                    || metadata.len() != self.len
                    || modified_ms(&metadata).ok() != Some(*expected)
                {
                    return Err(UploadError::permanent(
                        "The file changed after it was added. Add it again to upload the new version.",
                    ));
                }
                Ok(Box::new(LengthLimitedReader {
                    file,
                    remaining: self.len,
                }))
            }
        }
    }

    pub fn read_all(&self, limit: u64) -> Result<Vec<u8>, UploadError> {
        if self.len > limit {
            return Err(UploadError::TooLarge);
        }
        match &self.body {
            UploadBody::Bytes(bytes) => {
                if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != self.len {
                    return Err(UploadError::permanent("The upload body length is invalid."));
                }
                Ok(bytes.as_ref().clone())
            }
            UploadBody::Path { .. } => {
                let mut reader = self.open()?;
                let capacity = usize::try_from(self.len).unwrap_or(0);
                let mut bytes = Vec::with_capacity(capacity);
                reader.read_to_end(&mut bytes).map_err(|error| {
                    if error.kind() == io::ErrorKind::UnexpectedEof {
                        UploadError::permanent(
                            "The file became shorter while it was being uploaded.",
                        )
                    } else {
                        UploadError::permanent("The file could not be read.")
                    }
                })?;
                if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != self.len {
                    return Err(UploadError::permanent(
                        "The file became shorter while it was being uploaded.",
                    ));
                }
                Ok(bytes)
            }
        }
    }
}

struct LengthLimitedReader {
    file: std::fs::File,
    remaining: u64,
}

impl Read for LengthLimitedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || self.remaining == 0 {
            return Ok(0);
        }
        let limit =
            usize::try_from(self.remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = self.file.read(&mut buffer[..limit])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "The file became shorter while it was being uploaded.",
            ));
        }
        self.remaining -= u64::try_from(read).unwrap_or(u64::MAX);
        Ok(read)
    }
}

fn modified_ms(metadata: &std::fs::Metadata) -> Result<i64, io::Error> {
    let modified = metadata.modified()?;
    let elapsed = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid file timestamp"))?;
    i64::try_from(elapsed.as_millis())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "file timestamp is too large"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionLink {
    pub dest: String,
    pub url: String,
    pub token: Option<String>,
}

impl DeletionLink {
    pub fn new(dest: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            dest: dest.into(),
            url: url.into(),
            token: None,
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploaded {
    pub link: String,
    pub page_link: Option<String>,
    pub deletion_link: Option<DeletionLink>,
}

impl Uploaded {
    pub fn new(link: impl Into<String>) -> Self {
        Self {
            link: link.into(),
            page_link: None,
            deletion_link: None,
        }
    }

    pub fn with_page_link(mut self, page_link: impl Into<String>) -> Self {
        self.page_link = Some(page_link.into());
        self
    }

    pub fn with_deletion_link(mut self, deletion_link: DeletionLink) -> Self {
        self.deletion_link = Some(deletion_link);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SecretError {
    #[error("The saved secret could not be read.")]
    Unavailable,
}

pub trait SecretStore: Send + Sync {
    fn get(&self, dest_id: &str, field: &str) -> Result<Option<Vec<u8>>, SecretError>;

    fn secret(&self, dest_id: &str, field: &str) -> Result<Option<String>, UploadError> {
        match self.get(dest_id, field) {
            Ok(Some(bytes)) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| UploadError::Permanent("The saved secret is not valid.".to_owned())),
            Ok(None) => Ok(None),
            Err(_) => Err(UploadError::Permanent(
                "The saved secret could not be read.".to_owned(),
            )),
        }
    }
}

pub trait Uploader: Send + Sync {
    fn upload(&self, file: &UploadFile, secrets: &dyn SecretStore)
        -> Result<Uploaded, UploadError>;

    fn supports_in_flight_cancel(&self) -> bool {
        false
    }

    fn idempotent(&self) -> bool {
        false
    }

    fn reports_progress(&self) -> bool {
        false
    }

    fn upload_controlled(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
        _control: &dyn UploadControl,
    ) -> Result<Uploaded, UploadError> {
        self.upload(file, secrets)
    }

    fn delete(&self, link: &DeletionLink, secrets: &dyn SecretStore) -> Result<(), UploadError>;
}

pub trait UploadControl: Send + Sync {
    fn sent(&self, bytes: u64);
    fn cancelled(&self) -> bool;
}

pub struct NoopUploadControl;

impl UploadControl for NoopUploadControl {
    fn sent(&self, _bytes: u64) {}

    fn cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Queued,
    Uploading,
    BackingOff { retry_in: Duration },
    Paused,
    Done,
    Failed,
    OutcomeUnknown,
    Cancelled,
}

impl Status {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::OutcomeUnknown | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadSnapshot {
    pub id: UploadId,
    pub capture_id: Option<i64>,
    pub file_name: String,
    pub dest: String,
    pub run_id: Option<String>,
    pub status: Status,
    pub cancellable: bool,
    pub progress_available: bool,
    pub sent: u64,
    pub total: u64,
    pub link_batch: Option<LinkBatch>,
    pub attempts: u32,
    pub link: Option<String>,
    pub page_link: Option<String>,
    pub deletion_link: Option<String>,
    pub error: Option<UploadError>,
    pub restored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkBatch {
    pub id: String,
    pub index: u32,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadEvent(pub UploadSnapshot);

#[derive(Clone)]
pub struct UploadJob {
    pub uploader: Arc<dyn Uploader>,
    pub file: UploadFile,
    pub dest: String,
    pub run_id: Option<String>,
    pub capture_id: Option<i64>,
    pub link_batch: Option<LinkBatch>,
}

impl UploadJob {
    pub fn new(
        uploader: Arc<dyn Uploader>,
        file: UploadFile,
        dest: impl Into<String>,
        run_id: Option<String>,
    ) -> Self {
        Self {
            uploader,
            file,
            dest: dest.into(),
            run_id,
            capture_id: None,
            link_batch: None,
        }
    }

    pub fn with_capture_id(mut self, capture_id: i64) -> Self {
        self.capture_id = Some(capture_id);
        self
    }

    pub fn with_link_batch(mut self, batch: LinkBatch) -> Self {
        self.link_batch = Some(batch);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    max_retries: u32,
    backoff: Vec<Duration>,
}

impl RetryPolicy {
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            backoff: DEFAULT_BACKOFF.to_vec(),
        }
    }

    pub fn with_backoff(max_retries: u32, backoff: Vec<Duration>) -> Self {
        Self {
            max_retries,
            backoff,
        }
    }

    pub const fn max_retries(&self) -> u32 {
        self.max_retries
    }

    pub fn backoff(&self) -> &[Duration] {
        &self.backoff
    }

    pub fn delay(&self, attempt: u32) -> Duration {
        if self.backoff.is_empty() {
            return Duration::ZERO;
        }
        let index = usize::try_from(attempt.saturating_sub(1)).unwrap_or(usize::MAX);
        self.backoff[index.min(self.backoff.len() - 1)]
    }

    pub fn delay_for(&self, attempt: u32, error: &UploadError) -> Duration {
        retry_after(error)
            .map(|delay| delay.min(Duration::from_secs(3600)))
            .unwrap_or_else(|| self.delay(attempt))
    }
}

pub fn retry_after(error: &UploadError) -> Option<Duration> {
    let (UploadError::Retryable(message) | UploadError::Uncertain(message)) = error else {
        return None;
    };
    let marker = "retry after ";
    let start = message.rfind(marker)? + marker.len();
    let digits: String = message[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u64>().ok().map(Duration::from_secs)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QueueError {
    #[error("The upload queue could not start.")]
    Spawn,
    #[error("The upload queue is unavailable.")]
    Closed,
    #[error("The upload was not found.")]
    NotFound,
    #[error("The upload cannot be cancelled.")]
    NotCancellable,
    #[error("Only failed, cancelled or uncertain uploads can be retried.")]
    NotRetryable,
    #[error("The upload cannot be paused.")]
    NotPausable,
    #[error("The upload is not paused.")]
    NotPaused,
    #[error("The upload queue could not be saved.")]
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("The upload queue could not be saved.")]
pub struct JournalError;

pub struct Registered {
    pub id: UploadId,
    pub file: Option<UploadFile>,
}

pub enum Change<'a> {
    Retrying {
        retry_in: Duration,
        error: &'a UploadError,
    },
    Done(&'a Uploaded),
    Failed(&'a UploadError),
    OutcomeUnknown(&'a UploadError),
    Cancelled,
    Paused,
    Resumed,
    Requeued,
}

pub trait Journal: Send + Sync {
    fn register(&self, job: &UploadJob) -> Result<Option<Registered>, JournalError>;

    fn start(&self, id: UploadId) -> Result<bool, JournalError>;

    fn record(&self, id: UploadId, change: &Change<'_>) -> Result<(), JournalError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreStart {
    Queued,
    After(Duration),
    Paused {
        retry_in: Option<Duration>,
    },
    Stopped {
        status: Status,
        error: Option<UploadError>,
    },
}

pub struct RestoredJob {
    pub id: UploadId,
    pub job: UploadJob,
    pub attempts: u32,
    pub start: RestoreStart,
}

pub const EPHEMERAL_ID_BASE: u64 = 1 << 40;

enum Command {
    Enqueue(Vec<(UploadId, UploadJob)>),
    Cancel {
        id: UploadId,
        reply: Sender<Result<(), QueueError>>,
    },
    Retry {
        id: UploadId,
        reply: Sender<Result<(), QueueError>>,
    },
    Pause {
        id: UploadId,
        reply: Sender<Result<(), QueueError>>,
    },
    Resume {
        id: UploadId,
        reply: Sender<Result<(), QueueError>>,
    },
    SetPaused(bool),
    Restore(Vec<RestoredJob>),
    State {
        id: UploadId,
        reply: Sender<Option<UploadSnapshot>>,
    },
    List {
        reply: Sender<Vec<UploadSnapshot>>,
    },
    Shutdown,
}

#[derive(Clone)]
enum Listener {
    Channel(Arc<Mutex<Sender<UploadEvent>>>),
    Callback(Arc<dyn Fn(UploadEvent) + Send + Sync>),
}

impl Listener {
    fn emit(&self, event: UploadEvent) {
        match self {
            Self::Channel(sender) => {
                if let Ok(sender) = sender.lock() {
                    let _ = sender.send(event);
                }
            }
            Self::Callback(callback) => callback(event),
        }
    }
}

#[derive(Clone)]
pub struct Handle {
    commands: Arc<Mutex<Sender<Command>>>,
    next_id: Arc<AtomicU64>,
    journal: Option<Arc<dyn Journal>>,
    cancellations: Arc<Mutex<HashMap<UploadId, ActiveCancellation>>>,
}

#[derive(Clone)]
struct ActiveCancellation {
    cancelled: Arc<AtomicBool>,
    cancellable: bool,
}

impl Handle {
    pub fn enqueue(&self, job: UploadJob) -> Result<UploadId, QueueError> {
        let entry = self.register(job)?;
        let id = entry.0;
        if let Err(error) = self.send(Command::Enqueue(vec![entry])) {
            self.record_cancelled(&[id]);
            return Err(error);
        }
        Ok(id)
    }

    pub fn enqueue_batch(&self, jobs: Vec<UploadJob>) -> Result<Vec<UploadId>, QueueError> {
        let mut entries: Vec<(UploadId, UploadJob)> = Vec::with_capacity(jobs.len());
        for job in jobs {
            match self.register(job) {
                Ok(entry) => entries.push(entry),
                Err(error) => {
                    let ids: Vec<_> = entries.iter().map(|entry| entry.0).collect();
                    self.record_cancelled(&ids);
                    return Err(error);
                }
            }
        }
        let ids: Vec<UploadId> = entries.iter().map(|entry| entry.0).collect();
        if let Err(error) = self.send(Command::Enqueue(entries)) {
            self.record_cancelled(&ids);
            return Err(error);
        }
        Ok(ids)
    }

    pub fn restore(&self, jobs: Vec<RestoredJob>) -> Result<(), QueueError> {
        if jobs.is_empty() {
            return Ok(());
        }
        self.send(Command::Restore(jobs))
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), QueueError> {
        self.send(Command::SetPaused(paused))
    }

    pub fn pause(&self, id: UploadId) -> Result<(), QueueError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Pause { id, reply })?;
        response.recv().map_err(|_| QueueError::Closed)?
    }

    pub fn resume(&self, id: UploadId) -> Result<(), QueueError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Resume { id, reply })?;
        response.recv().map_err(|_| QueueError::Closed)?
    }

    fn register(&self, mut job: UploadJob) -> Result<(UploadId, UploadJob), QueueError> {
        if let Some(journal) = &self.journal {
            if let Some(registered) = journal.register(&job).map_err(|_| QueueError::Storage)? {
                if let Some(file) = registered.file {
                    job.file = file;
                }
                return Ok((registered.id, job));
            }
        }
        Ok((UploadId(self.next_id.fetch_add(1, Ordering::Relaxed)), job))
    }

    fn record_cancelled(&self, ids: &[UploadId]) {
        if let Some(journal) = &self.journal {
            for id in ids {
                let _ = journal.record(*id, &Change::Cancelled);
            }
        }
    }

    pub fn cancel(&self, id: UploadId) -> Result<(), QueueError> {
        if let Some(cancellation) = self
            .cancellations
            .lock()
            .map_err(|_| QueueError::Closed)?
            .get(&id)
            .cloned()
        {
            if !cancellation.cancellable {
                return Err(QueueError::NotCancellable);
            }
            cancellation.cancelled.store(true, Ordering::SeqCst);
            return Ok(());
        }
        let (reply, response) = mpsc::channel();
        self.send(Command::Cancel { id, reply })?;
        response.recv().map_err(|_| QueueError::Closed)?
    }

    pub fn retry(&self, id: UploadId) -> Result<(), QueueError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Retry { id, reply })?;
        response.recv().map_err(|_| QueueError::Closed)?
    }

    pub fn state(&self, id: UploadId) -> Result<Option<UploadSnapshot>, QueueError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::State { id, reply })?;
        response.recv().map_err(|_| QueueError::Closed)
    }

    pub fn list(&self) -> Result<Vec<UploadSnapshot>, QueueError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::List { reply })?;
        response.recv().map_err(|_| QueueError::Closed)
    }

    pub fn shutdown(&self) -> Result<(), QueueError> {
        self.send(Command::Shutdown)
    }

    fn send(&self, command: Command) -> Result<(), QueueError> {
        self.commands
            .lock()
            .map_err(|_| QueueError::Closed)?
            .send(command)
            .map_err(|_| QueueError::Closed)
    }
}

pub struct Queue {
    handle: Handle,
    thread: Option<JoinHandle<()>>,
}

impl Queue {
    pub fn start(
        secrets: Arc<dyn SecretStore>,
        policy: RetryPolicy,
    ) -> Result<(Self, Receiver<UploadEvent>), QueueError> {
        let (events, receiver) = mpsc::channel();
        let listener = Listener::Channel(Arc::new(Mutex::new(events)));
        Ok((Self::spawn(secrets, policy, listener, None)?, receiver))
    }

    pub fn start_with_sink<F>(
        secrets: Arc<dyn SecretStore>,
        policy: RetryPolicy,
        sink: F,
    ) -> Result<Self, QueueError>
    where
        F: Fn(UploadEvent) + Send + Sync + 'static,
    {
        Self::spawn(secrets, policy, Listener::Callback(Arc::new(sink)), None)
    }

    pub fn start_durable<F>(
        secrets: Arc<dyn SecretStore>,
        policy: RetryPolicy,
        journal: Arc<dyn Journal>,
        sink: F,
    ) -> Result<Self, QueueError>
    where
        F: Fn(UploadEvent) + Send + Sync + 'static,
    {
        Self::spawn(
            secrets,
            policy,
            Listener::Callback(Arc::new(sink)),
            Some(journal),
        )
    }

    fn spawn(
        secrets: Arc<dyn SecretStore>,
        policy: RetryPolicy,
        listener: Listener,
        journal: Option<Arc<dyn Journal>>,
    ) -> Result<Self, QueueError> {
        let (commands, receiver) = mpsc::channel();
        let first_id = if journal.is_some() {
            EPHEMERAL_ID_BASE
        } else {
            1
        };
        let thread_journal = journal.clone();
        let handle = Handle {
            commands: Arc::new(Mutex::new(commands)),
            journal,
            next_id: Arc::new(AtomicU64::new(first_id)),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        };
        let cancellations = Arc::clone(&handle.cancellations);
        let thread = thread::Builder::new()
            .name("snapline-upload".to_owned())
            .spawn(move || {
                run(
                    receiver,
                    secrets,
                    policy,
                    listener,
                    cancellations,
                    thread_journal,
                )
            })
            .map_err(|_| QueueError::Spawn)?;
        Ok(Self {
            handle,
            thread: Some(thread),
        })
    }

    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    pub fn join(mut self) -> Result<(), QueueError> {
        let result = self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        result
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        let _ = self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct JobRecord {
    id: UploadId,
    job: UploadJob,
    status: Status,
    attempts: u32,
    error: Option<UploadError>,
    retry_at: Option<Instant>,
    progress: Arc<Mutex<ProgressData>>,
}

impl JobRecord {
    fn new(id: UploadId, job: UploadJob) -> Self {
        let total = job.file.len;
        Self {
            id,
            job,
            status: Status::Queued,
            attempts: 0,
            error: None,
            retry_at: None,
            progress: Arc::new(Mutex::new(ProgressData {
                sent: 0,
                total,
                last_emit: Instant::now(),
                last_emitted_sent: 0,
            })),
        }
    }

    fn snapshot(&self) -> UploadSnapshot {
        let (sent, total) = self
            .progress
            .lock()
            .map(|progress| (progress.sent, progress.total))
            .unwrap_or_default();
        UploadSnapshot {
            id: self.id,
            capture_id: self.job.capture_id,
            file_name: self.job.file.file_name.clone(),
            dest: self.job.dest.clone(),
            run_id: self.job.run_id.clone(),
            status: self.status.clone(),
            cancellable: match &self.status {
                Status::Queued | Status::BackingOff { .. } | Status::Paused => true,
                Status::Uploading => self.job.uploader.supports_in_flight_cancel(),
                Status::Done | Status::Failed | Status::OutcomeUnknown | Status::Cancelled => false,
            },
            progress_available: self.job.uploader.reports_progress(),
            sent,
            total,
            link_batch: self.job.link_batch.clone(),
            attempts: self.attempts,
            link: None,
            page_link: None,
            deletion_link: None,
            error: self.error.clone(),
            restored: false,
        }
    }
}

struct ProgressData {
    sent: u64,
    total: u64,
    last_emit: Instant,
    last_emitted_sent: u64,
}

struct JobUploadControl {
    cancelled: Arc<AtomicBool>,
    progress: Arc<Mutex<ProgressData>>,
    listener: Listener,
    snapshot: UploadSnapshot,
}

impl UploadControl for JobUploadControl {
    fn sent(&self, bytes: u64) {
        let Ok(mut progress) = self.progress.lock() else {
            return;
        };
        progress.sent = progress.sent.max(bytes.min(progress.total));
        let increment = progress.total.div_ceil(100);
        let advanced = progress.sent.saturating_sub(progress.last_emitted_sent);
        let should_emit = progress.last_emit.elapsed() >= Duration::from_millis(100)
            || advanced >= increment
            || progress.sent == progress.total;
        if !should_emit || advanced == 0 {
            return;
        }
        progress.last_emit = Instant::now();
        progress.last_emitted_sent = progress.sent;
        let sent = progress.sent;
        let total = progress.total;
        drop(progress);
        let mut snapshot = self.snapshot.clone();
        snapshot.sent = sent;
        snapshot.total = total;
        self.listener.emit(UploadEvent(snapshot));
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

enum Control {
    Continue,
    Shutdown,
}

/// How many finished uploads are kept for `state` and `list` before the oldest are dropped.
const MAX_COMPLETED: usize = 200;

struct State {
    pending: VecDeque<UploadId>,
    delayed: Vec<(Instant, UploadId)>,
    records: HashMap<UploadId, JobRecord>,
    completed: HashMap<UploadId, UploadSnapshot>,
    listener: Listener,
    journal: Option<Arc<dyn Journal>>,
    paused_all: bool,
}

impl State {
    fn new(listener: Listener, journal: Option<Arc<dyn Journal>>) -> Self {
        Self {
            pending: VecDeque::new(),
            delayed: Vec::new(),
            records: HashMap::new(),
            completed: HashMap::new(),
            listener,
            journal,
            paused_all: false,
        }
    }

    fn journal(&self, id: UploadId, change: &Change<'_>) -> Result<(), QueueError> {
        if id.get() >= EPHEMERAL_ID_BASE {
            return Ok(());
        }
        let Some(journal) = &self.journal else {
            return Ok(());
        };
        for attempt in 0..3 {
            if journal.record(id, change).is_ok() {
                return Ok(());
            }
            if attempt < 2 {
                thread::sleep(Duration::from_millis(50));
            }
        }
        Err(QueueError::Storage)
    }

    /// Moves every job whose backoff has elapsed back to the front of the line.
    fn promote_due(&mut self) {
        let now = Instant::now();
        let mut due: Vec<(Instant, UploadId)> = Vec::new();
        self.delayed.retain(|entry| {
            if entry.0 <= now {
                due.push(*entry);
                false
            } else {
                true
            }
        });
        due.sort_by_key(|entry| entry.0);
        for (_, id) in due.into_iter().rev() {
            self.pending.push_front(id);
        }
    }

    fn next_due(&self) -> Option<Instant> {
        self.delayed.iter().map(|entry| entry.0).min()
    }

    fn snapshot(&self, id: UploadId) -> Option<UploadSnapshot> {
        self.records
            .get(&id)
            .map(JobRecord::snapshot)
            .or_else(|| self.completed.get(&id).cloned())
    }

    fn emit(&self, id: UploadId) {
        if let Some(snapshot) = self.snapshot(id) {
            self.listener.emit(UploadEvent(snapshot));
        }
    }

    fn complete(&mut self, id: UploadId, uploaded: Uploaded) {
        let _ = self.journal(id, &Change::Done(&uploaded));
        let Some(mut record) = self.records.remove(&id) else {
            return;
        };
        record.status = Status::Done;
        let mut snapshot = record.snapshot();
        drop(record);
        snapshot.sent = snapshot.total;
        snapshot.link = Some(uploaded.link);
        snapshot.page_link = uploaded.page_link;
        snapshot.deletion_link = uploaded.deletion_link.map(|link| link.url);
        self.completed.insert(id, snapshot);
        while self.completed.len() > MAX_COMPLETED {
            let Some(oldest) = self.completed.keys().min().copied() else {
                break;
            };
            self.completed.remove(&oldest);
        }
        self.emit(id);
    }

    fn pause(&mut self, id: UploadId) -> Result<(), QueueError> {
        let status = self
            .records
            .get(&id)
            .map(|record| record.status.clone())
            .ok_or_else(|| {
                if self.completed.contains_key(&id) {
                    QueueError::NotPausable
                } else {
                    QueueError::NotFound
                }
            })?;
        let retry_at = match status {
            Status::Queued if self.pending.contains(&id) => None,
            Status::BackingOff { .. } => self
                .delayed
                .iter()
                .find(|entry| entry.1 == id)
                .map(|entry| entry.0),
            _ => return Err(QueueError::NotPausable),
        };
        self.journal(id, &Change::Paused)?;
        self.pending.retain(|item| *item != id);
        self.delayed.retain(|entry| entry.1 != id);
        if let Some(record) = self.records.get_mut(&id) {
            record.status = Status::Paused;
            record.retry_at = retry_at;
        }
        Ok(())
    }

    fn resume(&mut self, id: UploadId) -> Result<(), QueueError> {
        let record = self.records.get(&id).ok_or_else(|| {
            if self.completed.contains_key(&id) {
                QueueError::NotPaused
            } else {
                QueueError::NotFound
            }
        })?;
        if record.status != Status::Paused {
            return Err(QueueError::NotPaused);
        }
        let retry_at = record.retry_at;
        self.journal(id, &Change::Resumed)?;
        let now = Instant::now();
        let Some(record) = self.records.get_mut(&id) else {
            return Ok(());
        };
        match retry_at {
            Some(at) if at > now => {
                record.status = Status::BackingOff {
                    retry_in: at.saturating_duration_since(now),
                };
                self.delayed.push((at, id));
            }
            _ => {
                record.status = Status::Queued;
                self.pending.push_back(id);
            }
        }
        record.retry_at = None;
        Ok(())
    }

    fn restore(&mut self, restored: RestoredJob) {
        let RestoredJob {
            id,
            job,
            attempts,
            start,
        } = restored;
        let mut record = JobRecord::new(id, job);
        record.attempts = attempts;
        match start {
            RestoreStart::Queued => {
                self.pending.push_back(id);
            }
            RestoreStart::After(delay) => {
                record.status = Status::BackingOff { retry_in: delay };
                self.delayed.push((Instant::now() + delay, id));
            }
            RestoreStart::Paused { retry_in } => {
                record.status = Status::Paused;
                record.retry_at = retry_in.map(|delay| Instant::now() + delay);
            }
            RestoreStart::Stopped { status, error } => {
                record.status = status;
                record.error = error;
            }
        }
        self.records.insert(id, record);
        if let Some(mut snapshot) = self.snapshot(id) {
            snapshot.restored = true;
            self.listener.emit(UploadEvent(snapshot));
        }
    }

    fn handle(&mut self, command: Command) -> Control {
        match command {
            Command::Enqueue(entries) => {
                for (id, job) in entries {
                    self.records.insert(id, JobRecord::new(id, job));
                    self.pending.push_back(id);
                    self.emit(id);
                }
                Control::Continue
            }
            Command::Cancel { id, reply } => {
                let status = self
                    .records
                    .get(&id)
                    .map(|record| record.status.clone())
                    .or_else(|| {
                        self.completed
                            .get(&id)
                            .map(|snapshot| snapshot.status.clone())
                    });
                match status {
                    None => {
                        let _ = reply.send(Err(QueueError::NotFound));
                        Control::Continue
                    }
                    Some(Status::BackingOff { .. } | Status::Paused) => {
                        if let Err(error) = self.journal(id, &Change::Cancelled) {
                            let _ = reply.send(Err(error));
                            return Control::Continue;
                        }
                        self.delayed.retain(|entry| entry.1 != id);
                        if let Some(record) = self.records.get_mut(&id) {
                            record.status = Status::Cancelled;
                            record.retry_at = None;
                        }
                        let _ = reply.send(Ok(()));
                        self.emit(id);
                        Control::Continue
                    }
                    Some(Status::Queued) => {
                        if let Some(position) = self.pending.iter().position(|item| *item == id) {
                            if let Err(error) = self.journal(id, &Change::Cancelled) {
                                let _ = reply.send(Err(error));
                                return Control::Continue;
                            }
                            self.pending.remove(position);
                            if let Some(record) = self.records.get_mut(&id) {
                                record.status = Status::Cancelled;
                            }
                            let _ = reply.send(Ok(()));
                            self.emit(id);
                        } else {
                            let _ = reply.send(Err(QueueError::NotCancellable));
                        }
                        Control::Continue
                    }
                    Some(_) => {
                        let _ = reply.send(Err(QueueError::NotCancellable));
                        Control::Continue
                    }
                }
            }
            Command::Retry { id, reply } => {
                let retryable = matches!(
                    self.records.get(&id).map(|record| &record.status),
                    Some(Status::Failed | Status::OutcomeUnknown | Status::Cancelled)
                );
                if retryable {
                    if let Err(error) = self.journal(id, &Change::Requeued) {
                        let _ = reply.send(Err(error));
                        return Control::Continue;
                    }
                    if let Some(record) = self.records.get_mut(&id) {
                        record.status = Status::Queued;
                        record.attempts = 0;
                        record.error = None;
                        if record.job.link_batch.is_some() {
                            record.job.link_batch = Some(LinkBatch {
                                id: format!("retry-{}", id.get()),
                                index: 0,
                                size: 1,
                            });
                        }
                        if let Ok(mut progress) = record.progress.lock() {
                            progress.sent = 0;
                            progress.last_emit = Instant::now();
                            progress.last_emitted_sent = 0;
                        }
                    }
                    self.pending.push_back(id);
                    let _ = reply.send(Ok(()));
                    self.emit(id);
                } else if self.records.contains_key(&id) || self.completed.contains_key(&id) {
                    let _ = reply.send(Err(QueueError::NotRetryable));
                } else {
                    let _ = reply.send(Err(QueueError::NotFound));
                }
                Control::Continue
            }
            Command::Pause { id, reply } => {
                let result = self.pause(id);
                let ok = result.is_ok();
                let _ = reply.send(result);
                if ok {
                    self.emit(id);
                }
                Control::Continue
            }
            Command::Resume { id, reply } => {
                let result = self.resume(id);
                let ok = result.is_ok();
                let _ = reply.send(result);
                if ok {
                    self.emit(id);
                }
                Control::Continue
            }
            Command::SetPaused(paused) => {
                self.paused_all = paused;
                Control::Continue
            }
            Command::Restore(jobs) => {
                for restored in jobs {
                    self.restore(restored);
                }
                Control::Continue
            }
            Command::State { id, reply } => {
                let _ = reply.send(self.snapshot(id));
                Control::Continue
            }
            Command::List { reply } => {
                let mut snapshots: Vec<UploadSnapshot> = self
                    .records
                    .values()
                    .map(JobRecord::snapshot)
                    .chain(self.completed.values().cloned())
                    .collect();
                snapshots.sort_by_key(|snapshot| snapshot.id);
                let _ = reply.send(snapshots);
                Control::Continue
            }
            Command::Shutdown => Control::Shutdown,
        }
    }
}

fn run(
    commands: Receiver<Command>,
    secrets: Arc<dyn SecretStore>,
    policy: RetryPolicy,
    listener: Listener,
    cancellations: Arc<Mutex<HashMap<UploadId, ActiveCancellation>>>,
    journal: Option<Arc<dyn Journal>>,
) {
    let mut state = State::new(listener, journal);
    loop {
        while let Ok(command) = commands.try_recv() {
            if matches!(state.handle(command), Control::Shutdown) {
                return;
            }
        }
        state.promote_due();
        let next = if state.paused_all {
            None
        } else {
            state.pending.pop_front()
        };
        let Some(id) = next else {
            let wake = if state.paused_all {
                None
            } else {
                state.next_due()
            };
            let received = match wake {
                Some(at) => commands.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => commands.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match received {
                Ok(command) => {
                    if matches!(state.handle(command), Control::Shutdown) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            continue;
        };
        if !run_job(
            id,
            &commands,
            secrets.as_ref(),
            &policy,
            &mut state,
            &cancellations,
        ) {
            return;
        }
    }
}

fn run_job(
    id: UploadId,
    commands: &Receiver<Command>,
    secrets: &dyn SecretStore,
    policy: &RetryPolicy,
    state: &mut State,
    cancellations: &Arc<Mutex<HashMap<UploadId, ActiveCancellation>>>,
) -> bool {
    if !state.records.contains_key(&id) {
        return true;
    }
    let durable = if id.get() >= EPHEMERAL_ID_BASE {
        None
    } else {
        state.journal.clone()
    };
    if let Some(journal) = durable {
        match journal.start(id) {
            Ok(true) => {}
            Ok(false) => {
                if let Some(record) = state.records.get_mut(&id) {
                    record.status = Status::Cancelled;
                }
                state.emit(id);
                return true;
            }
            Err(_) => {
                state
                    .delayed
                    .push((Instant::now() + Duration::from_secs(5), id));
                return true;
            }
        }
    }
    let Some(record) = state.records.get_mut(&id) else {
        return true;
    };
    record.attempts += 1;
    record.status = Status::Uploading;
    record.error = None;
    let uploader = Arc::clone(&record.job.uploader);
    let idempotent = uploader.idempotent();
    let file = record.job.file.clone();
    let attempts = record.attempts;
    let progress = Arc::clone(&record.progress);
    if let Ok(mut progress) = progress.lock() {
        progress.sent = 0;
        progress.total = file.len;
        progress.last_emit = Instant::now();
        progress.last_emitted_sent = 0;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    if let Ok(mut active) = cancellations.lock() {
        active.insert(
            id,
            ActiveCancellation {
                cancelled: Arc::clone(&cancelled),
                cancellable: record.job.uploader.supports_in_flight_cancel(),
            },
        );
    }
    let snapshot = record.snapshot();
    let control = JobUploadControl {
        cancelled: Arc::clone(&cancelled),
        progress,
        listener: state.listener.clone(),
        snapshot,
    };
    state.emit(id);
    let result =
        thread::scope(|scope| {
            let attempt = match thread::Builder::new()
                .name("snapline-upload-transfer".to_owned())
                .spawn_scoped(scope, move || {
                    uploader.upload_controlled(&file, secrets, &control)
                }) {
                Ok(attempt) => attempt,
                Err(_) => return Some(Err(UploadError::permanent("The upload could not start."))),
            };
            while !attempt.is_finished() {
                match commands.recv_timeout(Duration::from_millis(20)) {
                    Ok(command) => {
                        if !matches!(state.handle(command), Control::Shutdown) {
                            continue;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => {}
                }
                cancelled.store(true, Ordering::SeqCst);
                let _ = attempt.join();
                return None;
            }
            Some(attempt.join().unwrap_or_else(|_| {
                Err(UploadError::permanent("The upload stopped unexpectedly."))
            }))
        });
    if let Ok(mut active) = cancellations.lock() {
        active.remove(&id);
    }
    let Some(result) = result else {
        return false;
    };
    match result {
        Ok(uploaded) => {
            state.complete(id, uploaded);
            return true;
        }
        Err(error) => {
            if error == UploadError::Cancelled || cancelled.load(Ordering::SeqCst) {
                let _ = state.journal(id, &Change::Cancelled);
                if let Some(record) = state.records.get_mut(&id) {
                    record.status = Status::Cancelled;
                    record.error = None;
                }
                state.emit(id);
                return true;
            }
            let outcome_unknown = error.is_uncertain() && !idempotent;
            if outcome_unknown {
                let _ = state.journal(id, &Change::OutcomeUnknown(&error));
                if let Some(record) = state.records.get_mut(&id) {
                    record.status = Status::OutcomeUnknown;
                    record.error = Some(error);
                }
                state.emit(id);
                return true;
            }
            let retryable = error.is_transient() && attempts <= policy.max_retries();
            if !retryable {
                let _ = state.journal(id, &Change::Failed(&error));
                if let Some(record) = state.records.get_mut(&id) {
                    record.status = Status::Failed;
                    record.error = Some(error);
                }
                state.emit(id);
                return true;
            }
            let delay = policy.delay_for(attempts, &error);
            let _ = state.journal(
                id,
                &Change::Retrying {
                    retry_in: delay,
                    error: &error,
                },
            );
            if let Some(record) = state.records.get_mut(&id) {
                record.status = Status::BackingOff { retry_in: delay };
                record.error = Some(error);
            }
            // The job waits its turn in `delayed` so the worker can serve other uploads.
            state.delayed.push((Instant::now() + delay, id));
            state.emit(id);
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSecrets;

    impl SecretStore for TestSecrets {
        fn get(&self, _dest_id: &str, _field: &str) -> Result<Option<Vec<u8>>, SecretError> {
            Ok(None)
        }
    }

    struct FakeUploader {
        calls: Mutex<Vec<String>>,
        results: Mutex<VecDeque<Result<Uploaded, UploadError>>>,
        in_flight: AtomicU64,
        max_in_flight: AtomicU64,
    }

    impl FakeUploader {
        fn new(results: Vec<Result<Uploaded, UploadError>>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                results: Mutex::new(results.into()),
                in_flight: AtomicU64::new(0),
                max_in_flight: AtomicU64::new(0),
            })
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }

        fn max_in_flight(&self) -> u64 {
            self.max_in_flight.load(Ordering::SeqCst)
        }
    }

    impl Uploader for FakeUploader {
        fn upload(
            &self,
            file: &UploadFile,
            _secrets: &dyn SecretStore,
        ) -> Result<Uploaded, UploadError> {
            let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
            self.calls
                .lock()
                .expect("calls")
                .push(file.file_name.clone());
            let result = self
                .results
                .lock()
                .expect("results")
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(Uploaded::new(format!(
                        "https://example.test/{}",
                        file.file_name
                    )))
                });
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            result
        }

        fn delete(
            &self,
            _link: &DeletionLink,
            _secrets: &dyn SecretStore,
        ) -> Result<(), UploadError> {
            Ok(())
        }
    }

    struct GateUploader {
        started: Sender<()>,
        release: Mutex<Receiver<()>>,
    }

    impl GateUploader {
        fn new() -> (Arc<Self>, Receiver<()>, Sender<()>) {
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            (
                Arc::new(Self {
                    started: started_tx,
                    release: Mutex::new(release_rx),
                }),
                started_rx,
                release_tx,
            )
        }
    }

    impl Uploader for GateUploader {
        fn upload(
            &self,
            _file: &UploadFile,
            _secrets: &dyn SecretStore,
        ) -> Result<Uploaded, UploadError> {
            let _ = self.started.send(());
            if let Ok(release) = self.release.lock() {
                let _ = release.recv();
            }
            Ok(Uploaded::new("https://example.test/gated"))
        }

        fn delete(
            &self,
            _link: &DeletionLink,
            _secrets: &dyn SecretStore,
        ) -> Result<(), UploadError> {
            Ok(())
        }
    }

    fn file(name: &str) -> UploadFile {
        UploadFile::new(
            PathBuf::from(format!("C:\\captures\\{name}")),
            name,
            "image/png",
            vec![1_u8, 2, 3],
        )
    }

    fn job(uploader: Arc<dyn Uploader>, name: &str) -> UploadJob {
        UploadJob::new(uploader, file(name), "test", None)
    }

    fn queue(
        results: Vec<Result<Uploaded, UploadError>>,
        policy: RetryPolicy,
    ) -> (Arc<FakeUploader>, Queue, Receiver<UploadEvent>, Handle) {
        let uploader = FakeUploader::new(results);
        let (queue, events) = Queue::start(Arc::new(TestSecrets), policy).expect("queue");
        let handle = queue.handle();
        (uploader, queue, events, handle)
    }

    fn wait_for(events: &Receiver<UploadEvent>, id: UploadId, status: &Status) -> UploadSnapshot {
        loop {
            let event = events.recv_timeout(Duration::from_secs(5)).expect("event");
            if event.0.id == id && event.0.status == *status {
                return event.0;
            }
        }
    }

    fn wait_terminal(events: &Receiver<UploadEvent>, id: UploadId) -> UploadSnapshot {
        loop {
            let event = events.recv_timeout(Duration::from_secs(5)).expect("event");
            if event.0.id == id && event.0.status.terminal() {
                return event.0;
            }
        }
    }

    fn wait_all_terminal(events: &Receiver<UploadEvent>, count: usize) -> Vec<UploadSnapshot> {
        let mut finished = Vec::new();
        while finished.len() < count {
            let event = events.recv_timeout(Duration::from_secs(5)).expect("event");
            if event.0.status.terminal() {
                finished.push(event.0);
            }
        }
        finished
    }

    #[test]
    fn jobs_run_in_fifo_order_and_only_one_at_a_time() {
        let (uploader, queue, events, handle) = queue(Vec::new(), RetryPolicy::new(0));
        let first = handle
            .enqueue(job(uploader.clone(), "one.png"))
            .expect("first");
        let second = handle
            .enqueue(job(uploader.clone(), "two.png"))
            .expect("second");
        let third = handle
            .enqueue(job(uploader.clone(), "three.png"))
            .expect("third");
        let finished = wait_all_terminal(&events, 3);
        assert_eq!(finished.len(), 3);
        assert_eq!(uploader.calls(), vec!["one.png", "two.png", "three.png"]);
        assert_eq!(uploader.max_in_flight(), 1);
        assert_eq!(
            handle.state(first).expect("state").expect("first").status,
            Status::Done
        );
        assert_eq!(
            handle
                .state(second)
                .expect("state")
                .expect("second")
                .attempts,
            1
        );
        assert_eq!(
            handle.state(third).expect("state").expect("third").status,
            Status::Done
        );
        queue.join().expect("join");
    }

    #[test]
    fn successful_upload_releases_transfer_resources_before_emitting_done() {
        let uploaded = Uploaded::new("https://example.test/shot.png")
            .with_page_link("https://example.test/shot")
            .with_deletion_link(DeletionLink::new(
                "test",
                "https://example.test/delete/shot",
            ));
        let uploader = FakeUploader::new(vec![Ok(uploaded)]);
        let uploader_ref = Arc::downgrade(&uploader);
        let job = job(uploader, "shot.png").with_capture_id(42);
        let UploadBody::Bytes(payload) = &job.file.body else {
            panic!("expected an in-memory upload");
        };
        let payload_ref = Arc::downgrade(payload);
        let (done, completion) = mpsc::channel();
        let queue =
            Queue::start_with_sink(Arc::new(TestSecrets), RetryPolicy::new(0), move |event| {
                if event.0.status == Status::Done {
                    let _ = done.send((
                        event.0,
                        uploader_ref.upgrade().is_none(),
                        payload_ref.upgrade().is_none(),
                    ));
                }
            })
            .expect("queue");
        let handle = queue.handle();
        let id = handle.enqueue(job).expect("enqueue");
        let (snapshot, uploader_released, payload_released) = completion
            .recv_timeout(Duration::from_secs(5))
            .expect("completion");
        assert!(uploader_released);
        assert!(payload_released);
        assert_eq!(snapshot.id, id);
        assert_eq!(snapshot.capture_id, Some(42));
        assert_eq!(snapshot.sent, 3);
        assert_eq!(snapshot.total, 3);
        assert!(!snapshot.cancellable);
        assert_eq!(
            snapshot.link.as_deref(),
            Some("https://example.test/shot.png")
        );
        assert_eq!(
            snapshot.page_link.as_deref(),
            Some("https://example.test/shot")
        );
        assert_eq!(
            snapshot.deletion_link.as_deref(),
            Some("https://example.test/delete/shot")
        );
        assert_eq!(handle.state(id).expect("state"), Some(snapshot.clone()));
        assert_eq!(handle.list().expect("list"), vec![snapshot]);
        assert_eq!(handle.cancel(id), Err(QueueError::NotCancellable));
        assert_eq!(handle.retry(id), Err(QueueError::NotRetryable));
        queue.join().expect("join");
    }

    #[test]
    fn a_retryable_failure_retries_then_succeeds() {
        let policy = RetryPolicy::with_backoff(3, vec![Duration::from_millis(1)]);
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Retryable(
                "The request timed out.".to_owned(),
            ))],
            policy,
        );
        let id = handle
            .enqueue(job(uploader.clone(), "shot.png"))
            .expect("enqueue");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Done);
        assert_eq!(finished.attempts, 2);
        assert_eq!(uploader.calls(), vec!["shot.png", "shot.png"]);
        let state = handle.state(id).expect("state").expect("snapshot");
        assert_eq!(state.status, Status::Done);
        assert_eq!(state.attempts, 2);
        assert_eq!(state.link.as_deref(), Some("https://example.test/shot.png"));
        queue.join().expect("join");
    }

    #[test]
    fn non_retryable_errors_stop_at_the_first_attempt() {
        let policy = RetryPolicy::with_backoff(5, vec![Duration::from_millis(1)]);
        let (uploader, queue, events, handle) = queue(
            vec![
                Err(UploadError::TooLarge),
                Err(UploadError::Auth),
                Err(UploadError::Permanent("bad request".to_owned())),
            ],
            policy,
        );
        let first = handle
            .enqueue(job(uploader.clone(), "large.png"))
            .expect("first");
        let second = handle
            .enqueue(job(uploader.clone(), "auth.png"))
            .expect("second");
        let third = handle
            .enqueue(job(uploader.clone(), "permanent.png"))
            .expect("third");
        let finished = wait_all_terminal(&events, 3);
        assert_eq!(finished.len(), 3);
        assert_eq!(uploader.calls().len(), 3);
        for id in [first, second, third] {
            let state = handle.state(id).expect("state").expect("snapshot");
            assert_eq!(state.status, Status::Failed);
            assert_eq!(state.attempts, 1);
        }
        queue.join().expect("join");
    }

    #[test]
    fn the_backoff_schedule_matches_the_plan_and_caps() {
        assert_eq!(
            DEFAULT_BACKOFF,
            [
                Duration::from_secs(2),
                Duration::from_secs(8),
                Duration::from_secs(30),
                Duration::from_secs(120),
                Duration::from_secs(300),
            ]
        );
        let policy = RetryPolicy::new(5);
        assert_eq!(policy.delay(1), Duration::from_secs(2));
        assert_eq!(policy.delay(2), Duration::from_secs(8));
        assert_eq!(policy.delay(3), Duration::from_secs(30));
        assert_eq!(policy.delay(4), Duration::from_secs(120));
        assert_eq!(policy.delay(5), Duration::from_secs(300));
        assert_eq!(policy.delay(6), Duration::from_secs(300));
        assert_eq!(policy.delay(99), Duration::from_secs(300));
        assert_eq!(RetryPolicy::new(0).max_retries(), 0);
    }

    #[test]
    fn retry_after_overrides_the_backoff_schedule() {
        let error =
            UploadError::Retryable("The server returned HTTP 429, retry after 2 s.".to_owned());
        assert_eq!(retry_after(&error), Some(Duration::from_secs(2)));
        assert_eq!(
            RetryPolicy::new(3).delay_for(1, &error),
            Duration::from_secs(2)
        );
        assert_eq!(
            retry_after(&UploadError::Retryable("The request timed out.".to_owned())),
            None
        );
        let zero =
            UploadError::Retryable("The server returned HTTP 429, retry after 0 s.".to_owned());
        assert_eq!(RetryPolicy::new(3).delay_for(1, &zero), Duration::ZERO);
    }

    #[test]
    fn a_retry_after_retry_happens_without_the_standard_delay() {
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Retryable(
                "The server returned HTTP 429, retry after 0 s.".to_owned(),
            ))],
            RetryPolicy::new(3),
        );
        let id = handle
            .enqueue(job(uploader.clone(), "throttled.png"))
            .expect("enqueue");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Done);
        assert_eq!(finished.attempts, 2);
        queue.join().expect("join");
    }

    #[test]
    fn state_transitions_follow_queued_uploading_backing_off_done() {
        let policy = RetryPolicy::with_backoff(1, vec![Duration::from_millis(1)]);
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Retryable("try again".to_owned()))],
            policy,
        );
        let id = handle
            .enqueue(job(uploader.clone(), "shot.png"))
            .expect("enqueue");
        let mut seen = Vec::new();
        loop {
            let event = events.recv_timeout(Duration::from_secs(5)).expect("event");
            if event.0.id != id {
                continue;
            }
            let label = match event.0.status {
                Status::Queued => "queued",
                Status::Uploading => "uploading",
                Status::BackingOff { .. } => "backing-off",
                Status::Done => "done",
                Status::Failed => "failed",
                Status::Paused => "paused",
                Status::OutcomeUnknown => "outcome-unknown",
                Status::Cancelled => "cancelled",
            };
            seen.push(label);
            if event.0.status.terminal() {
                break;
            }
        }
        assert_eq!(
            seen,
            vec!["queued", "uploading", "backing-off", "uploading", "done"]
        );
        queue.join().expect("join");
    }

    #[test]
    fn a_backing_off_upload_does_not_block_the_next_one() {
        let slow = FakeUploader::new(vec![Err(UploadError::Retryable(
            "The server returned HTTP 429, retry after 3600 s.".to_owned(),
        ))]);
        let quick = FakeUploader::new(Vec::new());
        let (queue, events) =
            Queue::start(Arc::new(TestSecrets), RetryPolicy::new(1)).expect("queue");
        let handle = queue.handle();
        let first = handle.enqueue(job(slow, "slow.png")).expect("first");
        let second = handle.enqueue(job(quick, "quick.png")).expect("second");
        wait_for(
            &events,
            first,
            &Status::BackingOff {
                retry_in: Duration::from_secs(3600),
            },
        );
        let done = wait_for(&events, second, &Status::Done);
        assert_eq!(done.id, second);
        assert!(matches!(
            handle.state(first).expect("state").expect("first").status,
            Status::BackingOff { .. }
        ));
        handle.cancel(first).expect("cancel while backing off");
        assert_eq!(
            handle.state(first).expect("state").expect("first").status,
            Status::Cancelled
        );
        queue.join().expect("join");
    }

    #[test]
    fn only_the_newest_finished_uploads_are_kept() {
        let (_, queue, events, handle) = queue(Vec::new(), RetryPolicy::new(0));
        let mut ids = Vec::new();
        for index in 0..(MAX_COMPLETED + 5) {
            let uploader = FakeUploader::new(Vec::new());
            let name = format!("{index}.png");
            ids.push(handle.enqueue(job(uploader, &name)).expect("enqueue"));
        }
        let last = *ids.last().expect("last");
        wait_for(&events, last, &Status::Done);
        let listed = handle.list().expect("list");
        assert_eq!(listed.len(), MAX_COMPLETED);
        assert!(handle.state(ids[0]).expect("state").is_none());
        assert!(handle.state(last).expect("state").is_some());
        queue.join().expect("join");
    }

    #[test]
    fn a_queued_upload_can_be_cancelled() {
        let (gate, started, release) = GateUploader::new();
        let (queue, events) =
            Queue::start(Arc::new(TestSecrets), RetryPolicy::new(0)).expect("queue");
        let handle = queue.handle();
        let first = handle
            .enqueue(UploadJob::new(gate, file("first.png"), "test", None))
            .expect("first");
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("started");
        let second = handle
            .enqueue(job(FakeUploader::new(Vec::new()), "second.png"))
            .expect("second");
        let cancelling = handle.clone();
        let (result, completed) = mpsc::channel();
        let cancel = thread::spawn(move || {
            let outcome = cancelling.list().and_then(|states| {
                assert!(states
                    .iter()
                    .any(|state| state.id == first && state.status == Status::Uploading));
                cancelling.cancel(second)
            });
            let _ = result.send(outcome);
        });
        let while_uploading = completed.recv_timeout(Duration::from_secs(5));
        release.send(()).expect("release");
        cancel.join().expect("cancel thread");
        while_uploading
            .expect("queue responded during transfer")
            .expect("cancel");
        let finished = wait_all_terminal(&events, 2);
        assert_eq!(finished.len(), 2);
        assert_eq!(
            handle.state(first).expect("state").expect("first").status,
            Status::Done
        );
        assert_eq!(
            handle.state(second).expect("state").expect("second").status,
            Status::Cancelled
        );
        queue.join().expect("join");
    }

    #[test]
    fn a_backing_off_upload_can_be_cancelled() {
        let policy = RetryPolicy::with_backoff(3, vec![Duration::from_secs(30)]);
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Retryable("try again".to_owned()))],
            policy,
        );
        let id = handle
            .enqueue(job(uploader.clone(), "shot.png"))
            .expect("enqueue");
        wait_for(
            &events,
            id,
            &Status::BackingOff {
                retry_in: Duration::from_secs(30),
            },
        );
        handle.cancel(id).expect("cancel");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Cancelled);
        assert_eq!(uploader.calls(), vec!["shot.png"]);
        queue.join().expect("join");
    }

    #[test]
    fn finished_uploads_cannot_be_cancelled_and_failed_uploads_can_be_retried() {
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::TooLarge), Err(UploadError::TooLarge)],
            RetryPolicy::new(0),
        );
        let id = handle
            .enqueue(job(uploader.clone(), "large.png"))
            .expect("enqueue");
        wait_terminal(&events, id);
        assert_eq!(
            handle.cancel(id).expect_err("cannot cancel"),
            QueueError::NotCancellable
        );
        handle.retry(id).expect("retry");
        let retried = wait_terminal(&events, id);
        assert_eq!(retried.status, Status::Failed);
        assert_eq!(retried.attempts, 1);
        assert_eq!(uploader.calls().len(), 2);
        queue.join().expect("join");
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let (_uploader, queue, _events, handle) = queue(Vec::new(), RetryPolicy::new(0));
        assert_eq!(
            handle.cancel(UploadId::new(99)).expect_err("cancel"),
            QueueError::NotFound
        );
        assert_eq!(
            handle.retry(UploadId::new(99)).expect_err("retry"),
            QueueError::NotFound
        );
        assert!(handle.state(UploadId::new(99)).expect("state").is_none());
        queue.join().expect("join");
    }

    #[test]
    fn a_retry_resets_attempts_and_clears_the_error() {
        let (uploader, queue, events, handle) = queue(
            vec![
                Err(UploadError::Retryable("once".to_owned())),
                Err(UploadError::Retryable("twice".to_owned())),
                Err(UploadError::Retryable("three".to_owned())),
                Err(UploadError::Retryable("four".to_owned())),
            ],
            RetryPolicy::with_backoff(1, vec![Duration::from_millis(1)]),
        );
        let id = handle
            .enqueue(job(uploader.clone(), "shot.png"))
            .expect("enqueue");
        let failed = wait_terminal(&events, id);
        assert_eq!(failed.status, Status::Failed);
        assert_eq!(failed.attempts, 2);
        handle.retry(id).expect("retry");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Failed);
        assert_eq!(finished.attempts, 2);
        assert_eq!(uploader.calls().len(), 4);
        queue.join().expect("join");
    }

    struct SteppedUploader {
        started: Sender<()>,
        release: Mutex<Receiver<()>>,
        calls: AtomicU64,
    }

    impl SteppedUploader {
        fn new() -> (Arc<Self>, Receiver<()>, Sender<()>) {
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            (
                Arc::new(Self {
                    started: started_tx,
                    release: Mutex::new(release_rx),
                    calls: AtomicU64::new(0),
                }),
                started_rx,
                release_tx,
            )
        }
    }

    impl Uploader for SteppedUploader {
        fn upload(
            &self,
            file: &UploadFile,
            secrets: &dyn SecretStore,
        ) -> Result<Uploaded, UploadError> {
            self.upload_controlled(file, secrets, &NoopUploadControl)
        }

        fn supports_in_flight_cancel(&self) -> bool {
            true
        }

        fn reports_progress(&self) -> bool {
            true
        }

        fn upload_controlled(
            &self,
            file: &UploadFile,
            _secrets: &dyn SecretStore,
            control: &dyn UploadControl,
        ) -> Result<Uploaded, UploadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            for sent in 1..=file.len {
                control.sent(sent);
            }
            let _ = self.started.send(());
            let release = self.release.lock().expect("release");
            loop {
                if control.cancelled() {
                    return Err(UploadError::Cancelled);
                }
                match release.recv_timeout(Duration::from_millis(5)) {
                    Ok(()) => return Ok(Uploaded::new("https://example.test/stepped")),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(UploadError::permanent("released without a result"))
                    }
                }
            }
        }

        fn delete(
            &self,
            _link: &DeletionLink,
            _secrets: &dyn SecretStore,
        ) -> Result<(), UploadError> {
            Ok(())
        }
    }

    #[test]
    fn an_uploading_job_can_be_cancelled_when_the_uploader_supports_it() {
        let (uploader, started, _release) = SteppedUploader::new();
        let (queue, events) =
            Queue::start(Arc::new(TestSecrets), RetryPolicy::new(3)).expect("queue");
        let handle = queue.handle();
        let id = handle
            .enqueue(UploadJob::new(
                uploader.clone(),
                file("large.bin"),
                "test",
                None,
            ))
            .expect("enqueue");
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("started");
        let uploading = handle.state(id).expect("state").expect("snapshot");
        assert_eq!(uploading.status, Status::Uploading);
        assert!(uploading.cancellable);
        assert!(uploading.progress_available);
        handle.cancel(id).expect("cancel in flight");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Cancelled);
        assert_eq!(finished.error, None);
        assert!(!finished.cancellable);
        assert_eq!(uploader.calls.load(Ordering::SeqCst), 1);
        queue.join().expect("join");
    }

    #[test]
    fn an_uploading_job_cannot_be_cancelled_without_uploader_support() {
        let (gate, started, release) = GateUploader::new();
        let (queue, events) =
            Queue::start(Arc::new(TestSecrets), RetryPolicy::new(0)).expect("queue");
        let handle = queue.handle();
        let id = handle
            .enqueue(UploadJob::new(gate, file("fixed.png"), "test", None))
            .expect("enqueue");
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("started");
        let uploading = handle.state(id).expect("state").expect("snapshot");
        assert!(!uploading.cancellable);
        assert!(!uploading.progress_available);
        assert_eq!(
            handle.cancel(id).expect_err("not cancellable"),
            QueueError::NotCancellable
        );
        release.send(()).expect("release");
        assert_eq!(wait_terminal(&events, id).status, Status::Done);
        queue.join().expect("join");
    }

    #[test]
    fn progress_events_report_the_bytes_sent_and_finish_at_the_total() {
        let (uploader, started, release) = SteppedUploader::new();
        let (queue, events) =
            Queue::start(Arc::new(TestSecrets), RetryPolicy::new(0)).expect("queue");
        let handle = queue.handle();
        let id = handle
            .enqueue(UploadJob::new(uploader, file("steps.bin"), "test", None))
            .expect("enqueue");
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("started");
        release.send(()).expect("release");
        let mut sent = Vec::new();
        let finished = loop {
            let event = events.recv_timeout(Duration::from_secs(5)).expect("event");
            if event.0.id != id {
                continue;
            }
            if event.0.status == Status::Uploading {
                assert_eq!(event.0.total, 3);
                sent.push(event.0.sent);
            }
            if event.0.status.terminal() {
                break event.0;
            }
        };
        assert_eq!(sent, vec![0, 1, 2, 3]);
        assert_eq!(finished.status, Status::Done);
        assert_eq!((finished.sent, finished.total), (3, 3));
        queue.join().expect("join");
    }

    fn path_file(bytes: &[u8]) -> (tempfile::TempDir, PathBuf, UploadFile) {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("source.bin");
        std::fs::write(&path, bytes).expect("source file");
        let file = UploadFile::read(&path, "application/octet-stream").expect("upload file");
        (directory, path, file)
    }

    fn error_of<T>(result: Result<T, UploadError>) -> UploadError {
        match result {
            Ok(_) => panic!("the call succeeded"),
            Err(error) => error,
        }
    }

    const CHANGED: &str =
        "The file changed after it was added. Add it again to upload the new version.";
    const SHORTER: &str = "The file became shorter while it was being uploaded.";

    #[test]
    fn a_path_file_reads_back_its_bytes() {
        let (_directory, _path, file) = path_file(b"abcdef");
        assert_eq!(file.len, 6);
        assert_eq!(file.file_name, "source.bin");
        assert_eq!(file.read_all(6).expect("bytes"), b"abcdef");
        let mut streamed = Vec::new();
        file.open()
            .expect("open")
            .read_to_end(&mut streamed)
            .expect("stream");
        assert_eq!(streamed, b"abcdef");
    }

    #[test]
    fn a_file_with_a_new_length_is_refused() {
        let (_directory, path, file) = path_file(b"abcdef");
        std::fs::write(&path, b"abcdefgh").expect("rewrite");
        assert_eq!(error_of(file.open()), UploadError::permanent(CHANGED));
        assert_eq!(error_of(file.read_all(64)), UploadError::permanent(CHANGED));
    }

    #[test]
    fn a_file_with_a_new_modified_time_is_refused() {
        let (_directory, path, file) = path_file(b"abcdef");
        let handle = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("reopen");
        handle
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000_000))
            .expect("set modified");
        drop(handle);
        assert_eq!(std::fs::metadata(&path).expect("metadata").len(), file.len);
        assert_eq!(error_of(file.open()), UploadError::permanent(CHANGED));
    }

    #[test]
    fn a_removed_file_is_refused() {
        let (_directory, path, file) = path_file(b"abcdef");
        std::fs::remove_file(&path).expect("remove");
        assert_eq!(
            error_of(file.open()),
            UploadError::permanent(
                "The file is no longer available. Add it again before uploading."
            )
        );
    }

    #[test]
    fn a_file_over_the_buffer_limit_is_too_large() {
        let (_directory, _path, file) = path_file(b"abcdef");
        assert_eq!(error_of(file.read_all(5)), UploadError::TooLarge);
        assert_eq!(
            error_of(
                UploadFile::new("shot.png", "shot.png", "image/png", vec![0_u8; 6]).read_all(5)
            ),
            UploadError::TooLarge
        );
    }

    #[test]
    fn the_stream_never_reads_past_the_recorded_length() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("source.bin");
        std::fs::write(&path, b"abcdef").expect("source file");
        let mut reader = LengthLimitedReader {
            file: std::fs::File::open(&path).expect("open"),
            remaining: 4,
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).expect("read");
        assert_eq!(bytes, b"abcd");
    }

    #[test]
    fn a_file_that_ends_early_is_an_unexpected_end() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("source.bin");
        std::fs::write(&path, b"abc").expect("source file");
        let mut reader = LengthLimitedReader {
            file: std::fs::File::open(&path).expect("open"),
            remaining: 6,
        };
        let mut bytes = Vec::new();
        let error = reader.read_to_end(&mut bytes).expect_err("short file");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(error.to_string(), SHORTER);
    }

    struct Idempotent(Arc<FakeUploader>);

    impl Uploader for Idempotent {
        fn upload(
            &self,
            file: &UploadFile,
            secrets: &dyn SecretStore,
        ) -> Result<Uploaded, UploadError> {
            self.0.upload(file, secrets)
        }

        fn idempotent(&self) -> bool {
            true
        }

        fn delete(
            &self,
            link: &DeletionLink,
            secrets: &dyn SecretStore,
        ) -> Result<(), UploadError> {
            self.0.delete(link, secrets)
        }
    }

    #[test]
    fn an_uncertain_failure_is_never_retried_automatically() {
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Uncertain(
                "The request timed out.".to_owned(),
            ))],
            RetryPolicy::with_backoff(5, vec![Duration::ZERO]),
        );
        let id = handle
            .enqueue(job(uploader.clone(), "maybe.png"))
            .expect("enqueue");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::OutcomeUnknown);
        assert_eq!(finished.attempts, 1);
        assert_eq!(
            finished.error,
            Some(UploadError::Uncertain("The request timed out.".to_owned()))
        );
        assert!(!finished.cancellable);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(uploader.calls().len(), 1);
        queue.join().expect("join");
    }

    #[test]
    fn an_outcome_unknown_upload_retries_only_when_asked() {
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Uncertain(
                "The connection failed.".to_owned(),
            ))],
            RetryPolicy::new(3),
        );
        let id = handle
            .enqueue(job(uploader.clone(), "maybe.png"))
            .expect("enqueue");
        assert_eq!(wait_terminal(&events, id).status, Status::OutcomeUnknown);
        handle.retry(id).expect("explicit retry");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Done);
        assert_eq!(finished.attempts, 1);
        assert_eq!(uploader.calls().len(), 2);
        queue.join().expect("join");
    }

    #[test]
    fn an_idempotent_destination_retries_an_uncertain_failure() {
        let inner = FakeUploader::new(vec![Err(UploadError::Uncertain(
            "The request timed out.".to_owned(),
        ))]);
        let (queue, events) = Queue::start(
            Arc::new(TestSecrets),
            RetryPolicy::with_backoff(2, vec![Duration::ZERO]),
        )
        .expect("queue");
        let handle = queue.handle();
        let id = handle
            .enqueue(job(Arc::new(Idempotent(inner.clone())), "same-key.png"))
            .expect("enqueue");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Done);
        assert_eq!(finished.attempts, 2);
        assert_eq!(inner.calls().len(), 2);
        queue.join().expect("join");
    }

    #[test]
    fn an_idempotent_destination_still_respects_the_retry_limit() {
        let inner = FakeUploader::new(vec![
            Err(UploadError::Uncertain("one".to_owned())),
            Err(UploadError::Uncertain("two".to_owned())),
        ]);
        let (queue, events) = Queue::start(
            Arc::new(TestSecrets),
            RetryPolicy::with_backoff(1, vec![Duration::ZERO]),
        )
        .expect("queue");
        let handle = queue.handle();
        let id = handle
            .enqueue(job(Arc::new(Idempotent(inner.clone())), "same-key.png"))
            .expect("enqueue");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Failed);
        assert_eq!(finished.attempts, 2);
        queue.join().expect("join");
    }

    #[test]
    fn a_safe_retry_still_backs_off_and_retries() {
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Retryable(
                "The connection failed.".to_owned(),
            ))],
            RetryPolicy::with_backoff(2, vec![Duration::ZERO]),
        );
        let id = handle
            .enqueue(job(uploader.clone(), "safe.png"))
            .expect("enqueue");
        let finished = wait_terminal(&events, id);
        assert_eq!(finished.status, Status::Done);
        assert_eq!(finished.attempts, 2);
        queue.join().expect("join");
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Logged {
        Registered(String),
        Started(u64),
        Retrying(u64),
        Done(u64),
        Failed(u64),
        OutcomeUnknown(u64),
        Cancelled(u64),
        Paused(u64),
        Resumed(u64),
        Requeued(u64),
    }

    struct FakeJournal {
        log: Mutex<Vec<Logged>>,
        next: AtomicU64,
        deny_start: AtomicBool,
        fail_register_at: Mutex<Option<usize>>,
        registered: AtomicU64,
    }

    impl FakeJournal {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                log: Mutex::new(Vec::new()),
                next: AtomicU64::new(100),
                deny_start: AtomicBool::new(false),
                fail_register_at: Mutex::new(None),
                registered: AtomicU64::new(0),
            })
        }

        fn log(&self) -> Vec<Logged> {
            self.log.lock().expect("log").clone()
        }
    }

    impl Journal for FakeJournal {
        fn register(&self, job: &UploadJob) -> Result<Option<Registered>, JournalError> {
            let position = usize::try_from(self.registered.fetch_add(1, Ordering::SeqCst))
                .unwrap_or(usize::MAX);
            if *self.fail_register_at.lock().expect("fail") == Some(position) {
                return Err(JournalError);
            }
            if job.run_id.as_deref() == Some("ephemeral") {
                return Ok(None);
            }
            let id = self.next.fetch_add(1, Ordering::SeqCst);
            self.log
                .lock()
                .expect("log")
                .push(Logged::Registered(job.file.file_name.clone()));
            let staged = UploadFile::new(
                PathBuf::from("C:\\staged\\copy.png"),
                job.file.file_name.clone(),
                "image/png",
                vec![9_u8, 9],
            );
            Ok(Some(Registered {
                id: UploadId::new(id),
                file: Some(staged),
            }))
        }

        fn start(&self, id: UploadId) -> Result<bool, JournalError> {
            if self.deny_start.load(Ordering::SeqCst) {
                return Ok(false);
            }
            self.log
                .lock()
                .expect("log")
                .push(Logged::Started(id.get()));
            Ok(true)
        }

        fn record(&self, id: UploadId, change: &Change<'_>) -> Result<(), JournalError> {
            let id = id.get();
            let entry = match change {
                Change::Retrying { .. } => Logged::Retrying(id),
                Change::Done(_) => Logged::Done(id),
                Change::Failed(_) => Logged::Failed(id),
                Change::OutcomeUnknown(_) => Logged::OutcomeUnknown(id),
                Change::Cancelled => Logged::Cancelled(id),
                Change::Paused => Logged::Paused(id),
                Change::Resumed => Logged::Resumed(id),
                Change::Requeued => Logged::Requeued(id),
            };
            self.log.lock().expect("log").push(entry);
            Ok(())
        }
    }

    fn durable(
        results: Vec<Result<Uploaded, UploadError>>,
        policy: RetryPolicy,
    ) -> (
        Arc<FakeUploader>,
        Arc<FakeJournal>,
        Queue,
        Receiver<(UploadEvent, usize)>,
        Handle,
    ) {
        let uploader = FakeUploader::new(results);
        let journal = FakeJournal::new();
        let (sender, receiver) = mpsc::channel();
        let sink_journal = Arc::clone(&journal);
        let queue = Queue::start_durable(
            Arc::new(TestSecrets),
            policy,
            journal.clone(),
            move |event| {
                let _ = sender.send((event, sink_journal.log().len()));
            },
        )
        .expect("queue");
        let handle = queue.handle();
        (uploader, journal, queue, receiver, handle)
    }

    fn wait_logged(
        events: &Receiver<(UploadEvent, usize)>,
        id: UploadId,
        status: &Status,
    ) -> (UploadSnapshot, usize) {
        loop {
            let (event, logged) = events.recv_timeout(Duration::from_secs(5)).expect("event");
            if event.0.id == id && event.0.status == *status {
                return (event.0, logged);
            }
        }
    }

    #[test]
    fn a_durable_job_takes_its_id_and_staged_file_from_the_journal() {
        let (uploader, journal, queue, events, handle) = durable(Vec::new(), RetryPolicy::new(0));
        let id = handle
            .enqueue(job(uploader.clone(), "shot.png"))
            .expect("enqueue");
        assert_eq!(id, UploadId::new(100));
        wait_logged(&events, id, &Status::Done);
        assert_eq!(
            journal.log(),
            vec![
                Logged::Registered("shot.png".to_owned()),
                Logged::Started(100),
                Logged::Done(100)
            ]
        );
        queue.join().expect("join");
    }

    #[test]
    fn state_is_saved_before_the_matching_event_is_published() {
        let (uploader, _journal, queue, events, handle) = durable(
            vec![Err(UploadError::Retryable("again".to_owned()))],
            RetryPolicy::with_backoff(2, vec![Duration::ZERO]),
        );
        let id = handle
            .enqueue(job(uploader.clone(), "shot.png"))
            .expect("enqueue");
        let (_, retry_logged) = wait_logged(
            &events,
            id,
            &Status::BackingOff {
                retry_in: Duration::ZERO,
            },
        );
        assert_eq!(retry_logged, 3);
        let (_, done_logged) = wait_logged(&events, id, &Status::Done);
        assert_eq!(done_logged, 5);
        queue.join().expect("join");
    }

    #[test]
    fn a_job_the_journal_will_not_start_never_uploads() {
        let (uploader, journal, queue, events, handle) = durable(Vec::new(), RetryPolicy::new(0));
        journal.deny_start.store(true, Ordering::SeqCst);
        let id = handle
            .enqueue(job(uploader.clone(), "shot.png"))
            .expect("enqueue");
        wait_logged(&events, id, &Status::Cancelled);
        assert!(uploader.calls().is_empty());
        queue.join().expect("join");
    }

    #[test]
    fn jobs_the_journal_declines_use_ephemeral_ids() {
        let (uploader, journal, queue, events, handle) = durable(Vec::new(), RetryPolicy::new(0));
        let mut ephemeral = job(uploader.clone(), "test.png");
        ephemeral.run_id = Some("ephemeral".to_owned());
        let id = handle.enqueue(ephemeral).expect("enqueue");
        assert!(id.get() >= EPHEMERAL_ID_BASE);
        wait_logged(&events, id, &Status::Done);
        assert!(journal.log().is_empty());
        queue.join().expect("join");
    }

    #[test]
    fn a_registration_failure_refuses_the_job_and_cancels_earlier_batch_entries() {
        let (uploader, journal, queue, _events, handle) = durable(Vec::new(), RetryPolicy::new(0));
        *journal.fail_register_at.lock().expect("fail") = Some(1);
        let error = handle
            .enqueue_batch(vec![
                job(uploader.clone(), "a.png"),
                job(uploader.clone(), "b.png"),
            ])
            .expect_err("batch");
        assert_eq!(error, QueueError::Storage);
        assert_eq!(
            journal.log(),
            vec![
                Logged::Registered("a.png".to_owned()),
                Logged::Cancelled(100)
            ]
        );
        std::thread::sleep(Duration::from_millis(100));
        assert!(uploader.calls().is_empty());
        queue.join().expect("join");
    }

    #[test]
    fn a_paused_queue_starts_nothing_until_resumed() {
        let (uploader, queue, events, handle) = queue(Vec::new(), RetryPolicy::new(0));
        handle.set_paused(true).expect("pause");
        let id = handle
            .enqueue(job(uploader.clone(), "held.png"))
            .expect("enqueue");
        std::thread::sleep(Duration::from_millis(150));
        assert!(uploader.calls().is_empty());
        assert_eq!(
            handle.state(id).expect("state").map(|state| state.status),
            Some(Status::Queued)
        );
        handle.set_paused(false).expect("resume");
        assert_eq!(wait_terminal(&events, id).status, Status::Done);
        queue.join().expect("join");
    }

    #[test]
    fn a_paused_job_stays_put_while_others_run_and_resumes_on_request() {
        let (uploader, journal, queue, events, handle) = durable(Vec::new(), RetryPolicy::new(0));
        handle.set_paused(true).expect("hold the queue");
        let first = handle
            .enqueue(job(uploader.clone(), "first.png"))
            .expect("enqueue");
        let second = handle
            .enqueue(job(uploader.clone(), "second.png"))
            .expect("enqueue");
        handle.pause(first).expect("pause first");
        handle.set_paused(false).expect("release");
        wait_logged(&events, second, &Status::Done);
        assert_eq!(uploader.calls(), vec!["second.png".to_owned()]);
        assert_eq!(handle.pause(second), Err(QueueError::NotPausable));
        assert_eq!(handle.resume(second), Err(QueueError::NotPaused));
        handle.resume(first).expect("resume first");
        wait_logged(&events, first, &Status::Done);
        assert!(journal.log().contains(&Logged::Paused(100)));
        assert!(journal.log().contains(&Logged::Resumed(100)));
        queue.join().expect("join");
    }

    #[test]
    fn a_paused_job_can_be_cancelled_and_never_uploads() {
        let (uploader, journal, queue, events, handle) = durable(Vec::new(), RetryPolicy::new(0));
        handle.set_paused(true).expect("hold");
        let id = handle
            .enqueue(job(uploader.clone(), "p.png"))
            .expect("enqueue");
        handle.pause(id).expect("pause");
        handle.cancel(id).expect("cancel");
        handle.set_paused(false).expect("release");
        wait_logged(&events, id, &Status::Cancelled);
        std::thread::sleep(Duration::from_millis(100));
        assert!(uploader.calls().is_empty());
        assert!(journal.log().contains(&Logged::Cancelled(100)));
        queue.join().expect("join");
    }

    #[test]
    fn restored_jobs_resume_where_they_left_off() {
        let (uploader, queue, events, handle) = queue(Vec::new(), RetryPolicy::new(3));
        let make = |name: &str| job(uploader.clone(), name);
        handle
            .restore(vec![
                RestoredJob {
                    id: UploadId::new(7),
                    job: make("queued.png"),
                    attempts: 1,
                    start: RestoreStart::Queued,
                },
                RestoredJob {
                    id: UploadId::new(8),
                    job: make("waiting.png"),
                    attempts: 2,
                    start: RestoreStart::After(Duration::from_millis(80)),
                },
                RestoredJob {
                    id: UploadId::new(9),
                    job: make("paused.png"),
                    attempts: 0,
                    start: RestoreStart::Paused { retry_in: None },
                },
            ])
            .expect("restore");
        let queued = wait_terminal(&events, UploadId::new(7));
        assert_eq!(queued.status, Status::Done);
        assert_eq!(queued.attempts, 2);
        let waiting = wait_terminal(&events, UploadId::new(8));
        assert_eq!(waiting.status, Status::Done);
        assert_eq!(waiting.attempts, 3);
        assert_eq!(
            handle
                .state(UploadId::new(9))
                .expect("state")
                .map(|state| state.status),
            Some(Status::Paused)
        );
        assert_eq!(
            uploader.calls(),
            vec!["queued.png".to_owned(), "waiting.png".to_owned()]
        );
        queue.join().expect("join");
    }

    #[test]
    fn a_restored_stopped_job_keeps_its_status_and_can_be_retried_or_cancelled() {
        let (uploader, queue, events, handle) = queue(Vec::new(), RetryPolicy::new(0));
        handle
            .restore(vec![
                RestoredJob {
                    id: UploadId::new(3),
                    job: job(uploader.clone(), "maybe.png"),
                    attempts: 1,
                    start: RestoreStart::Stopped {
                        status: Status::OutcomeUnknown,
                        error: Some(UploadError::Uncertain("It may have uploaded.".to_owned())),
                    },
                },
                RestoredJob {
                    id: UploadId::new(4),
                    job: job(uploader.clone(), "attention.png"),
                    attempts: 1,
                    start: RestoreStart::Stopped {
                        status: Status::Failed,
                        error: Some(UploadError::Auth),
                    },
                },
            ])
            .expect("restore");
        std::thread::sleep(Duration::from_millis(100));
        assert!(uploader.calls().is_empty());
        let maybe = handle.state(UploadId::new(3)).expect("state").expect("job");
        assert_eq!(maybe.status, Status::OutcomeUnknown);
        assert_eq!(
            maybe.error,
            Some(UploadError::Uncertain("It may have uploaded.".to_owned()))
        );
        handle.retry(UploadId::new(3)).expect("retry");
        wait_for(&events, UploadId::new(3), &Status::Done);
        assert_eq!(
            handle.cancel(UploadId::new(4)),
            Err(QueueError::NotCancellable)
        );
        queue.join().expect("join");
    }

    #[test]
    fn a_restored_job_still_obeys_the_retry_limit() {
        let (uploader, queue, events, handle) = queue(
            vec![Err(UploadError::Retryable("again".to_owned()))],
            RetryPolicy::with_backoff(2, vec![Duration::ZERO]),
        );
        handle
            .restore(vec![RestoredJob {
                id: UploadId::new(5),
                job: job(uploader.clone(), "limit.png"),
                attempts: 2,
                start: RestoreStart::Queued,
            }])
            .expect("restore");
        let finished = wait_terminal(&events, UploadId::new(5));
        assert_eq!(finished.status, Status::Failed);
        assert_eq!(finished.attempts, 3);
        queue.join().expect("join");
    }
}
