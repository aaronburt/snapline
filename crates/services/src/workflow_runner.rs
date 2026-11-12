use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use snapline_core::actions::{ActionId, WORKFLOW_PREFIX};
use snapline_core::ids::DestId;
use snapline_core::image::Image;
use snapline_core::media::{classify, content_type};
use snapline_core::ports::{Clipboard, ClipboardError, Clock, Notifier, Ocr, Sound};
use snapline_core::workflow::{
    self, AnnotatePort, CaptureKind, CapturePort, GifPort, RecordKind, RecordPort, Recorded,
    RunContext, RunError, RunReport, SavePort, SaveRequest, Step, TextPort, UploadId, UploadPort,
    UploadRequest, Uploaded, Workflow, WorkflowError,
};
use snapline_store::settings::uploads::DestinationRef;
use snapline_store::settings::workflows::default_workflow;
use snapline_upload::queue::{self, UploadFile};

use crate::capture;
use crate::dto::hotkeys::{HotkeysReq, HotkeysResp};
use crate::recording::{
    recording_saved_notice, recording_upload_failed_notice, RecordError, SavedRecording,
};
use crate::saver::{CaptureRequest, SystemClock};
use crate::uploads::DestinationSource;
use crate::worker;
use crate::{ServiceError, Services};

pub struct UnavailableCapture;

impl CapturePort for UnavailableCapture {
    fn capture(&self, _kind: CaptureKind) -> Result<Image, WorkflowError> {
        Err(WorkflowError::NotAvailableYet)
    }
}

pub struct UnavailableRecord;

impl RecordPort for UnavailableRecord {
    fn record(&self, _kind: RecordKind) -> Result<Recorded, WorkflowError> {
        Err(WorkflowError::NotAvailableYet)
    }
}

pub struct UnavailableAnnotate;

impl AnnotatePort for UnavailableAnnotate {
    fn annotate(&self, _image: &Image) -> Result<Image, WorkflowError> {
        Err(WorkflowError::NotAvailableYet)
    }
}

pub struct ServiceAnnotate;

impl AnnotatePort for ServiceAnnotate {
    fn annotate(&self, image: &Image) -> Result<Image, WorkflowError> {
        crate::editor::annotate_image(image)
    }
}

pub struct UnavailableSave;

impl SavePort for UnavailableSave {
    fn save(&self, _request: SaveRequest<'_>) -> Result<PathBuf, WorkflowError> {
        Err(WorkflowError::NotAvailableYet)
    }
}

pub struct UnavailableUpload;

impl UploadPort for UnavailableUpload {
    fn hand_off(&self, _request: UploadRequest<'_>) -> Result<UploadId, WorkflowError> {
        Err(WorkflowError::NotAvailableYet)
    }
}

pub struct UnavailableText;

impl TextPort for UnavailableText {
    fn extract(&self, _image: &Image) -> Result<String, WorkflowError> {
        Err(WorkflowError::NotAvailableYet)
    }
}

pub struct UnavailableGif;

impl GifPort for UnavailableGif {
    fn convert(&self, _recording: &Path) -> Result<PathBuf, WorkflowError> {
        Err(WorkflowError::NotAvailableYet)
    }
}

struct SilentClipboard;

impl Clipboard for SilentClipboard {
    fn set_image(&self, _image: &Image) -> Result<(), ClipboardError> {
        Err(ClipboardError::Write)
    }

    fn set_text(&self, _text: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::Write)
    }

    fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
        Err(ClipboardError::Read)
    }
}

struct SilentNotifier;

impl Notifier for SilentNotifier {
    fn toast(&self, _notice: snapline_core::ports::Notice) {}
}

struct SilentSound;

impl Sound for SilentSound {
    fn play(&self, _sound: snapline_core::ports::SoundId) {}
}

#[derive(Clone)]
pub struct RunnerPorts {
    pub capture: Arc<dyn CapturePort + Send + Sync>,
    pub record: Arc<dyn RecordPort + Send + Sync>,
    pub annotate: Arc<dyn AnnotatePort + Send + Sync>,
    pub save: Arc<dyn SavePort + Send + Sync>,
    pub upload: Arc<dyn UploadPort + Send + Sync>,
    pub clipboard: Arc<dyn Clipboard + Send + Sync>,
    pub text: Arc<dyn TextPort + Send + Sync>,
    pub gif: Arc<dyn GifPort + Send + Sync>,
    pub notify: Arc<dyn Notifier + Send + Sync>,
    pub sound: Arc<dyn Sound + Send + Sync>,
    pub clock: Arc<dyn Clock + Send + Sync>,
}

impl RunnerPorts {
    pub fn unavailable() -> Self {
        Self {
            capture: Arc::new(UnavailableCapture),
            record: Arc::new(UnavailableRecord),
            annotate: Arc::new(UnavailableAnnotate),
            save: Arc::new(UnavailableSave),
            upload: Arc::new(UnavailableUpload),
            clipboard: Arc::new(SilentClipboard),
            text: Arc::new(UnavailableText),
            gif: Arc::new(UnavailableGif),
            notify: Arc::new(SilentNotifier),
            sound: Arc::new(SilentSound),
            clock: Arc::new(SystemClock),
        }
    }

    fn core(&self) -> workflow::WorkflowPorts<'_> {
        workflow::WorkflowPorts {
            capture: self.capture.as_ref(),
            record: self.record.as_ref(),
            annotate: self.annotate.as_ref(),
            save: self.save.as_ref(),
            upload: self.upload.as_ref(),
            clipboard: self.clipboard.as_ref(),
            text: self.text.as_ref(),
            gif: self.gif.as_ref(),
            notify: self.notify.as_ref(),
            sound: self.sound.as_ref(),
            clock: self.clock.as_ref(),
        }
    }
}

pub struct ServiceWorkflowOptions {
    pub worker: worker::Handle,
    pub queue: queue::Handle,
    pub destinations: Arc<dyn DestinationSource>,
    pub clipboard: Arc<dyn Clipboard + Send + Sync>,
    pub notifier: Arc<dyn Notifier + Send + Sync>,
    pub sound: Arc<dyn Sound + Send + Sync>,
    pub ocr: Option<Arc<dyn Ocr + Send + Sync>>,
    pub annotate: Option<Arc<dyn AnnotatePort + Send + Sync>>,
}

