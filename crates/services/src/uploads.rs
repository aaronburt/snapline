use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver};
use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_core::media::MediaKind;
use snapline_core::ports::SecretStore as CoreSecretStore;
use snapline_core::secret::SecretKey;
use snapline_store::history::HistoryRepo;
use snapline_store::history::{UploadState as HistoryUploadState, UploadUpdate};
use snapline_store::settings::uploads::Retry;
use snapline_store::settings::uploads::{DestinationRef, Uploads};
use snapline_upload::queue;
use ts_rs::TS;

use crate::destinations::DestinationOperations;
use crate::dto::upload::{
    DestinationReq, DestinationResp, DestinationTestOutcomeDto, DestinationTestStatusDto,
    UploadLinkBatch, UploadReq, UploadResp, UploadState, UploadStatus,
};
use crate::{AppEvent, ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "UploadAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
    StateChanged(UploadState),
}

pub const TEST_RUN_ID: &str = "snapline-destination-test";

pub const UPLOAD_COMPLETE_TITLE: &str = "Upload complete";

pub struct LocalCapture {
    pub path: std::path::PathBuf,
    pub thumbnail: Option<std::path::PathBuf>,
}

pub fn upload_notice(
    state: &UploadState,
    local: Option<&LocalCapture>,
) -> Option<snapline_core::ports::RichNotice> {
    use crate::saver::file_url;
    use snapline_core::ports::{NoticeButton, RichNotice};

    if !notifies_for(&state.file_name) {
        return None;
    }
    let link = state.link.as_deref().filter(|link| !link.is_empty());
    let folder = local.and_then(|local| local.path.parent());
    let mut buttons = Vec::new();
    let mut lines = vec![state.file_name.clone()];
    let open = if let Some(link) = link {
        lines.push(link.to_owned());
        buttons.push(NoticeButton {
            label: "Open link".to_owned(),
            target: link.to_owned(),
        });
        link.to_owned()
    } else {
        let folder = folder?;
        lines.push(folder.display().to_string());
        file_url(folder)
    };
    if let Some(folder) = folder {
        buttons.push(NoticeButton {
            label: "Open folder".to_owned(),
            target: file_url(folder),
        });
    }
    Some(RichNotice {
        title: UPLOAD_COMPLETE_TITLE.to_owned(),
        lines,
        image: local.and_then(|local| local.thumbnail.clone()),
        open: Some(open),
        buttons,
        tag: state.capture_id.map(crate::saver::capture_tag),
    })
}

fn notifies_for(file_name: &str) -> bool {
    matches!(
        snapline_core::media::classify(file_name, &[]),
        MediaKind::Image | MediaKind::Video | MediaKind::Gif
    )
}

fn open_label(file_name: &str) -> &'static str {
    match snapline_core::media::classify(file_name, &[]) {
        MediaKind::Video | MediaKind::Gif => "Open video",
        _ => "Open image",
    }
}

pub const UPLOAD_FAILED_TITLE: &str = "Upload failed";

pub fn upload_failed_notice(
    state: &UploadState,
    local: Option<&LocalCapture>,
) -> Option<snapline_core::ports::RichNotice> {
    use crate::saver::file_url;
    use snapline_core::ports::{NoticeButton, RichNotice};

    if state.status != UploadStatus::Failed || !notifies_for(&state.file_name) {
        return None;
    }
    let reason = state
        .error
        .clone()
        .filter(|error| !error.is_empty())
        .unwrap_or_else(|| "The upload did not finish.".to_owned());
    let mut buttons = Vec::new();
    let mut open = None;
    if let Some(local) = local {
        let image_url = file_url(&local.path);
        buttons.push(NoticeButton {
            label: open_label(&state.file_name).to_owned(),
            target: image_url.clone(),
        });
        if let Some(folder) = local.path.parent() {
            buttons.push(NoticeButton {
                label: "Open folder".to_owned(),
                target: file_url(folder),
            });
        }
        open = Some(image_url);
    }
    Some(RichNotice {
        title: UPLOAD_FAILED_TITLE.to_owned(),
        lines: vec![state.file_name.clone(), reason],
        image: local.and_then(|local| local.thumbnail.clone()),
        open,
        buttons,
        tag: state.capture_id.map(crate::saver::capture_tag),
    })
}

fn notify_uploaded(services: &Services, state: &UploadState) {
    let Some(notifier) = services.notifier() else {
        return;
    };
    let local = state.capture_id.and_then(|id| local_capture(services, id));
    let notice = if state.status == UploadStatus::Failed {
        upload_failed_notice(state, local.as_ref())
    } else {
        upload_notice(state, local.as_ref())
    };
    if let Some(notice) = notice {
        notifier.toast_rich(notice);
    }
}

fn local_capture(services: &Services, id: i64) -> Option<LocalCapture> {
    let paths = services.paths().ok()?;
    let repo = HistoryRepo::open_read(&paths.history_file).ok()?;
    let capture = repo.get(id).ok()??;
    let thumbnail = crate::thumbs::existing_thumbnail(&paths.thumbs_dir, id).map(|(path, _)| path);
    Some(LocalCapture {
        path: std::path::PathBuf::from(capture.path),
        thumbnail,
    })
}

