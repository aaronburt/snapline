use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use snapline_core::actions::{find, ActionId};
use snapline_services::actions::dispatch;
use snapline_services::{ServiceError, Services};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IconMenuItem, Menu, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::windows::WindowManager;

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconState {
    Normal,
    Recording,
    Paused,
}

#[derive(Debug, Clone, Copy)]
struct TrayState {
    paused: bool,
    recording_seconds: Option<u64>,
    system_audio: bool,
    microphone: bool,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            paused: false,
            recording_seconds: None,
            system_audio: true,
            microphone: false,
        }
    }
}

impl TrayState {
    fn icon_state(self) -> IconState {
        if self.recording_seconds.is_some() {
            IconState::Recording
        } else if self.paused {
            IconState::Paused
        } else {
            IconState::Normal
        }
    }
}

pub struct TrayHandle {
    tray: TrayIcon<Wry>,
    state: Mutex<TrayState>,
}

impl TrayHandle {
    fn state(&self) -> std::sync::MutexGuard<'_, TrayState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn refresh(&self) -> tauri::Result<()> {
        let state = *self.state();
        let menu = build_menu(&self.tray.app_handle().clone(), state)?;
        self.tray.set_menu(Some(menu))?;
        self.tray
            .set_icon(Some(icon_for_state(state.icon_state())?))?;
        self.tray.set_tooltip(Some(tooltip(state)))?;
        Ok(())
    }
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let state = TrayState::default();
    let menu = build_menu(app, state)?;
    let icon = icon_for_state(state.icon_state())?;
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip(tooltip(state))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } => {
                let app = tray.app_handle();
                let rect = crate::panel::TrayRect::from_tauri(rect);
                crate::panel::show(app, rect.x, rect.y, rect.width, rect.height);
            }
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                let app = tray.app_handle();
                let manager = app.state::<WindowManager>();
                if let Err(error) = manager.open_main(app, "/history") {
                    tracing::error!(%error, "could not open the main window");
                }
            }
            _ => {}
        })
        .build(app)?;
    app.manage(TrayHandle {
        tray,
        state: Mutex::new(state),
    });
    Ok(())
}

pub fn exit_requested() -> bool {
    EXIT_REQUESTED.load(Ordering::SeqCst)
}

pub fn handle_second_instance(app: &AppHandle, argv: &[String]) {
    if let Some(position) = argv.iter().position(|argument| argument == "--action") {
        if let Some(action) = argv.get(position + 1) {
            run_action(app, action);
            return;
        }
    }
    open_main(app, "/history");
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    let handle = app.state::<TrayHandle>();
    {
        let mut state = handle.state();
        state.paused = paused;
    }
    if let Err(error) = handle.refresh() {
        tracing::error!(%error, "could not refresh the tray");
    }
}

#[allow(dead_code)]
pub fn set_recording(app: &AppHandle, elapsed_seconds: Option<u64>) {
    let handle = app.state::<TrayHandle>();
    {
        let mut state = handle.state();
        state.recording_seconds = elapsed_seconds;
    }
    if let Err(error) = handle.refresh() {
        tracing::error!(%error, "could not refresh the tray");
    }
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "app_exit" => {
            EXIT_REQUESTED.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        "app_open" => open_main(app, "/history"),
        "app_settings" => open_main(app, "/settings/general"),
        "app_pause_hotkeys" => {
            let handle = app.state::<TrayHandle>();
            let paused = !handle.state().paused;
            drop(handle);
            set_paused(app, paused);
        }
        "record_system_audio" => toggle_check(app, CheckKind::SystemAudio),
        "record_microphone" => toggle_check(app, CheckKind::Microphone),
        action => run_action(app, action),
    }
}

enum CheckKind {
    SystemAudio,
    Microphone,
}

fn toggle_check(app: &AppHandle, kind: CheckKind) {
    let handle = app.state::<TrayHandle>();
    {
        let mut state = handle.state();
        match kind {
            CheckKind::SystemAudio => state.system_audio = !state.system_audio,
            CheckKind::Microphone => state.microphone = !state.microphone,
        }
    }
    if let Err(error) = handle.refresh() {
        tracing::error!(%error, "could not refresh the tray");
    }
}

