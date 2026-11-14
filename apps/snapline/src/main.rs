#![windows_subsystem = "windows"]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod cli;
mod crash;
mod crash_report;
mod history_host;
mod logging;
mod main_loop;
mod settings_update;
mod shell;
mod shortcut;
mod single_instance;
mod slint_ui;
mod theme;
mod tray;
mod upload_files;

use std::collections::{BTreeMap, HashMap};
use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use ::windows::Win32::System::Ole::CF_HDROP;
use ::windows::Win32::System::Threading::CREATE_NO_WINDOW;
use ::windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use cli::{Args, BudgetWindow};
use shell::Shell;
use snapline_core::chord::Chord;
use snapline_core::ffmpeg_args::{fallback_encoder, FfmpegCommand, VideoEncoder, VideoFormat};
use snapline_core::geometry::{Point, Px};
use snapline_core::hotkey_plan::{Blocked, Owner};
use snapline_core::image::Image;
use snapline_core::ports::{MagnifierZoom, Notice, Notifier, OverlayTheme, ScreenCapture};
use snapline_services::capture::{CaptureContext, CaptureService, Countdown, LastRegion};
use snapline_services::dto::record::RecordStateDto;
use snapline_services::dto::upload::{UploadState, UploadStatus};
use snapline_services::editor::{
    EditorAction, EditorClipboardContent, EditorClipboardSource, EditorFileDialog, EditorHost,
    EditorPin, EditorPorts, EditorResult,
};
use snapline_services::hotkeys::{HotkeyInput, PrtscRegistry};
use snapline_services::recording::{
    self, AudioOptions, AudioPort, AudioSession, BorderHandle, BorderHost, BorderRequest,
    ControlsHandle, ControlsHost, ControlsRequest, EventSink, FfmpegExit, FfmpegProcess,
    FfmpegProgress, FfmpegRunner, MediaPort, PickOptions, PickedTarget, PickedWindow,
    RecordingPorts, TargetPicker, WorkerRecordingStore,
};
use snapline_services::saver::{CaptureRequest, ServicePorts, SystemClock};
use snapline_services::tools::convert::{
    BackendExit, BackendProgress, ConvertError, ConvertFilePicker, FfmpegBackend,
    FfmpegProcess as ConvertProcess,
};
use snapline_services::worker;
use snapline_services::{AppEvent, CodecFactory, ServiceError, Services};
use snapline_store::io::Paths;
use snapline_store::settings::Settings;
use snapline_upload::error::UploadError;
use snapline_upload::sftp::{HostKeyConfirmer, HostKeyPrompt, HostKeyState, HostKeyStore};
use snapline_win::audio::{AudioCapture, AudioRequest, PcmFormat};
use snapline_win::capture::{CaptureBackend, CaptureOptions};
use snapline_win::clipboard::Clipboard;
use snapline_win::ffmpeg::{detect, RunningFfmpeg, StopOutcome};
use snapline_win::input::{InputEvent, InputHandle};
use snapline_win::known_hosts::{HostKeyStatus, Store};
use snapline_win::ports_impls::{SoundPlayer, ToastNotifier};
use snapline_win::wic::WicCodec;

use ::windows::Win32::Foundation::POINT;
use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

struct InputPort(InputHandle);

impl HotkeyInput for InputPort {
    fn rebind(&self, bindings: Vec<(Owner, Chord)>) -> Result<Vec<Blocked>, ServiceError> {
        self.0
            .rebind(bindings)
            .map_err(|error| ServiceError::Internal(error.to_string()))
    }

    fn pause(&self) -> Result<(), ServiceError> {
        self.0
            .pause()
            .map_err(|error| ServiceError::Internal(error.to_string()))
    }

    fn resume(&self) -> Result<(), ServiceError> {
        self.0
            .resume()
            .map_err(|error| ServiceError::Internal(error.to_string()))
    }
}

struct PrtscPort;

impl PrtscRegistry for PrtscPort {
    fn current(&self) -> Result<Option<u32>, ServiceError> {
        snapline_win::prtsc::current().map_err(|error| ServiceError::Io(error.to_string()))
    }

    fn enable(&self) -> Result<Option<u32>, ServiceError> {
        snapline_win::prtsc::enable().map_err(|error| ServiceError::Io(error.to_string()))
    }

    fn restore(&self, previous: Option<u32>) -> Result<(), ServiceError> {
        snapline_win::prtsc::restore(previous).map_err(|error| ServiceError::Io(error.to_string()))
    }
}

struct CombineHostImpl {
    worker: worker::Handle,
}

impl snapline_services::tools::combine::CombineHost for CombineHostImpl {
    fn pick_images(&self) -> Result<Vec<std::path::PathBuf>, ServiceError> {
        snapline_win::image_dialog::pick_images()
            .map_err(|error| ServiceError::Io(error.to_string()))
    }

    fn save_capture(
        &self,
        image: snapline_core::image::Image,
    ) -> Result<Option<i64>, ServiceError> {
        let outcome = self.worker.save_capture(CaptureRequest::new(
            image,
            Some("Snapline".to_string()),
            "Combined image".to_string(),
        ))?;
        Ok(outcome.capture_id)
    }
}

struct EditorHostImpl;

impl EditorHost for EditorHostImpl {
    fn open(&self, image: snapline_core::image::Image) -> Result<EditorResult, ServiceError> {
        let mut request = snapline_overlay::EditRequest::new(image);
        request.theme = if snapline_win::theme::is_light() {
            snapline_overlay::Theme::Light
        } else {
            snapline_overlay::Theme::Dark
        };
        match snapline_overlay::OverlayHostImpl::new().edit_image(request) {
            Ok(result) => Ok(EditorResult {
                action: result.action.map(editor_action),
                image: result.image,
            }),
            Err(snapline_overlay::OverlayError::Cancelled) => Ok(EditorResult::cancelled()),
            Err(error) => Err(ServiceError::Invalid(error.to_string())),
        }
    }
}

fn editor_action(action: snapline_overlay::OverlayAction) -> EditorAction {
    match action {
        snapline_overlay::OverlayAction::Copy => EditorAction::Copy,
        snapline_overlay::OverlayAction::Save => EditorAction::Save,
        snapline_overlay::OverlayAction::Pin => EditorAction::Pin,
        snapline_overlay::OverlayAction::Upload => EditorAction::Upload,
        snapline_overlay::OverlayAction::Record | snapline_overlay::OverlayAction::Gif => {
            EditorAction::Copy
        }
    }
}