pub fn missing_destination_message(media: MediaKind) -> String {
    let label = match media {
        MediaKind::Image => "images",
        MediaKind::Video | MediaKind::Gif => "videos and GIFs",
        MediaKind::Text => "text and code",
        MediaKind::File => "other files",
    };
    format!("Choose a destination for {label} in Settings, under Upload destinations.")
}

pub fn destination_for(uploads: &Uploads, media: MediaKind) -> &DestinationRef {
    match media {
        MediaKind::Image => &uploads.images,
        MediaKind::Video | MediaKind::Gif => &uploads.videos_and_gifs,
        MediaKind::Text => &uploads.text_and_code,
        MediaKind::File => &uploads.other_files,
    }
}

pub fn last_link(services: &Services) -> Result<Option<String>, ServiceError> {
    let path = services.paths()?.history_file;
    if !path.is_file() {
        return Ok(None);
    }
    HistoryRepo::open_read(&path)
        .and_then(|repo| repo.latest_upload_url())
        .map_err(|error| ServiceError::Io(error.to_string()))
}

#[cfg(test)]
pub(crate) fn seed_done_upload(services: &Services, name: &str, url: &str, updated_at: i64) -> i64 {
    use snapline_store::history::{CaptureKind, NewCapture};

    let path = services.paths().expect("paths").history_file;
    let mut repo = HistoryRepo::open_write(&path).expect("history");
    let id = repo
        .record(NewCapture {
            kind: CaptureKind::Image,
            path: format!("C:\\captures\\{name}"),
            bytes: 3,
            width: None,
            height: None,
            duration_ms: None,
            source_app: None,
            created_at: updated_at,
            ocr_text: None,
        })
        .expect("capture");
    repo.set_upload(UploadUpdate {
        sent: 0,
        total: 0,
        capture_id: id,
        dest_id: "test".to_owned(),
        state: HistoryUploadState::Done,
        url: Some(url.to_owned()),
        deletion_url: None,
        error: None,
        attempts: 1,
        updated_at,
    })
    .expect("upload");
    id
}

pub fn maintain_history(services: &Services, startup: bool) -> Result<(), ServiceError> {
    use snapline_store::settings::uploads::KeepHistory;

    let settings = services.settings_snapshot()?;
    let days = match settings.uploads.keep_history {
        KeepHistory::Forever => None,
        KeepHistory::OneYear => Some(365),
        KeepHistory::Days90 => Some(90),
        KeepHistory::Days30 => Some(30),
    };
    let path = services.paths()?.history_file;
    if !path.is_file() || (!startup && days.is_none()) {
        return Ok(());
    }
    let repo =
        HistoryRepo::open_write(&path).map_err(|error| ServiceError::Io(error.to_string()))?;
    if startup {
        repo.mark_uploads_interrupted()
            .map_err(|error| ServiceError::Io(error.to_string()))?;
    }
    if let Some(days) = days {
        let cutoff = time::OffsetDateTime::now_utc().unix_timestamp() * 1000 - days * 86_400_000;
        repo.expire_uploads(cutoff)
            .map_err(|error| ServiceError::Io(error.to_string()))?;
    }
    Ok(())
}

pub fn persist(services: &Services, state: &UploadState) {
    let Some(capture_id) = state.capture_id else {
        return;
    };
    if state.run_id.as_deref() == Some(TEST_RUN_ID) {
        return;
    }
    let paths = match services.paths() {
        Ok(paths) => paths,
        Err(error) => {
            tracing::warn!(%error, "the upload state could not be saved");
            return;
        }
    };
    let mut repo = match HistoryRepo::open_write(&paths.history_file) {
        Ok(repo) => repo,
        Err(error) => {
            tracing::warn!(%error, "the upload state could not be saved");
            return;
        }
    };
    let updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default();
    let update = UploadUpdate {
        sent: i64::try_from(state.sent).unwrap_or(i64::MAX),
        total: i64::try_from(state.total).unwrap_or(i64::MAX),
        capture_id,
        dest_id: state.dest.clone(),
        state: match state.status {
            UploadStatus::Queued => HistoryUploadState::Queued,
            UploadStatus::Uploading | UploadStatus::BackingOff => HistoryUploadState::Uploading,
            UploadStatus::Done => HistoryUploadState::Done,
            UploadStatus::Failed => HistoryUploadState::Failed,
            UploadStatus::Cancelled => HistoryUploadState::Cancelled,
        },
        url: state.link.clone(),
        deletion_url: state.deletion_link.clone(),
        error: state.error.clone(),
        attempts: i64::from(state.attempts),
        updated_at,
    };
    if let Err(error) = repo.set_upload(update) {
        tracing::warn!(%error, "the upload state could not be saved");
    }
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
    Persist(UploadState),
}

pub trait FileSource: Send + Sync {
    fn file(&self, capture_id: i64) -> Result<Option<queue::UploadFile>, ServiceError>;
}

pub trait DestinationSource: Send + Sync {
    fn uploader(&self, dest: &str) -> Option<Arc<dyn queue::Uploader>>;

    fn streams_files(&self, _dest: &str) -> bool {
        false
    }

