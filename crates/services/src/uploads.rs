use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver};
use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_core::ports::SecretStore as CoreSecretStore;
use snapline_core::secret::SecretKey;
use snapline_store::settings::uploads::Retry;
use snapline_upload::queue;
use ts_rs::TS;

use crate::destinations::DestinationOperations;
use crate::dto::upload::{
    DestinationReq, DestinationResp, DestinationTestOutcomeDto, DestinationTestStatusDto,
    UploadReq, UploadResp, UploadState, UploadStatus,
};
use crate::{AppEvent, ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "UploadAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
    StateChanged(UploadState),
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub trait FileSource: Send + Sync {
    fn file(&self, capture_id: i64) -> Result<Option<queue::UploadFile>, ServiceError>;
}

pub trait DestinationSource: Send + Sync {
    fn uploader(&self, dest: &str) -> Option<Arc<dyn queue::Uploader>>;

    fn operations(&self) -> Option<Arc<dyn DestinationOperations>> {
        None
    }
}

#[derive(Clone)]
pub struct UploadRuntime {
    queue: queue::Handle,
    files: Arc<dyn FileSource>,
    destinations: Arc<dyn DestinationSource>,
}

#[derive(Clone)]
pub struct UploadService {
    _services: Services,
    runtime: Option<UploadRuntime>,
}

impl UploadService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            _services: services.clone(),
            runtime: None,
        }
    }

    pub fn with_runtime(services: &Services, runtime: UploadRuntime) -> Self {
        Self {
            _services: services.clone(),
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
                    .enqueue(queue::UploadJob::new(uploader, file, dest, None))
                    .map_err(queue_error)?;
                Ok(UploadResp::Accepted { id: id.get() })
            }
            UploadReq::Destination(req) => self.handle_destination(req),
        }
    }

    fn handle_destination(&self, req: DestinationReq) -> Result<UploadResp, ServiceError> {
        let runtime = self.runtime()?;
        let operations = runtime.destinations.operations().ok_or(ServiceError::NotYet)?;
        let response = match req {
            DestinationReq::List => {
                DestinationResp::Destinations(operations.list()?)
            }
            DestinationReq::Add { kind } => DestinationResp::Destination(operations.add(kind)?),
            DestinationReq::Save { input } => {
                DestinationResp::Destination(operations.save(input)?)
            }
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
                    .enqueue(queue::UploadJob::new(uploader, file, id.clone(), None))
                    .map_err(queue_error)?;
                let deadline = Instant::now() + Duration::from_secs(120);
                loop {
                    let snapshot = runtime
                        .queue
                        .state(upload_id)
                        .map_err(queue_error)?;
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
                            return Ok(UploadResp::Destination(DestinationResp::Test {
                                outcome,
                            }));
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

    pub fn start_with_events(
        services: &Services,
        secrets: Arc<dyn CoreSecretStore + Send + Sync>,
        policy: queue::RetryPolicy,
        files: Arc<dyn FileSource>,
        destinations: Arc<dyn DestinationSource>,
        events: crossbeam_channel::Sender<AppEvent>,
    ) -> Result<Self, ServiceError> {
        Self::spawn(services, secrets, policy, files, destinations, events)
    }

    fn spawn(
        services: &Services,
        secrets: Arc<dyn CoreSecretStore + Send + Sync>,
        policy: queue::RetryPolicy,
        files: Arc<dyn FileSource>,
        destinations: Arc<dyn DestinationSource>,
        events: crossbeam_channel::Sender<AppEvent>,
    ) -> Result<Self, ServiceError> {
        let sink = move |event: queue::UploadEvent| {
            let state = UploadState::from(&event.0);
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
            dest: snapshot.dest.clone(),
            run_id: snapshot.run_id.clone(),
            status: UploadStatus::from(&snapshot.status),
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

pub fn run(_action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    Err(ServiceError::NotYet)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    use snapline_core::ports::{SecretError as CoreSecretError, SecretStore};
    use snapline_core::secret::Secret;
    use snapline_upload::error::UploadError;

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
}