struct EditorFileDialogImpl;

impl EditorFileDialog for EditorFileDialogImpl {
    fn pick_image(&self) -> Result<Option<PathBuf>, ServiceError> {
        snapline_win::image_dialog::pick_images()
            .map(|mut paths| paths.pop())
            .map_err(|error| ServiceError::Io(error.to_string()))
    }
}

struct SxcuFileDialog;

impl snapline_services::destinations::SxcuDialog for SxcuFileDialog {
    fn open(&self) -> Result<Option<Vec<u8>>, ServiceError> {
        use std::io::Read;
        let Some(path) = snapline_win::dialogs::pick_sxcu()
            .map_err(|error| ServiceError::Io(error.to_string()))?
        else {
            return Ok(None);
        };
        let file = std::fs::File::open(path)
            .map_err(|_| ServiceError::Io("The .sxcu file could not be opened.".to_owned()))?;
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| ServiceError::Io("The .sxcu file could not be read.".to_owned()))?;
        Ok(Some(bytes))
    }

    fn save(&self, suggested_name: &str, contents: &str) -> Result<Option<String>, ServiceError> {
        snapline_win::dialogs::save_sxcu(suggested_name, contents)
            .map(|path| path.map(|path| path.to_string_lossy().into_owned()))
            .map_err(|error| ServiceError::Io(error.to_string()))
    }
}

struct UploadFileDialogImpl;

impl snapline_services::FilePicker for UploadFileDialogImpl {
    fn pick_files(&self) -> Result<Vec<PathBuf>, ServiceError> {
        snapline_win::dialogs::pick_files().map_err(|error| ServiceError::Io(error.to_string()))
    }

    fn pick_ffmpeg(&self) -> Result<Option<PathBuf>, ServiceError> {
        snapline_win::dialogs::pick_ffmpeg().map_err(|error| ServiceError::Io(error.to_string()))
    }
}

struct EditorClipboardSourceImpl;

impl EditorClipboardSource for EditorClipboardSourceImpl {
    fn read(&self) -> Result<Option<EditorClipboardContent>, ServiceError> {
        let clipboard = Clipboard::new();
        match clipboard.get_image() {
            Ok(Some(image)) => Ok(Some(EditorClipboardContent::Image(image))),
            Ok(None) => {
                let files = clipboard_files()?;
                if files.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(EditorClipboardContent::Files(files)))
                }
            }
            Err(error) => Err(ServiceError::Invalid(error.to_string())),
        }
    }
}

fn clipboard_files() -> Result<Vec<PathBuf>, ServiceError> {
    // SAFETY: opens the clipboard on the current thread, which is closed below on every path.
    if unsafe { OpenClipboard(None) }.is_err() {
        return Err(ServiceError::Io(
            "The clipboard could not be read.".to_owned(),
        ));
    }
    let files = read_clipboard_files();
    // SAFETY: balances the successful OpenClipboard on this thread.
    unsafe {
        let _ = CloseClipboard();
    }
    files
}

fn read_clipboard_files() -> Result<Vec<PathBuf>, ServiceError> {
    // SAFETY: the clipboard is open on this thread for the duration of the call.
    let handle = match unsafe { GetClipboardData(u32::from(CF_HDROP.0)) } {
        Ok(handle) => handle,
        Err(_) => return Ok(Vec::new()),
    };
    let drop = HDROP(handle.0);
    // SAFETY: the handle is a live CF_HDROP value and a null filter asks for the count.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut files = Vec::new();
    for index in 0..count {
        // SAFETY: the index is below the count reported for this CF_HDROP value.
        let length = unsafe { DragQueryFileW(drop, index, None) };
        let Ok(length) = usize::try_from(length) else {
            continue;
        };
        let mut buffer = vec![0_u16; length.saturating_add(1)];
        // SAFETY: the buffer has room for the length just read plus the terminator.
        let copied = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
        let Ok(copied) = usize::try_from(copied) else {
            continue;
        };
        if copied == 0 {
            continue;
        }
        if let Ok(text) = String::from_utf16(&buffer[..copied]) {
            files.push(PathBuf::from(text));
        }
    }
    Ok(files)
}

struct EditorPinImpl {
    handles: Mutex<Vec<snapline_overlay::pin::PinHandle>>,
}

impl Default for EditorPinImpl {
    fn default() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
        }
    }
}

impl EditorPin for EditorPinImpl {
    fn pin(&self, image: snapline_core::image::Image) -> Result<(), ServiceError> {
        let handle = snapline_overlay::pin::PinHostImpl::new()
            .open(snapline_overlay::pin::PinRequest::new(image))
            .map_err(|error| ServiceError::Invalid(error.to_string()))?;
        self.handles
            .lock()
            .map_err(|_| ServiceError::Internal("The pin window is unavailable.".to_owned()))?
            .push(handle);
        Ok(())
    }
}

struct PinMenuImpl;

impl snapline_services::tools::pin::PinMenu for PinMenuImpl {
    fn choose(&self) -> Result<Option<snapline_services::editor::PinSource>, ServiceError> {
        snapline_win::menu::choose_pin_source()
            .map_err(|error| ServiceError::Internal(error.to_string()))
            .map(|choice| {
                choice.map(|choice| match choice {
                    snapline_win::menu::PinSourceChoice::File => {
                        snapline_services::editor::PinSource::File
                    }
                    snapline_win::menu::PinSourceChoice::Clipboard => {
                        snapline_services::editor::PinSource::Clipboard
                    }
                })
            })
    }
}

struct ShellHostImpl;

impl snapline_services::shell_actions::ShellHost for ShellHostImpl {
    fn copy_text(&self, text: &str) -> Result<(), ServiceError> {
        snapline_win::clipboard::Clipboard::new()
            .set_text(text)
            .map_err(|error| ServiceError::Io(error.to_string()))
    }

    fn open_folder(&self, path: &std::path::Path) -> Result<(), ServiceError> {
        snapline_win::shell::open_file(path).map_err(|error| ServiceError::Io(error.to_string()))
    }
}

struct AppCaptureContext {
    services: Services,
}

