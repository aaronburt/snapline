#![windows_subsystem = "windows"]

mod cli;
mod crash;
mod events;
mod history_host;
mod ipc;
mod logging;
mod panel;
mod protocol;
mod theme;
mod tray;
mod webview_tuning;
mod windows;

use std::sync::Arc;

use cli::{Args, BudgetWindow};
use snapline_core::chord::Chord;
use snapline_core::hotkey_plan::{Blocked, Owner};
use snapline_services::hotkeys::{HotkeyInput, PrtscRegistry};
use snapline_services::saver::{ServicePorts, SystemClock};
use snapline_services::worker;
use snapline_services::{CodecFactory, ServiceError, Services};
use snapline_store::io::Paths;
use snapline_win::clipboard::Clipboard;
use snapline_win::input::{InputEvent, InputHandle};
use snapline_win::ports_impls::{SoundPlayer, ToastNotifier};
use snapline_win::wic::WicCodec;
use tauri::{Manager, RunEvent};
use windows::WindowManager;

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
    crash::install();
    logging::init();

    let startup_args = Args::from_env();
    let services = Services::with_paths(Paths::discover()?)?
        .with_codec_factory(codec_factory())
        .with_history_host(Arc::new(history_host::WindowsHistoryHost::new(Arc::new(
            snapline_win::clipboard::Clipboard::new(),
        ))));
    let startup_services = services.clone();
    let thumb_services = services.clone();
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

    let app = tauri::Builder::default()
        .manage(WindowManager::new())
        .manage(worker_handle)
        .register_uri_scheme_protocol("thumb", move |_context, request| {
            protocol::handle(&thumb_services, request)
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