    fn operations(&self) -> Option<Arc<dyn DestinationOperations>> {
        None
    }
}

#[derive(Clone)]
pub struct UploadRuntime {
    pub(crate) queue: queue::Handle,
    pub(crate) files: Arc<dyn FileSource>,
    pub(crate) destinations: Arc<dyn DestinationSource>,
}

#[derive(Clone)]
pub struct UploadService {
    services: Services,
    runtime: Option<UploadRuntime>,
}

impl UploadService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
            runtime: None,
        }
    }

    pub fn with_runtime(services: &Services, runtime: UploadRuntime) -> Self {
        Self {
            services: services.clone(),
            runtime: Some(runtime),
        }
    }

    pub fn handle(&self, req: UploadReq) -> Result<UploadResp, ServiceError> {
        match req {
            UploadReq::Ping => Ok(UploadResp::Pong),
            UploadReq::State { id } => {
                let snapshot = self
                    .runtime()?
                    .queue
                    .state(queue::UploadId::new(id))
                    .map_err(queue_error)?;
                snapshot
                    .map(|snapshot| UploadResp::State(UploadState::from(&snapshot)))
                    .ok_or(ServiceError::NotFound)
            }
            UploadReq::Cancel { id } => {
                self.runtime()?
                    .queue
                    .cancel(queue::UploadId::new(id))
                    .map_err(queue_error)?;
                Ok(UploadResp::Cancelled { id })
            }
            UploadReq::Retry { id } => {
                self.runtime()?
                    .queue
                    .retry(queue::UploadId::new(id))
                    .map_err(queue_error)?;
                Ok(UploadResp::Retried { id })
            }
            UploadReq::List => {
                let snapshots = self.runtime()?.queue.list().map_err(queue_error)?;
                Ok(UploadResp::States(
                    snapshots.iter().map(UploadState::from).collect(),
                ))
            }
            UploadReq::LastLink => Ok(UploadResp::LastLink {
                link: last_link(&self.services)?,
            }),
            UploadReq::PickAndUpload => crate::intake::Intake::new(&self.services, self.runtime()?)
                .pick_and_upload()
                .map(|queued| UploadResp::AcceptedMany {
                    ids: queued.into_iter().map(|item| item.upload_id).collect(),
                }),
            UploadReq::CopyLink { .. } => Err(ServiceError::NotYet),
            UploadReq::UploadElsewhere { capture_id, dest } => {
                if capture_id <= 0 {
                    return Err(ServiceError::Invalid(
                        "The capture was not valid.".to_owned(),
                    ));
                }
                if dest.trim().is_empty() {
                    return Err(ServiceError::Invalid(
                        "Choose an upload destination.".to_owned(),
                    ));
                }
                let runtime = self.runtime()?;
                let file = runtime
                    .files
                    .file(capture_id)?
                    .ok_or(ServiceError::NotFound)?;
                let uploader = runtime.destinations.uploader(&dest).ok_or_else(|| {
                    ServiceError::Invalid("The upload destination was not found.".to_owned())
                })?;
                let id = runtime
                    .queue
                    .enqueue(
                        queue::UploadJob::new(uploader, file, dest, None)
                            .with_capture_id(capture_id),
                    )
                    .map_err(queue_error)?;
                Ok(UploadResp::Accepted { id: id.get() })
            }
            UploadReq::Destination(req) => self.handle_destination(req),
        }
    }

    fn handle_destination(&self, req: DestinationReq) -> Result<UploadResp, ServiceError> {
        let runtime = self.runtime()?;
        let operations = runtime
            .destinations
            .operations()
            .ok_or(ServiceError::NotYet)?;
        let response = match req {
            DestinationReq::List => DestinationResp::Destinations(operations.list()?),
            DestinationReq::Add { kind } => DestinationResp::Destination(operations.add(kind)?),
            DestinationReq::Save { input } => DestinationResp::Destination(operations.save(input)?),
            DestinationReq::Remove { id } => {
                operations.remove(&id)?;
                DestinationResp::Removed { id }
            }
            DestinationReq::ImportSxcu => {
                let (destination, warnings) = operations.import_sxcu()?;
                DestinationResp::Imported {
                    destination,
                    warnings,
                }
            }
            DestinationReq::ExportSxcu {
                id,
                include_header_values,
            } => DestinationResp::Exported {
                path: operations.export_sxcu(&id, include_header_values)?,
            },
            DestinationReq::Test { id } => {
                let (uploader, file) = operations.test_uploader(&id)?;
                let upload_id = runtime
                    .queue
                    .enqueue(queue::UploadJob::new(
                        uploader,
                        file,
                        id.clone(),
                        Some(TEST_RUN_ID.to_owned()),
                    ))
                    .map_err(queue_error)?;
                let deadline = Instant::now() + Duration::from_secs(120);
                loop {
                    let snapshot = runtime.queue.state(upload_id).map_err(queue_error)?;
                    if let Some(snapshot) = snapshot {
                        if snapshot.status.terminal() {
                            let outcome = match snapshot.status {
                                queue::Status::Done => DestinationTestOutcomeDto {
                                    status: DestinationTestStatusDto::Succeeded,
                                    link: snapshot.link.clone(),
                                    message: None,
                                },
                                queue::Status::Failed => DestinationTestOutcomeDto {
                                    status: DestinationTestStatusDto::Failed,
                                    link: None,
                                    message: Some(
                                        snapshot
                                            .error
                                            .map(|error| error.to_string())
                                            .unwrap_or_else(|| {
                                                "The test upload failed.".to_owned()
                                            }),
                                    ),
                                },
                                queue::Status::Cancelled => DestinationTestOutcomeDto {
                                    status: DestinationTestStatusDto::Failed,
                                    link: None,
                                    message: Some("The test upload was cancelled.".to_owned()),
                                },
                                _ => DestinationTestOutcomeDto {
                                    status: DestinationTestStatusDto::NotTested,
                                    link: None,
                                    message: None,
                                },
                            };
                            return Ok(UploadResp::Destination(DestinationResp::Test { outcome }));
                        }
                    }
                    if Instant::now() >= deadline {
                        let _ = runtime.queue.cancel(upload_id);
                        return Ok(UploadResp::Destination(DestinationResp::Test {
                            outcome: DestinationTestOutcomeDto {
                                status: DestinationTestStatusDto::Failed,
                                link: None,
                                message: Some("The test upload timed out.".to_owned()),
                            },
                        }));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };
        Ok(UploadResp::Destination(response))
    }

    fn runtime(&self) -> Result<&UploadRuntime, ServiceError> {
        self.runtime.as_ref().ok_or(ServiceError::NotYet)
    }
}

pub struct UploaderThread {
    queue: queue::Queue,
    events: Option<Receiver<AppEvent>>,
    service: UploadService,
    runtime: UploadRuntime,
}

impl UploaderThread {
    pub fn start(
        services: &Services,
        secrets: Arc<dyn CoreSecretStore + Send + Sync>,
        policy: queue::RetryPolicy,
        files: Arc<dyn FileSource>,
        destinations: Arc<dyn DestinationSource>,
    ) -> Result<Self, ServiceError> {
        let (events, receiver) = unbounded();
        let mut thread = Self::spawn(services, secrets, policy, files, destinations, events)?;
        thread.events = Some(receiver);
        Ok(thread)
    }

    fn spawn(
        services: &Services,
        secrets: Arc<dyn CoreSecretStore + Send + Sync>,
        policy: queue::RetryPolicy,
        files: Arc<dyn FileSource>,
        destinations: Arc<dyn DestinationSource>,
        events: crossbeam_channel::Sender<AppEvent>,
    ) -> Result<Self, ServiceError> {
        let sink_services = services.clone();
        let sink = move |event: queue::UploadEvent| {
            let state = UploadState::from(&event.0);
            if state.status == UploadStatus::Failed && state.run_id.as_deref() != Some(TEST_RUN_ID)
            {
                notify_uploaded(&sink_services, &state);
            }
            if state.status == UploadStatus::Done && state.run_id.as_deref() != Some(TEST_RUN_ID) {
                notify_uploaded(&sink_services, &state);
            }
            let _ = events.send(AppEvent::Upload(Event::StateChanged(state)));
        };
        let queue = queue::Queue::start_with_sink(Arc::new(CoreSecrets(secrets)), policy, sink)
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        let runtime = UploadRuntime {
            queue: queue.handle(),
            files,
            destinations,
        };
        let service = UploadService::with_runtime(services, runtime.clone());
        Ok(Self {
            queue,
            events: None,
            service,
            runtime,
        })
    }

    pub fn service(&self) -> UploadService {
        self.service.clone()
    }

    pub fn runtime(&self) -> UploadRuntime {
        self.runtime.clone()
    }

    pub fn handle(&self) -> queue::Handle {
        self.queue.handle()
    }

    pub fn events(&self) -> Option<Receiver<AppEvent>> {
        self.events.clone()
    }

    pub fn enqueue(
        &self,
        uploader: Arc<dyn queue::Uploader>,
        file: queue::UploadFile,
        dest: impl Into<String>,
        run_id: Option<String>,
    ) -> Result<u64, ServiceError> {
        self.queue
            .handle()
            .enqueue(queue::UploadJob::new(uploader, file, dest, run_id))
            .map(|id| id.get())
            .map_err(queue_error)
    }

    pub fn join(self) -> Result<(), ServiceError> {
        self.queue.join().map_err(queue_error)
    }
}

struct CoreSecrets(Arc<dyn CoreSecretStore + Send + Sync>);

impl queue::SecretStore for CoreSecrets {
    fn get(&self, dest_id: &str, field: &str) -> Result<Option<Vec<u8>>, queue::SecretError> {
        let key = SecretKey::new(dest_id, field).map_err(|_| queue::SecretError::Unavailable)?;
        match self.0.get(&key) {
            Ok(secret) => Ok(secret.map(|secret| secret.expose().to_vec())),
            Err(_) => Err(queue::SecretError::Unavailable),
        }
    }
}

fn queue_error(error: queue::QueueError) -> ServiceError {
    let message = error.to_string();
    match error {
        queue::QueueError::NotFound => ServiceError::NotFound,
        queue::QueueError::NotCancellable | queue::QueueError::NotRetryable => {
            ServiceError::Invalid(message)
        }
        queue::QueueError::Spawn => ServiceError::Internal(message),
        queue::QueueError::Closed => ServiceError::Busy,
    }
}

impl From<&queue::Status> for UploadStatus {
    fn from(status: &queue::Status) -> Self {
        match status {
            queue::Status::Queued => Self::Queued,
            queue::Status::Uploading => Self::Uploading,
            queue::Status::BackingOff { .. } => Self::BackingOff,
            queue::Status::Done => Self::Done,
            queue::Status::Failed => Self::Failed,
            queue::Status::Cancelled => Self::Cancelled,
        }
    }
}

impl From<&queue::UploadSnapshot> for UploadState {
    fn from(snapshot: &queue::UploadSnapshot) -> Self {
        let retry_in_ms = match snapshot.status {
            queue::Status::BackingOff { retry_in } => {
                Some(u64::try_from(retry_in.as_millis()).unwrap_or(u64::MAX))
            }
            _ => None,
        };
        Self {
            id: snapshot.id.get(),
            capture_id: snapshot.capture_id,
            file_name: snapshot.file_name.clone(),
            dest: snapshot.dest.clone(),
            run_id: snapshot.run_id.clone(),
            status: UploadStatus::from(&snapshot.status),
            cancellable: snapshot.cancellable,
            progress_available: snapshot.progress_available,
            sent: snapshot.sent,
            total: snapshot.total,
            link_batch: snapshot.link_batch.as_ref().map(|batch| UploadLinkBatch {
                id: batch.id.clone(),
                index: batch.index,
                size: batch.size,
            }),
            attempts: snapshot.attempts,
            link: snapshot.link.clone(),
            page_link: snapshot.page_link.clone(),
            deletion_link: snapshot.deletion_link.clone(),
            error: snapshot.error.as_ref().map(ToString::to_string),
            retry_in_ms,
        }
    }
}

pub fn retry_policy(retry: Retry) -> queue::RetryPolicy {
    let retries = match retry {
        Retry::Never => 0,
        Retry::Once => 1,
        Retry::ThreeTimes => 3,
        Retry::FiveTimes => 5,
    };
    queue::RetryPolicy::new(retries)
}

pub fn run(action: &ActionId, services: &Services) -> Result<(), ServiceError> {
    match action.as_str() {
        "upload_clipboard" => crate::clipboard_upload::run(services),
        "upload_file" => services
            .upload()
            .handle(UploadReq::PickAndUpload)
            .map(|_| ()),
        crate::shell_actions::COPY_LAST_LINK => {
            crate::shell_actions::run(action.as_str(), services)
        }
        _ => Err(ServiceError::NotYet),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    use snapline_core::ports::{SecretError as CoreSecretError, SecretStore};
    use snapline_core::secret::Secret;
    use snapline_store::io::Paths;
    use snapline_upload::error::UploadError;
    use tempfile::TempDir;

    use super::*;

    struct TestSecrets;

    impl SecretStore for TestSecrets {
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

    struct TestUploader {
        calls: Mutex<u32>,
    }

    impl queue::Uploader for TestUploader {
        fn upload(
            &self,
            file: &queue::UploadFile,
            _secrets: &dyn queue::SecretStore,
        ) -> Result<queue::Uploaded, UploadError> {
            if let Ok(mut calls) = self.calls.lock() {
                *calls += 1;
            }
            Ok(queue::Uploaded::new(format!(
                "https://example.test/{}",
                file.file_name
            )))
        }

        fn delete(
            &self,
            _link: &queue::DeletionLink,
            _secrets: &dyn queue::SecretStore,
        ) -> Result<(), UploadError> {
            Ok(())
        }
    }

    struct TestFiles;

    impl FileSource for TestFiles {
        fn file(&self, capture_id: i64) -> Result<Option<queue::UploadFile>, ServiceError> {
            if capture_id == 1 {
                Ok(Some(queue::UploadFile::new(
                    PathBuf::from("C:\\captures\\shot.png"),
                    "shot.png",
                    "image/png",
                    vec![1_u8, 2, 3],
                )))
            } else {
                Ok(None)
            }
        }
    }

    struct TestDestinations {
        uploader: Arc<dyn queue::Uploader>,
    }

    impl DestinationSource for TestDestinations {
        fn uploader(&self, dest: &str) -> Option<Arc<dyn queue::Uploader>> {
            if dest == "test" {
                Some(Arc::clone(&self.uploader))
            } else {
                None
            }
        }
    }

    fn thread() -> UploaderThread {
        let uploader = Arc::new(TestUploader {
            calls: Mutex::new(0),
        });
        UploaderThread::start(
            &Services::new(),
            Arc::new(TestSecrets),
            queue::RetryPolicy::new(0),
            Arc::new(TestFiles),
            Arc::new(TestDestinations { uploader }),
        )
        .expect("uploader thread")
    }

    struct SettingsFixture {
        _directory: TempDir,
        services: Services,
    }

    fn settings_fixture() -> SettingsFixture {
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
        SettingsFixture {
            _directory: directory,
            services,
        }
    }

    fn event(thread: &UploaderThread) -> AppEvent {
        thread
            .events()
            .expect("events")
            .recv_timeout(Duration::from_secs(5))
            .expect("event")
    }

    #[test]
    fn the_settings_retry_count_maps_to_the_queue_policy() {
        assert_eq!(retry_policy(Retry::Never).max_retries(), 0);
        assert_eq!(retry_policy(Retry::Once).max_retries(), 1);
        assert_eq!(retry_policy(Retry::ThreeTimes).max_retries(), 3);
        assert_eq!(retry_policy(Retry::FiveTimes).max_retries(), 5);
    }

    #[test]
    fn ping_round_trips() {
        let services = Services::new();
        assert_eq!(
            services.upload().handle(UploadReq::Ping),
            Ok(UploadResp::Pong)
        );
    }

    #[test]
    fn queue_operations_need_a_started_uploader() {
        let services = Services::new();
        assert_eq!(
            services.upload().handle(UploadReq::List).expect_err("list"),
            ServiceError::NotYet
        );
    }

    #[test]
    fn upload_elsewhere_enqueues_and_emits_a_state_event() {
        let thread = thread();
        let service = thread.service();
        let accepted = service
            .handle(UploadReq::UploadElsewhere {
                capture_id: 1,
                dest: "test".to_owned(),
            })
            .expect("accepted");
        let UploadResp::Accepted { id } = accepted else {
            panic!("unexpected response");
        };
        loop {
            match event(&thread) {
                AppEvent::Upload(Event::StateChanged(state)) => {
                    if state.id != id || state.status != UploadStatus::Done {
                        continue;
                    }
                    assert_eq!(state.status, UploadStatus::Done);
                    assert_eq!(state.link.as_deref(), Some("https://example.test/shot.png"));
                    break;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        let UploadResp::State(state) = service.handle(UploadReq::State { id }).expect("state")
        else {
            panic!("unexpected state response");
        };
        assert_eq!(state.status, UploadStatus::Done);
        assert_eq!(state.attempts, 1);
        let UploadResp::States(states) = service.handle(UploadReq::List).expect("list") else {
            panic!("unexpected list response");
        };
        assert_eq!(states.len(), 1);
        thread.join().expect("join");
    }

    #[test]
    fn upload_elsewhere_rejects_unknown_captures_and_destinations() {
        let thread = thread();
        let service = thread.service();
        assert_eq!(
            service
                .handle(UploadReq::UploadElsewhere {
                    capture_id: 99,
                    dest: "test".to_owned(),
                })
                .expect_err("missing capture"),
            ServiceError::NotFound
        );
        assert!(matches!(
            service
                .handle(UploadReq::UploadElsewhere {
                    capture_id: 1,
                    dest: "missing".to_owned(),
                })
                .expect_err("missing destination"),
            ServiceError::Invalid(_)
        ));
        thread.join().expect("join");
    }

    #[test]
    fn cancel_and_retry_of_unknown_ids_report_not_found() {
        let thread = thread();
        let service = thread.service();
        assert_eq!(
            service
                .handle(UploadReq::Cancel { id: 99 })
                .expect_err("cancel"),
            ServiceError::NotFound
        );
        assert_eq!(
            service
                .handle(UploadReq::Retry { id: 99 })
                .expect_err("retry"),
            ServiceError::NotFound
        );
        thread.join().expect("join");
    }

    #[test]
    fn the_last_link_is_empty_until_an_upload_succeeds() {
        let fixture = settings_fixture();
        assert_eq!(
            fixture.services.upload().handle(UploadReq::LastLink),
            Ok(UploadResp::LastLink { link: None })
        );
    }

    fn done_state(file_name: &str, link: Option<&str>) -> UploadState {
        UploadState {
            id: 1,
            capture_id: Some(7),
            file_name: file_name.to_owned(),
            dest: "imgur".to_owned(),
            run_id: None,
            status: UploadStatus::Done,
            cancellable: false,
            progress_available: false,
            sent: 1,
            total: 1,
            link_batch: None,
            attempts: 1,
            link: link.map(str::to_owned),
            page_link: None,
            deletion_link: None,
            error: None,
            retry_in_ms: None,
        }
    }

    #[test]
    fn an_image_upload_notice_carries_the_link_thumbnail_and_buttons() {
        let local = LocalCapture {
            path: std::path::PathBuf::from(r"C:\Users\Aaron\Pictures\Snap line\shot.png"),
            thumbnail: Some(std::path::PathBuf::from(r"C:\Thumbs\7.webp")),
        };
        let notice = upload_notice(
            &done_state("shot.png", Some("https://example.test/shot.png")),
            Some(&local),
        )
        .expect("notice");
        assert_eq!(notice.title, "Upload complete");
        assert_eq!(notice.tag.as_deref(), Some("capture-7"));
        assert_eq!(notice.lines, ["shot.png", "https://example.test/shot.png"]);
        assert_eq!(
            notice.open.as_deref(),
            Some("https://example.test/shot.png")
        );
        assert_eq!(notice.image, local.thumbnail);
        let buttons: Vec<(&str, &str)> = notice
            .buttons
            .iter()
            .map(|button| (button.label.as_str(), button.target.as_str()))
            .collect();
        assert_eq!(
            buttons,
            [
                ("Open link", "https://example.test/shot.png"),
                ("Open folder", "file:///C:/Users/Aaron/Pictures/Snap%20line"),
            ]
        );
    }

    #[test]
    fn an_upload_without_a_link_falls_back_to_the_folder() {
        let local = LocalCapture {
            path: std::path::PathBuf::from(r"C:\Pics\shot.png"),
            thumbnail: None,
        };
        let notice = upload_notice(&done_state("shot.png", None), Some(&local)).expect("notice");
        assert_eq!(notice.open.as_deref(), Some("file:///C:/Pics"));
        assert_eq!(notice.buttons.len(), 1);
        assert_eq!(notice.buttons[0].label, "Open folder");
        assert!(upload_notice(&done_state("shot.png", None), None).is_none());
    }

    #[test]
    fn a_failed_image_upload_gets_a_final_notice_with_the_reason() {
        let mut state = done_state("shot.png", None);
        state.status = UploadStatus::Failed;
        state.error = Some("The server refused the file.".to_owned());
        let local = LocalCapture {
            path: std::path::PathBuf::from(r"C:\Pics\shot.png"),
            thumbnail: None,
        };
        let notice = upload_failed_notice(&state, Some(&local)).expect("notice");
        assert_eq!(notice.title, "Upload failed");
        assert_eq!(notice.lines[1], "The server refused the file.");
        assert_eq!(notice.tag.as_deref(), Some("capture-7"));
        assert_eq!(notice.open.as_deref(), Some("file:///C:/Pics/shot.png"));
        assert_eq!(notice.buttons.len(), 2);
        assert!(upload_failed_notice(&done_state("shot.png", None), Some(&local)).is_none());
    }

    #[test]
    fn an_upload_without_a_history_row_has_no_tag() {
        let mut state = done_state("shot.png", Some("https://example.test/a.png"));
        state.capture_id = None;
        let notice = upload_notice(&state, None).expect("notice");
        assert_eq!(notice.tag, None);
    }

    #[test]
    fn a_link_only_upload_has_just_the_link_button() {
        let notice = upload_notice(
            &done_state("shot.png", Some("https://example.test/a.png")),
            None,
        )
        .expect("notice");
        assert_eq!(notice.buttons.len(), 1);
        assert_eq!(notice.buttons[0].label, "Open link");
    }

    #[test]
    fn a_video_upload_gets_an_upload_complete_notice_with_the_link() {
        let state = done_state("clip.mp4", Some("https://example.test/clip.mp4"));
        let notice = upload_notice(&state, None).expect("notice");
        assert_eq!(notice.title, "Upload complete");
        assert_eq!(notice.tag.as_deref(), Some("capture-7"));
        assert_eq!(notice.buttons[0].label, "Open link");
        let gif = done_state("clip.gif", Some("https://example.test/clip.gif"));
        assert!(upload_notice(&gif, None).is_some());
    }

    #[test]
    fn a_failed_video_upload_offers_to_open_the_video() {
        let mut state = done_state("clip.mp4", None);
        state.status = UploadStatus::Failed;
        let local = LocalCapture {
            path: std::path::PathBuf::from(r"C:\Pics\clip.mp4"),
            thumbnail: None,
        };
        let notice = upload_failed_notice(&state, Some(&local)).expect("notice");
        assert_eq!(notice.title, "Upload failed");
        assert_eq!(notice.buttons[0].label, "Open video");
    }

    #[test]
    fn documents_and_text_uploads_get_no_upload_notice() {
        let state = done_state("notes.txt", Some("https://example.test/notes.txt"));
        assert!(upload_notice(&state, None).is_none());
    }

    #[test]
    fn the_last_link_is_the_newest_finished_upload_in_history() {
        let fixture = settings_fixture();
        seed_done_upload(&fixture.services, "a.png", "https://example.test/a.png", 10);
        seed_done_upload(&fixture.services, "b.png", "https://example.test/b.png", 20);
        assert_eq!(
            last_link(&fixture.services),
            Ok(Some("https://example.test/b.png".to_owned()))
        );
    }

    #[test]
    fn deleting_the_newest_upload_falls_back_and_deleting_all_clears_it() {
        let fixture = settings_fixture();
        let first = seed_done_upload(&fixture.services, "a.png", "https://example.test/a.png", 10);
        let second = seed_done_upload(&fixture.services, "b.png", "https://example.test/b.png", 20);
        let path = fixture.services.paths().expect("paths").history_file;
        let mut repo = HistoryRepo::open_write(&path).expect("history");
        repo.delete(second).expect("delete");
        assert_eq!(
            last_link(&fixture.services),
            Ok(Some("https://example.test/a.png".to_owned()))
        );
        repo.delete(first).expect("delete");
        assert_eq!(last_link(&fixture.services), Ok(None));
        assert_eq!(
            fixture.services.upload().handle(UploadReq::LastLink),
            Ok(UploadResp::LastLink { link: None })
        );
    }

    #[test]
    fn a_failed_upload_does_not_replace_the_last_link() {
        let fixture = settings_fixture();
        seed_done_upload(&fixture.services, "a.png", "https://example.test/a.png", 10);
        let id = seed_upload(&fixture.services, "b.png", HistoryUploadState::Failed, 20);
        let path = fixture.services.paths().expect("paths").history_file;
        let mut repo = HistoryRepo::open_write(&path).expect("history");
        repo.set_upload(UploadUpdate {
            sent: 0,
            total: 0,
            capture_id: id,
            dest_id: "other".to_owned(),
            state: HistoryUploadState::Failed,
            url: Some("https://example.test/failed.png".to_owned()),
            deletion_url: None,
            error: Some("refused".to_owned()),
            attempts: 1,
            updated_at: 30,
        })
        .expect("failed upload");
        assert_eq!(
            last_link(&fixture.services),
            Ok(Some("https://example.test/a.png".to_owned()))
        );
    }

    const DAY_MS: i64 = 86_400_000;

    fn now_ms() -> i64 {
        time::OffsetDateTime::now_utc().unix_timestamp() * 1000
    }

    fn seed_upload(
        services: &Services,
        name: &str,
        state: HistoryUploadState,
        updated_at: i64,
    ) -> i64 {
        use snapline_store::history::{CaptureKind, NewCapture};

        let path = services.paths().expect("paths").history_file;
        let mut repo = HistoryRepo::open_write(&path).expect("history");
        let id = repo
            .record(NewCapture {
                kind: CaptureKind::Image,
                path: format!("C:\\captures\\{name}"),
                bytes: 3,
                width: None,
                height: None,
                duration_ms: None,
                source_app: None,
                created_at: updated_at,
                ocr_text: None,
            })
            .expect("capture");
        repo.set_upload(UploadUpdate {
            sent: 0,
            total: 0,
            capture_id: id,
            dest_id: "test".to_owned(),
            state,
            url: None,
            deletion_url: None,
            error: None,
            attempts: 1,
            updated_at,
        })
        .expect("upload");
        id
    }

    fn upload_state(services: &Services, id: i64) -> Option<HistoryUploadState> {
        let path = services.paths().expect("paths").history_file;
        HistoryRepo::open_read(&path)
            .expect("history")
            .get(id)
            .expect("get")
            .expect("capture")
            .upload
            .map(|upload| upload.state)
    }

    fn keep_history(services: &Services, value: &str) {
        services
            .update_settings(&serde_json::json!({ "uploads": { "keep_history": value } }))
            .expect("settings");
    }

    #[test]
    fn maintenance_without_a_history_file_does_nothing() {
        let fixture = settings_fixture();
        maintain_history(&fixture.services, true).expect("maintain");
        assert!(!fixture
            .services
            .paths()
            .expect("paths")
            .history_file
            .exists());
    }

    #[test]
    fn keeping_history_forever_expires_nothing() {
        let fixture = settings_fixture();
        let old = seed_upload(
            &fixture.services,
            "old.png",
            HistoryUploadState::Done,
            now_ms() - 5000 * DAY_MS,
        );
        maintain_history(&fixture.services, false).expect("maintain");
        maintain_history(&fixture.services, true).expect("maintain at startup");
        assert_eq!(
            upload_state(&fixture.services, old),
            Some(HistoryUploadState::Done)
        );
    }

    #[test]
    fn each_retention_setting_expires_uploads_past_its_age() {
        for (setting, days) in [("days30", 30), ("days90", 90), ("one_year", 365)] {
            let fixture = settings_fixture();
            keep_history(&fixture.services, setting);
            let now = now_ms();
            let expired = seed_upload(
                &fixture.services,
                "expired.png",
                HistoryUploadState::Done,
                now - (days + 1) * DAY_MS,
            );
            let kept = seed_upload(
                &fixture.services,
                "kept.png",
                HistoryUploadState::Done,
                now - (days - 1) * DAY_MS,
            );
            maintain_history(&fixture.services, false).expect("maintain");
            assert_eq!(upload_state(&fixture.services, expired), None, "{setting}");
            assert_eq!(
                upload_state(&fixture.services, kept),
                Some(HistoryUploadState::Done),
                "{setting}"
            );
        }
    }

    #[test]
    fn only_startup_maintenance_fails_interrupted_uploads() {
        let fixture = settings_fixture();
        keep_history(&fixture.services, "days30");
        let now = now_ms();
        let queued = seed_upload(
            &fixture.services,
            "queued.png",
            HistoryUploadState::Queued,
            now,
        );
        let uploading = seed_upload(
            &fixture.services,
            "uploading.png",
            HistoryUploadState::Uploading,
            now,
        );
        maintain_history(&fixture.services, false).expect("maintain");
        assert_eq!(
            upload_state(&fixture.services, queued),
            Some(HistoryUploadState::Queued)
        );
        maintain_history(&fixture.services, true).expect("maintain at startup");
        assert_eq!(
            upload_state(&fixture.services, queued),
            Some(HistoryUploadState::Failed)
        );
        assert_eq!(
            upload_state(&fixture.services, uploading),
            Some(HistoryUploadState::Failed)
        );
    }
}
