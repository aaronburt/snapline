use std::path::{Path, PathBuf};

use thiserror::Error;
use time::OffsetDateTime;

use crate::image::Image;
use crate::ports::{Notice, SoundId};

use super::{
    Step, UploadId, Violation, Workflow, WorkflowError, WorkflowId, WorkflowPorts,
};

pub struct SaveRequest<'a> {
    pub image: Option<&'a Image>,
    pub recording: Option<&'a Path>,
    pub converted: Option<&'a Path>,
    pub file_name: Option<&'a str>,
}

pub struct UploadRequest<'a> {
    pub dest: Option<&'a super::DestId>,
    pub image: Option<&'a Image>,
    pub saved_file: Option<&'a Path>,
    pub recording: Option<&'a Path>,
    pub converted: Option<&'a Path>,
    pub file_name: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploaded {
    pub upload_id: UploadId,
    pub link: String,
    pub page_link: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunContext {
    pub image: Option<Image>,
    pub recording: Option<PathBuf>,
    pub converted: Option<PathBuf>,
    pub saved_file: Option<PathBuf>,
    pub file_name: Option<String>,
    pub link: Option<String>,
    pub page_link: Option<String>,
    pub text: Option<String>,
    pub upload_id: Option<UploadId>,
    pub copied_image: bool,
    pub copied_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Done,
    HandedOff { upload_id: UploadId },
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepOutcome {
    pub index: usize,
    pub step: Step,
    pub status: StepStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub workflow: WorkflowId,
    pub started_at: OffsetDateTime,
    pub finished_at: OffsetDateTime,
    pub outcomes: Vec<StepOutcome>,
    pub deferred: Vec<usize>,
    pub image: bool,
    pub recording: Option<PathBuf>,
    pub converted: Option<PathBuf>,
    pub saved_file: Option<PathBuf>,
    pub link: Option<String>,
    pub text: Option<String>,
    pub upload_id: Option<UploadId>,
    pub copied_image: bool,
    pub copied_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunErrorKind {
    #[error("This workflow is off.")]
    Disabled,
    #[error("This workflow is not valid.")]
    Invalid(Vec<Violation>),
    #[error("{step} is not available yet")]
    NotAvailableYet { step: &'static str },
    #[error("Step {number} ({step}) failed: {message}")]
    Step {
        number: usize,
        step: &'static str,
        message: String,
    },
    #[error("{step} was cancelled")]
    Cancelled { step: &'static str },
    #[error("This upload does not belong to this workflow run.")]
    UploadMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind}")]
pub struct RunError {
    pub kind: RunErrorKind,
    pub report: RunReport,
}

pub fn run(
    workflow: &Workflow,
    ports: &WorkflowPorts<'_>,
    context: &mut RunContext,
) -> Result<RunReport, RunError> {
    let started_at = ports.clock.now_local();
    let mut outcomes = Vec::new();
    let mut deferred = Vec::new();
    if !workflow.enabled {
        return Err(RunError {
            kind: RunErrorKind::Disabled,
            report: build_report(
                workflow,
                started_at,
                started_at,
                outcomes,
                deferred,
                context,
            ),
        });
    }
    let violations = workflow.validate();
    if !violations.is_empty() {
        return Err(RunError {
            kind: RunErrorKind::Invalid(violations),
            report: build_report(
                workflow,
                started_at,
                started_at,
                outcomes,
                deferred,
                context,
            ),
        });
    }
    for (index, step) in workflow.steps.iter().enumerate() {
        match execute_step(index, step, workflow, ports, context) {
            Ok(status) => {
                if status == StepStatus::Deferred {
                    deferred.push(index);
                }
                outcomes.push(StepOutcome {
                    index,
                    step: step.clone(),
                    status,
                });
            }
            Err(kind) => {
                let finished_at = ports.clock.now_local();
                return Err(RunError {
                    kind,
                    report: build_report(
                        workflow,
                        started_at,
                        finished_at,
                        outcomes,
                        deferred,
                        context,
                    ),
                });
            }
        }
    }
    let finished_at = ports.clock.now_local();
    Ok(build_report(
        workflow,
        started_at,
        finished_at,
        outcomes,
        deferred,
        context,
    ))
}

pub fn resume_after_upload(
    workflow: &Workflow,
    ports: &WorkflowPorts<'_>,
    context: &mut RunContext,
    uploaded: Uploaded,
) -> Result<RunReport, RunError> {
    let started_at = ports.clock.now_local();
    if context.upload_id != Some(uploaded.upload_id) {
        return Err(RunError {
            kind: RunErrorKind::UploadMismatch,
            report: build_report(
                workflow,
                started_at,
                started_at,
                Vec::new(),
                Vec::new(),
                context,
            ),
        });
    }
    context.link = Some(uploaded.link);
    if uploaded.page_link.is_some() {
        context.page_link = uploaded.page_link;
    }
    if uploaded.file_name.is_some() {
        context.file_name = uploaded.file_name;
    }
    let mut outcomes = Vec::new();
    for (index, step) in workflow.steps.iter().enumerate() {
        let Step::CopyLink { format } = step else {
            continue;
        };
        let link = context.link.as_deref().unwrap_or_default();
        let file_name = context.file_name.as_deref().unwrap_or("");
        let formatted = format.format(link, file_name, context.page_link.as_deref());
        if let Err(error) = ports.clipboard.set_text(&formatted) {
            let finished_at = ports.clock.now_local();
            return Err(RunError {
                kind: RunErrorKind::Step {
                    number: index + 1,
                    step: step.label(),
                    message: error.to_string(),
                },
                report: build_report(
                    workflow,
                    started_at,
                    finished_at,
                    outcomes,
                    Vec::new(),
                    context,
                ),
            });
        }
        context.copied_text = Some(formatted);
        outcomes.push(StepOutcome {
            index,
            step: step.clone(),
            status: StepStatus::Done,
        });
    }
    let finished_at = ports.clock.now_local();
    Ok(build_report(
        workflow,
        started_at,
        finished_at,
        outcomes,
        Vec::new(),
        context,
    ))
}

fn execute_step(
    index: usize,
    step: &Step,
    workflow: &Workflow,
    ports: &WorkflowPorts<'_>,
    context: &mut RunContext,
) -> Result<StepStatus, RunErrorKind> {
    match step {
        Step::Capture { kind } => {
            let image = ports
                .capture
                .capture(*kind)
                .map_err(|error| port_error(index, step, error))?;
            context.image = Some(image);
            Ok(StepStatus::Done)
        }
        Step::Record { kind } => {
            let recorded = ports
                .record
                .record(*kind)
                .map_err(|error| port_error(index, step, error))?;
            context.recording = Some(recorded.path);
            Ok(StepStatus::Done)
        }
        Step::CopyImage => {
            let image = context
                .image
                .as_ref()
                .ok_or_else(|| missing(index, step, "There is no image to copy."))?;
            ports
                .clipboard
                .set_image(image)
                .map_err(|error| failure(index, step, error.to_string()))?;
            context.copied_image = true;
            Ok(StepStatus::Done)
        }
        Step::Annotate => {
            let annotated = {
                let image = context
                    .image
                    .as_ref()
                    .ok_or_else(|| missing(index, step, "There is no image to annotate."))?;
                ports
                    .annotate
                    .annotate(image)
                    .map_err(|error| port_error(index, step, error))?
            };
            context.image = Some(annotated);
            Ok(StepStatus::Done)
        }
        Step::Save => {
            let request = SaveRequest {
                image: context.image.as_ref(),
                recording: context.recording.as_deref(),
                converted: context.converted.as_deref(),
                file_name: context.file_name.as_deref(),
            };
            if request.image.is_none() && request.recording.is_none() && request.converted.is_none()
            {
                return Err(missing(index, step, "There is nothing to save."));
            }
            let path = ports
                .save
                .save(request)
                .map_err(|error| port_error(index, step, error))?;
            context.saved_file = Some(path);
            Ok(StepStatus::Done)
        }
        Step::Upload { dest } => {
            let request = UploadRequest {
                dest: dest.as_ref(),
                image: context.image.as_ref(),
                saved_file: context.saved_file.as_deref(),
                recording: context.recording.as_deref(),
                converted: context.converted.as_deref(),
                file_name: context.file_name.as_deref(),
            };
            if request.image.is_none()
                && request.saved_file.is_none()
                && request.recording.is_none()
                && request.converted.is_none()
            {
                return Err(missing(index, step, "There is nothing to upload."));
            }
            let upload_id = ports
                .upload
                .hand_off(request)
                .map_err(|error| port_error(index, step, error))?;
            context.upload_id = Some(upload_id);
            Ok(StepStatus::HandedOff { upload_id })
        }
        Step::CopyLink { format } => match context.link.as_deref() {
            Some(link) => {
                let file_name = context.file_name.as_deref().unwrap_or("");
                let formatted = format.format(link, file_name, context.page_link.as_deref());
                ports
                    .clipboard
                    .set_text(&formatted)
                    .map_err(|error| failure(index, step, error.to_string()))?;
                context.copied_text = Some(formatted);
                Ok(StepStatus::Done)
            }
            None if context.upload_id.is_some() => Ok(StepStatus::Deferred),
            None => Err(missing(index, step, "There is no link to copy.")),
        },
        Step::ExtractText => {
            let text = {
                let image = context
                    .image
                    .as_ref()
                    .ok_or_else(|| missing(index, step, "There is no image to read."))?;
                ports
                    .text
                    .extract(image)
                    .map_err(|error| port_error(index, step, error))?
            };
            context.text = Some(text);
            Ok(StepStatus::Done)
        }
        Step::CopyText => {
            let text = context
                .text
                .as_deref()
                .ok_or_else(|| missing(index, step, "There is no text to copy."))?;
            ports
                .clipboard
                .set_text(text)
                .map_err(|error| failure(index, step, error.to_string()))?;
            context.copied_text = Some(text.to_string());
            Ok(StepStatus::Done)
        }
        Step::ConvertToGif => {
            let converted = {
                let recording = context
                    .recording
                    .as_deref()
                    .ok_or_else(|| missing(index, step, "There is no recording to convert."))?;
                ports
                    .gif
                    .convert(recording)
                    .map_err(|error| port_error(index, step, error))?
            };
            context.converted = Some(converted);
            Ok(StepStatus::Done)
        }
        Step::Notify => {
            ports
                .notify
                .toast(Notice::new("Snapline", format!("{} finished.", workflow.name)));
            Ok(StepStatus::Done)
        }
        Step::PlaySound => {
            ports.sound.play(SoundId::Done);
            Ok(StepStatus::Done)
        }
    }
}

fn failure(index: usize, step: &Step, message: impl Into<String>) -> RunErrorKind {
    RunErrorKind::Step {
        number: index + 1,
        step: step.label(),
        message: message.into(),
    }
}

fn missing(index: usize, step: &Step, message: &'static str) -> RunErrorKind {
    failure(index, step, message)
}

fn port_error(index: usize, step: &Step, error: WorkflowError) -> RunErrorKind {
    match error {
        WorkflowError::NotAvailableYet => RunErrorKind::NotAvailableYet {
            step: step.unavailable_label(),
        },
        WorkflowError::Failed(message) => failure(index, step, message),
        WorkflowError::Cancelled => RunErrorKind::Cancelled { step: step.label() },
    }
}

fn build_report(
    workflow: &Workflow,
    started_at: OffsetDateTime,
    finished_at: OffsetDateTime,
    outcomes: Vec<StepOutcome>,
    deferred: Vec<usize>,
    context: &RunContext,
) -> RunReport {
    RunReport {
        workflow: workflow.id.clone(),
        started_at,
        finished_at,
        outcomes,
        deferred,
        image: context.image.is_some(),
        recording: context.recording.clone(),
        converted: context.converted.clone(),
        saved_file: context.saved_file.clone(),
        link: context.link.clone(),
        text: context.text.clone(),
        upload_id: context.upload_id,
        copied_image: context.copied_image,
        copied_text: context.copied_text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use time::macros::datetime;

    use super::*;
    use crate::ids::DestId;
    use crate::link_format::LinkFormat;
    use crate::ports::{Clipboard, ClipboardError, Clock, Notifier, Sound, SoundId};
    use crate::workflow::{
        AnnotatePort, CaptureKind, CapturePort, GifPort, RecordKind, RecordPort, SavePort,
        TextPort, UploadPort, DEFAULT_WORKFLOW_ID, DEFAULT_WORKFLOW_NAME,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Ready,
        NotAvailable,
        Fail,
    }

    struct Fake {
        image: Image,
        capture_calls: RefCell<Vec<CaptureKind>>,
        record_calls: RefCell<Vec<RecordKind>>,
        annotate_calls: Cell<u32>,
        save_calls: Cell<u32>,
        save_had_image: Cell<bool>,
        upload_calls: RefCell<Vec<(Option<DestId>, bool)>>,
        text_calls: Cell<u32>,
        gif_calls: Cell<u32>,
        clipboard_images: RefCell<Vec<Image>>,
        clipboard_texts: RefCell<Vec<String>>,
        notices: RefCell<Vec<Notice>>,
        sounds: RefCell<Vec<SoundId>>,
        capture_mode: Cell<Mode>,
        record_mode: Cell<Mode>,
        annotate_mode: Cell<Mode>,
        save_mode: Cell<Mode>,
        upload_mode: Cell<Mode>,
        text_mode: Cell<Mode>,
        gif_mode: Cell<Mode>,
        clipboard_mode: Cell<Mode>,
        next_upload: Cell<u64>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                image: Image::bgra(2, 2).expect("image"),
                capture_calls: RefCell::new(Vec::new()),
                record_calls: RefCell::new(Vec::new()),
                annotate_calls: Cell::new(0),
                save_calls: Cell::new(0),
                save_had_image: Cell::new(false),
                upload_calls: RefCell::new(Vec::new()),
                text_calls: Cell::new(0),
                gif_calls: Cell::new(0),
                clipboard_images: RefCell::new(Vec::new()),
                clipboard_texts: RefCell::new(Vec::new()),
                notices: RefCell::new(Vec::new()),
                sounds: RefCell::new(Vec::new()),
                capture_mode: Cell::new(Mode::Ready),
                record_mode: Cell::new(Mode::Ready),
                annotate_mode: Cell::new(Mode::Ready),
                save_mode: Cell::new(Mode::Ready),
                upload_mode: Cell::new(Mode::Ready),
                text_mode: Cell::new(Mode::Ready),
                gif_mode: Cell::new(Mode::Ready),
                clipboard_mode: Cell::new(Mode::Ready),
                next_upload: Cell::new(7),
            }
        }

        fn ports(&self) -> WorkflowPorts<'_> {
            WorkflowPorts {
                capture: self,
                record: self,
                annotate: self,
                save: self,
                upload: self,
                clipboard: self,
                text: self,
                gif: self,
                notify: self,
                sound: self,
                clock: self,
            }
        }
    }

    impl CapturePort for Fake {
        fn capture(&self, kind: CaptureKind) -> Result<Image, WorkflowError> {
            self.capture_calls.borrow_mut().push(kind);
            match self.capture_mode.get() {
                Mode::Ready => Ok(self.image.clone()),
                Mode::NotAvailable => Err(WorkflowError::NotAvailableYet),
                Mode::Fail => Err(WorkflowError::Failed("capture failed".to_string())),
            }
        }
    }

    impl RecordPort for Fake {
        fn record(&self, kind: RecordKind) -> Result<Recorded, WorkflowError> {
            self.record_calls.borrow_mut().push(kind);
            match self.record_mode.get() {
                Mode::Ready => Ok(Recorded {
                    path: PathBuf::from("C:/Temp/recording.mp4"),
                }),
                Mode::NotAvailable => Err(WorkflowError::NotAvailableYet),
                Mode::Fail => Err(WorkflowError::Failed("recording failed".to_string())),
            }
        }
    }

    impl AnnotatePort for Fake {
        fn annotate(&self, _image: &Image) -> Result<Image, WorkflowError> {
            self.annotate_calls.set(self.annotate_calls.get() + 1);
            match self.annotate_mode.get() {
                Mode::Ready => Ok(self.image.clone()),
                Mode::NotAvailable => Err(WorkflowError::NotAvailableYet),
                Mode::Fail => Err(WorkflowError::Failed("annotate failed".to_string())),
            }
        }
    }

    impl SavePort for Fake {
        fn save(&self, request: SaveRequest<'_>) -> Result<PathBuf, WorkflowError> {
            self.save_calls.set(self.save_calls.get() + 1);
            self.save_had_image.set(request.image.is_some());
            match self.save_mode.get() {
                Mode::Ready => Ok(PathBuf::from("C:/Pictures/shot.png")),
                Mode::NotAvailable => Err(WorkflowError::NotAvailableYet),
                Mode::Fail => Err(WorkflowError::Failed("save failed".to_string())),
            }
        }
    }

    impl UploadPort for Fake {
        fn hand_off(&self, request: UploadRequest<'_>) -> Result<UploadId, WorkflowError> {
            self.upload_calls
                .borrow_mut()
                .push((request.dest.cloned(), request.image.is_some()));
            match self.upload_mode.get() {
                Mode::Ready => {
                    let id = UploadId::new(self.next_upload.get());
                    self.next_upload.set(id.get() + 1);
                    Ok(id)
                }
                Mode::NotAvailable => Err(WorkflowError::NotAvailableYet),
                Mode::Fail => Err(WorkflowError::Failed("upload failed".to_string())),
            }
        }
    }

    impl TextPort for Fake {
        fn extract(&self, _image: &Image) -> Result<String, WorkflowError> {
            self.text_calls.set(self.text_calls.get() + 1);
            match self.text_mode.get() {
                Mode::Ready => Ok("recognised text".to_string()),
                Mode::NotAvailable => Err(WorkflowError::NotAvailableYet),
                Mode::Fail => Err(WorkflowError::Failed("text failed".to_string())),
            }
        }
    }

    impl GifPort for Fake {
        fn convert(&self, _recording: &Path) -> Result<PathBuf, WorkflowError> {
            self.gif_calls.set(self.gif_calls.get() + 1);
            match self.gif_mode.get() {
                Mode::Ready => Ok(PathBuf::from("C:/Temp/recording.gif")),
                Mode::NotAvailable => Err(WorkflowError::NotAvailableYet),
                Mode::Fail => Err(WorkflowError::Failed("codec failed".to_string())),
            }
        }
    }

    impl Clipboard for Fake {
        fn set_image(&self, image: &Image) -> Result<(), ClipboardError> {
            if self.clipboard_mode.get() == Mode::Fail {
                return Err(ClipboardError::Write);
            }
            self.clipboard_images.borrow_mut().push(image.clone());
            Ok(())
        }

        fn set_text(&self, text: &str) -> Result<(), ClipboardError> {
            if self.clipboard_mode.get() == Mode::Fail {
                return Err(ClipboardError::Write);
            }
            self.clipboard_texts.borrow_mut().push(text.to_string());
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    impl Notifier for Fake {
        fn toast(&self, notice: Notice) {
            self.notices.borrow_mut().push(notice);
        }
    }

    impl Sound for Fake {
        fn play(&self, sound: SoundId) {
            self.sounds.borrow_mut().push(sound);
        }
    }

    impl Clock for Fake {
        fn now_local(&self) -> OffsetDateTime {
            datetime!(2026-11-09 12:00:00 UTC)
        }
    }

    fn flow(name: &str, steps: Vec<Step>) -> Workflow {
        let mut workflow = Workflow::new("test", name);
        workflow.steps = steps;
        workflow
    }

    fn capture(kind: CaptureKind) -> Step {
        Step::Capture { kind }
    }

    fn record(kind: RecordKind) -> Step {
        Step::Record { kind }
    }

    #[test]
    fn a_workflow_runs_capture_to_clipboard() {
        let fake = Fake::new();
        let workflow = flow(
            "Screenshot to link",
            vec![
                capture(CaptureKind::Region),
                Step::Annotate,
                Step::CopyImage,
            ],
        );
        let mut context = RunContext::default();
        let report = run(&workflow, &fake.ports(), &mut context).expect("run");
        assert_eq!(report.outcomes.len(), 3);
        assert!(context.image.is_some());
        assert!(context.copied_image);
        assert_eq!(fake.capture_calls.borrow().as_slice(), &[CaptureKind::Region]);
        assert_eq!(fake.annotate_calls.get(), 1);
        assert_eq!(fake.clipboard_images.borrow().len(), 1);
        assert!(report.image);
        assert!(report.copied_image);
    }

    #[test]
    fn every_capture_kind_reaches_the_capture_port() {
        for kind in [
            CaptureKind::Region,
            CaptureKind::Window,
            CaptureKind::Screen,
            CaptureKind::LastRegion,
        ] {
            let fake = Fake::new();
            let workflow = flow("Capture", vec![capture(kind)]);
            let mut context = RunContext::default();
            run(&workflow, &fake.ports(), &mut context).expect("run");
            assert_eq!(fake.capture_calls.borrow().as_slice(), &[kind]);
            assert!(context.image.is_some());
        }
    }

    #[test]
    fn scrolling_capture_reports_not_available_before_running_later_steps() {
        let fake = Fake::new();
        fake.capture_mode.set(Mode::NotAvailable);
        let workflow = flow(
            "Scroll",
            vec![
                capture(CaptureKind::Scrolling),
                Step::Save,
                Step::Notify,
            ],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("not available");
        assert_eq!(
            error.kind,
            RunErrorKind::NotAvailableYet {
                step: "Scrolling capture"
            }
        );
        assert_eq!(error.to_string(), "Scrolling capture is not available yet");
        assert!(error.report.outcomes.is_empty());
        assert_eq!(fake.save_calls.get(), 0);
        assert!(fake.notices.borrow().is_empty());
    }

    #[test]
    fn record_stops_before_running_when_not_available() {
        let fake = Fake::new();
        fake.record_mode.set(Mode::NotAvailable);
        let workflow = flow(
            "Bug report clip",
            vec![
                capture(CaptureKind::Region),
                record(RecordKind::Video),
                Step::ConvertToGif,
                Step::Notify,
            ],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("not available");
        assert_eq!(
            error.kind,
            RunErrorKind::NotAvailableYet { step: "Record" }
        );
        assert_eq!(error.to_string(), "Record is not available yet");
        assert_eq!(error.report.outcomes.len(), 1);
        assert!(context.image.is_some());
        assert!(context.recording.is_none());
        assert_eq!(fake.gif_calls.get(), 0);
        assert!(fake.notices.borrow().is_empty());
    }

    #[test]
    fn a_ready_record_sets_the_recording() {
        let fake = Fake::new();
        let workflow = flow("Record", vec![record(RecordKind::Gif)]);
        let mut context = RunContext::default();
        let report = run(&workflow, &fake.ports(), &mut context).expect("run");
        assert_eq!(fake.record_calls.borrow().as_slice(), &[RecordKind::Gif]);
        assert_eq!(
            context.recording.as_deref(),
            Some(Path::new("C:/Temp/recording.mp4"))
        );
        assert_eq!(report.recording, context.recording);
    }

    #[test]
    fn save_receives_the_captured_image() {
        let fake = Fake::new();
        let workflow = flow(
            "Save quietly",
            vec![capture(CaptureKind::Screen), Step::Save],
        );
        let mut context = RunContext::default();
        run(&workflow, &fake.ports(), &mut context).expect("run");
        assert_eq!(fake.save_calls.get(), 1);
        assert!(fake.save_had_image.get());
        assert_eq!(
            context.saved_file.as_deref(),
            Some(Path::new("C:/Pictures/shot.png"))
        );
    }

    #[test]
    fn copy_image_requires_an_image() {
        let fake = Fake::new();
        let workflow = flow(
            "Record and copy",
            vec![record(RecordKind::Video), Step::CopyImage],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("no image");
        assert_eq!(
            error.kind,
            RunErrorKind::Step {
                number: 2,
                step: "Copy image",
                message: "There is no image to copy.".to_string(),
            }
        );
        assert_eq!(error.report.outcomes.len(), 1);
        assert!(fake.clipboard_images.borrow().is_empty());
    }

    #[test]
    fn clipboard_failures_stop_the_workflow() {
        let fake = Fake::new();
        fake.clipboard_mode.set(Mode::Fail);
        let workflow = flow(
            "Screenshot to link",
            vec![capture(CaptureKind::Region), Step::CopyImage],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("clipboard");
        assert_eq!(
            error.kind,
            RunErrorKind::Step {
                number: 2,
                step: "Copy image",
                message: "The clipboard could not be written.".to_string(),
            }
        );
        assert!(!context.copied_image);
    }

    #[test]
    fn upload_hands_off_and_copy_link_runs_later() {
        let fake = Fake::new();
        let workflow = flow(
            "Screenshot to link",
            vec![
                capture(CaptureKind::Region),
                Step::Upload {
                    dest: Some(DestId::new("imgur")),
                },
                Step::CopyLink {
                    format: LinkFormat::Markdown,
                },
                Step::Notify,
            ],
        );
        let mut context = RunContext::default();
        let report = run(&workflow, &fake.ports(), &mut context).expect("run");
        assert_eq!(report.outcomes.len(), 4);
        assert_eq!(report.deferred, vec![2]);
        assert_eq!(report.outcomes[1].status, StepStatus::HandedOff {
            upload_id: UploadId::new(7)
        });
        assert_eq!(
            fake.upload_calls.borrow().as_slice(),
            &[(Some(DestId::new("imgur")), true)]
        );
        assert!(context.link.is_none());
        assert!(fake.clipboard_texts.borrow().is_empty());
        assert_eq!(context.upload_id, Some(UploadId::new(7)));
        assert_eq!(fake.notices.borrow().len(), 1);
        let resumed = resume_after_upload(
            &workflow,
            &fake.ports(),
            &mut context,
            Uploaded {
                upload_id: UploadId::new(7),
                link: "https://i.example.com/shot.png".to_string(),
                page_link: None,
                file_name: Some("shot.png".to_string()),
            },
        )
        .expect("resume");
        assert_eq!(resumed.outcomes.len(), 1);
        assert_eq!(
            fake.clipboard_texts.borrow().as_slice(),
            &["![shot.png](https://i.example.com/shot.png)".to_string()]
        );
        assert_eq!(
            context.link.as_deref(),
            Some("https://i.example.com/shot.png")
        );
        assert_eq!(
            context.copied_text.as_deref(),
            Some("![shot.png](https://i.example.com/shot.png)")
        );
    }

    #[test]
    fn resume_rejects_an_upload_from_another_run() {
        let fake = Fake::new();
        let workflow = flow(
            "Screenshot to link",
            vec![capture(CaptureKind::Region), Step::Upload { dest: None }],
        );
        let mut context = RunContext::default();
        run(&workflow, &fake.ports(), &mut context).expect("run");
        let error = resume_after_upload(
            &workflow,
            &fake.ports(),
            &mut context,
            Uploaded {
                upload_id: UploadId::new(99),
                link: "https://i.example.com/shot.png".to_string(),
                page_link: None,
                file_name: None,
            },
        )
        .expect_err("mismatch");
        assert_eq!(error.kind, RunErrorKind::UploadMismatch);
        assert!(fake.clipboard_texts.borrow().is_empty());
    }

    #[test]
    fn upload_failure_reports_the_saved_file_and_stops() {
        let fake = Fake::new();
        fake.upload_mode.set(Mode::Fail);
        let workflow = flow(
            "Screenshot to link",
            vec![
                capture(CaptureKind::Region),
                Step::Save,
                Step::Upload { dest: None },
                Step::Notify,
            ],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("upload");
        assert_eq!(
            error.kind,
            RunErrorKind::Step {
                number: 3,
                step: "Upload",
                message: "upload failed".to_string(),
            }
        );
        assert_eq!(error.report.outcomes.len(), 2);
        assert_eq!(
            error.report.saved_file.as_deref(),
            Some(Path::new("C:/Pictures/shot.png"))
        );
        assert!(error.report.upload_id.is_none());
        assert!(fake.notices.borrow().is_empty());
    }

    #[test]
    fn save_failure_reports_the_capture_and_stops() {
        let fake = Fake::new();
        fake.save_mode.set(Mode::Fail);
        let workflow = flow(
            "Save quietly",
            vec![capture(CaptureKind::Screen), Step::Save, Step::Notify],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("save");
        assert_eq!(
            error.kind,
            RunErrorKind::Step {
                number: 2,
                step: "Save",
                message: "save failed".to_string(),
            }
        );
        assert!(error.report.image);
        assert_eq!(error.report.outcomes.len(), 1);
        assert!(error.report.saved_file.is_none());
        assert!(fake.notices.borrow().is_empty());
    }

    #[test]
    fn extract_text_is_not_available_until_it_is_implemented() {
        let fake = Fake::new();
        fake.text_mode.set(Mode::NotAvailable);
        let workflow = flow(
            "Grab text",
            vec![
                capture(CaptureKind::Region),
                Step::ExtractText,
                Step::CopyText,
            ],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("text");
        assert_eq!(
            error.kind,
            RunErrorKind::NotAvailableYet {
                step: "Extract text"
            }
        );
        assert_eq!(error.to_string(), "Extract text is not available yet");
        assert_eq!(error.report.outcomes.len(), 1);
        assert!(context.text.is_none());
        assert!(fake.clipboard_texts.borrow().is_empty());
    }

    #[test]
    fn extract_text_and_copy_text_work_when_the_port_is_ready() {
        let fake = Fake::new();
        let workflow = flow(
            "Grab text",
            vec![
                capture(CaptureKind::Region),
                Step::ExtractText,
                Step::CopyText,
            ],
        );
        let mut context = RunContext::default();
        run(&workflow, &fake.ports(), &mut context).expect("run");
        assert_eq!(fake.text_calls.get(), 1);
        assert_eq!(context.text.as_deref(), Some("recognised text"));
        assert_eq!(
            fake.clipboard_texts.borrow().as_slice(),
            &["recognised text".to_string()]
        );
        assert_eq!(context.copied_text.as_deref(), Some("recognised text"));
    }

    #[test]
    fn convert_to_gif_is_not_available_until_it_is_implemented() {
        let fake = Fake::new();
        fake.gif_mode.set(Mode::NotAvailable);
        let workflow = flow(
            "Bug report clip",
            vec![record(RecordKind::Video), Step::ConvertToGif],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("gif");
        assert_eq!(
            error.kind,
            RunErrorKind::NotAvailableYet {
                step: "Convert to GIF"
            }
        );
        assert_eq!(error.to_string(), "Convert to GIF is not available yet");
        assert!(context.recording.is_some());
        assert!(context.converted.is_none());
    }

    #[test]
    fn a_ready_gif_conversion_sets_the_output() {
        let fake = Fake::new();
        let workflow = flow(
            "Bug report clip",
            vec![record(RecordKind::Video), Step::ConvertToGif],
        );
        let mut context = RunContext::default();
        let report = run(&workflow, &fake.ports(), &mut context).expect("run");
        assert_eq!(fake.gif_calls.get(), 1);
        assert_eq!(
            context.converted.as_deref(),
            Some(Path::new("C:/Temp/recording.gif"))
        );
        assert_eq!(report.converted, context.converted);
    }

    #[test]
    fn a_codec_failure_stops_after_the_recording() {
        let fake = Fake::new();
        fake.gif_mode.set(Mode::Fail);
        let workflow = flow(
            "Bug report clip",
            vec![record(RecordKind::Video), Step::ConvertToGif, Step::Notify],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("gif");
        assert_eq!(
            error.kind,
            RunErrorKind::Step {
                number: 2,
                step: "Convert to GIF",
                message: "codec failed".to_string(),
            }
        );
        assert_eq!(error.report.outcomes.len(), 1);
        assert!(context.recording.is_some());
        assert!(fake.notices.borrow().is_empty());
    }

    #[test]
    fn notify_and_play_sound_reach_their_ports() {
        let fake = Fake::new();
        let workflow = flow(
            "Save quietly",
            vec![capture(CaptureKind::Screen), Step::Notify, Step::PlaySound],
        );
        let mut context = RunContext::default();
        run(&workflow, &fake.ports(), &mut context).expect("run");
        assert_eq!(fake.notices.borrow().len(), 1);
        assert_eq!(fake.notices.borrow()[0].title, "Snapline");
        assert_eq!(fake.notices.borrow()[0].body, "Save quietly finished.");
        assert_eq!(fake.sounds.borrow().as_slice(), &[SoundId::Done]);
    }

    #[test]
    fn a_disabled_workflow_does_not_run() {
        let fake = Fake::new();
        let mut workflow = flow("Save quietly", vec![capture(CaptureKind::Screen)]);
        workflow.enabled = false;
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("disabled");
        assert_eq!(error.kind, RunErrorKind::Disabled);
        assert_eq!(error.to_string(), "This workflow is off.");
        assert!(fake.capture_calls.borrow().is_empty());
        assert!(context.image.is_none());
    }

    #[test]
    fn an_invalid_workflow_does_not_run() {
        let fake = Fake::new();
        let workflow = flow(
            "Screenshot to link",
            vec![Step::CopyImage, Step::Upload { dest: None }],
        );
        let mut context = RunContext::default();
        let error = run(&workflow, &fake.ports(), &mut context).expect_err("invalid");
        match error.kind {
            RunErrorKind::Invalid(violations) => {
                assert_eq!(violations.len(), 1);
                assert_eq!(
                    violations[0].error,
                    crate::workflow::ValidationError::FirstStep
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(fake.clipboard_images.borrow().is_empty());
    }

    #[test]
    fn the_default_workflow_has_no_chord_of_its_own() {
        let workflow = Workflow::new(DEFAULT_WORKFLOW_ID, DEFAULT_WORKFLOW_NAME);
        assert!(workflow.is_default());
        assert_eq!(workflow.hotkey, None);
    }
}
