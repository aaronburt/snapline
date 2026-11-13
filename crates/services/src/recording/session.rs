use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::{bounded, unbounded, Receiver, RecvTimeoutError, Sender};
use snapline_core::ffmpeg_args::{
    self, GifCaptureRequest, GifConvertRequest, GifSettings, VideoFormat, VideoFrameRate,
    VideoQuality, VideoRequest,
};
use snapline_core::ports::OverlayTheme;
use snapline_core::record_place;
use snapline_core::recording::{RecordingEvent, RecordingMachine, RecordingState};

use crate::AppEvent;

use super::notices::mark_reported;
use super::{
    remove_file, status_dto, AudioOptions, AudioSession, BorderHandle, BorderRequest,
    ControlsHandle, ControlsRequest, Event, EventSink, FfmpegExit, FfmpegProcess, FfmpegRunner,
    PickedTarget, RecordError, RecordStatus, RecordedFile, RecordedKind, RecordingPorts,
    SessionOutcome, TICK,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionMode {
    Save,
    Temporary,
}

#[derive(Debug, Clone)]
pub(super) struct SessionSettings {
    pub(super) ffmpeg: PathBuf,
    pub(super) frame_rate: VideoFrameRate,
    pub(super) quality: VideoQuality,
    pub(super) format: VideoFormat,
    pub(super) gif: GifSettings,
    pub(super) border: bool,
    pub(super) audio: AudioOptions,
    pub(super) theme: OverlayTheme,
}

pub(super) struct SessionPlan {
    pub(super) kind: RecordedKind,
    pub(super) mode: SessionMode,
    pub(super) target: PickedTarget,
    pub(super) output: PathBuf,
    pub(super) converted: Option<PathBuf>,
    pub(super) settings: SessionSettings,
}

pub(super) enum Command {
    Stop,
    Cancel,
}

enum Ended {
    Stopped(u64),
    Cancelled,
}

#[derive(Clone)]
pub(super) struct SessionControl {
    stop: Sender<Command>,
    status: Arc<Mutex<RecordStatus>>,
}

impl SessionControl {
    pub(super) fn stop(&self) {
        let _ = self.stop.try_send(Command::Stop);
    }

    pub(super) fn cancel(&self) {
        let _ = self.stop.try_send(Command::Cancel);
    }

    pub(super) fn status(&self) -> RecordStatus {
        status_lock(&self.status).clone()
    }

    pub(super) fn is_active(&self) -> bool {
        self.status().state.is_active()
    }
}

pub(super) struct Session {
    control: SessionControl,
    results: Receiver<Result<SessionOutcome, RecordError>>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub(super) fn start(plan: SessionPlan, ports: RecordingPorts) -> Result<Self, RecordError> {
        if plan.target.rect.is_empty() {
            return Err(RecordError::NoTarget);
        }
        let status = Arc::new(Mutex::new(RecordStatus {
            state: RecordingState::Idle,
            kind: Some(plan.kind),
            elapsed_ms: 0,
            width: plan.target.width(),
            height: plan.target.height(),
            duration_ms: 0,
            capture_id: None,
            path: None,
            error: None,
        }));
        let mut machine = RecordingMachine::new();
        machine
            .apply(RecordingEvent::Start {
                target: plan.target.recording_target(),
                output: plan.output.clone(),
            })
            .map_err(|error| RecordError::Invalid(error.to_string()))?;
        publish(&status, &machine, ports.events.as_ref());
        let (commands, inbox) = unbounded();
        let (results, receiver) = bounded(1);
        let thread_status = Arc::clone(&status);
        let thread_ports = ports.clone();
        let stop_commands = commands.clone();
        let stop: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = stop_commands.try_send(Command::Stop);
        });
        let cancel_commands = commands.clone();
        let cancel: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = cancel_commands.try_send(Command::Cancel);
        });
        let thread = thread::Builder::new()
            .name("snapline-recorder".to_owned())
            .spawn(move || {
                let outcome = run_session(
                    &plan,
                    &thread_ports,
                    &mut machine,
                    &thread_status,
                    &inbox,
                    stop,
                    cancel,
                );
                let outcome = match outcome {
                    Err(error) => Err(finish_failed_session(
                        &plan,
                        &thread_ports,
                        &mut machine,
                        &thread_status,
                        error,
                    )),
                    ok => ok,
                };
                let _ = results.send(outcome);
            })
            .map_err(|error| RecordError::Invalid(error.to_string()))?;
        Ok(Self {
            control: SessionControl {
                stop: commands,
                status,
            },
            results: receiver,
            thread: Some(thread),
        })
    }

    pub(super) fn control(&self) -> SessionControl {
        self.control.clone()
    }

    pub(super) fn wait(mut self) -> Result<SessionOutcome, RecordError> {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        match self.results.recv() {
            Ok(result) => result,
            Err(_) => Err(RecordError::Invalid(
                "The recorder stopped before it finished.".to_owned(),
            )),
        }
    }
}