impl CaptureContext for AppCaptureContext {
    fn capture_complete(&self, _image: &snapline_core::image::Image, sound: bool) {
        if sound {
            snapline_win::sound::play(snapline_win::sound::SoundId::Shutter);
        }
    }

    fn capture_settings(&self) -> Result<Settings, ServiceError> {
        self.services.settings_snapshot()
    }

    fn save_last_region(&self, region: LastRegion) -> Result<(), ServiceError> {
        <Services as CaptureContext>::save_last_region(&self.services, region)
    }

    fn start_recording(
        &self,
        gif: bool,
        rect: snapline_core::geometry::Rect,
        monitor: u32,
    ) -> Result<(), ServiceError> {
        <Services as CaptureContext>::start_recording(&self.services, gif, rect, monitor)
    }

    fn foreground_window(&self) -> Option<snapline_core::ids::WindowId> {
        CaptureBackend::foreground_window()
    }

    fn process_name(&self, window: snapline_core::ids::WindowId) -> Option<String> {
        CaptureBackend::process_name(window)
    }

    fn window_at(&self, point: Point) -> Option<snapline_core::ids::WindowId> {
        CaptureBackend::window_at(point)
    }

    fn window_under_cursor(&self) -> Option<snapline_core::ids::WindowId> {
        CaptureBackend::window_under_cursor()
    }

    fn list_windows(&self) -> Vec<snapline_services::capture::WindowChoice> {
        snapline_win::source_app::capturable_windows()
            .into_iter()
            .map(|window| snapline_services::capture::WindowChoice {
                app: CaptureBackend::process_name(window.id),
                id: window.id,
                title: window.title,
            })
            .collect()
    }

    fn choose_window(
        &self,
        choices: &[snapline_services::capture::WindowChoice],
    ) -> Result<Option<snapline_services::capture::WindowPick>, ServiceError> {
        if choices.is_empty() {
            return snapline_services::capture::resolve_pick(choices, None);
        }
        let labels = snapline_services::capture::menu_labels(choices);
        let selected = snapline_win::menu::choose_from(&labels)
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        snapline_services::capture::resolve_pick(choices, selected)
    }

    fn cursor_position(&self) -> Option<Point> {
        let mut point = POINT::default();
        // SAFETY: the point pointer is writable and GetCursorPos does not retain it.
        if unsafe { GetCursorPos(&mut point) }.is_err() {
            return None;
        }
        Some(Point::new(Px(point.x), Px(point.y)))
    }