impl RunnerPorts {
    pub fn service(services: &Services, options: ServiceWorkflowOptions) -> Self {
        let save = Arc::new(WorkerSavePort::new(options.worker));
        let hand_off = Arc::new(QueueUploadHandOff::new(
            services.clone(),
            options.queue,
            options.destinations,
        ));
        let capture = Arc::new(CaptureServicePort);
        let text = options
            .ocr
            .map(|ocr| Arc::new(ServiceTextPort::new(ocr)) as Arc<dyn TextPort + Send + Sync>)
            .unwrap_or_else(|| Arc::new(UnavailableText));
        let annotate = options
            .annotate
            .unwrap_or_else(|| Arc::new(ServiceAnnotate));
        Self {
            capture,
            record: Arc::new(ServiceRecordPort::new(services)),
            annotate,
            save: save.clone(),
            upload: Arc::new(QueueUploadPort::new(save, hand_off)),
            clipboard: options.clipboard,
            text,
            gif: Arc::new(ServiceGifPort::new(services)),
            notify: options.notifier,
            sound: options.sound,
            clock: Arc::new(SystemClock),
        }
    }
}

pub(crate) fn default_from_general(
    steps: &[snapline_store::settings::general::WorkflowStep],
) -> Workflow {
    use snapline_store::settings::general::WorkflowStepKind;
    let mut workflow = default_workflow();
    workflow.steps = steps
        .iter()
        .filter(|step| step.enabled)
        .map(|step| match step.kind {
            WorkflowStepKind::CopyImage => Step::CopyImage,
            WorkflowStepKind::Annotate => Step::Annotate,
            WorkflowStepKind::Save => Step::Save,
            WorkflowStepKind::Upload => Step::Upload { dest: None },
            WorkflowStepKind::CopyLink => Step::CopyLink {
                format: snapline_core::link_format::LinkFormat::Direct,
            },
            WorkflowStepKind::PlaySound => Step::PlaySound,
        })
        .collect();
    workflow
}

pub struct CaptureServicePort;

impl CapturePort for CaptureServicePort {
    fn includes_annotation(&self, kind: CaptureKind) -> bool {
        kind == CaptureKind::Region
    }

    fn capture(&self, kind: CaptureKind) -> Result<Image, WorkflowError> {
        let service =
            capture::installed().map_err(|error| WorkflowError::Failed(error.to_string()))?;
        match kind {
            CaptureKind::Region => service.region_result(true),
            CaptureKind::Window => service.window_image(),
            CaptureKind::Screen => service.screen_image(),
            CaptureKind::LastRegion => service.last_region_image(),
        }
        .map_err(|error| WorkflowError::Failed(error.to_string()))?
        .map(|shot| shot.image)
        .ok_or(WorkflowError::Cancelled)
    }

    fn capture_for_workflow(
        &self,
        kind: CaptureKind,
        annotate: bool,
    ) -> Result<workflow::Captured, WorkflowError> {
        let service =
            capture::installed().map_err(|error| WorkflowError::Failed(error.to_string()))?;
        let region = kind == CaptureKind::Region;
        let shot = match kind {
            CaptureKind::Region => service.region_result(annotate),
            CaptureKind::Window => service.window_image(),
            CaptureKind::Screen => service.screen_image(),
            CaptureKind::LastRegion => service.last_region_image(),
        }
        .map_err(|error| WorkflowError::Failed(error.to_string()))?
        .ok_or(WorkflowError::Cancelled)?;
        Ok(workflow::Captured {
            image: shot.image,
            annotated: region && annotate,
            action: shot.action,
            source_app: shot.source_app,
        })
    }
}

pub struct WorkerSavePort {
    worker: worker::Handle,
}

impl WorkerSavePort {
    pub fn new(worker: worker::Handle) -> Self {
        Self { worker }
    }
}

impl SavePort for WorkerSavePort {
    fn save(&self, request: SaveRequest<'_>) -> Result<PathBuf, WorkflowError> {
        let Some(image) = request.image else {
            let source = request
                .converted
                .or(request.recording)
                .ok_or(WorkflowError::NotAvailableYet)?;
            let details = request
                .recording_details
                .ok_or(WorkflowError::NotAvailableYet)?;
            let staging =
                tempfile::tempdir().map_err(|error| WorkflowError::Failed(error.to_string()))?;
            let name = source.file_name().ok_or(WorkflowError::NotAvailableYet)?;
            let media = staging.path().join(name);
            std::fs::copy(source, &media)
                .map_err(|error| WorkflowError::Failed(error.to_string()))?;
            let kind = if content_type(source) == "image/gif" {
                crate::recording::RecordedKind::Gif
            } else {
                crate::recording::RecordedKind::Video
            };
            return self
                .worker
                .save_recording(crate::recording::RecordedFile {
                    media,
                    first_frame: None,
                    kind,
                    width: details.width,
                    height: details.height,
                    duration_ms: details.duration_ms,
                    source_app: None,
                    window_title: request.file_name.unwrap_or_default().to_owned(),
                })
                .map(|saved| saved.path)
                .map_err(|error| WorkflowError::Failed(error.to_string()));
        };
        let window_title = request.file_name.unwrap_or_default();
        let source_app = request.source_app.map(str::to_owned);
        let mut capture = CaptureRequest::new(image.clone(), source_app, window_title);
        capture.workflow_save = true;
        capture.quiet = request.upload_follows;
        let outcome = self
            .worker
            .save_capture(capture)
            .map_err(|error| WorkflowError::Failed(error.to_string()))?;
        outcome
            .path
            .ok_or_else(|| WorkflowError::Failed("The capture was not saved.".to_owned()))
    }
}

pub struct ServiceRecordPort {
    services: Services,
}

impl ServiceRecordPort {
    pub fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }
}

impl RecordPort for ServiceRecordPort {
    fn record(&self, kind: RecordKind) -> Result<Recorded, WorkflowError> {
        crate::recording::record_blocking(&self.services, kind).map_err(record_error)
    }
}

pub struct ServiceGifPort {
    services: Services,
}

impl ServiceGifPort {
    pub fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }
}

impl GifPort for ServiceGifPort {
    fn convert(&self, recording: &Path) -> Result<PathBuf, WorkflowError> {
        crate::recording::convert_to_gif(&self.services, recording).map_err(record_error)
    }
}