#[derive(Default)]
struct SessionResources {
    border: Option<Box<dyn BorderHandle>>,
    controls: Option<Box<dyn ControlsHandle>>,
    audio: Option<Box<dyn AudioSession>>,
    ffmpeg: Option<Box<dyn FfmpegProcess>>,
}

impl SessionResources {
    fn close_controls_border_and_audio(&mut self) {
        if let Some(mut controls) = self.controls.take() {
            controls.close();
        }
        if let Some(mut border) = self.border.take() {
            border.close();
        }
        if let Some(mut audio) = self.audio.take() {
            audio.stop();
        }
    }

    fn abort(&mut self) {
        self.close_controls_border_and_audio();
        if let Some(mut ffmpeg) = self.ffmpeg.take() {
            let _ = ffmpeg.kill();
        }
    }

    fn stop_ffmpeg(&mut self) -> Result<FfmpegExit, RecordError> {
        match self.ffmpeg.take() {
            Some(mut ffmpeg) => ffmpeg
                .stop()
                .map_err(|error| RecordError::Stop(error.to_string())),
            None => Ok(FfmpegExit::success()),
        }
    }
}

impl Drop for SessionResources {
    fn drop(&mut self) {
        self.abort();
    }
}

/// Cleans up after a session that did not finish. The part of a recording FFmpeg managed to
/// write before it died is kept (and its location reported); every other failure discards it.
pub(super) fn finish_failed_session(
    plan: &SessionPlan,
    ports: &RecordingPorts,
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    error: RecordError,
) -> RecordError {
    let keep_partial = matches!(error, RecordError::FfmpegStopped(_))
        && plan.mode == SessionMode::Save
        && std::fs::metadata(&plan.output).is_ok_and(|meta| meta.len() > 0);
    let error = if keep_partial {
        RecordError::FfmpegStopped(format!(
            "{error} The part recorded so far was kept at {}.",
            plan.output.display()
        ))
    } else {
        remove_file(&plan.output);
        error
    };
    if let Some(converted) = &plan.converted {
        remove_file(converted);
    }
    if matches!(error, RecordError::Cancelled) {
        ports.publisher.cancelled();
    } else {
        let message = error.to_string();
        ports.publisher.failed(&message);
        mark_reported(&message);
        if !machine.state().is_terminal() {
            let _ = machine.apply(RecordingEvent::Failed(message));
            publish(status, machine, ports.events.as_ref());
        }
    }
    error
}

