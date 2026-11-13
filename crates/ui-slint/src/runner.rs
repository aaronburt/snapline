use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use slint::winit_030::winit::event::WindowEvent as WinitWindowEvent;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::winit::window::Theme as WinitTheme;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::ComponentHandle;
use snapline_services::app::Event as AppAreaEvent;
use snapline_services::uploads::Event as UploadEvent;
use snapline_services::{AppEvent, Services};

use crate::backend::Backend;
use crate::history::{HistoryHandle, ThumbCache};
use crate::host::Host;
use crate::{tools, SnaplineWindow, Tokens, ToolsState, WorkflowsState};

thread_local! {
    static WINDOW: RefCell<Option<SnaplineWindow>> = const { RefCell::new(None) };
    static BACKEND: RefCell<Option<Rc<Backend>>> = const { RefCell::new(None) };
    static CACHE: RefCell<Option<Arc<ThumbCache>>> = const { RefCell::new(None) };
    static HISTORY: RefCell<Option<HistoryHandle>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct UiHandle {
    services: Services,
    host: Arc<dyn Host>,
}

impl std::fmt::Debug for UiHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiHandle").finish_non_exhaustive()
    }
}

pub fn start(services: Services, host: Arc<dyn Host>) -> Result<UiHandle, String> {
    let (ready, wait) = mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("snapline-ui".to_owned())
        .spawn(move || {
            let selected = slint::BackendSelector::new()
                .backend_name("winit".into())
                .renderer_name("software".into())
                .select();
            let failed = selected.is_err();
            let _ = ready.send(selected.map_err(|error| error.to_string()));
            if !failed {
                if let Err(error) = slint::run_event_loop_until_quit() {
                    tracing_free_log(&format!("the Snapline window loop stopped: {error}"));
                }
            }
        })
        .map_err(|error| error.to_string())?;
    wait.recv().map_err(|error| error.to_string())??;
    Ok(UiHandle { services, host })
}

fn tracing_free_log(message: &str) {
    eprintln!("{message}");
}

impl UiHandle {
    pub fn open(&self, route: &str) {
        let services = self.services.clone();
        let host = self.host.clone();
        let route = route.to_owned();
        let _ = slint::invoke_from_event_loop(move || open_here(services, host, &route));
    }

    pub fn set_light_theme(&self, light: bool) {
        let host = self.host.clone();
        let _ = slint::invoke_from_event_loop(move || {
            WINDOW.with(|slot| {
                let slot = slot.borrow();
                let Some(window) = slot.as_ref() else {
                    return;
                };
                window.global::<Tokens>().set_light(light);
                apply_hwnd_theme(window.window(), host.as_ref(), light);
            });
        });
    }

    pub fn emit_event(&self, event: &AppEvent) {
        let event = event.clone();
        let _ = slint::invoke_from_event_loop(move || apply_app_event(&event));
    }
}

fn apply_app_event(event: &AppEvent) {
    WINDOW.with(|win_slot| {
        let win_slot = win_slot.borrow();
        let Some(window) = win_slot.as_ref() else {
            return;
        };
        BACKEND.with(|backend_slot| {
            let backend_slot = backend_slot.borrow();
            let Some(backend) = backend_slot.as_ref() else {
                return;
            };
            let weak = window.as_weak();
            match event {
                AppEvent::App(AppAreaEvent::Navigate { route }) => {
                    navigate(window, backend, route);
                }
                AppEvent::Settings(_) => {
                    let _ = crate::reload(&weak, backend);
                    HISTORY.with(|history_slot| {
                        if let Some(history) = history_slot.borrow().as_ref() {
                            history.refresh_destinations(backend);
                        }
                    });
                }
                AppEvent::Hotkeys(_) | AppEvent::Workflow(_) => {
                    let _ = crate::reload(&weak, backend);
                }
                AppEvent::History(history_event) => {
                    let mut handled = false;
                    HISTORY.with(|history_slot| {
                        if let Some(history) = history_slot.borrow().as_ref() {
                            history.handle_history_event(backend, history_event);
                            handled = true;
                        }
                    });
                    if !handled {
                        tools::refresh_history_choices(&weak, backend);
                    }
                }
                AppEvent::Upload(UploadEvent::StateChanged(state)) => {
                    HISTORY.with(|history_slot| {
                        if let Some(history) = history_slot.borrow().as_ref() {
                            history.apply_upload_state(backend, state);
                        }
                    });
                }
                AppEvent::Upload(UploadEvent::Ping)
                | AppEvent::App(AppAreaEvent::Ping)
                | AppEvent::Record(_)
                | AppEvent::Tool(_) => {}
            }
        });
    });
}

pub(crate) fn target(route: &str) -> (i32, Option<&str>) {
    let path = route.split(['?', '#']).next().unwrap_or(route);
    match path.trim_end_matches('/') {
        "/settings/capture" => (1, None),
        "/settings/hotkeys" => (2, None),
        "/history" | "" => (3, None),
        "/workflows" => (4, None),
        "/settings/uploads" => (5, None),
        "/tools" => (6, None),
        other => match other.strip_prefix("/workflows/") {
            Some(id) => (4, Some(id)),
            None => (0, None),
        },
    }
}

