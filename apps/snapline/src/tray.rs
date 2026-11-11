use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use snapline_core::actions::ActionId;
use snapline_services::actions::dispatch_notifying;
use snapline_services::{ServiceError, Services};
use tauri::image::Image;
use tauri::menu::{IconMenuItem, Menu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::windows::WindowManager;

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconState {
    Normal,
    Recording,
}

#[derive(Debug, Clone, Copy, Default)]
struct TrayState {
    recording_seconds: Option<u64>,
}

impl TrayState {
    fn icon_state(self) -> IconState {
        if self.recording_seconds.is_some() {
            IconState::Recording
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftClick {
    Panel,
    Capture,
    Ignore,
}

fn left_click(capture_on_click: bool, recording: bool, follows_double_click: bool) -> LeftClick {
    if !capture_on_click || recording {
        LeftClick::Panel
    } else if follows_double_click {
        LeftClick::Ignore
    } else {
        LeftClick::Capture
    }
}

fn capture_on_click(app: &AppHandle) -> bool {
    app.try_state::<Services>()
        .and_then(|services| services.settings_snapshot().ok())
        .is_some_and(|settings| settings.general.tray_click_capture)
}

fn is_recording(app: &AppHandle) -> bool {
    app.try_state::<TrayHandle>()
        .is_some_and(|handle| handle.state().recording_seconds.is_some())
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let state = TrayState::default();
    let menu = build_menu(app)?;
    let icon = icon_for_state(state.icon_state())?;
    let capture_click = Arc::new(AtomicU64::new(0));
    let double_click = AtomicBool::new(false);
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip(tooltip(state))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id().as_ref() == EXIT_ID {
                request_exit(app);
            }
        })
        .on_tray_icon_event(move |tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Down,
                ..
            } => double_click.store(false, Ordering::SeqCst),
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } => {
                let follows_double_click = double_click.swap(false, Ordering::SeqCst);
                let app = tray.app_handle();
                let capture = capture_on_click(app);
                let recording = is_recording(app);
                let decision = left_click(capture, recording, follows_double_click);
                tracing::info!(
                    capture,
                    recording,
                    follows_double_click,
                    ?decision,
                    "tray click"
                );
                match decision {
                    LeftClick::Panel => {
                        let rect = crate::panel::TrayRect::from_tauri(rect);
                        crate::panel::show(app, rect.x, rect.y, rect.width, rect.height);
                        return;
                    }
                    LeftClick::Ignore => return,
                    LeftClick::Capture => {}
                }
                let ticket = capture_click.fetch_add(1, Ordering::SeqCst) + 1;
                let capture_click = Arc::clone(&capture_click);
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    std::thread::sleep(snapline_win::input::double_click_interval());
                    let current = capture_click.load(Ordering::SeqCst) == ticket;
                    let still = left_click(capture_on_click(&app), is_recording(&app), false)
                        == LeftClick::Capture;
                    tracing::info!(current, still, "tray click capture check");
                    if current && still {
                        run_action(&app, "capture_region");
                    }
                });
            }
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                capture_click.fetch_add(1, Ordering::SeqCst);
                double_click.store(true, Ordering::SeqCst);
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

pub fn set_recording(app: &AppHandle, elapsed_seconds: Option<u64>) {
    let handle = app.state::<TrayHandle>();
    let state = {
        let mut state = handle.state();
        state.recording_seconds = elapsed_seconds;
        *state
    };
    let icon = match icon_for_state(state.icon_state()) {
        Ok(icon) => icon,
        Err(error) => {
            tracing::error!(%error, "could not update the tray icon");
            return;
        }
    };
    if let Err(error) = handle.tray.set_icon(Some(icon)) {
        tracing::error!(%error, "could not update the tray icon");
    }
    if let Err(error) = handle.tray.set_tooltip(Some(tooltip(state))) {
        tracing::error!(%error, "could not update the tray tooltip");
    }
}