fn run_session(
    plan: &SessionPlan,
    ports: &RecordingPorts,
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    inbox: &Receiver<Command>,
    stop: Arc<dyn Fn() + Send + Sync>,
    cancel: Arc<dyn Fn() + Send + Sync>,
) -> Result<SessionOutcome, RecordError> {
    let mut resources = SessionResources::default();
    let outputs = ports.ffmpeg.outputs();
    let (output_index, target) = if outputs.is_empty() {
        (0, plan.target.recording_target())
    } else {
        let placed = record_place::place(plan.target.rect, &outputs)
            .map_err(|error| RecordError::Invalid(error.to_string()))?;
        (
            placed.output_index,
            plan.target.recording_target_at(placed.rect),
        )
    };
    if plan.settings.border {
        let border = ports
            .border
            .show(&BorderRequest {
                rect: plan.target.rect,
                monitor: plan.target.monitor,
                theme: plan.settings.theme,
            })
            .map_err(|error| RecordError::Border(error.to_string()))?;
        resources.border = Some(border);
    }
    match ports.controls.show(&ControlsRequest {
        rect: plan.target.rect,
        monitor: plan.target.monitor,
        theme: plan.settings.theme,
        stop,
        cancel,
    }) {
        Ok(controls) => resources.controls = Some(controls),
        Err(error) => tracing::warn!(%error, "could not show the recording controls"),
    }
    let pipes = if plan.settings.audio.enabled() {
        let session = ports
            .audio
            .start(&plan.settings.audio)
            .map_err(|error| RecordError::Audio(error.to_string()))?;
        let pipes = session.pipes();
        resources.audio = Some(session);
        pipes
    } else {
        Vec::new()
    };
    let command = match plan.kind {
        RecordedKind::Video => ffmpeg_args::video_command(&VideoRequest {
            ffmpeg: plan.settings.ffmpeg.clone(),
            target,
            output: plan.output.clone(),
            output_index,
            format: plan.settings.format,
            encoder: ports
                .ffmpeg
                .best_encoder(&plan.settings.ffmpeg, plan.settings.format),
            frame_rate: plan.settings.frame_rate,
            quality: plan.settings.quality,
            audio: pipes,
        })
        .map_err(|error| RecordError::Invalid(error.to_string()))?,
        RecordedKind::Gif => ffmpeg_args::gif_capture_command(&GifCaptureRequest {
            ffmpeg: plan.settings.ffmpeg.clone(),
            target,
            output: plan.output.clone(),
            output_index,
            frame_rate: plan.settings.gif.frame_rate,
        })
        .map_err(|error| RecordError::Invalid(error.to_string()))?,
    };
    let process = ports
        .ffmpeg
        .start(command)
        .map_err(|error| RecordError::FfmpegStart(error.to_string()))?;
    resources.ffmpeg = Some(process);
    transition(
        machine,
        status,
        ports.events.as_ref(),
        RecordingEvent::Started,
    )?;

    let duration_ms = match record_until_stop(
        machine,
        status,
        ports.events.as_ref(),
        &mut resources,
        inbox,
    )? {
        Ended::Stopped(duration_ms) => duration_ms,
        Ended::Cancelled => return discard(plan, ports, machine, status, &mut resources),
    };

    transition(machine, status, ports.events.as_ref(), RecordingEvent::Stop)?;
    resources.close_controls_border_and_audio();
    let exit = resources.stop_ffmpeg()?;
    transition(
        machine,
        status,
        ports.events.as_ref(),
        RecordingEvent::Stopped,
    )?;
    if !exit.success {
        return Err(RecordError::Ffmpeg(exit.message));
    }
    if cancel_requested(inbox) {
        return discard(plan, ports, machine, status, &mut resources);
    }
    update_status(status, |current| {
        current.duration_ms = duration_ms;
    });

    let media = if plan.kind == RecordedKind::Gif {
        convert_capture(plan, ports)?
    } else {
        plan.output.clone()
    };
    if cancel_requested(inbox) {
        return discard(plan, ports, machine, status, &mut resources);
    }

    let details = snapline_core::workflow::RecordingDetails {
        width: plan.target.width(),
        height: plan.target.height(),
        duration_ms,
    };
    match plan.mode {
        SessionMode::Temporary => {
            update_status(status, |current| {
                current.path = Some(media.clone());
            });
            transition(
                machine,
                status,
                ports.events.as_ref(),
                RecordingEvent::Encoded,
            )?;
            Ok(SessionOutcome {
                path: media,
                details,
            })
        }
        SessionMode::Save => {
            let first_frame = ports
                .media
                .first_frame(&media, &plan.settings.ffmpeg)
                .map_err(|error| {
                    tracing::warn!(%error, "the first frame of the recording is unavailable");
                    error
                })
                .ok();
            let saved = ports
                .store
                .save(RecordedFile {
                    media,
                    first_frame,
                    kind: plan.kind,
                    width: plan.target.width(),
                    height: plan.target.height(),
                    duration_ms,
                    source_app: None,
                    window_title: plan.target.title(),
                })
                .map_err(|error| RecordError::Save(error.to_string()))?;
            update_status(status, |current| {
                current.capture_id = saved.capture_id;
                current.path = Some(saved.path.clone());
                current.duration_ms = saved.duration_ms;
                current.kind = Some(saved.kind);
            });
            transition(
                machine,
                status,
                ports.events.as_ref(),
                RecordingEvent::Encoded,
            )?;
            if let Some(id) = saved.capture_id {
                ports
                    .events
                    .emit(AppEvent::History(crate::history::Event::Added { id }));
            }
            ports.publisher.published(&saved);
            Ok(SessionOutcome {
                path: saved.path,
                details,
            })
        }
    }
}

pub(super) fn cancel_requested(inbox: &Receiver<Command>) -> bool {
    inbox
        .try_iter()
        .any(|command| matches!(command, Command::Cancel))
}