    fn overlay_theme(&self) -> OverlayTheme {
        if snapline_win::theme::is_light() {
            OverlayTheme::Light
        } else {
            OverlayTheme::Dark
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct CountdownAdapter;

impl Countdown for CountdownAdapter {
    fn run(&self, seconds: u8) -> Result<(), ServiceError> {
        snapline_win::countdown::Countdown::new()
            .run(seconds)
            .map_err(|error| ServiceError::Internal(error.to_string()))
    }
}

struct FfmpegBackendImpl;

impl FfmpegBackend for FfmpegBackendImpl {
    fn start(&self, command: FfmpegCommand) -> Result<Box<dyn ConvertProcess>, ConvertError> {
        RunningFfmpeg::start(command)
            .map(|running| Box::new(FfmpegProcessImpl(running)) as Box<dyn ConvertProcess>)
            .map_err(|error| ConvertError::Ffmpeg(error.to_string()))
    }
}

struct FfmpegProcessImpl(RunningFfmpeg);

impl ConvertProcess for FfmpegProcessImpl {
    fn progress(&mut self) -> Option<BackendProgress> {
        self.0
            .progress()
            .try_recv()
            .ok()
            .map(|progress| BackendProgress {
                out_time_us: progress
                    .out_time_us
                    .or_else(|| progress.out_time_ms.map(|millis| millis * 1_000)),
                frame: progress.frame,
                fps: progress.fps,
                speed: progress.speed,
                done: progress.done,
            })
    }

    fn finished(&mut self) -> Result<bool, ConvertError> {
        Ok(!self.0.is_running())
    }

    fn cancel(&mut self) -> Result<(), ConvertError> {
        self.0
            .kill()
            .map(|_| ())
            .map_err(|error| ConvertError::Ffmpeg(error.to_string()))
    }

    fn wait(&mut self) -> Result<BackendExit, ConvertError> {
        let outcome = self
            .0
            .wait()
            .map_err(|error| ConvertError::Ffmpeg(error.to_string()))?;
        Ok(BackendExit {
            success: outcome.success(),
            message: outcome.stderr().to_string(),
        })
    }
}

struct VideoPickerImpl;

impl ConvertFilePicker for VideoPickerImpl {
    fn pick_video(&self) -> Result<Option<std::path::PathBuf>, ConvertError> {
        snapline_win::video_dialog::pick_video()
            .map_err(|error| ConvertError::Save(error.to_string()))
    }
}

struct RecordingPickerImpl;

impl TargetPicker for RecordingPickerImpl {
    fn pick(&self, options: PickOptions) -> Result<PickedTarget, recording::RecordError> {
        let desktop = CaptureBackend::new(CaptureOptions::default())
            .freeze_all()
            .map_err(|error| recording::RecordError::Invalid(error.to_string()))?;
        let mut request = snapline_overlay::record_border::RecordPickRequest::new(desktop);
        request.magnifier = overlay_magnifier(options);
        request.theme = overlay_theme(current_overlay_theme());
        match snapline_overlay::OverlayHostImpl::new().pick_record_target(request) {
            Ok(target) => Ok(picked_target(target)),
            Err(snapline_overlay::record_border::RecordPickError::Cancelled) => {
                Err(recording::RecordError::Cancelled)
            }
            Err(error) => Err(recording::RecordError::Invalid(error.to_string())),
        }
    }

    fn active_window(&self) -> Result<PickedTarget, recording::RecordError> {
        active_window_target()
    }
}

fn picked_target(target: snapline_overlay::record_border::RecordTarget) -> PickedTarget {
    PickedTarget {
        rect: target.rect,
        monitor: target.monitor,
        window: target.window.map(|window| PickedWindow {
            handle: window.handle,
            title: window.title,
        }),
    }
}

fn overlay_magnifier(options: PickOptions) -> snapline_overlay::MagnifierSettings {
    snapline_overlay::MagnifierSettings {
        enabled: options.magnifier,
        zoom: match options.magnifier_zoom {
            MagnifierZoom::X2 => snapline_overlay::MagnifierZoom::X2,
            MagnifierZoom::X4 => snapline_overlay::MagnifierZoom::X4,
            MagnifierZoom::X8 => snapline_overlay::MagnifierZoom::X8,
        },
    }
}

fn overlay_theme(theme: OverlayTheme) -> snapline_overlay::Theme {
    match theme {
        OverlayTheme::Light => snapline_overlay::Theme::Light,
        OverlayTheme::Dark => snapline_overlay::Theme::Dark,
    }
}

fn current_overlay_theme() -> OverlayTheme {
    if snapline_win::theme::is_light() {
        OverlayTheme::Light
    } else {
        OverlayTheme::Dark
    }
}

fn active_window_target() -> Result<PickedTarget, recording::RecordError> {
    let id = CaptureBackend::foreground_window().ok_or(recording::RecordError::NoTarget)?;
    let hwnd = HWND(id.get() as *mut c_void);
    let bounds = snapline_win::windows_enum::frame_bounds(hwnd);
    if bounds.is_empty() {
        return Err(recording::RecordError::NoTarget);
    }
    let title = snapline_win::windows_enum::enumerate(&[])
        .into_iter()
        .find(|entry| entry.hwnd == hwnd)
        .and_then(|entry| entry.title)
        .unwrap_or_default();
    let monitors = snapline_win::monitors::enumerate();
    let monitor = monitors
        .iter()
        .find(|monitor| monitor.bounds.contains(bounds.center()))
        .or_else(|| monitors.iter().find(|monitor| monitor.primary))
        .or_else(|| monitors.first())
        .ok_or(recording::RecordError::NoTarget)?;
    Ok(PickedTarget {
        rect: bounds,
        monitor: monitor.id,
        window: Some(PickedWindow {
            handle: id.get() as isize,
            title,
        }),
    })
}

struct RecordingBorderImpl;

impl BorderHost for RecordingBorderImpl {
    fn show(
        &self,
        request: &BorderRequest,
    ) -> Result<Box<dyn BorderHandle>, recording::RecordError> {
        let monitors = snapline_win::monitors::enumerate();
        let monitor = monitors
            .iter()
            .find(|monitor| monitor.id == request.monitor)
            .or_else(|| {
                monitors
                    .iter()
                    .find(|monitor| monitor.bounds.contains(request.rect.center()))
            })
            .or_else(|| monitors.first())
            .cloned()
            .ok_or_else(|| {
                recording::RecordError::Border("There is no monitor for the border.".to_owned())
            })?;
        snapline_overlay::record_border::RecordBorder::show(
            request.rect,
            monitor,
            overlay_theme(current_overlay_theme()),
        )
        .map(|handle| Box::new(RecordingBorderHandle(handle)) as Box<dyn BorderHandle>)
        .map_err(|error| recording::RecordError::Border(error.to_string()))
    }
}

struct RecordingBorderHandle(snapline_overlay::record_border::RecordBorderHandle);

impl BorderHandle for RecordingBorderHandle {
    fn close(&mut self) {
        self.0.close();
    }
}

struct RecordingControlsImpl;

impl ControlsHost for RecordingControlsImpl {
    fn show(
        &self,
        request: &ControlsRequest,
    ) -> Result<Box<dyn ControlsHandle>, recording::RecordError> {
        let monitors = snapline_win::monitors::enumerate();
        let monitor = monitors
            .iter()
            .find(|monitor| monitor.id == request.monitor)
            .or_else(|| {
                monitors
                    .iter()
                    .find(|monitor| monitor.bounds.contains(request.rect.center()))
            })
            .or_else(|| monitors.first())
            .cloned()
            .ok_or_else(|| {
                recording::RecordError::Border("There is no monitor for the controls.".to_owned())
            })?;
        snapline_overlay::record_controls::RecordControls::show(
            request.rect,
            monitor,
            overlay_theme(current_overlay_theme()),
            Arc::clone(&request.stop),
            Arc::clone(&request.cancel),
        )
        .map(|handle| Box::new(RecordingControlsHandle(handle)) as Box<dyn ControlsHandle>)
        .map_err(|error| recording::RecordError::Border(error.to_string()))
    }
}

struct RecordingControlsHandle(snapline_overlay::record_controls::RecordControlsHandle);

impl ControlsHandle for RecordingControlsHandle {
    fn close(&mut self) {
        self.0.close();
    }
}

struct RecordingFfmpegImpl;

impl FfmpegRunner for RecordingFfmpegImpl {
    fn start(
        &self,
        command: FfmpegCommand,
    ) -> Result<Box<dyn FfmpegProcess>, recording::RecordError> {
        RunningFfmpeg::start(command)
            .map(|running| {
                Box::new(RecordingFfmpegProcess(Some(running))) as Box<dyn FfmpegProcess>
            })
            .map_err(|error| recording::RecordError::FfmpegStart(error.to_string()))
    }

    fn outputs(&self) -> Vec<snapline_core::record_place::OutputBounds> {
        snapline_win::capture::dxgi::output_bounds()
    }

    fn best_encoder(&self, ffmpeg: &std::path::Path, format: VideoFormat) -> VideoEncoder {
        match detect(ffmpeg) {
            Ok(support) => support
                .best(format)
                .unwrap_or_else(|| fallback_encoder(format)),
            Err(error) => {
                tracing::warn!(%error, "hardware encoders could not be detected");
                fallback_encoder(format)
            }
        }
    }
}

struct RecordingFfmpegProcess(Option<RunningFfmpeg>);

impl RecordingFfmpegProcess {
    fn running(&mut self) -> Result<&mut RunningFfmpeg, recording::RecordError> {
        self.0
            .as_mut()
            .ok_or_else(|| recording::RecordError::Stop("FFmpeg is not running.".to_owned()))
    }
}

fn ffmpeg_exit(outcome: StopOutcome) -> FfmpegExit {
    FfmpegExit {
        success: outcome.success(),
        timed_out: outcome.timed_out(),
        message: outcome.stderr().to_owned(),
    }
}

impl FfmpegProcess for RecordingFfmpegProcess {
    fn progress(&mut self) -> Option<FfmpegProgress> {
        let running = self.0.as_mut()?;
        running
            .progress()
            .try_recv()
            .ok()
            .map(|progress| FfmpegProgress {
                out_time_us: progress
                    .out_time_us
                    .or_else(|| progress.out_time_ms.map(|millis| millis * 1_000)),
                frame: progress.frame,
                fps: progress.fps,
                speed: progress.speed,
                done: progress.done,
            })
    }

    fn is_running(&mut self) -> bool {
        self.0.as_mut().is_some_and(|running| running.is_running())
    }

    fn stop(&mut self) -> Result<FfmpegExit, recording::RecordError> {
        let outcome = self
            .running()?
            .stop()
            .map_err(|error| recording::RecordError::Stop(error.to_string()))?;
        self.0 = None;
        Ok(ffmpeg_exit(outcome))
    }

    fn kill(&mut self) -> Result<FfmpegExit, recording::RecordError> {
        let Some(mut running) = self.0.take() else {
            return Ok(FfmpegExit::success());
        };
        let outcome = running
            .kill()
            .map_err(|error| recording::RecordError::Stop(error.to_string()))?;
        Ok(ffmpeg_exit(outcome))
    }

    fn wait(&mut self) -> Result<FfmpegExit, recording::RecordError> {
        let outcome = self
            .running()?
            .wait()
            .map_err(|error| recording::RecordError::Stop(error.to_string()))?;
        self.0 = None;
        Ok(ffmpeg_exit(outcome))
    }
}

struct RecordingAudioImpl;

impl AudioPort for RecordingAudioImpl {
    fn start(
        &self,
        options: &AudioOptions,
    ) -> Result<Box<dyn AudioSession>, recording::RecordError> {
        let request = AudioRequest {
            system: options.system,
            microphone: options.microphone,
            microphone_id: microphone_id(&options.microphone_device),
            format: PcmFormat::default(),
            events: None,
        };
        AudioCapture::start(request)
            .map(|running| {
                let pipes = running.pipes();
                Box::new(RecordingAudioSession {
                    running: Some(running),
                    pipes,
                }) as Box<dyn AudioSession>
            })
            .map_err(|error| recording::RecordError::Audio(error.to_string()))
    }
}

struct RecordingAudioSession {
    running: Option<snapline_win::audio::RunningAudio>,
    pipes: Vec<snapline_core::ffmpeg_args::AudioPipe>,
}

impl AudioSession for RecordingAudioSession {
    fn pipes(&self) -> Vec<snapline_core::ffmpeg_args::AudioPipe> {
        self.pipes.clone()
    }

    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.stop();
        }
    }
}

fn microphone_id(device: &str) -> Option<String> {
    let name = device.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(recording::DEFAULT_MICROPHONE) {
        return None;
    }
    AudioCapture::microphones()
        .into_iter()
        .find(|candidate| candidate.id == name || candidate.name.eq_ignore_ascii_case(name))
        .map(|candidate| candidate.id)
}

static FRAME_ID: AtomicU64 = AtomicU64::new(1);

struct RecordingMediaImpl;

impl MediaPort for RecordingMediaImpl {
    fn first_frame(
        &self,
        video: &std::path::Path,
        ffmpeg: &std::path::Path,
    ) -> Result<Image, recording::RecordError> {
        let folder = recording::temp_folder();
        std::fs::create_dir_all(&folder)
            .map_err(|error| recording::RecordError::Save(error.to_string()))?;
        let output = folder.join(format!(
            "frame-{}-{}.png",
            std::process::id(),
            FRAME_ID.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        let status = Command::new(ffmpeg)
            .args(["-y", "-ss", "0", "-i"])
            .arg(video)
            .args(["-frames:v", "1", "-vf"])
            .arg(format!(
                "scale=w='min({},iw)':h=-2",
                snapline_services::saver::THUMBNAIL_WIDTH
            ))
            .args(["-f", "image2"])
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW.0)
            .status()
            .map_err(|error| recording::RecordError::Save(error.to_string()))?;
        if !status.success() {
            let _ = std::fs::remove_file(&output);
            return Err(recording::RecordError::Save(
                "The first frame of the recording could not be read.".to_owned(),
            ));
        }
        let bytes = std::fs::read(&output)
            .map_err(|error| recording::RecordError::Save(error.to_string()))?;
        let codec =
            WicCodec::new().map_err(|error| recording::RecordError::Save(error.to_string()))?;
        let image = codec
            .decode(&bytes)
            .map_err(|error| recording::RecordError::Save(error.to_string()))?;
        let _ = std::fs::remove_file(&output);
        Ok(image)
    }
}

struct RecordingEventsImpl {
    shell: Shell,
}

impl EventSink for RecordingEventsImpl {
    fn emit(&self, event: AppEvent) {
        if let AppEvent::Record(recording::Event::State(status)) = &event {
            let elapsed =
                (status.state == RecordStateDto::Recording).then_some(status.elapsed_ms / 1_000);
            crate::tray::set_recording(elapsed);
        }
        self.shell.emit(&event);
    }
}

fn recording_ports(shell: Shell, worker: worker::Handle) -> RecordingPorts {
    RecordingPorts {
        picker: Arc::new(RecordingPickerImpl),
        border: Arc::new(RecordingBorderImpl),
        controls: Arc::new(RecordingControlsImpl),
        ffmpeg: Arc::new(RecordingFfmpegImpl),
        audio: Arc::new(RecordingAudioImpl),
        media: Arc::new(RecordingMediaImpl),
        store: Arc::new(WorkerRecordingStore::new(worker)),
        events: Arc::new(RecordingEventsImpl { shell }),
        publisher: Arc::new(recording::RunnerPublisher),
    }
}

struct KnownHosts(Store);

impl HostKeyStore for KnownHosts {
    fn state(&self, host: &str, port: u16, key: &[u8]) -> Result<HostKeyState, UploadError> {
        self.0
            .status(host, port, key)
            .map(|status| match status {
                HostKeyStatus::Match => HostKeyState::Match,
                HostKeyStatus::Mismatch => HostKeyState::Mismatch,
                HostKeyStatus::Unknown => HostKeyState::Unknown,
            })
            .map_err(|error| UploadError::Permanent(error.to_string()))
    }

