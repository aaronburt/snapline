use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use snapline_core::actions::ActionId;
use snapline_store::history::HistoryRepo;
use snapline_store::io::Paths;

use crate::saver::{CaptureRequest, SaveOutcome, Saver, ServicePorts, SettingsAccess};
use crate::{
    actions, app, history, hotkeys, recording, settings, tools, uploads, workflows, AppEvent,
    AreaJob, Job, ServiceError, Services,
};

enum Command {
    Job(Job),
    Action {
        action: ActionId,
        reply: Sender<Result<(), ServiceError>>,
    },
    Save {
        request: CaptureRequest,
        reply: Sender<Result<SaveOutcome, ServiceError>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct Handle {
    commands: Sender<Command>,
}

impl Handle {
    pub fn enqueue(&self, job: Job) -> Result<(), ServiceError> {
        self.commands
            .send(Command::Job(job))
            .map_err(|_| ServiceError::Busy)
    }

    pub fn run_action(&self, action: ActionId) -> Result<(), ServiceError> {
        let (reply, response) = bounded(1);
        self.commands
            .send(Command::Action { action, reply })
            .map_err(|_| ServiceError::Busy)?;
        response.recv().map_err(|_| ServiceError::Busy)?
    }

    pub fn save_capture(&self, request: CaptureRequest) -> Result<SaveOutcome, ServiceError> {
        let (reply, response) = bounded(1);
        self.commands
            .send(Command::Save { request, reply })
            .map_err(|_| ServiceError::Busy)?;
        response.recv().map_err(|_| ServiceError::Busy)?
    }

    pub fn shutdown(&self) -> Result<(), ServiceError> {
        self.commands
            .send(Command::Shutdown)
            .map_err(|_| ServiceError::Busy)
    }
}

pub struct Worker {
    pub handle: Handle,
    pub events: Receiver<AppEvent>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start<F>(services: Services, ports: F) -> Result<Self, ServiceError>
    where
        F: FnOnce() -> Result<ServicePorts, ServiceError> + Send + 'static,
    {
        let paths = services.paths()?;
        let repo = HistoryRepo::open_write(&paths.history_file)
            .map_err(|error| ServiceError::Io(error.to_string()))?;
        let thumbs_dir = paths.thumbs_dir.clone();
        let profile = user_profile(&paths);
        let (commands, inbox) = unbounded();
        let (outbox, events) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let thread = thread::Builder::new()
            .name("snapline-worker".to_owned())
            .spawn(move || {
                let ports = match ports() {
                    Ok(ports) => ports,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let settings: Arc<dyn SettingsAccess> = Arc::new(services.clone());
                let saver = Saver::new(settings, ports, thumbs_dir, profile);
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                run(inbox, outbox, services, saver, repo);
            })
            .map_err(|error| ServiceError::Io(error.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(ServiceError::Internal(
                    "The worker could not start.".to_owned(),
                ))
            }
        }
        Ok(Self {
            handle: Handle { commands },
            events,
            thread: Some(thread),
        })
    }

    pub fn join(mut self) {
        let _ = self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    pub fn into_parts(self) -> (Handle, Receiver<AppEvent>) {
        let Self {
            handle,
            events,
            thread,
        } = self;
        drop(thread);
        (handle, events)
    }
}

fn run(
    inbox: Receiver<Command>,
    events: Sender<AppEvent>,
    services: Services,
    saver: Saver,
    mut repo: HistoryRepo,
) {
    while let Ok(command) = inbox.recv() {
        match command {
            Command::Job(job) => handle_job(job, &services, &events),
            Command::Action { action, reply } => {
                let result = run_action(&action, &services);
                let _ = reply.send(result);
            }
            Command::Save { request, reply } => {
                let outcome = saver.run(&mut repo, request);
                if let Ok(saved) = &outcome {
                    if let Some(id) = saved.capture_id {
                        let _ = events.send(AppEvent::History(history::Event::Added { id }));
                    }
                }
                if let Err(error) = &outcome {
                    tracing::warn!(%error, "a capture could not be saved");
                }
                let _ = reply.send(outcome.map_err(ServiceError::from));
            }
            Command::Shutdown => break,
        }
    }
}

fn handle_job(job: Job, services: &Services, events: &Sender<AppEvent>) {
    match job {
        Job::Action(action) => {
            if let Err(error) = run_action(&action, services) {
                tracing::debug!(%error, action = %action, "the worker could not run an action");
            }
        }
        Job::Area(area) => send_area(area, events),
    }
}

fn run_action(action: &ActionId, services: &Services) -> Result<(), ServiceError> {
    actions::dispatch(action, services)
}

fn send_area(area: AreaJob, events: &Sender<AppEvent>) {
    match area {
        AreaJob::App(job) => match job {
            app::Job::Ping => drop(events.send(AppEvent::App(app::Event::Ping))),
        },
        AreaJob::Settings(job) => match job {
            settings::Job::Ping => drop(events.send(AppEvent::Settings(settings::Event::Ping))),
        },
        AreaJob::Hotkeys(job) => match job {
            hotkeys::Job::Ping => drop(events.send(AppEvent::Hotkeys(hotkeys::Event::Ping))),
        },
        AreaJob::History(job) => match job {
            history::Job::Ping => drop(events.send(AppEvent::History(history::Event::Ping))),
        },
        AreaJob::Upload(job) => match job {
            uploads::Job::Ping => drop(events.send(AppEvent::Upload(uploads::Event::Ping))),
        },
        AreaJob::Workflow(job) => match job {
            workflows::Job::Ping => drop(events.send(AppEvent::Workflow(workflows::Event::Ping))),
        },
        AreaJob::Record(job) => match job {
            recording::Job::Ping => drop(events.send(AppEvent::Record(recording::Event::Ping))),
        },
        AreaJob::Tool(job) => match job {
            tools::Job::Ping => drop(events.send(AppEvent::Tool(tools::Event::Ping))),
        },
    }
}

fn user_profile(paths: &Paths) -> PathBuf {
    paths
        .captures_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crossbeam_channel::TryRecvError;
    use snapline_core::actions::ActionId;
    use snapline_core::image::{Image, ImageFormat};
    use snapline_core::ports::{
        Clipboard, ClipboardError, Clock, CodecError, ImageCodec, Notice, Notifier, Sound, SoundId,
    };
    use snapline_store::io::Paths;
    use tempfile::tempdir;
    use time::macros::datetime;

    use super::*;

    struct TestCodec;

    impl ImageCodec for TestCodec {
        fn encode(
            &self,
            _image: &Image,
            _format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            Ok(vec![3_u8; 12])
        }

        fn decode(&self, _bytes: &[u8]) -> Result<Image, CodecError> {
            Err(CodecError::Decode("not used".to_owned()))
        }
    }

    #[derive(Default)]
    struct TestClipboard {
        images: AtomicUsize,
    }

    impl Clipboard for TestClipboard {
        fn set_image(&self, _image: &Image) -> Result<(), ClipboardError> {
            self.images.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn set_text(&self, _text: &str) -> Result<(), ClipboardError> {
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct TestNotifier {
        notices: Mutex<Vec<Notice>>,
    }

    impl Notifier for TestNotifier {
        fn toast(&self, notice: Notice) {
            self.notices.lock().expect("notices").push(notice);
        }
    }

    struct TestSound;

    impl Sound for TestSound {
        fn play(&self, _sound: SoundId) {}
    }

    struct TestClock;

    impl Clock for TestClock {
        fn now_local(&self) -> time::OffsetDateTime {
            datetime!(2026-11-08 14:31:07 UTC)
        }
    }

    fn test_paths(directory: &Path) -> Paths {
        Paths::resolve(
            &directory.join("exe"),
            &directory.join("roaming"),
            &directory.join("local"),
            &directory.join("user"),
            false,
        )
    }

    fn services(directory: &Path) -> Services {
        Services::with_paths(test_paths(directory)).expect("services")
    }

    fn ports() -> ServicePorts {
        ServicePorts::new(
            Arc::new(TestCodec),
            Arc::new(TestClipboard::default()),
            Arc::new(TestNotifier::default()),
            Arc::new(TestSound),
            Arc::new(TestClock),
        )
    }

    fn request() -> CaptureRequest {
        CaptureRequest::new(
            Image::bgra(640, 480).expect("image"),
            Some("chrome.exe".to_owned()),
            "Pull-request-42",
        )
    }

    fn event(worker: &Worker) -> AppEvent {
        worker
            .events
            .recv_timeout(Duration::from_secs(5))
            .expect("event")
    }

    #[test]
    fn a_saved_capture_sends_history_added_and_persists_the_counter() {
        let directory = tempdir().expect("temp");
        let services = services(directory.path());
        let worker = Worker::start(services.clone(), || Ok(ports())).expect("worker");
        let outcome = worker.handle.save_capture(request()).expect("save capture");
        assert!(outcome.saved);
        let id = outcome.capture_id.expect("id");
        match event(&worker) {
            AppEvent::History(history::Event::Added { id: received }) => {
                assert_eq!(received, id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        let paths = test_paths(directory.path());
        let repo = HistoryRepo::open_read(&paths.history_file).expect("read");
        let capture = repo.get(id).expect("get").expect("capture");
        assert_eq!(capture.name(), "chrome_2026-11-08_14-31-07.png");
        assert!(paths.thumbs_dir.join(format!("{id}.webp")).is_file());
        let settings = services.settings_snapshot().expect("settings");
        assert_eq!(
            settings.general.extra.get("counter"),
            Some(&toml::Value::Integer(1))
        );
        assert!(matches!(worker.events.try_recv(), Err(TryRecvError::Empty)));
        worker.join();
    }

    #[test]
    fn jobs_run_in_the_order_they_are_queued() {
        let directory = tempdir().expect("temp");
        let services = services(directory.path());
        let worker = Worker::start(services, || Ok(ports())).expect("worker");
        assert!(matches!(worker.events.try_recv(), Err(TryRecvError::Empty)));
        worker
            .handle
            .enqueue(Job::Area(AreaJob::App(app::Job::Ping)))
            .expect("app");
        worker
            .handle
            .enqueue(Job::Area(AreaJob::Settings(settings::Job::Ping)))
            .expect("settings");
        assert!(matches!(event(&worker), AppEvent::App(app::Event::Ping)));
        assert!(matches!(
            event(&worker),
            AppEvent::Settings(settings::Event::Ping)
        ));
        assert!(matches!(worker.events.try_recv(), Err(TryRecvError::Empty)));
        worker.join();
    }

    #[test]
    fn an_action_is_handled_before_the_next_queued_job() {
        let directory = tempdir().expect("temp");
        let services = services(directory.path());
        let worker = Worker::start(services, || Ok(ports())).expect("worker");
        worker
            .handle
            .enqueue(Job::Action(ActionId::new("capture_region")))
            .expect("action");
        worker
            .handle
            .enqueue(Job::Area(AreaJob::App(app::Job::Ping)))
            .expect("app");
        assert!(matches!(event(&worker), AppEvent::App(app::Event::Ping)));
        assert!(matches!(worker.events.try_recv(), Err(TryRecvError::Empty)));
        worker.join();
    }

    #[test]
    fn the_worker_starts_blocked_and_has_no_pending_events() {
        let directory = tempdir().expect("temp");
        let services = services(directory.path());
        let worker = Worker::start(services, || Ok(ports())).expect("worker");
        std::thread::sleep(Duration::from_millis(20));
        assert!(matches!(worker.events.try_recv(), Err(TryRecvError::Empty)));
        worker
            .handle
            .enqueue(Job::Area(AreaJob::App(app::Job::Ping)))
            .expect("app");
        assert!(matches!(event(&worker), AppEvent::App(app::Event::Ping)));
        worker.join();
    }

    #[test]
    fn later_milestone_actions_report_not_yet() {
        let directory = tempdir().expect("temp");
        let services = services(directory.path());
        let worker = Worker::start(services, || Ok(ports())).expect("worker");
        let error = worker
            .handle
            .run_action(ActionId::new("record_video"))
            .expect_err("not yet");
        assert!(matches!(error, ServiceError::NotYet));
        let error = worker
            .handle
            .run_action(ActionId::new("tool_ocr"))
            .expect_err("not yet");
        assert!(matches!(error, ServiceError::NotYet));
        worker.join();
    }

    #[test]
    fn save_requests_keep_their_reply_channel() {
        let directory = tempdir().expect("temp");
        let services = services(directory.path());
        let worker = Worker::start(services, || Ok(ports())).expect("worker");
        let first = worker.handle.save_capture(request()).expect("first");
        let second = worker.handle.save_capture(request()).expect("second");
        assert_eq!(first.capture_id, Some(1));
        assert_eq!(second.capture_id, Some(2));
        assert_eq!(
            first.file_name.as_deref(),
            Some("chrome_2026-11-08_14-31-07.png")
        );
        assert_eq!(
            second.file_name.as_deref(),
            Some("chrome_2026-11-08_14-31-07 (2).png")
        );
        worker.join();
    }

    #[test]
    fn the_worker_refuses_work_after_shutdown() {
        let directory = tempdir().expect("temp");
        let services = services(directory.path());
        let worker = Worker::start(services, || Ok(ports())).expect("worker");
        let handle = worker.handle.clone();
        worker.join();
        assert_eq!(
            handle
                .enqueue(Job::Area(AreaJob::App(app::Job::Ping)))
                .expect_err("closed")
                .code(),
            "busy"
        );
        assert_eq!(
            handle.save_capture(request()).expect_err("closed").code(),
            "busy"
        );
    }
}