fn open_main(app: &AppHandle, route: &str) {
    let manager = app.state::<WindowManager>();
    if let Err(error) = manager.open_main(app, route) {
        tracing::error!(%error, "could not open the main window");
    }
}

fn run_action(app: &AppHandle, action: &str) {
    if action == "app_exit" {
        EXIT_REQUESTED.store(true, Ordering::SeqCst);
        app.exit(0);
        return;
    }
    if action == "app_open" {
        open_main(app, "/history");
        return;
    }
    if action == "app_settings" {
        open_main(app, "/settings/general");
        return;
    }
    if action == "app_pause_hotkeys" {
        let handle = app.state::<TrayHandle>();
        let paused = !handle.state().paused;
        drop(handle);
        set_paused(app, paused);
        return;
    }
    let services = app.state::<Services>();
    match dispatch(&ActionId::new(action), services.inner()) {
        Ok(()) => {}
        Err(ServiceError::NotYet) => {
            tracing::info!(%action, "action is not available yet");
        }
        Err(error) => {
            tracing::warn!(%action, %error, "action failed");
        }
    }
}

fn chord_label(action: &str, label: &str) -> String {
    match find(&ActionId::new(action)).and_then(|definition| definition.default_chord) {
        Some(chord) => format!("{label} · {chord}"),
        None => label.to_string(),
    }
}

fn icon_item(
    app: &AppHandle,
    id: &str,
    text: String,
    icon: &str,
) -> tauri::Result<IconMenuItem<Wry>> {
    IconMenuItem::with_id(app, id, text, true, menu_icon(icon), None::<&str>)
}

fn menu_icon(name: &str) -> Option<Image<'static>> {
    let light = crate::theme::is_light();
    let bytes: &'static [u8] = match (name, light) {
        ("region", false) => include_bytes!("../icons/menu/region-16-dark.png"),
        ("region", true) => include_bytes!("../icons/menu/region-16-light.png"),
        ("window", false) => include_bytes!("../icons/menu/window-16-dark.png"),
        ("window", true) => include_bytes!("../icons/menu/window-16-light.png"),
        ("screen", false) => include_bytes!("../icons/menu/screen-16-dark.png"),
        ("screen", true) => include_bytes!("../icons/menu/screen-16-light.png"),
        ("scroll", false) => include_bytes!("../icons/menu/scroll-16-dark.png"),
        ("scroll", true) => include_bytes!("../icons/menu/scroll-16-light.png"),
        ("record", false) => include_bytes!("../icons/menu/record-16-dark.png"),
        ("record", true) => include_bytes!("../icons/menu/record-16-light.png"),
        ("gif", false) => include_bytes!("../icons/menu/gif-16-dark.png"),
        ("gif", true) => include_bytes!("../icons/menu/gif-16-light.png"),
        ("open", false) => include_bytes!("../icons/menu/open-16-dark.png"),
        ("open", true) => include_bytes!("../icons/menu/open-16-light.png"),
        ("gear", false) => include_bytes!("../icons/menu/gear-16-dark.png"),
        ("gear", true) => include_bytes!("../icons/menu/gear-16-light.png"),
        ("pause", false) => include_bytes!("../icons/menu/pause-16-dark.png"),
        ("pause", true) => include_bytes!("../icons/menu/pause-16-light.png"),
        ("power", false) => include_bytes!("../icons/menu/power-16-dark.png"),
        ("power", true) => include_bytes!("../icons/menu/power-16-light.png"),
        _ => return None,
    };
    Image::from_bytes(bytes).ok()
}