    fn remember(
        &self,
        host: &str,
        port: u16,
        algorithm: &str,
        key: &[u8],
    ) -> Result<(), UploadError> {
        self.0
            .remember(host, port, algorithm, key)
            .map_err(|error| UploadError::Permanent(error.to_string()))
    }
}

struct HostKeyDialog;

impl HostKeyConfirmer for HostKeyDialog {
    fn confirm(&self, prompt: &HostKeyPrompt) -> bool {
        let message = format!(
            "The host key for {}:{} has not been seen before.\n{} {}\nAccept this key and continue?",
            prompt.host, prompt.port, prompt.algorithm, prompt.fingerprint
        );
        matches!(
            snapline_win::task_dialog::TaskDialog::new(None).confirm(&message, "Accept", "Cancel"),
            Ok(snapline_win::task_dialog::TaskDialogChoice::Upload)
        )
    }
}

struct ClipboardConfirm;

impl snapline_services::clipboard_upload::ConfirmHost for ClipboardConfirm {
    fn confirm(
        &self,
        message: &str,
    ) -> Result<
        snapline_services::clipboard_upload::Confirmation,
        snapline_services::clipboard_upload::ConfirmError,
    > {
        let dialog = snapline_win::task_dialog::TaskDialog::new(None);
        dialog
            .confirm(message, "Upload", "Cancel")
            .map(|choice| match choice {
                snapline_win::task_dialog::TaskDialogChoice::Upload => {
                    snapline_services::clipboard_upload::Confirmation::Upload
                }
                snapline_win::task_dialog::TaskDialogChoice::Cancel => {
                    snapline_services::clipboard_upload::Confirmation::Cancel
                }
            })
            .map_err(|_| snapline_services::clipboard_upload::ConfirmError::Unavailable)
    }
}

fn forward_input(shell: &Shell, event: InputEvent) {
    match event {
        InputEvent::Action(action) => {
            let Some(services) = shell.services() else {
                return;
            };
            let background = action.as_str().starts_with("workflow:");
            let run = move || {
                if let Err(error) =
                    snapline_services::actions::dispatch_notifying(&action, &services)
                {
                    if !matches!(error, ServiceError::NotYet) {
                        tracing::warn!(%action, %error, "hotkey action failed");
                    }
                }
            };
            if background {
                std::thread::spawn(run);
            } else {
                run();
            }
        }
        InputEvent::Resumed => {
            if let Some(services) = shell.services() {
                if let Err(error) = services.hotkeys().on_resume() {
                    tracing::warn!(%error, "hotkeys could not be restored after sleep");
                }
            }
        }
        InputEvent::ThemeChanged => shell.theme_changed(),
    }
}

fn main() -> anyhow::Result<()> {
    let startup_args = Args::from_env();
    if startup_args.uninstall_cleanup {
        std::process::exit(cli::run_uninstall_cleanup());
    }

    if let Some(shortcut) = &startup_args.shortcut_app_id {
        let _ = snapline_win::apartment::init_ui_thread();
        let result = shortcut::set_app_user_model_id(
            std::path::Path::new(shortcut),
            snapline_win::notify::APP_USER_MODEL_ID,
        );
        std::process::exit(i32::from(result.is_err()));
    }

    crash::install();
    logging::init();
    let instance = match single_instance::acquire(&std::env::args().collect::<Vec<_>>()) {
        single_instance::Outcome::First(guard) => guard,
        single_instance::Outcome::HandedOff => return Ok(()),
    };
    std::thread::spawn(crash::offer_previous_report);
    let apartment = snapline_win::apartment::init_ui_thread();
    if apartment != snapline_win::apartment::UiApartment::Ready {
        tracing::warn!(
            ?apartment,
            "the main thread apartment could not be prepared"
        );
    }

    let services =
        Services::with_paths(Paths::discover()?)?.with_file_picker(Arc::new(UploadFileDialogImpl));
    let settings = services.settings_snapshot()?;
    let secrets: Arc<dyn snapline_core::ports::SecretStore + Send + Sync> =
        Arc::new(snapline_win::credentials::CredentialStore::new());
    let host_keys: Arc<dyn HostKeyStore> = match Store::discover() {
        Ok(store) => Arc::new(KnownHosts(store)),
        Err(_) => Arc::new(snapline_services::destinations::UnavailableHostKeyStore),
    };
    let registry: Arc<dyn snapline_services::uploads::DestinationSource> = Arc::new(
        snapline_services::destinations::Registry::new(&services, secrets.clone())
            .with_dialog(Arc::new(SxcuFileDialog))
            .with_sftp(host_keys, Arc::new(HostKeyDialog)),
    );
    let upload_destinations = registry.clone();
    let files = Arc::new(upload_files::HistoryFileSource::new(
        services.paths()?.history_file,
    ));
    let uploader = snapline_services::uploads::UploaderThread::start(
        &services,
        secrets,
        snapline_services::uploads::retry_policy(settings.uploads.retry),
        files,
        registry,
    )?;
    let upload_events = uploader.events();
    let services = services
        .with_codec_factory(codec_factory())
        .with_history_host(Arc::new(history_host::WindowsHistoryHost::new(Arc::new(
            snapline_win::clipboard::Clipboard::new(),
        ))))
        .with_upload_runtime(uploader.runtime());
    let startup_services = services.clone();
    let capture_settings = services.settings_snapshot()?;
    snapline_services::capture::install(CaptureService::new(
        Arc::new(CaptureBackend::new(CaptureOptions {
            include_cursor: capture_settings.capture.include_cursor,
        })),
        Arc::new(snapline_overlay::OverlayHostImpl::new()),
        Arc::new(Clipboard::new()),
        Arc::new(CountdownAdapter),
        Arc::new(AppCaptureContext {
            services: services.clone(),
        }),
    ))?;
    let worker = worker::Worker::start(services.clone(), || {
        let codec = WicCodec::new().map_err(|error| ServiceError::Internal(error.to_string()))?;
        Ok(ServicePorts::new(
            Arc::new(codec),
            Arc::new(Clipboard::new()),
            Arc::new(ToastNotifier),
            Arc::new(SoundPlayer),
            Arc::new(SystemClock),
        ))
    })?;
    let worker_handle = worker.handle.clone();
    let worker_events = worker.events;
    snapline_services::tools::combine::install_host(Arc::new(CombineHostImpl {
        worker: worker_handle.clone(),
    }));
    snapline_services::editor::install(EditorPorts {
        host: Arc::new(EditorHostImpl),
        file_dialog: Arc::new(EditorFileDialogImpl),
        clipboard: Arc::new(Clipboard::new()),
        clipboard_source: Arc::new(EditorClipboardSourceImpl),
        saver: Arc::new(worker_handle.clone()),
        uploader: Arc::new(services.upload()),
        pin: Arc::new(EditorPinImpl::default()),
    })?;
    snapline_services::tools::pin::install_menu(Arc::new(PinMenuImpl))?;
    snapline_services::shell_actions::install_host(Arc::new(ShellHostImpl))?;
    snapline_services::tools::convert::install_backend(Arc::new(FfmpegBackendImpl));
    snapline_services::tools::convert::install_file_picker(Arc::new(VideoPickerImpl));
    snapline_services::clipboard_upload::install(
        snapline_services::clipboard_upload::ClipboardUpload::new(
            Arc::new(
                snapline_services::clipboard_upload::CoreClipboardSource::new(Arc::new(
                    Clipboard::new(),
                )),
            ),
            codec_factory(),
            Arc::new(worker_handle.clone()),
            Arc::new(services.upload()),
            Arc::new(ClipboardConfirm),
        ),
    )?;
    let runner_clipboard: Arc<dyn snapline_core::ports::Clipboard + Send + Sync> =
        Arc::new(Clipboard::new());
    let runner_ports = snapline_services::workflows::RunnerPorts::service(
        &services,
        snapline_services::workflows::ServiceWorkflowOptions {
            worker: worker_handle.clone(),
            queue: uploader.handle(),
            destinations: upload_destinations,
            clipboard: runner_clipboard,
            notifier: Arc::new(ToastNotifier),
            sound: Arc::new(SoundPlayer),
            ocr: None,
            annotate: None,
        },
    );
    snapline_services::workflows::install_runner(
        snapline_services::workflows::WorkflowRunner::new(&services, runner_ports),
    )?;

    let shell = Shell::new();
    main_loop::init({
        let shell = shell.clone();
        move |argv| {
            tracing::info!(?argv, "second instance started");
            let forwarded = Args::parse_forwarded(&argv);
            if let Some(window) = forwarded.budget_open {
                open_budget_window(&shell, window);
                return;
            }
            tray::handle_second_instance(&shell, &argv);
        }
    })
    .map_err(|error| anyhow::anyhow!(error))?;

    if let Err(error) = snapline_win::notify::register_app_name() {
        tracing::warn!(%error, "the notification identity could not be registered");
    }
    if let Err(error) = snapline_services::uploads::maintain_history(&startup_services, true) {
        tracing::warn!(%error, "upload history could not be maintained");
    }
    let activation_services = startup_services.clone();
    snapline_win::notify::set_activation_handler(move |activation| match activation {
        snapline_win::notify::ToastActivation::OpenCapture(id) => {
            let Ok(id) = i64::try_from(id) else {
                return;
            };
            if let Err(error) = activation_services.history().open_from_notification(id) {
                tracing::warn!(%error, "the notified capture could not be opened");
            }
        }
        snapline_win::notify::ToastActivation::CopyImage(id) => {
            let services = activation_services.clone();
            std::thread::spawn(move || {
                if let Err(error) = services
                    .history()
                    .handle(snapline_services::dto::history::HistoryReq::CopyImage { id })
                {
                    snapline_services::actions::notify_failure(&error, &services);
                }
            });
        }
    });
    let maintenance_services = startup_services.clone();
    let _ = std::thread::Builder::new()
        .name("snapline-history-maintenance".to_owned())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            if let Err(error) =
                snapline_services::uploads::maintain_history(&maintenance_services, false)
            {
                tracing::warn!(%error, "upload history could not be expired");
            }
        });
    let input_shell = shell.clone();
    let input = snapline_win::input::spawn_with(move |event| forward_input(&input_shell, event))?;
    let services = startup_services
        .clone()
        .with_hotkey_input(Arc::new(InputPort(input.clone())))
        .with_prtsc_registry(Arc::new(PrtscPort))
        .with_notifier(Arc::new(ToastNotifier));
    shell.set_services(services.clone());
    snapline_services::recording::install_ports(recording_ports(
        shell.clone(),
        worker_handle.clone(),
    ))?;
    services.hotkeys().on_settings_changed()?;
    let event_shell = shell.clone();
    std::thread::spawn(move || {
        while let Ok(event) = worker_events.recv() {
            event_shell.emit(&event);
        }
    });
    if let Some(upload_events) = upload_events {
        let upload_shell = shell.clone();
        let persist_services = services.clone();
        std::thread::spawn(move || {
            let mut batches = IntakeLinkBatches::default();
            let mut gate = snapline_services::uploads::PersistGate::default();
            while let Ok(event) = upload_events.recv() {
                if let snapline_services::AppEvent::Upload(
                    snapline_services::uploads::Event::StateChanged(state),
                ) = &event
                {
                    if gate.should_persist(state) {
                        snapline_services::uploads::persist(&persist_services, state);
                    }
                    if state.status == snapline_services::dto::upload::UploadStatus::Done {
                        if let Some(link) = &state.link {
                            if let Ok(runner) = snapline_services::workflows::installed_runner() {
                                let id = state.id;
                                let link = link.clone();
                                let page_link = state.page_link.clone();
                                std::thread::spawn(move || {
                                    let _ = runner.resume_upload(id, link, page_link, None);
                                });
                            }
                        }
                    }
                    batches.handle(&persist_services, state);
                }
                upload_shell.emit(&event);
            }
        });
    }
    tray::init(&shell).map_err(|error| anyhow::anyhow!(error))?;
    tracing::info!(
        budget_run = startup_args.budget_run,
        budget_open = ?startup_args.budget_open,
        action = ?startup_args.action,
        autostart = startup_args.autostart,
        uninstall_cleanup = startup_args.uninstall_cleanup,
        "command-line arguments"
    );
    if !startup_args.budget_run {
        if let Some(window) = startup_args.budget_open {
            open_budget_window(&shell, window);
        }
    }
    if startup_args.autostart {
        let start_minimised = startup_services
            .settings_snapshot()
            .map(|settings| settings.general.start_minimised)
            .unwrap_or(true);
        if !start_minimised {
            shell.open_main("/history");
        }
    }