fn record_error(error: RecordError) -> WorkflowError {
    match error {
        RecordError::Cancelled => WorkflowError::Cancelled,
        RecordError::NotYet => WorkflowError::NotAvailableYet,
        other => WorkflowError::Failed(other.to_string()),
    }
}

pub trait WorkflowUploadHandOff: Send + Sync {
    fn hand_off(
        &self,
        file: UploadFile,
        dest: Option<&str>,
        run_id: Option<String>,
    ) -> Result<u64, ServiceError>;
}

pub struct QueueUploadHandOff {
    services: Services,
    queue: queue::Handle,
    destinations: Arc<dyn DestinationSource>,
}

impl QueueUploadHandOff {
    pub fn new(
        services: Services,
        queue: queue::Handle,
        destinations: Arc<dyn DestinationSource>,
    ) -> Self {
        Self {
            services,
            queue,
            destinations,
        }
    }
}

impl WorkflowUploadHandOff for QueueUploadHandOff {
    fn hand_off(
        &self,
        file: UploadFile,
        dest: Option<&str>,
        run_id: Option<String>,
    ) -> Result<u64, ServiceError> {
        let settings = self.services.settings_snapshot()?;
        let mut first_bytes = [0_u8; 512];
        let first_len = std::fs::File::open(&file.path)
            .ok()
            .and_then(|mut input| input.read(&mut first_bytes).ok())
            .unwrap_or_default();
        let media = classify(&file.file_name, &first_bytes[..first_len]);
        let dest = dest.map_or_else(
            || {
                crate::uploads::destination_for(&settings.uploads, media)
                    .as_str()
                    .to_owned()
            },
            ToOwned::to_owned,
        );
        if !DestinationRef::is_upload_id(&dest) {
            return Err(ServiceError::Invalid(
                crate::uploads::missing_destination_message(media),
            ));
        }
        let uploader = self.destinations.uploader(&dest).ok_or_else(|| {
            ServiceError::Invalid("The upload destination was not found.".to_owned())
        })?;
        let capture_id =
            snapline_store::history::HistoryRepo::open_read(&self.services.paths()?.history_file)
                .ok()
                .and_then(|repo| {
                    repo.id_for_path(&file.path.to_string_lossy())
                        .ok()
                        .flatten()
                });
        let job = queue::UploadJob::new(uploader, file, dest, run_id);
        let job = match capture_id {
            Some(capture_id) => job.with_capture_id(capture_id),
            None => job,
        };
        self.queue
            .enqueue(job)
            .map(|id| id.get())
            .map_err(|error| ServiceError::Internal(error.to_string()))
    }
}

pub struct QueueUploadPort {
    save: Arc<dyn SavePort + Send + Sync>,
    queue: Arc<dyn WorkflowUploadHandOff>,
}

impl QueueUploadPort {
    pub fn new(
        save: Arc<dyn SavePort + Send + Sync>,
        queue: Arc<dyn WorkflowUploadHandOff>,
    ) -> Self {
        Self { save, queue }
    }
}

impl UploadPort for QueueUploadPort {
    fn hand_off(&self, request: UploadRequest<'_>) -> Result<UploadId, WorkflowError> {
        let path = match request.saved_file {
            Some(path) => path.to_path_buf(),
            None => match request.converted.or(request.recording) {
                Some(path) => path.to_path_buf(),
                None => {
                    let image = request.image.ok_or(WorkflowError::NotAvailableYet)?;
                    self.save.save(SaveRequest {
                        image: Some(image),
                        recording: request.recording,
                        recording_details: None,
                        converted: request.converted,
                        file_name: request.file_name,
                        source_app: request.source_app,
                        upload_follows: true,
                    })?
                }
            },
        };
        let file = UploadFile::read(&path, content_type(&path))
            .map_err(|error| WorkflowError::Failed(error.to_string()))?;
        let dest = request.dest.map(DestId::as_str);
        let id = self
            .queue
            .hand_off(file, dest, None)
            .map_err(|error| WorkflowError::Failed(error.to_string()))?;
        Ok(UploadId::new(id))
    }
}

pub struct ServiceTextPort {
    ocr: Arc<dyn Ocr + Send + Sync>,
}

impl ServiceTextPort {
    pub fn new(ocr: Arc<dyn Ocr + Send + Sync>) -> Self {
        Self { ocr }
    }
}

impl TextPort for ServiceTextPort {
    fn extract(&self, image: &Image) -> Result<String, WorkflowError> {
        crate::tools::ocr::OcrTextPort::new(self.ocr.as_ref()).extract(image)
    }
}

pub trait WorkflowGate: Send + Sync {
    fn allows(&self, owner: &str) -> Result<bool, ServiceError>;
}

pub struct HotkeyGate {
    services: Services,
}

impl HotkeyGate {
    pub fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }
}

impl WorkflowGate for HotkeyGate {
    fn allows(&self, owner: &str) -> Result<bool, ServiceError> {
        let HotkeysResp::State {
            blocked, paused, ..
        } = self.services.hotkeys().handle(HotkeysReq::Get)?
        else {
            return Ok(false);
        };
        if paused {
            return Ok(false);
        }
        Ok(!blocked.iter().any(|blocked| blocked.owner == owner))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Ran(RunReport),
    Skipped,
}

struct Pending {
    workflow: Workflow,
    context: RunContext,
}

struct RunUpload<'a> {
    port: &'a dyn UploadPort,
    pending: &'a Mutex<HashMap<u64, Pending>>,
    guard: std::cell::RefCell<Option<std::sync::MutexGuard<'a, HashMap<u64, Pending>>>>,
}

impl UploadPort for RunUpload<'_> {
    fn hand_off(&self, request: UploadRequest<'_>) -> Result<UploadId, WorkflowError> {
        let mut guard = self.guard.borrow_mut();
        if guard.is_none() {
            *guard = Some(self.pending.lock().map_err(|_| {
                WorkflowError::Failed("The workflow state is unavailable.".to_owned())
            })?);
        }
        self.port.hand_off(request)
    }
}

struct Inner {
    services: Services,
    ports: RunnerPorts,
    gate: Arc<dyn WorkflowGate>,
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
}

#[derive(Clone)]
pub struct WorkflowRunner {
    inner: Arc<Inner>,
}

impl WorkflowRunner {
    pub fn new(services: &Services, ports: RunnerPorts) -> Self {
        Self {
            inner: Arc::new(Inner {
                services: services.clone(),
                ports,
                gate: Arc::new(HotkeyGate::new(services)),
                pending: Arc::new(Mutex::new(HashMap::new())),
            }),
        }
    }