fn build_menu(app: &AppHandle, state: TrayState) -> tauri::Result<Menu<Wry>> {
    let capture_region = icon_item(
        app,
        "capture_region",
        chord_label("capture_region", "Capture region"),
        "region",
    )?;
    let capture_window = icon_item(
        app,
        "capture_window",
        chord_label("capture_window", "Capture window"),
        "window",
    )?;
    let capture_screen = icon_item(
        app,
        "capture_screen",
        chord_label("capture_screen", "Capture full screen"),
        "screen",
    )?;
    let capture_scrolling = icon_item(
        app,
        "capture_scrolling",
        chord_label("capture_scrolling", "Scrolling capture"),
        "scroll",
    )?;
    let record_video = icon_item(
        app,
        "record_video",
        chord_label("record_video", "Record video (MP4)"),
        "record",
    )?;
    let record_gif = icon_item(
        app,
        "record_gif",
        chord_label("record_gif", "Record GIF"),
        "gif",
    )?;
    let record_window = icon_item(
        app,
        "record_window",
        "Record active window".to_string(),
        "window",
    )?;
    let record_separator = PredefinedMenuItem::separator(app)?;
    let system_audio = CheckMenuItem::with_id(
        app,
        "record_system_audio",
        "System audio",
        true,
        state.system_audio,
        None::<&str>,
    )?;
    let microphone = CheckMenuItem::with_id(
        app,
        "record_microphone",
        "Microphone",
        true,
        state.microphone,
        None::<&str>,
    )?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let open = icon_item(app, "app_open", "Open Snapline".to_string(), "open")?;
    let settings = icon_item(app, "app_settings", "Settings".to_string(), "gear")?;
    let pause = icon_item(
        app,
        "app_pause_hotkeys",
        if state.paused {
            "Resume hotkeys".to_string()
        } else {
            "Pause hotkeys".to_string()
        },
        "pause",
    )?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let exit = icon_item(app, "app_exit", "Exit".to_string(), "power")?;

    if state.recording_seconds.is_some() {
        let stop = icon_item(
            app,
            "record_stop",
            chord_label("record_video", "Stop recording"),
            "record",
        )?;
        Menu::with_items(
            app,
            &[
                &capture_region,
                &capture_window,
                &capture_screen,
                &capture_scrolling,
                &stop,
                &separator_one,
                &open,
                &settings,
                &pause,
                &separator_two,
                &exit,
            ],
        )
    } else {
        let record = Submenu::with_items(
            app,
            "Record screen",
            true,
            &[
                &record_video,
                &record_gif,
                &record_window,
                &record_separator,
                &system_audio,
                &microphone,
            ],
        )?;
        Menu::with_items(
            app,
            &[
                &capture_region,
                &capture_window,
                &capture_screen,
                &capture_scrolling,
                &record,
                &separator_one,
                &open,
                &settings,
                &pause,
                &separator_two,
                &exit,
            ],
        )
    }
}

fn tooltip(state: TrayState) -> String {
    if let Some(seconds) = state.recording_seconds {
        return format!("Snapline · recording {}", format_elapsed(seconds));
    }
    if state.paused {
        return "Snapline · hotkeys paused".to_string();
    }
    "Snapline".to_string()
}

fn format_elapsed(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn icon_for_state(state: IconState) -> tauri::Result<Image<'static>> {
    match state {
        IconState::Normal => {
            Image::from_bytes(include_bytes!("../../../assets/icons/tray-normal-32.png"))
        }
        IconState::Recording => Image::from_bytes(include_bytes!(
            "../../../assets/icons/tray-recording-32.png"
        )),
        IconState::Paused => {
            Image::from_bytes(include_bytes!("../../../assets/icons/tray-paused-32.png"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_has_three_states() {
        let normal = TrayState::default();
        assert_eq!(tooltip(normal), "Snapline");
        let paused = TrayState {
            paused: true,
            ..normal
        };
        assert_eq!(tooltip(paused), "Snapline · hotkeys paused");
        let recording = TrayState {
            recording_seconds: Some(42),
            ..normal
        };
        assert_eq!(tooltip(recording), "Snapline · recording 0:42");
    }

    #[test]
    fn recording_wins_over_paused_icon() {
        let state = TrayState {
            paused: true,
            recording_seconds: Some(1),
            ..TrayState::default()
        };
        assert_eq!(state.icon_state(), IconState::Recording);
    }
}