    main_loop::run();

    input.shutdown();
    let _ = worker_handle.shutdown();
    drop(instance);
    Ok(())
}

struct IntakeLink {
    url: String,
    name: String,
    page_url: Option<String>,
    capture_id: Option<i64>,
}

#[derive(Default)]
struct IntakeLinkBatches(HashMap<String, BTreeMap<u32, Option<IntakeLink>>>);

impl IntakeLinkBatches {
    fn handle(&mut self, services: &Services, state: &UploadState) {
        let Some(batch) = &state.link_batch else {
            return;
        };
        if batch.index >= batch.size {
            return;
        }
        let link = match state.status {
            UploadStatus::Done => state.link.clone().map(|url| IntakeLink {
                url,
                name: state.file_name.clone(),
                page_url: state.page_link.clone(),
                capture_id: state.capture_id,
            }),
            UploadStatus::Failed | UploadStatus::Cancelled => None,
            _ => return,
        };
        let completed = self.0.entry(batch.id.clone()).or_default();
        completed.insert(batch.index, link);
        if completed.len() != batch.size as usize {
            return;
        }

        let Some(completed) = self.0.remove(&batch.id) else {
            return;
        };
        let completed: Vec<IntakeLink> = completed.into_values().flatten().collect();
        let capture_id = completed.iter().find_map(|link| link.capture_id);
        let format = intake_link_format(services);
        let links: Vec<_> = completed
            .iter()
            .map(|link| format.format(&link.url, &link.name, link.page_url.as_deref()))
            .collect();
        if links.is_empty() {
            return;
        }
        if let Err(error) = Clipboard::new().set_text(&links.join("\n")) {
            tracing::warn!(%error, "upload links could not be copied to the clipboard");
            return;
        }
        let message = if links.len() == 1 {
            "1 link copied to the clipboard.".to_owned()
        } else {
            format!("{} links copied to the clipboard.", links.len())
        };
        ToastNotifier.toast(Notice {
            capture_id,
            ..Notice::new("Upload links copied", message)
        });
    }
}