pub(crate) fn qr_query(route: &str) -> Option<String> {
    let before_hash = route.split('#').next().unwrap_or(route);
    let query = before_hash.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some(raw) = pair.strip_prefix("qr=") {
            return Some(percent_decode(raw));
        }
    }
    None
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn navigate(window: &SnaplineWindow, backend: &Backend, route: &str) {
    let (page, workflow) = target(route);
    window.set_page(page);
    if page == 6 {
        if let Some(qr_text) = qr_query(route) {
            tools::open_qr(&window.as_weak(), backend, &qr_text);
        } else {
            window.global::<ToolsState>().set_active_panel("".into());
        }
    }
    match workflow {
        Some("new") => window.global::<WorkflowsState>().invoke_new_workflow(),
        Some(id) => window.global::<WorkflowsState>().invoke_edit(id.into()),
        None => {}
    }
}

fn apply_hwnd_theme(slint_window: &slint::Window, host: &dyn Host, light: bool) {
    slint_window.with_winit_window(|winit| {
        if let Ok(handle) = winit.window_handle() {
            if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                host.configure_window(win32.hwnd.get(), light);
            }
        }
    });
}

fn raise(window: &SnaplineWindow, host: &dyn Host) {
    let _ = window.show();
    let light = host.is_light();
    window.global::<Tokens>().set_light(light);
    apply_hwnd_theme(window.window(), host, light);
    window.window().with_winit_window(|winit| {
        winit.set_minimized(false);
        winit.focus_window();
    });
}

fn open_here(services: Services, host: Arc<dyn Host>, route: &str) {
    let reused = WINDOW.with(|slot| {
        let slot = slot.borrow();
        let Some(window) = slot.as_ref() else {
            return false;
        };
        BACKEND.with(|backend_slot| {
            let backend_slot = backend_slot.borrow();
            if let Some(backend) = backend_slot.as_ref() {
                navigate(window, backend, route);
                HISTORY.with(|history_slot| {
                    if let Some(history) = history_slot.borrow().as_ref() {
                        history.refresh_keep(backend);
                    }
                });
                let weak = window.as_weak();
                tools::refresh_history_choices(&weak, backend);
                crate::update_history_count(&weak, backend);
            }
        });
        raise(window, host.as_ref());
        true
    });
    if reused {
        return;
    }
    let backend = Rc::new(Backend::new(services, host.clone()));
    let cache = Arc::new(ThumbCache::default());
    let (window, history_handle) =
        match crate::create_window_with_cache(backend.clone(), cache.clone(), false) {
            Ok(created) => created,
            Err(error) => {
                eprintln!("the Snapline window could not be created: {error:#}");
                return;
            }
        };
    let configured = Rc::new(Cell::new(false));
    let event_host = host.clone();
    window
        .window()
        .on_winit_window_event(move |slint_win, event| {
            if !configured.get() {
                configured.set(true);
                apply_hwnd_theme(slint_win, event_host.as_ref(), event_host.is_light());
            }
            if let WinitWindowEvent::ThemeChanged(theme) = event {
                let light = matches!(theme, WinitTheme::Light);
                WINDOW.with(|slot| {
                    if let Some(win) = slot.borrow().as_ref() {
                        win.global::<Tokens>().set_light(light);
                    }
                });
                apply_hwnd_theme(slint_win, event_host.as_ref(), light);
            }
            EventResult::Propagate
        });
    window.window().on_close_requested(|| {
        slint::Timer::single_shot(Duration::ZERO, || {
            WINDOW.with(|slot| slot.borrow_mut().take());
            BACKEND.with(|slot| slot.borrow_mut().take());
            CACHE.with(|slot| slot.borrow_mut().take());
            HISTORY.with(|slot| slot.borrow_mut().take());
        });
        slint::CloseRequestResponse::HideWindow
    });
    navigate(&window, &backend, route);
    raise(&window, host.as_ref());
    WINDOW.with(|slot| *slot.borrow_mut() = Some(window));
    BACKEND.with(|slot| *slot.borrow_mut() = Some(backend));
    CACHE.with(|slot| *slot.borrow_mut() = Some(cache));
    HISTORY.with(|slot| *slot.borrow_mut() = Some(history_handle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_maps_all_routes_and_ignores_trailing_slash_query_and_hash() {
        assert_eq!(target("/settings/capture"), (1, None));
        assert_eq!(target("/settings/capture/?foo=1#bar"), (1, None));
        assert_eq!(target("/settings/hotkeys/"), (2, None));
        assert_eq!(target("/history"), (3, None));
        assert_eq!(target(""), (3, None));
        assert_eq!(target("/"), (3, None));
        assert_eq!(target("/workflows"), (4, None));
        assert_eq!(target("/workflows/new"), (4, Some("new")));
        assert_eq!(target("/workflows/wf-1?x=2#frag"), (4, Some("wf-1")));
        assert_eq!(target("/settings/uploads"), (5, None));
        assert_eq!(target("/tools?qr=hello#top"), (6, None));
        assert_eq!(target("/tools/"), (6, None));
        assert_eq!(target("/unknown"), (0, None));
    }

    #[test]
    fn qr_query_extracts_and_percent_decodes_value() {
        assert_eq!(qr_query("/tools"), None);
        assert_eq!(qr_query("/tools?foo=1&bar=2"), None);
        assert_eq!(
            qr_query("/tools?foo=1&qr=https%3A%2F%2Fexample.com%2Fa%20b%3Fx%3D1&other=2#sec"),
            Some("https://example.com/a b?x=1".to_owned())
        );
        assert_eq!(qr_query("/tools?qr="), Some(String::new()));
        assert_eq!(
            qr_query("/tools?qr=100%ZZok%FF"),
            Some("100%ZZok\u{FFFD}".to_owned())
        );
        assert_eq!(qr_query("/tools#qr=ignored"), None);
    }
}