fn discard(
    plan: &SessionPlan,
    ports: &RecordingPorts,
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    resources: &mut SessionResources,
) -> Result<SessionOutcome, RecordError> {
    resources.abort();
    remove_file(&plan.output);
    if let Some(converted) = &plan.converted {
        remove_file(converted);
    }
    machine.reset();
    update_status(status, |current| {
        current.capture_id = None;
        current.path = None;
        current.duration_ms = 0;
    });
    publish(status, machine, ports.events.as_ref());
    Err(RecordError::Cancelled)
}

fn record_until_stop(
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    events: &dyn EventSink,
    resources: &mut SessionResources,
    inbox: &Receiver<Command>,
) -> Result<Ended, RecordError> {
    let started = Instant::now();
    loop {
        let elapsed = elapsed_millis(started);
        match inbox.recv_timeout(TICK) {
            Ok(Command::Cancel) => return Ok(Ended::Cancelled),
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => {
                if machine.state() == RecordingState::Recording {
                    let _ = machine.apply(RecordingEvent::Tick(elapsed));
                }
                publish(status, machine, events);
                return Ok(Ended::Stopped(elapsed));
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(process) = resources.ffmpeg.as_mut() {
                    let _ = process.progress();
                    if !process.is_running() {
                        let detail = process
                            .wait()
                            .ok()
                            .map(|exit| ffmpeg_reason(&exit.message))
                            .unwrap_or_default();
                        return Err(RecordError::FfmpegStopped(format!(
                            "FFmpeg stopped before the recording was stopped. {detail}"
                        )));
                    }
                }
                if machine.state() == RecordingState::Recording {
                    let _ = machine.apply(RecordingEvent::Tick(elapsed));
                }
                publish(status, machine, events);
            }
        }
    }
}

pub(super) fn ffmpeg_reason(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(3);
    lines[start..].join(" ").trim().to_owned()
}

fn transition(
    machine: &mut RecordingMachine,
    status: &Arc<Mutex<RecordStatus>>,
    events: &dyn EventSink,
    event: RecordingEvent,
) -> Result<(), RecordError> {
    machine
        .apply(event)
        .map_err(|error| RecordError::Invalid(error.to_string()))?;
    publish(status, machine, events);
    Ok(())
}

fn publish(status: &Arc<Mutex<RecordStatus>>, machine: &RecordingMachine, events: &dyn EventSink) {
    let dto = {
        let mut guard = status_lock(status);
        guard.state = machine.state();
        guard.elapsed_ms = machine.elapsed_ms();
        guard.error = machine.error().map(str::to_owned);
        status_dto(&guard)
    };
    events.emit(AppEvent::Record(Event::State(dto)));
}

fn update_status(status: &Arc<Mutex<RecordStatus>>, update: impl FnOnce(&mut RecordStatus)) {
    update(&mut status_lock(status));
}

fn status_lock(status: &Mutex<RecordStatus>) -> MutexGuard<'_, RecordStatus> {
    status.lock().unwrap_or_else(PoisonError::into_inner)
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn convert_capture(plan: &SessionPlan, ports: &RecordingPorts) -> Result<PathBuf, RecordError> {
    let output = plan
        .converted
        .as_ref()
        .ok_or_else(|| RecordError::Convert("The GIF output path is missing.".to_owned()))?;
    let media = convert_with(
        ports.ffmpeg.as_ref(),
        &plan.settings.ffmpeg,
        &plan.output,
        output,
        plan.settings.gif,
    )?;
    remove_file(&plan.output);
    Ok(media)
}

pub(super) fn convert_with(
    runner: &dyn FfmpegRunner,
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    settings: GifSettings,
) -> Result<PathBuf, RecordError> {
    if !input.is_file() {
        return Err(RecordError::Convert(
            "The recording could not be found.".to_owned(),
        ));
    }
    let command = ffmpeg_args::gif_convert_command(&GifConvertRequest {
        ffmpeg: ffmpeg.to_path_buf(),
        input: input.to_path_buf(),
        output: output.to_path_buf(),
        settings,
    })
    .map_err(|error| RecordError::Invalid(error.to_string()))?;
    let mut process = runner
        .start(command)
        .map_err(|error| RecordError::Convert(error.to_string()))?;
    let exit = process
        .wait()
        .map_err(|error| RecordError::Convert(error.to_string()))?;
    if !exit.success {
        remove_file(output);
        return Err(RecordError::Convert(exit.message));
    }
    Ok(output.to_path_buf())
}