    pub fn with_gate(self, gate: Arc<dyn WorkflowGate>) -> Self {
        Self {
            inner: Arc::new(Inner {
                services: self.inner.services.clone(),
                ports: self.inner.ports.clone(),
                gate,
                pending: Arc::clone(&self.inner.pending),
            }),
        }
    }

    pub fn run_action(&self, action: &ActionId) -> Result<RunOutcome, ServiceError> {
        let id = action.as_str();
        if let Some(rest) = id.strip_prefix(WORKFLOW_PREFIX) {
            if rest.is_empty() {
                return Err(ServiceError::Invalid(
                    "The workflow was not found.".to_owned(),
                ));
            }
            return self.run_workflow(rest);
        }
        if let Some(kind) = capture_kind_for(id) {
            if !self.allowed(id)? {
                return Ok(RunOutcome::Skipped);
            }
            return self.run_default_capture(kind);
        }
        Err(ServiceError::NotYet)
    }

    pub fn run_workflow(&self, id: &str) -> Result<RunOutcome, ServiceError> {
        let owner = format!("{WORKFLOW_PREFIX}{id}");
        if !self.allowed(&owner)? {
            return Ok(RunOutcome::Skipped);
        }
        if id == snapline_core::workflow::DEFAULT_WORKFLOW_ID {
            return self.run_default_from_capture();
        }
        let workflow = self.load_workflow(id)?;
        self.execute(&workflow)
    }

    pub fn resume_upload(
        &self,
        upload_id: u64,
        link: impl Into<String>,
        page_link: Option<String>,
        file_name: Option<String>,
    ) -> Result<Option<RunReport>, ServiceError> {
        let pending = self
            .inner
            .pending
            .lock()
            .map_err(|_| ServiceError::Internal("The workflow state is unavailable.".to_owned()))?
            .remove(&upload_id);
        let Some(pending) = pending else {
            return Ok(None);
        };
        let ports = self.inner.ports.core();
        let mut context = pending.context;
        let uploaded = Uploaded {
            upload_id: UploadId::new(upload_id),
            link: link.into(),
            page_link,
            file_name,
        };
        match workflow::resume_after_upload(&pending.workflow, &ports, &mut context, uploaded) {
            Ok(report) => Ok(Some(report)),
            Err(error) => Err(map_run_error(error)),
        }
    }

    pub fn publish_recording(&self, saved: &SavedRecording) -> Result<(), ServiceError> {
        let settings = self.inner.services.settings_snapshot()?;
        let local = self.recording_thumbnail(saved);
        if !settings.uploads.videos_and_gifs.is_upload() {
            if settings.general.notifications {
                self.inner
                    .ports
                    .notify
                    .toast_rich(recording_saved_notice(saved, local));
            }
            return Ok(());
        }
        let copy_link: Vec<Step> = settings
            .workflows
            .list
            .iter()
            .find(|workflow| workflow.is_default())
            .cloned()
            .unwrap_or_else(|| default_from_general(&settings.general.default_workflow))
            .steps
            .into_iter()
            .filter(|step| matches!(step, Step::CopyLink { .. }))
            .collect();
        match self.hand_off_recording(saved, copy_link) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.inner
                    .ports
                    .notify
                    .toast_rich(recording_upload_failed_notice(
                        saved,
                        &error.to_string(),
                        local,
                    ));
                Err(error)
            }
        }
    }

    pub fn recording_failed(&self, reason: &str) {
        self.inner
            .ports
            .notify
            .toast(snapline_core::ports::Notice::new(
                "Recording failed",
                reason.trim(),
            ));
    }

    pub fn recording_cancelled(&self) {
        let notifications = self
            .inner
            .services
            .settings_snapshot()
            .map(|settings| settings.general.notifications)
            .unwrap_or(true);
        if notifications {
            self.inner
                .ports
                .notify
                .toast(snapline_core::ports::Notice::new(
                    "Recording cancelled",
                    "The recording was discarded.",
                ));
        }
    }

    fn hand_off_recording(
        &self,
        saved: &SavedRecording,
        copy_link: Vec<Step>,
    ) -> Result<(), ServiceError> {
        let mut pending =
            self.inner.pending.lock().map_err(|_| {
                ServiceError::Internal("The workflow state is unavailable.".to_owned())
            })?;
        let upload_id = self
            .inner
            .ports
            .upload
            .hand_off(UploadRequest {
                dest: None,
                image: None,
                saved_file: Some(&saved.path),
                recording: None,
                converted: None,
                file_name: Some(&saved.file_name),
                source_app: None,
            })
            .map_err(|error| ServiceError::Invalid(error.to_string()))?;
        if copy_link.is_empty() {
            return Ok(());
        }
        let mut workflow = Workflow::new("recording-upload", "Recording upload");
        workflow.steps = std::iter::once(Step::Upload { dest: None })
            .chain(copy_link)
            .collect();
        let context = RunContext {
            saved_file: Some(saved.path.clone()),
            file_name: Some(saved.file_name.clone()),
            upload_id: Some(upload_id),
            ..RunContext::default()
        };
        pending.insert(upload_id.get(), Pending { workflow, context });
        Ok(())
    }

    fn recording_thumbnail(&self, saved: &SavedRecording) -> Option<PathBuf> {
        let id = saved.capture_id?;
        let paths = self.inner.services.paths().ok()?;
        crate::thumbs::existing_thumbnail(&paths.thumbs_dir, id).map(|(path, _)| path)
    }

    fn allowed(&self, owner: &str) -> Result<bool, ServiceError> {
        self.inner.gate.allows(owner)
    }

    fn load_workflow(&self, id: &str) -> Result<Workflow, ServiceError> {
        self.inner
            .services
            .settings_snapshot()?
            .workflows
            .list
            .into_iter()
            .find(|workflow| workflow.id.as_str() == id)
            .ok_or(ServiceError::NotFound)
    }

    pub fn run_default_from_capture(&self) -> Result<RunOutcome, ServiceError> {
        self.run_default_capture(CaptureKind::Region)
    }

    pub fn run_default_capture(&self, kind: CaptureKind) -> Result<RunOutcome, ServiceError> {
        let settings = self.inner.services.settings_snapshot()?;
        let mut workflow = settings
            .workflows
            .list
            .into_iter()
            .find(Workflow::is_default)
            .unwrap_or_else(|| default_from_general(&settings.general.default_workflow));
        if !settings.uploads.images.is_upload() {
            let mut skipped_upload = false;
            workflow.steps.retain(|step| match step {
                Step::Upload { dest: None } => {
                    skipped_upload = true;
                    false
                }
                Step::Upload { .. } => {
                    skipped_upload = false;
                    true
                }
                Step::CopyLink { .. } if skipped_upload => false,
                _ => true,
            });
        }
        let mut has_upload = false;
        workflow.steps.retain(|step| match step {
            Step::Upload { .. } => {
                has_upload = true;
                true
            }
            Step::CopyLink { .. } => has_upload,
            _ => true,
        });
        match workflow.steps.first_mut() {
            Some(Step::Capture { kind: first }) => *first = kind,
            Some(Step::Record { .. }) => {}
            _ => workflow.steps.insert(0, Step::Capture { kind }),
        }
        self.execute(&workflow)
    }

    fn execute(&self, workflow: &Workflow) -> Result<RunOutcome, ServiceError> {
        if !workflow.enabled {
            return Ok(RunOutcome::Skipped);
        }
        let upload = RunUpload {
            port: self.inner.ports.upload.as_ref(),
            pending: &self.inner.pending,
            guard: std::cell::RefCell::new(None),
        };
        let mut ports = self.inner.ports.core();
        ports.upload = &upload;
        let mut context = RunContext::default();
        match workflow::run(workflow, &ports, &mut context) {
            Ok(report) => {
                if let Some(upload_id) = report.upload_id {
                    if !report.deferred.is_empty() {
                        if let Some(pending) = upload.guard.borrow_mut().as_mut() {
                            context.image = None;
                            pending.insert(
                                upload_id.get(),
                                Pending {
                                    workflow: workflow.clone(),
                                    context,
                                },
                            );
                        }
                    }
                }
                Ok(RunOutcome::Ran(report))
            }
            Err(error) => {
                if (context.explicit_capture_output || context.saved_file.is_some())
                    && !matches!(error.kind, workflow::RunErrorKind::Cancelled { .. })
                    && !crate::recording::was_reported(&error.to_string())
                {
                    self.inner
                        .ports
                        .notify
                        .toast(snapline_core::ports::Notice::new(
                            "Workflow could not be completed",
                            error.to_string(),
                        ));
                }
                Err(map_run_error(error))
            }
        }
    }
}

