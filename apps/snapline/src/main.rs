#![windows_subsystem = "windows"]

mod cli;
mod crash;
mod events;
mod history_host;
mod ipc;
mod logging;
mod panel;
mod protocol;
mod protocol_combine;
mod theme;
mod tray;
mod upload_files;
mod webview_tuning;
mod windows;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ::windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use ::windows::Win32::System::Ole::CF_HDROP;
use ::windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use cli::{Args, BudgetWindow};
use snapline_core::chord::Chord;
use snapline_core::geometry::{Point, Px};
use snapline_core::hotkey_plan::{Blocked, Owner};
use snapline_core::ports::OverlayTheme;
use snapline_core::ffmpeg_args::FfmpegCommand;
use snapline_services::capture::{CaptureContext, CaptureService, Countdown, LastRegion};
use snapline_services::editor::{
    EditorAction, EditorClipboardContent, EditorClipboardSource, EditorFileDialog, EditorHost,
    EditorPin, EditorPorts, EditorResult,
};
use snapline_services::hotkeys::{HotkeyInput, PrtscRegistry};
use snapline_services::saver::{CaptureRequest, ServicePorts, SystemClock};
use snapline_services::tools::convert::{
    BackendExit, BackendProgress, ConvertError, ConvertFilePicker, FfmpegBackend, FfmpegProcess,
};
use snapline_services::worker;
use snapline_services::{CodecFactory, ServiceError, Services};
use snapline_store::io::Paths;
use snapline_store::settings::Settings;
use snapline_win::capture::{CaptureBackend, CaptureOptions};
use snapline_win::clipboard::Clipboard;
use snapline_win::input::{InputEvent, InputHandle};
use snapline_win::ports_impls::{SoundPlayer, ToastNotifier};
use snapline_win::ffmpeg::RunningFfmpeg;
use snapline_win::wic::WicCodec;
use tauri::{Manager, RunEvent};
use windows::WindowManager;

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

struct InputState(InputHandle);

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
        let outcome = self.worker.save_capture(CaptureRequest {
            image,
            source_app: Some("Snapline".to_string()),
            window_title: "Combined image".to_string(),
        })?;
        Ok(outcome.capture_id)
    }
}

struct EditorHostImpl;

