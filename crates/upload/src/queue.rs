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
    Done,
    Failed,
    Cancelled,
}

impl Status {
    pub fn terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
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
    let UploadError::Retryable(message) = error else {
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
    #[error("Only failed or cancelled uploads can be retried.")]
    NotRetryable,
}

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
    cancellations: Arc<Mutex<HashMap<UploadId, ActiveCancellation>>>,
}

#[derive(Clone)]
struct ActiveCancellation {
    cancelled: Arc<AtomicBool>,
    cancellable: bool,
}

impl Handle {
    pub fn enqueue(&self, job: UploadJob) -> Result<UploadId, QueueError> {
        let id = UploadId(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.send(Command::Enqueue(vec![(id, job)]))?;
        Ok(id)
    }

    pub fn enqueue_batch(&self, jobs: Vec<UploadJob>) -> Result<Vec<UploadId>, QueueError> {
        let entries: Vec<_> = jobs
            .into_iter()
            .map(|job| (UploadId(self.next_id.fetch_add(1, Ordering::Relaxed)), job))
            .collect();
        let ids = entries.iter().map(|(id, _)| *id).collect();
        self.send(Command::Enqueue(entries))?;
        Ok(ids)
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
        Ok((Self::spawn(secrets, policy, listener)?, receiver))
    }

    pub fn start_with_sink<F>(
        secrets: Arc<dyn SecretStore>,
        policy: RetryPolicy,
        sink: F,
    ) -> Result<Self, QueueError>
    where
        F: Fn(UploadEvent) + Send + Sync + 'static,
    {
        Self::spawn(secrets, policy, Listener::Callback(Arc::new(sink)))
    }

    fn spawn(
        secrets: Arc<dyn SecretStore>,
        policy: RetryPolicy,
        listener: Listener,
    ) -> Result<Self, QueueError> {
        let (commands, receiver) = mpsc::channel();
        let handle = Handle {
            commands: Arc::new(Mutex::new(commands)),
            next_id: Arc::new(AtomicU64::new(1)),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        };
        let cancellations = Arc::clone(&handle.cancellations);
        let thread = thread::Builder::new()
            .name("snapline-upload".to_owned())
            .spawn(move || run(receiver, secrets, policy, listener, cancellations))
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
    link: Option<String>,
    page_link: Option<String>,
    deletion_link: Option<String>,
    error: Option<UploadError>,
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
            link: None,
            page_link: None,
            deletion_link: None,
            error: None,
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
                Status::Queued | Status::BackingOff { .. } => true,
                Status::Uploading => self.job.uploader.supports_in_flight_cancel(),
                Status::Done | Status::Failed | Status::Cancelled => false,
            },
            progress_available: self.job.uploader.reports_progress(),
            sent,
            total,
            link_batch: self.job.link_batch.clone(),
            attempts: self.attempts,
            link: self.link.clone(),
            page_link: self.page_link.clone(),
            deletion_link: self.deletion_link.clone(),
            error: self.error.clone(),
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
    Cancelled(UploadId),
}

enum Wait {
    Elapsed,
    Cancelled,
    Shutdown,
}

struct State {
    pending: VecDeque<UploadId>,
    records: HashMap<UploadId, JobRecord>,
    listener: Listener,
}

impl State {
    fn new(listener: Listener) -> Self {
        Self {
            pending: VecDeque::new(),
            records: HashMap::new(),
            listener,
        }
    }

    fn emit(&self, id: UploadId) {
        if let Some(record) = self.records.get(&id) {
            self.listener.emit(UploadEvent(record.snapshot()));
        }
    }

