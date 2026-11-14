use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use snapline_core::actions::ActionId;
use snapline_services::actions::dispatch_notifying;
use snapline_services::ServiceError;
use tray_icon::menu::{IconMenuItem, Menu, MenuEvent};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::main_loop;
use crate::shell::Shell;

static STATE: Mutex<TrayState> = Mutex::new(TrayState {
    recording_seconds: None,
});

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

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

fn state() -> TrayState {
    *STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftClick {
    Main,
    Capture,
    Ignore,
}

fn left_click(capture_on_click: bool, recording: bool, follows_double_click: bool) -> LeftClick {
    if !capture_on_click || recording {
        LeftClick::Main
    } else if follows_double_click {
        LeftClick::Ignore
    } else {
        LeftClick::Capture
    }
}

fn capture_on_click(shell: &Shell) -> bool {
    shell
        .services()
        .and_then(|services| services.settings_snapshot().ok())
        .is_some_and(|settings| settings.general.tray_click_capture)
}

fn is_recording() -> bool {
    state().recording_seconds.is_some()
}

pub fn init(shell: &Shell) -> Result<(), String> {
    let current = state();
    let exit = IconMenuItem::with_id(EXIT_ID, "Exit", true, menu_icon(), None);
    let menu = Menu::new();
    menu.append(&exit).map_err(|error| error.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_id("main")
        .with_icon(icon_for_state(current.icon_state())?)
        .with_tooltip(tooltip(current))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|error| error.to_string())?;
    TRAY.with(|slot| *slot.borrow_mut() = Some(tray));

    let exit_shell = shell.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id.as_ref() == EXIT_ID {
            request_exit(&exit_shell);
        }
    }));

    let shell = shell.clone();
    let capture_click = Arc::new(AtomicU64::new(0));
    let double_click = AtomicBool::new(false);
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Down,
            ..
        } => double_click.store(false, Ordering::SeqCst),
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => {
            let follows_double_click = double_click.swap(false, Ordering::SeqCst);
            let capture = capture_on_click(&shell);
            let recording = is_recording();
            let decision = left_click(capture, recording, follows_double_click);
            tracing::info!(
                capture,
                recording,
                follows_double_click,
                ?decision,
                "tray click"
            );
            match decision {
                LeftClick::Main => {
                    shell.open_main("/history");
                    return;
                }
                LeftClick::Ignore => return,
                LeftClick::Capture => {}
            }
            let ticket = capture_click.fetch_add(1, Ordering::SeqCst) + 1;
            let capture_click = Arc::clone(&capture_click);
            let shell = shell.clone();
            std::thread::spawn(move || {
                std::thread::sleep(snapline_win::input::double_click_interval());
                let current = capture_click.load(Ordering::SeqCst) == ticket;
                let still = left_click(capture_on_click(&shell), is_recording(), false)
                    == LeftClick::Capture;
                tracing::info!(current, still, "tray click capture check");
                if current && still {
                    run_action(&shell, "capture_region");
                }
            });
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            capture_click.fetch_add(1, Ordering::SeqCst);
            double_click.store(true, Ordering::SeqCst);
            shell.open_main("/history");
        }
        _ => {}
    }));
    Ok(())
}

pub fn handle_second_instance(shell: &Shell, argv: &[String]) {
    if let Some(position) = argv.iter().position(|argument| argument == "--action") {
        if let Some(action) = argv.get(position + 1) {
            run_action(shell, action);
            return;
        }
    }
    shell.open_main("/history");
}

pub fn set_recording(elapsed_seconds: Option<u64>) {
    let current = {
        let mut guard = STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.recording_seconds = elapsed_seconds;
        *guard
    };
    main_loop::post(move || {
        let icon = match icon_for_state(current.icon_state()) {
            Ok(icon) => icon,
            Err(error) => {
                tracing::error!(%error, "could not update the tray icon");
                return;
            }
        };
        TRAY.with(|slot| {
            let slot = slot.borrow();
            let Some(tray) = slot.as_ref() else {
                return;
            };
            if let Err(error) = tray.set_icon(Some(icon)) {
                tracing::error!(%error, "could not update the tray icon");
            }
            if let Err(error) = tray.set_tooltip(Some(tooltip(current))) {
                tracing::error!(%error, "could not update the tray tooltip");
            }
        });
    });
}

const EXIT_ID: &str = "app_exit";

pub fn request_exit(_shell: &Shell) {
    main_loop::quit();
}

pub fn run_action(shell: &Shell, action: &str) {
    if action == EXIT_ID {
        request_exit(shell);
        return;
    }
    if action == "app_open" {
        shell.open_main("/history");
        return;
    }
    if action == "app_settings" {
        shell.open_main("/settings/general");
        return;
    }
    let Some(services) = shell.services() else {
        return;
    };
    match dispatch_notifying(&ActionId::new(action), &services) {
        Ok(()) => {}
        Err(ServiceError::NotYet) => {
            tracing::info!(%action, "action is not available yet");
        }
        Err(error) => {
            tracing::warn!(%action, %error, "action failed");
        }
    }
}

fn rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let (width, height) = image.dimensions();
    Ok((image.into_raw(), width, height))
}

fn menu_icon() -> Option<tray_icon::menu::Icon> {
    let bytes: &'static [u8] = if crate::theme::is_light() {
        include_bytes!("../icons/menu/power-16-light.png")
    } else {
        include_bytes!("../icons/menu/power-16-dark.png")
    };
    let (pixels, width, height) = rgba(bytes).ok()?;
    tray_icon::menu::Icon::from_rgba(pixels, width, height).ok()
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

fn icon_for_state(state: IconState) -> Result<Icon, String> {
    let bytes: &[u8] = match state {
        IconState::Normal => include_bytes!("../../../assets/icons/tray-normal-32.png"),
        IconState::Recording => include_bytes!("../../../assets/icons/tray-recording-32.png"),
    };
    let (pixels, width, height) = rgba(bytes)?;
    Icon::from_rgba(pixels, width, height).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_capture_on_click_off_a_click_opens_the_main_window() {
        assert_eq!(left_click(false, false, false), LeftClick::Main);
        assert_eq!(left_click(false, false, true), LeftClick::Main);
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
        assert_eq!(left_click(false, false, false), LeftClick::Main);
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
    }

    #[test]
    fn a_recording_started_while_a_capture_is_pending_cancels_it() {
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
        assert_eq!(left_click(true, true, false), LeftClick::Main);
        assert_eq!(left_click(true, false, false), LeftClick::Capture);
    }

    #[test]
    fn while_recording_a_click_always_opens_the_main_window() {
        assert_eq!(left_click(true, true, false), LeftClick::Main);
        assert_eq!(left_click(false, true, false), LeftClick::Main);
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
