#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::RunEvent;

static EXIT_CHOSEN: AtomicBool = AtomicBool::new(false);

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |_app, argv, cwd| {
                tracing::info!(?argv, ?cwd, "second instance started");
            },
        ))
        .setup(|app| {
            let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&exit])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.ico"))?;
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .tooltip("Snapline")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id().as_ref() == "exit" {
                        EXIT_CHOSEN.store(true, Ordering::SeqCst);
                        app.exit(0);
                    }
                })
                .build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())?;

    app.run(|_app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            if !EXIT_CHOSEN.load(Ordering::SeqCst) {
                api.prevent_exit();
            }
        }
    });
    Ok(())
}