fn intake_link_format(services: &Services) -> snapline_core::link_format::LinkFormat {
    use snapline_core::link_format::LinkFormat;
    use snapline_store::settings::general::LinkFormat as StoredFormat;

    match services
        .settings_snapshot()
        .map(|settings| settings.general.copied_link_format)
    {
        Ok(StoredFormat::Markdown) => LinkFormat::Markdown,
        Ok(StoredFormat::Html) => LinkFormat::Html,
        Ok(StoredFormat::Bbcode) => LinkFormat::Bbcode,
        Ok(StoredFormat::Page) => LinkFormat::Page,
        Ok(StoredFormat::Direct) | Err(_) => LinkFormat::Direct,
    }
}

fn codec_factory() -> Arc<CodecFactory> {
    Arc::new(|| {
        let codec = snapline_win::wic::WicCodec::new()
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        let codec: Arc<dyn snapline_core::ports::ImageCodec> = Arc::new(codec);
        Ok(codec)
    })
}

fn open_budget_window(shell: &Shell, window: BudgetWindow) {
    match window {
        BudgetWindow::Main => shell.open_main("/history"),
    }
}

#[cfg(test)]
mod recording_desktop_tests {
    use std::time::Duration;

    use snapline_core::ffmpeg_args::{video_command, VideoFrameRate, VideoQuality, VideoRequest};
    use snapline_core::geometry::Rect;
    use snapline_core::recording::RecordingTarget;