impl EditorHost for EditorHostImpl {
    fn open(
        &self,
        image: snapline_core::image::Image,
    ) -> Result<EditorResult, ServiceError> {
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

struct AppCaptureContext {
    services: Services,
}

impl CaptureContext for AppCaptureContext {
    fn capture_settings(&self) -> Result<Settings, ServiceError> {
        self.services.settings_snapshot()
    }

    fn save_last_region(&self, region: LastRegion) -> Result<(), ServiceError> {
        <Services as CaptureContext>::save_last_region(&self.services, region)
    }

    fn foreground_window(&self) -> Option<snapline_core::ids::WindowId> {
        CaptureBackend::foreground_window()
    }

    fn window_under_cursor(&self) -> Option<snapline_core::ids::WindowId> {
        CaptureBackend::window_under_cursor()
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
    fn start(&self, command: FfmpegCommand) -> Result<Box<dyn FfmpegProcess>, ConvertError> {
        RunningFfmpeg::start(command)
            .map(|running| Box::new(FfmpegProcessImpl(running)) as Box<dyn FfmpegProcess>)
            .map_err(|error| ConvertError::Ffmpeg(error.to_string()))
    }
}

struct FfmpegProcessImpl(RunningFfmpeg);

impl FfmpegProcess for FfmpegProcessImpl {
    fn progress(&mut self) -> Option<BackendProgress> {
        self.0.progress().try_recv().ok().map(|progress| BackendProgress {
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

fn forward_input(app: &tauri::AppHandle, event: InputEvent) {
    match event {
        InputEvent::Action(action) => {
            let services = app.state::<Services>();
            if let Err(error) = snapline_services::actions::dispatch(&action, services.inner()) {
                if !matches!(error, ServiceError::NotYet) {
                    tracing::warn!(%action, %error, "hotkey action failed");
                }
            }
        }
        InputEvent::Resumed => {
            if let Err(error) = app.state::<Services>().hotkeys().on_resume() {
                tracing::warn!(%error, "hotkeys could not be restored after sleep");
            }
        }
        InputEvent::ThemeChanged => {
            theme::refresh_open_windows(app);
        }
    }
}

fn main() -> anyhow::Result<()> {
    let startup_args = Args::from_env();
    if startup_args.uninstall_cleanup {
        std::process::exit(cli::run_uninstall_cleanup());
    }

    crash::install();
    logging::init();

    let services = Services::with_paths(Paths::discover()?)?;
    let settings = services.settings_snapshot()?;
    let secrets: Arc<dyn snapline_core::ports::SecretStore + Send + Sync> =
        Arc::new(snapline_win::credentials::CredentialStore::new());
    let registry: Arc<dyn snapline_services::uploads::DestinationSource> =
        Arc::new(snapline_services::destinations::Registry::new(
            &services,
            secrets.clone(),
        ));
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
    let thumb_services = services.clone();
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
        let codec = WicCodec::new()
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
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
    snapline_services::tools::convert::install_backend(Arc::new(FfmpegBackendImpl));
    snapline_services::tools::convert::install_file_picker(Arc::new(VideoPickerImpl));
    snapline_services::clipboard_upload::install(
        snapline_services::clipboard_upload::ClipboardUpload::new(
            Arc::new(snapline_services::clipboard_upload::CoreClipboardSource::new(
                Arc::new(Clipboard::new()),
            )),
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

    let app = tauri::Builder::default()
        .manage(WindowManager::new())
        .manage(worker_handle)
        .register_uri_scheme_protocol("thumb", move |_context, request| {
            protocol::handle(&thumb_services, request)
        })
        .register_uri_scheme_protocol("combine", |_context, request| {
            protocol_combine::handle(request)
        })
        .invoke_handler(tauri::generate_handler![
            ipc::app::ipc_app,
            ipc::settings::ipc_settings,
            ipc::hotkeys::ipc_hotkeys,
            ipc::history::ipc_history,
            ipc::upload::ipc_upload,
            ipc::workflow::ipc_workflow,
            ipc::record::ipc_record,
            ipc::tool::ipc_tool
        ])
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            tracing::info!(?argv, ?cwd, "second instance started");
            let forwarded = Args::parse_forwarded(&argv);
            if let Some(window) = forwarded.budget_open {
                open_budget_window(app, window);
                return;
            }
            tray::handle_second_instance(app, &argv);
        }))
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let input = snapline_win::input::spawn_with(move |event| forward_input(&app_handle, event))?;
            let services = startup_services
                .clone()
                .with_hotkey_input(Arc::new(InputPort(input.clone())))
                .with_prtsc_registry(Arc::new(PrtscPort))
                .with_notifier(Arc::new(ToastNotifier));
            app.manage(services.clone());
            services.hotkeys().on_settings_changed()?;
            app.manage(InputState(input));
            let event_app = app.handle().clone();
            std::thread::spawn(move || {
                while let Ok(event) = worker_events.recv() {
                    events::emit(&event_app, &event);
                }
            });
            if let Some(upload_events) = upload_events {
                let upload_app = app.handle().clone();
                let persist_worker = app.state::<snapline_services::worker::Handle>().inner().clone();
                std::thread::spawn(move || {
                    while let Ok(event) = upload_events.recv() {
                        if let snapline_services::AppEvent::Upload(
                            snapline_services::uploads::Event::StateChanged(state),
                        ) = &event
                        {
                            let _ = persist_worker.enqueue(snapline_services::Job::Area(
                                snapline_services::AreaJob::Upload(
                                    snapline_services::uploads::Job::Persist(state.clone()),
                                ),
                            ));
                            if state.status == snapline_services::dto::upload::UploadStatus::Done {
                                if let Some(link) = &state.link {
                                    if let Ok(runner) =
                                        snapline_services::workflows::installed_runner()
                                    {
                                        let _ = runner.resume_upload(
                                            state.id,
                                            link.clone(),
                                            state.page_link.clone(),
                                            None,
                                        );
                                    }
                                }
                            }
                        }
                        events::emit(&upload_app, &event);
                    }
                });
            }
            tray::init(app.handle())?;
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
                    open_budget_window(app.handle(), window);
                }
            }
            if startup_args.autostart {
                let start_minimised = startup_services
                    .settings_snapshot()
                    .map(|settings| settings.general.start_minimised)
                    .unwrap_or(true);
                if !start_minimised {
                    let manager = app.state::<WindowManager>();
                    if let Err(error) = manager.open_main(app.handle(), "/history") {
                        tracing::error!(%error, "could not open the main window after autostart");
                    }
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())?;

    app.run(|_app, event| {
        if let RunEvent::ExitRequested { api, .. } = &event {
            if !tray::exit_requested() {
                api.prevent_exit();
            }
        }
        if let RunEvent::Exit = event {
            _app.state::<InputState>().0.shutdown();
            let _ = _app.state::<worker::Handle>().shutdown();
        }
    });
    Ok(())
}

fn codec_factory() -> Arc<CodecFactory> {
    Arc::new(|| {
        let codec = snapline_win::wic::WicCodec::new()
            .map_err(|error| ServiceError::Internal(error.to_string()))?;
        let codec: Arc<dyn snapline_core::ports::ImageCodec> = Arc::new(codec);
        Ok(codec)
    })
}

fn open_budget_window(app: &tauri::AppHandle, window: BudgetWindow) {
    match window {
        BudgetWindow::Panel => {
            let (x, y, width, height) = budget_tray_rect(app);
            panel::show(app, x, y, width, height);
        }
        BudgetWindow::Main => {
            let manager = app.state::<WindowManager>();
            if let Err(error) = manager.open_main(app, "/history") {
                tracing::error!(%error, "could not open the main window for the budget check");
            }
        }
    }
}

fn budget_tray_rect(app: &tauri::AppHandle) -> (f64, f64, f64, f64) {
    match app.primary_monitor() {
        Ok(Some(monitor)) => {
            let work = monitor.work_area();
            let right = f64::from(work.position.x) + f64::from(work.size.width);
            let bottom = f64::from(work.position.y) + f64::from(work.size.height);
            (right - 36.0, bottom - 36.0, 24.0, 24.0)
        }
        _ => (0.0, 0.0, 24.0, 24.0),
    }
}