fn capture_kind_for(id: &str) -> Option<CaptureKind> {
    match id {
        "capture_region" => Some(CaptureKind::Region),
        "capture_window" => Some(CaptureKind::Window),
        "capture_screen" => Some(CaptureKind::Screen),
        "capture_last_region" => Some(CaptureKind::LastRegion),
        _ => None,
    }
}

fn map_run_error(error: RunError) -> ServiceError {
    ServiceError::Invalid(error.to_string())
}

static RUNNER: OnceLock<Mutex<Option<WorkflowRunner>>> = OnceLock::new();

pub fn install_runner(runner: WorkflowRunner) -> Result<(), ServiceError> {
    let slot = RUNNER.get_or_init(|| Mutex::new(None));
    let mut guard = slot
        .lock()
        .map_err(|_| ServiceError::Internal("The workflow runner is unavailable.".to_owned()))?;
    if guard.is_some() {
        return Err(ServiceError::Internal(
            "The workflow runner is already installed.".to_owned(),
        ));
    }
    *guard = Some(runner);
    Ok(())
}

pub fn installed_runner() -> Result<WorkflowRunner, ServiceError> {
    let slot = RUNNER.get().ok_or(ServiceError::NotYet)?;
    slot.lock()
        .map_err(|_| ServiceError::Internal("The workflow runner is unavailable.".to_owned()))?
        .clone()
        .ok_or(ServiceError::NotYet)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use snapline_core::chord::Chord;
    use snapline_core::hotkey_plan::{Blocked, Owner};
    use snapline_core::ports::{Notice, SoundId};
    use snapline_store::io::Paths;
    use time::macros::datetime;
    use time::OffsetDateTime;

    use super::*;
    use crate::hotkeys::HotkeyInput;

    struct FakeGate {
        allowed: AtomicBool,
    }

    impl FakeGate {
        fn new(allowed: bool) -> Self {
            Self {
                allowed: AtomicBool::new(allowed),
            }
        }
    }

    impl WorkflowGate for FakeGate {
        fn allows(&self, _owner: &str) -> Result<bool, ServiceError> {
            Ok(self.allowed.load(Ordering::SeqCst))
        }
    }

    struct FakeInput {
        external: Vec<Chord>,
    }

    impl HotkeyInput for FakeInput {
        fn rebind(&self, bindings: Vec<(Owner, Chord)>) -> Result<Vec<Blocked>, ServiceError> {
            Ok(bindings
                .into_iter()
                .filter(|(_, chord)| self.external.contains(chord))
                .map(|(owner, chord)| Blocked::external(chord, owner))
                .collect())
        }

        fn pause(&self) -> Result<(), ServiceError> {
            Ok(())
        }

        fn resume(&self) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    struct FakePorts {
        log: Mutex<Vec<String>>,
        images: Mutex<Vec<Image>>,
        texts: Mutex<Vec<String>>,
        uploads: Mutex<Vec<Option<String>>>,
        rich: Mutex<Vec<snapline_core::ports::RichNotice>>,
        next_upload: AtomicU64,
        capture_available: AtomicBool,
        text_available: AtomicBool,
        save_available: AtomicBool,
        upload_available: AtomicBool,
    }

    impl Default for FakePorts {
        fn default() -> Self {
            Self {
                log: Mutex::new(Vec::new()),
                images: Mutex::new(Vec::new()),
                texts: Mutex::new(Vec::new()),
                uploads: Mutex::new(Vec::new()),
                rich: Mutex::new(Vec::new()),
                next_upload: AtomicU64::new(7),
                capture_available: AtomicBool::new(true),
                text_available: AtomicBool::new(true),
                save_available: AtomicBool::new(true),
                upload_available: AtomicBool::new(true),
            }
        }
    }

    impl FakePorts {
        fn log(&self, entry: impl Into<String>) {
            self.log.lock().expect("log").push(entry.into());
        }

        fn entries(&self) -> Vec<String> {
            self.log.lock().expect("log").clone()
        }

        fn ports(self: &Arc<Self>) -> RunnerPorts {
            RunnerPorts {
                capture: self.clone(),
                record: self.clone(),
                annotate: self.clone(),
                save: self.clone(),
                upload: self.clone(),
                clipboard: self.clone(),
                text: self.clone(),
                gif: self.clone(),
                notify: self.clone(),
                sound: self.clone(),
                clock: self.clone(),
            }
        }
    }

    fn sample_image() -> Image {
        Image::bgra(2, 2).expect("image")
    }

    impl CapturePort for FakePorts {
        fn capture(&self, kind: CaptureKind) -> Result<Image, WorkflowError> {
            self.log(format!("capture:{}", kind.label().to_ascii_lowercase()));
            if !self.capture_available.load(Ordering::SeqCst) {
                return Err(WorkflowError::NotAvailableYet);
            }
            Ok(sample_image())
        }
    }

    impl RecordPort for FakePorts {
        fn record(&self, kind: RecordKind) -> Result<Recorded, WorkflowError> {
            self.log(format!("record:{}", kind.label().to_ascii_lowercase()));
            Err(WorkflowError::NotAvailableYet)
        }
    }

    impl AnnotatePort for FakePorts {
        fn annotate(&self, image: &Image) -> Result<Image, WorkflowError> {
            self.log("annotate");
            Ok(image.clone())
        }
    }

    impl SavePort for FakePorts {
        fn save(&self, request: SaveRequest<'_>) -> Result<PathBuf, WorkflowError> {
            self.log("save");
            if !self.save_available.load(Ordering::SeqCst) || request.image.is_none() {
                return Err(WorkflowError::NotAvailableYet);
            }
            Ok(PathBuf::from("C:/captures/shot.png"))
        }
    }

    impl UploadPort for FakePorts {
        fn hand_off(&self, request: UploadRequest<'_>) -> Result<UploadId, WorkflowError> {
            self.log("upload");
            if !self.upload_available.load(Ordering::SeqCst) {
                return Err(WorkflowError::NotAvailableYet);
            }
            self.uploads
                .lock()
                .expect("uploads")
                .push(request.dest.map(|dest| dest.as_str().to_owned()));
            Ok(UploadId::new(
                self.next_upload.fetch_add(1, Ordering::SeqCst),
            ))
        }
    }

    impl Clipboard for FakePorts {
        fn set_image(&self, image: &Image) -> Result<(), ClipboardError> {
            self.log("copy-image");
            self.images.lock().expect("images").push(image.clone());
            Ok(())
        }

        fn set_text(&self, text: &str) -> Result<(), ClipboardError> {
            self.log(format!("copy-text:{text}"));
            self.texts.lock().expect("texts").push(text.to_owned());
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    impl TextPort for FakePorts {
        fn extract(&self, _image: &Image) -> Result<String, WorkflowError> {
            self.log("extract");
            if !self.text_available.load(Ordering::SeqCst) {
                return Err(WorkflowError::NotAvailableYet);
            }
            Ok("recognised text".to_owned())
        }
    }

    impl GifPort for FakePorts {
        fn convert(&self, _recording: &Path) -> Result<PathBuf, WorkflowError> {
            self.log("gif");
            Err(WorkflowError::NotAvailableYet)
        }
    }

    impl Notifier for FakePorts {
        fn toast(&self, _notice: Notice) {
            self.log("notify");
        }

        fn toast_rich(&self, notice: snapline_core::ports::RichNotice) {
            self.log(format!("rich:{}", notice.title));
            self.rich.lock().expect("rich").push(notice);
        }
    }

    impl Sound for FakePorts {
        fn play(&self, _sound: SoundId) {
            self.log("sound");
        }
    }

    impl Clock for FakePorts {
        fn now_local(&self) -> OffsetDateTime {
            datetime!(2026-11-09 12:00:00 UTC)
        }
    }

    fn paths(directory: &Path) -> Paths {
        Paths::resolve(
            directory,
            &directory.join("Roaming"),
            &directory.join("Local"),
            &directory.join("Profile"),
            false,
        )
    }

    fn services(directory: &Path) -> Services {
        Services::with_paths(paths(directory)).expect("services")
    }

    fn seed(services: &Services, workflow: serde_json::Value) {
        services
            .update_settings(&serde_json::json!({ "workflows": { "list": [workflow] } }))
            .expect("seed");
    }

    fn flow(id: &str, name: &str, enabled: bool, steps: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "name": name,
            "hotkey": null,
            "enabled": enabled,
            "steps": steps,
        })
    }

    fn runner(services: &Services, ports: &Arc<FakePorts>, allowed: bool) -> WorkflowRunner {
        WorkflowRunner::new(services, ports.ports()).with_gate(Arc::new(FakeGate::new(allowed)))
    }

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("chord")
    }

    #[test]
    fn a_workflow_runs_its_steps_in_order() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "save-quietly",
                "Save quietly",
                true,
                serde_json::json!([
                    { "step": "capture", "kind": "region" },
                    { "step": "save" },
                    { "step": "copy_image" },
                    { "step": "notify" },
                    { "step": "play_sound" }
                ]),
            ),
        );
        let outcome = runner(&services, &ports, true)
            .run_workflow("save-quietly")
            .expect("run");
        let RunOutcome::Ran(report) = outcome else {
            panic!("the workflow was skipped");
        };
        assert_eq!(report.outcomes.len(), 5);
        assert_eq!(
            ports.entries(),
            vec![
                "capture:region".to_owned(),
                "save".to_owned(),
                "copy-image".to_owned(),
                "notify".to_owned(),
                "sound".to_owned(),
            ]
        );
    }

    #[test]
    fn a_not_available_step_stops_before_later_steps() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        ports.text_available.store(false, Ordering::SeqCst);
        seed(
            &services,
            flow(
                "grab-text",
                "Grab text",
                true,
                serde_json::json!([
                    { "step": "capture", "kind": "region" },
                    { "step": "extract_text" },
                    { "step": "copy_text" },
                    { "step": "notify" }
                ]),
            ),
        );
        let error = runner(&services, &ports, true)
            .run_workflow("grab-text")
            .expect_err("not available");
        assert_eq!(error.to_string(), "Extract text is not available yet");
        assert_eq!(
            ports.entries(),
            vec!["capture:region".to_owned(), "extract".to_owned()]
        );
    }

    #[test]
    fn a_saved_scrolling_workflow_runs_as_a_region_capture() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "scroll",
                "Old scrolling capture",
                true,
                serde_json::json!([
                    { "step": "capture", "kind": "scrolling" },
                    { "step": "save" }
                ]),
            ),
        );
        runner(&services, &ports, true)
            .run_workflow("scroll")
            .expect("runs");
        assert_eq!(ports.entries()[0], "capture:region".to_owned());
    }

    #[test]
    fn the_default_workflow_runs_from_capture_region_after_the_capture() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "default",
                "Screenshot to link",
                true,
                serde_json::json!([
                    { "step": "copy_image" },
                    { "step": "notify" }
                ]),
            ),
        );
        let outcome = runner(&services, &ports, true)
            .run_action(&ActionId::new("capture_region"))
            .expect("run");
        assert!(matches!(outcome, RunOutcome::Ran(_)));
        assert_eq!(
            ports.entries(),
            vec![
                "capture:region".to_owned(),
                "copy-image".to_owned(),
                "notify".to_owned(),
            ]
        );
        assert_eq!(ports.images.lock().expect("images").len(), 1);
    }

    #[test]
    fn default_settings_skip_upload_and_apply_annotation_toggle() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        let mut steps = snapline_store::settings::general::default_workflow();
        steps
            .iter_mut()
            .find(|step| step.kind == snapline_store::settings::general::WorkflowStepKind::Annotate)
            .expect("annotate")
            .enabled = true;
        services
            .update_settings(&serde_json::json!({ "general": { "default_workflow": steps } }))
            .expect("settings");
        let result = runner(&services, &ports, true)
            .run_action(&ActionId::new("capture_region"))
            .expect("run");
        assert!(matches!(result, RunOutcome::Ran(_)));
        let entries = ports.entries();
        assert!(entries.iter().any(|entry| entry == "annotate"));
        assert!(!entries.iter().any(|entry| entry.starts_with("upload")));
    }

    fn saved_recording() -> SavedRecording {
        SavedRecording {
            capture_id: Some(4),
            path: PathBuf::from(r"C:\Pics\clip.mp4"),
            file_name: "clip.mp4".to_owned(),
            bytes: 2_097_152,
            width: 640,
            height: 360,
            duration_ms: 12_000,
            kind: crate::recording::RecordedKind::Video,
        }
    }

    fn choose_video_destination(services: &Services) {
        services
            .update_settings(&serde_json::json!({
                "uploads": { "videos_and_gifs": "my-host" }
            }))
            .expect("settings");
    }

    #[test]
    fn a_recording_without_a_destination_shows_a_recording_saved_toast() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        runner(&services, &ports, true)
            .publish_recording(&saved_recording())
            .expect("publish");
        assert_eq!(ports.entries(), ["rich:Recording saved"]);
        let rich = ports.rich.lock().expect("rich");
        assert_eq!(rich[0].lines, ["clip.mp4", "2.0 MB, 0:12"]);
        assert_eq!(rich[0].tag.as_deref(), Some("capture-4"));
        assert_eq!(rich[0].buttons[0].label, "Open video");
        assert_eq!(rich[0].buttons[1].label, "Open folder");
    }

    #[test]
    fn a_cancelled_recording_shows_one_short_notice_with_no_buttons() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        runner(&services, &ports, true).recording_cancelled();
        assert_eq!(ports.entries(), ["notify"]);
        assert!(ports.rich.lock().expect("rich").is_empty());
    }

    #[test]
    fn a_recording_with_notifications_off_stays_quiet() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        services
            .update_settings(&serde_json::json!({ "general": { "notifications": false } }))
            .expect("settings");
        let ports = Arc::new(FakePorts::default());
        runner(&services, &ports, true)
            .publish_recording(&saved_recording())
            .expect("publish");
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn a_recording_with_a_destination_uploads_and_copies_the_link_without_a_saved_toast() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        choose_video_destination(&services);
        let ports = Arc::new(FakePorts::default());
        let runner = runner(&services, &ports, true);
        runner
            .publish_recording(&saved_recording())
            .expect("publish");
        assert_eq!(ports.entries(), ["upload"]);
        assert!(ports.texts.lock().expect("texts").is_empty());
        runner
            .resume_upload(
                7,
                "https://i.example.com/clip.mp4",
                None,
                Some("clip.mp4".to_owned()),
            )
            .expect("resume")
            .expect("pending run");
        assert_eq!(
            *ports.texts.lock().expect("texts"),
            ["https://i.example.com/clip.mp4"]
        );
        assert!(!ports
            .entries()
            .iter()
            .any(|entry| entry.starts_with("rich:")));
    }

    #[test]
    fn a_recording_upload_that_cannot_start_shows_upload_failed() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        choose_video_destination(&services);
        let ports = Arc::new(FakePorts::default());
        ports.upload_available.store(false, Ordering::SeqCst);
        let result = runner(&services, &ports, true).publish_recording(&saved_recording());
        assert!(result.is_err());
        assert_eq!(ports.entries(), ["upload", "rich:Upload failed"]);
        let rich = ports.rich.lock().expect("rich");
        assert_eq!(rich[0].tag.as_deref(), Some("capture-4"));
        assert_eq!(rich[0].buttons[0].label, "Open video");
    }

    #[test]
    fn the_window_screen_and_last_region_actions_run_the_default_workflow() {
        for (action, label) in [
            ("capture_window", "capture:window"),
            ("capture_screen", "capture:screen"),
            ("capture_last_region", "capture:last region"),
        ] {
            let directory = tempfile::tempdir().expect("temp");
            let services = services(directory.path());
            let ports = Arc::new(FakePorts::default());
            let outcome = runner(&services, &ports, true)
                .run_action(&ActionId::new(action))
                .expect("run");
            assert!(matches!(outcome, RunOutcome::Ran(_)), "{action}");
            let entries = ports.entries();
            assert_eq!(entries.first().map(String::as_str), Some(label), "{action}");
            assert!(entries.iter().any(|entry| entry == "save"), "{action}");
        }
    }

    #[test]
    fn a_blocked_window_capture_is_skipped() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        let outcome = runner(&services, &ports, false)
            .run_action(&ActionId::new("capture_window"))
            .expect("run");
        assert_eq!(outcome, RunOutcome::Skipped);
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn a_blocked_workflow_does_not_run() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "save-quietly",
                "Save quietly",
                true,
                serde_json::json!([
                    { "step": "capture", "kind": "screen" },
                    { "step": "notify" }
                ]),
            ),
        );
        let outcome = runner(&services, &ports, false)
            .run_action(&ActionId::new("workflow:save-quietly"))
            .expect("run");
        assert_eq!(outcome, RunOutcome::Skipped);
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn a_disabled_workflow_does_not_run() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "save-quietly",
                "Save quietly",
                false,
                serde_json::json!([
                    { "step": "capture", "kind": "screen" },
                    { "step": "notify" }
                ]),
            ),
        );
        let outcome = runner(&services, &ports, true)
            .run_action(&ActionId::new("workflow:save-quietly"))
            .expect("run");
        assert_eq!(outcome, RunOutcome::Skipped);
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn an_upload_hands_off_and_copy_link_resumes_with_the_run_id() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "screenshot",
                "Screenshot to link",
                true,
                serde_json::json!([
                    { "step": "capture", "kind": "region" },
                    { "step": "upload" },
                    { "step": "copy_link", "format": "direct" },
                    { "step": "notify" }
                ]),
            ),
        );
        let runner = runner(&services, &ports, true);
        let outcome = runner.run_workflow("screenshot").expect("run");
        let RunOutcome::Ran(report) = outcome else {
            panic!("the workflow was skipped");
        };
        assert_eq!(report.deferred, vec![2]);
        assert_eq!(*ports.uploads.lock().expect("uploads"), vec![None]);
        assert!(ports.texts.lock().expect("texts").is_empty());

        assert_eq!(
            runner
                .resume_upload(99, "https://wrong.example", None, None)
                .expect("wrong run"),
            None
        );
        let resumed = runner
            .resume_upload(
                7,
                "https://i.example.com/shot.png",
                None,
                Some("shot.png".to_owned()),
            )
            .expect("resume")
            .expect("pending run");
        assert_eq!(resumed.outcomes.len(), 1);
        assert_eq!(
            ports.texts.lock().expect("texts").as_slice(),
            &["https://i.example.com/shot.png".to_owned()]
        );
    }

    #[test]
    fn a_paused_hotkey_plan_skips_the_default_workflow() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path()).with_hotkey_input(Arc::new(FakeInput {
            external: Vec::new(),
        }));
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "default",
                "Screenshot to link",
                true,
                serde_json::json!([
                    { "step": "copy_image" },
                    { "step": "notify" }
                ]),
            ),
        );
        services.hotkeys().set_paused(true).expect("pause");
        let outcome = WorkflowRunner::new(&services, ports.ports())
            .run_action(&ActionId::new("capture_region"))
            .expect("run");
        assert_eq!(outcome, RunOutcome::Skipped);
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn an_externally_blocked_workflow_does_not_run() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path()).with_hotkey_input(Arc::new(FakeInput {
            external: vec![chord("Ctrl+Shift+A")],
        }));
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            serde_json::json!({
                "id": "annotate-and-share",
                "name": "Annotate and share",
                "hotkey": "Ctrl+Shift+A",
                "enabled": true,
                "steps": [
                    { "step": "capture", "kind": "region" },
                    { "step": "notify" }
                ],
            }),
        );
        services.hotkeys().on_settings_changed().expect("refresh");
        let outcome = WorkflowRunner::new(&services, ports.ports())
            .run_action(&ActionId::new("workflow:annotate-and-share"))
            .expect("run");
        assert_eq!(outcome, RunOutcome::Skipped);
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn a_workflow_clashing_with_an_action_does_not_run() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path()).with_hotkey_input(Arc::new(FakeInput {
            external: Vec::new(),
        }));
        let ports = Arc::new(FakePorts::default());
        services
            .update_settings(&serde_json::json!({
                "hotkeys": {
                    "bindings": { "capture_window": "Ctrl+Shift+A" }
                },
                "workflows": {
                    "list": [{
                        "id": "annotate-and-share",
                        "name": "Annotate and share",
                        "hotkey": "Ctrl+Shift+A",
                        "enabled": true,
                        "steps": [
                            { "step": "capture", "kind": "region" },
                            { "step": "notify" }
                        ],
                    }]
                }
            }))
            .expect("settings");
        services.hotkeys().on_settings_changed().expect("refresh");
        let outcome = WorkflowRunner::new(&services, ports.ports())
            .run_action(&ActionId::new("workflow:annotate-and-share"))
            .expect("run");
        assert_eq!(outcome, RunOutcome::Skipped);
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn the_default_does_not_run_when_prtsc_is_externally_blocked() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path()).with_hotkey_input(Arc::new(FakeInput {
            external: vec![chord("PrtSc")],
        }));
        let ports = Arc::new(FakePorts::default());
        seed(
            &services,
            flow(
                "default",
                "Screenshot to link",
                true,
                serde_json::json!([
                    { "step": "copy_image" },
                    { "step": "notify" }
                ]),
            ),
        );
        services.hotkeys().on_settings_changed().expect("refresh");
        let outcome = WorkflowRunner::new(&services, ports.ports())
            .run_action(&ActionId::new("capture_region"))
            .expect("run");
        assert_eq!(outcome, RunOutcome::Skipped);
        assert!(ports.entries().is_empty());
    }

    #[test]
    fn recording_ports_report_not_available_without_the_recorder() {
        let directory = tempfile::tempdir().expect("temp");
        let services = services(directory.path());
        let record = ServiceRecordPort::new(&services);
        assert!(matches!(
            record.record(RecordKind::Video),
            Err(WorkflowError::NotAvailableYet)
        ));
        let gif = ServiceGifPort::new(&services);
        assert!(matches!(
            gif.convert(Path::new("C:/Temp/clip.mp4")),
            Err(WorkflowError::NotAvailableYet)
        ));
    }

    #[test]
    fn recording_errors_map_to_workflow_errors() {
        assert!(matches!(
            record_error(RecordError::Cancelled),
            WorkflowError::Cancelled
        ));
        assert!(matches!(
            record_error(RecordError::NotYet),
            WorkflowError::NotAvailableYet
        ));
        assert!(matches!(
            record_error(RecordError::NoTarget),
            WorkflowError::Failed(_)
        ));
    }
}