    fn handle(&mut self, command: Command, current: Option<UploadId>) -> Control {
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
                let status = self.records.get(&id).map(|record| record.status.clone());
                match status {
                    None => {
                        let _ = reply.send(Err(QueueError::NotFound));
                        Control::Continue
                    }
                    Some(Status::BackingOff { .. }) if current == Some(id) => {
                        if let Some(record) = self.records.get_mut(&id) {
                            record.status = Status::Cancelled;
                        }
                        let _ = reply.send(Ok(()));
                        self.emit(id);
                        Control::Cancelled(id)
                    }
                    Some(Status::Queued) => {
                        if let Some(position) = self.pending.iter().position(|item| *item == id) {
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
                    Some(Status::Failed | Status::Cancelled)
                );
                if retryable {
                    if let Some(record) = self.records.get_mut(&id) {
                        record.status = Status::Queued;
                        record.attempts = 0;
                        record.link = None;
                        record.page_link = None;
                        record.deletion_link = None;
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
                } else if self.records.contains_key(&id) {
                    let _ = reply.send(Err(QueueError::NotRetryable));
                } else {
                    let _ = reply.send(Err(QueueError::NotFound));
                }
                Control::Continue
            }
            Command::State { id, reply } => {
                let snapshot = self.records.get(&id).map(JobRecord::snapshot);
                let _ = reply.send(snapshot);
                Control::Continue
            }
            Command::List { reply } => {
                let mut snapshots: Vec<UploadSnapshot> =
                    self.records.values().map(JobRecord::snapshot).collect();
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
) {
    let mut state = State::new(listener);
    loop {
        while let Ok(command) = commands.try_recv() {
            if matches!(state.handle(command, None), Control::Shutdown) {
                return;
            }
        }
        let Some(id) = state.pending.pop_front() else {
            match commands.recv() {
                Ok(command) => {
                    if matches!(state.handle(command, None), Control::Shutdown) {
                        return;
                    }
                }
                Err(_) => return,
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
    loop {
        let Some(record) = state.records.get_mut(&id) else {
            return true;
        };
        record.attempts += 1;
        record.status = Status::Uploading;
        record.error = None;
        let uploader = Arc::clone(&record.job.uploader);
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
            progress: Arc::clone(&progress),
            listener: state.listener.clone(),
            snapshot,
        };
        state.emit(id);
        let result = thread::scope(|scope| {
            let attempt = match thread::Builder::new()
                .name("snapline-upload-transfer".to_owned())
                .spawn_scoped(scope, || {
                    uploader.upload_controlled(&file, secrets, &control)
                }) {
                Ok(attempt) => attempt,
                Err(_) => return Some(Err(UploadError::permanent("The upload could not start."))),
            };
            while !attempt.is_finished() {
                match commands.recv_timeout(Duration::from_millis(20)) {
                    Ok(command) => {
                        if !matches!(state.handle(command, None), Control::Shutdown) {
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
                if let Some(record) = state.records.get_mut(&id) {
                    record.status = Status::Done;
                    if let Ok(mut progress) = record.progress.lock() {
                        progress.sent = progress.total;
                    }
                    if matches!(&record.job.file.body, UploadBody::Bytes(_)) {
                        record.job.file.body = UploadBody::Bytes(Arc::new(Vec::new()));
                    }
                    record.link = Some(uploaded.link);
                    record.page_link = uploaded.page_link;
                    record.deletion_link = uploaded.deletion_link.map(|link| link.url);
                    record.error = None;
                }
                state.emit(id);
                return true;
            }
            Err(error) => {
                if error == UploadError::Cancelled || cancelled.load(Ordering::SeqCst) {
                    if let Some(record) = state.records.get_mut(&id) {
                        record.status = Status::Cancelled;
                        record.error = None;
                    }
                    state.emit(id);
                    return true;
                }
                let retryable = error.is_retryable() && attempts <= policy.max_retries();
                if !retryable {
                    if let Some(record) = state.records.get_mut(&id) {
                        record.status = Status::Failed;
                        record.error = Some(error);
                    }
                    state.emit(id);
                    return true;
                }
                let delay = policy.delay_for(attempts, &error);
                if let Some(record) = state.records.get_mut(&id) {
                    record.status = Status::BackingOff { retry_in: delay };
                    record.error = Some(error);
                }
                state.emit(id);
                match wait(commands, delay, id, state) {
                    Wait::Elapsed => continue,
                    Wait::Cancelled => return true,
                    Wait::Shutdown => return false,
                }
            }
        }
    }
}

fn wait(
    commands: &Receiver<Command>,
    delay: Duration,
    current: UploadId,
    state: &mut State,
) -> Wait {
    let deadline = Instant::now() + delay;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match commands.recv_timeout(remaining) {
            Ok(command) => match state.handle(command, Some(current)) {
                Control::Continue => continue,
                Control::Cancelled(id) if id == current => return Wait::Cancelled,
                Control::Cancelled(_) => continue,
                Control::Shutdown => return Wait::Shutdown,
            },
            Err(RecvTimeoutError::Timeout) => return Wait::Elapsed,
            Err(RecvTimeoutError::Disconnected) => return Wait::Shutdown,
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
}
