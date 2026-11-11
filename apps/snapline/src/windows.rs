use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use snapline_services::app::Event as AppAreaEvent;
use snapline_services::AppEvent;
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::{theme, webview_tuning};

const LINGER: Duration = Duration::from_secs(15);
const MAIN_LABEL: &str = "main";
const PANEL_LABEL: &str = "panel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowKind {
    Panel,
    Main,
}

impl WindowKind {
    fn label(self) -> &'static str {
        match self {
            Self::Panel => PANEL_LABEL,
            Self::Main => MAIN_LABEL,
        }
    }

    fn initial_route(self) -> &'static str {
        match self {
            Self::Panel => "/panel",
            Self::Main => "/history",
        }
    }

    fn background(self) -> Color {
        let light = theme::is_light();
        match self {
            Self::Main => theme::mica_background(light),
            Self::Panel => theme::layer_background(light),
        }
    }

    fn uses_mica(self) -> bool {
        matches!(self, Self::Main) && theme::has_mica()
    }
}

#[derive(Debug, Default)]
pub struct WindowManager {
    generation: AtomicU64,
}

impl WindowManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_main(&self, app: &AppHandle, route: &str) -> tauri::Result<()> {
        self.open(app, WindowKind::Main, route)
    }

    pub fn open_panel(&self, app: &AppHandle) -> tauri::Result<()> {
        self.open(app, WindowKind::Panel, WindowKind::Panel.initial_route())
    }

    pub fn close_panel(&self, app: &AppHandle) {
        if let Some(window) = app.get_webview_window(PANEL_LABEL) {
            let _ = window.destroy();
        }
        self.after_close(app);
    }

    pub fn ready(&self, window: &WebviewWindow) -> tauri::Result<()> {
        window.show()?;
        window.set_focus()?;
        Ok(())
    }

    fn open(&self, app: &AppHandle, kind: WindowKind, route: &str) -> tauri::Result<()> {
        if let Some(window) = app.get_webview_window(kind.label()) {
            let event = AppEvent::App(AppAreaEvent::Navigate {
                route: route.to_string(),
            });
            let _ = window.emit("snapline://event", event);
            window.show()?;
            window.set_focus()?;
            self.generation.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }

        let url = WebviewUrl::App(format!("index.html#{route}").into());
        let mut builder = WebviewWindowBuilder::new(app, kind.label(), url)
            .title("Snapline")
            .visible(false)
            .decorations(matches!(kind, WindowKind::Main))
            .additional_browser_args(webview_tuning::ADDITIONAL_BROWSER_ARGS);
        if kind.uses_mica() {
            builder = builder.transparent(true);
        } else {
            builder = builder.background_color(kind.background());
        }
        if let Some(directory) = webview_tuning::data_directory() {
            builder = builder.data_directory(directory);
        }
        builder = match kind {
            WindowKind::Main => builder
                .inner_size(1000.0, 700.0)
                .min_inner_size(900.0, 600.0)
                .resizable(true)
                .center(),
            WindowKind::Panel => builder
                .inner_size(344.0, 1.0)
                .min_inner_size(344.0, 1.0)
                .resizable(false)
                .skip_taskbar(true)
                .always_on_top(true),
        };
        let window = builder.build()?;
        theme::apply_to_window(&window);
        self.generation.fetch_add(1, Ordering::SeqCst);
        webview_tuning::harden(&window);

        let app_handle = app.clone();
        let label = kind.label().to_string();
        window.on_window_event(move |event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                if let Some(target) = app_handle.get_webview_window(&label) {
                    let _ = target.destroy();
                }
                app_handle.state::<Self>().after_close(&app_handle);
            }
            WindowEvent::ThemeChanged(_) => theme::refresh_open_windows(&app_handle),
            WindowEvent::Focused(false) if label == PANEL_LABEL => {
                if let Some(target) = app_handle.get_webview_window(&label) {
                    if target.is_visible().unwrap_or(false) {
                        let _ = target.destroy();
                        app_handle.state::<Self>().after_close(&app_handle);
                    }
                }
            }
            _ => {}
        });
        Ok(())
    }

    fn after_close(&self, app: &AppHandle) {
        if !app.webview_windows().is_empty() {
            return;
        }
        let token = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(LINGER);
            let manager = app.state::<Self>();
            if manager.generation.load(Ordering::SeqCst) != token {
                return;
            }
            if app.webview_windows().is_empty() {
                tracing::info!("webview linger elapsed with no windows");
            }
        });
    }
}
