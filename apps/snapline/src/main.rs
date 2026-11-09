#![windows_subsystem = "windows"]

mod cli;
mod crash;
mod ipc;
mod logging;
mod panel;
mod theme;
mod tray;
mod webview_tuning;
mod windows;

use cli::{Args, BudgetWindow};
use snapline_services::Services;
use tauri::{Manager, RunEvent};
use windows::WindowManager;

fn main() -> anyhow::Result<()> {
    crash::install();
    logging::init();

    let startup_args = Args::from_env();

    let app = tauri::Builder::default()
        .manage(Services::new())
        .manage(WindowManager::new())
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
            Ok(())
        })
        .build(tauri::generate_context!())?;

    app.run(|_app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            if !tray::exit_requested() {
                api.prevent_exit();
            }
        }
    });
    Ok(())
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