const EXIT_ID: &str = "app_exit";

fn request_exit(app: &AppHandle) {
    EXIT_REQUESTED.store(true, Ordering::SeqCst);
    app.exit(0);
}

fn open_main(app: &AppHandle, route: &str) {
    let manager = app.state::<WindowManager>();
    if let Err(error) = manager.open_main(app, route) {
        tracing::error!(%error, "could not open the main window");
    }
}

fn run_action(app: &AppHandle, action: &str) {
    if action == EXIT_ID {
        request_exit(app);
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
    let services = app.state::<Services>();
    match dispatch_notifying(&ActionId::new(action), services.inner()) {
        Ok(()) => {}
        Err(ServiceError::NotYet) => {
            tracing::info!(%action, "action is not available yet");
        }
        Err(error) => {
            tracing::warn!(%action, %error, "action failed");
        }
    }
}

fn menu_icon() -> Option<Image<'static>> {
    let bytes: &'static [u8] = if crate::theme::is_light() {
        include_bytes!("../icons/menu/power-16-light.png")
    } else {
        include_bytes!("../icons/menu/power-16-dark.png")
    };
    Image::from_bytes(bytes).ok()
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let exit = IconMenuItem::with_id(app, EXIT_ID, "Exit", true, menu_icon(), None::<&str>)?;
    let menu = Menu::new(app)?;
    menu.append(&exit)?;
    Ok(menu)
}

fn tooltip(state: TrayState) -> String {
    if let Some(seconds) = state.recording_seconds {
        return format!("Snapline · recording {}", format_elapsed(seconds));
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_capture_on_click_off_a_click_opens_the_quick_panel() {
        assert_eq!(left_click(false, false, false), LeftClick::Panel);
        assert_eq!(left_click(false, false, true), LeftClick::Panel);
    }

    #[test]
    fn with_capture_on_click_on_a_click_starts_a_region_capture() {
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
    }

    #[test]
    fn the_second_click_of_a_double_click_does_not_capture() {
        assert_eq!(left_click(true, false, true), LeftClick::Ignore);
    }

    #[test]
    fn toggling_the_setting_live_changes_the_next_click_and_cancels_a_pending_capture() {
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
        assert_ne!(left_click(false, false, false), LeftClick::Capture);
        assert_eq!(left_click(false, false, false), LeftClick::Panel);
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
    }

    #[test]
    fn a_recording_started_while_a_capture_is_pending_cancels_it() {
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
        assert_eq!(left_click(true, true, false), LeftClick::Panel);
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
    }

    #[test]
    fn while_recording_a_click_always_opens_the_quick_panel() {
        assert_eq!(left_click(true, true, false), LeftClick::Panel);
        assert_eq!(left_click(false, true, false), LeftClick::Panel);
    }

    #[test]
    fn the_tray_menu_is_just_exit_and_exit_is_a_catalogue_action() {
        assert_eq!(EXIT_ID, "app_exit");
        assert!(snapline_core::actions::find(&ActionId::new(EXIT_ID)).is_some());
    }

    #[test]
    fn tooltip_has_two_states() {
        assert_eq!(tooltip(TrayState::default()), "Snapline");
        let recording = TrayState {
            recording_seconds: Some(42),
        };
        assert_eq!(tooltip(recording), "Snapline · recording 0:42");
    }

    #[test]
    fn the_icon_follows_the_recording_state() {
        let recording = TrayState {
            recording_seconds: Some(5),
        };
        assert_eq!(recording.icon_state(), IconState::Recording);
        assert_eq!(TrayState::default().icon_state(), IconState::Normal);
    }

    #[test]
    fn elapsed_minutes_and_seconds_are_padded() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(42), "0:42");
        assert_eq!(format_elapsed(60), "1:00");
        assert_eq!(format_elapsed(3599), "59:59");
        assert_eq!(format_elapsed(3600), "60:00");
    }
}