    use super::*;

    fn bundled_ffmpeg() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("third_party")
            .join("ffmpeg")
            .join("ffmpeg.exe")
    }

    #[test]
    #[ignore = "desktop"]
    fn a_real_ffmpeg_recording_writes_a_playable_file() {
        let exe = bundled_ffmpeg();
        assert!(exe.is_file());
        let directory = std::env::temp_dir().join("snapline-desktop-tests");
        std::fs::create_dir_all(&directory).expect("temp folder");
        let output = directory.join("desktop-recording.mp4");
        let command = video_command(&VideoRequest {
            ffmpeg: exe,
            target: RecordingTarget::region(Rect::new(Px(0), Px(0), Px(320), Px(240))),
            output: output.clone(),
            output_index: 0,
            format: VideoFormat::Mp4H264,
            encoder: fallback_encoder(VideoFormat::Mp4H264),
            frame_rate: VideoFrameRate::Fps30,
            quality: VideoQuality::High,
            audio: Vec::new(),
        })
        .expect("recording command");
        let mut running = RunningFfmpeg::start(command).expect("start FFmpeg");
        std::thread::sleep(Duration::from_secs(1));
        let outcome = running.stop().expect("stop FFmpeg");
        assert!(outcome.success());
        let bytes = std::fs::metadata(&output).expect("recording file").len();
        assert!(bytes > 0);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    #[ignore = "desktop"]
    fn the_media_adapter_reads_the_first_frame_of_a_real_video() {
        let exe = bundled_ffmpeg();
        assert!(exe.is_file());
        let directory = std::env::temp_dir().join("snapline-desktop-tests");
        std::fs::create_dir_all(&directory).expect("temp folder");
        let video = directory.join("first-frame.mp4");
        let status = Command::new(&exe)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=320x180:r=1",
                "-frames:v",
                "1",
            ])
            .arg(&video)
            .creation_flags(CREATE_NO_WINDOW.0)
            .status()
            .expect("encode the test video");
        assert!(status.success());
        let image = RecordingMediaImpl
            .first_frame(&video, &exe)
            .expect("first frame");
        assert_eq!(image.width(), 320);
        assert_eq!(image.height(), 180);
        let _ = std::fs::remove_file(video);
    }

    #[test]
    #[ignore = "desktop"]
    fn the_audio_adapter_opens_and_stops_the_loopback_pipe() {
        let mut session = RecordingAudioImpl
            .start(&AudioOptions {
                system: true,
                microphone: false,
                microphone_device: recording::DEFAULT_MICROPHONE.to_owned(),
            })
            .expect("loopback audio");
        assert_eq!(session.pipes().len(), 1);
        assert!(session
            .pipes()
            .iter()
            .any(|pipe| pipe.source == snapline_core::ffmpeg_args::AudioSource::System));
        session.stop();
    }
}
