use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use snapline_overlay::quick_panel::{
    self, Activation, PanelData, PanelHandle, PanelRequest, RecentView, UploadView,
};
use snapline_overlay::quick_panel_logic::{format_duration, relative_time};
use snapline_overlay::Theme;
use snapline_services::dto::app::{AppReq, AppResp, RecentCaptureDto, RecentStatusDto};
use snapline_services::dto::history::CaptureKindDto;
use snapline_services::dto::upload::{UploadReq, UploadResp, UploadState, UploadStatus};
use snapline_services::Services;
use tauri::{AppHandle, Manager};

use crate::ipc::app::run_native_panel_action;
use crate::panel::TrayRect;
use crate::windows::WindowManager;

const REOPEN_GUARD: Duration = Duration::from_millis(300);
const WEB_PANEL_VARIABLE: &str = "SNAPLINE_WEB_PANEL";

struct Slot {
    handle: Option<PanelHandle>,
    closed_at: Option<Instant>,
    opening: bool,
}

static SLOT: Mutex<Slot> = Mutex::new(Slot {
    handle: None,
    closed_at: None,
    opening: false,
});

fn with_slot<T>(apply: impl FnOnce(&mut Slot) -> T) -> T {
    let mut slot = SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    apply(&mut slot)
}

pub fn use_native(web_panel: Option<&str>) -> bool {
    !matches!(
        web_panel
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

pub fn enabled() -> bool {
    use_native(std::env::var(WEB_PANEL_VARIABLE).ok().as_deref())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Toggle {
    Close,
    Ignore,
    Open,
}

fn toggle(open: bool, opening: bool, since_closed: Option<Duration>) -> Toggle {
    if open {
        Toggle::Close
    } else if opening || since_closed.is_some_and(|elapsed| elapsed < REOPEN_GUARD) {
        Toggle::Ignore
    } else {
        Toggle::Open
    }
}

pub fn show(app: &AppHandle, tray: TrayRect) {
    let decision = with_slot(|slot| {
        let open = slot.handle.as_ref().is_some_and(PanelHandle::is_open);
        let decision = toggle(
            open,
            slot.opening,
            slot.closed_at.map(|closed| closed.elapsed()),
        );
        match decision {
            Toggle::Close => {
                if let Some(handle) = &slot.handle {
                    handle.close();
                }
            }
            Toggle::Open => slot.opening = true,
            Toggle::Ignore => {}
        }
        decision
    });
    if decision != Toggle::Open {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || match open(&app, tray) {
        Some((handle, services)) => {
            with_slot(|slot| {
                slot.handle = Some(handle.clone());
                slot.opening = false;
            });
            populate(&handle, &services);
        }
        None => {
            with_slot(|slot| slot.opening = false);
            crate::panel::show_web(&app, tray.x, tray.y, tray.width, tray.height);
        }
    });
}

fn open(app: &AppHandle, tray: TrayRect) -> Option<(PanelHandle, Services)> {
    let services = app.try_state::<Services>()?;
    let services = services.inner().clone();
    let theme = if crate::theme::is_light() {
        Theme::Light
    } else {
        Theme::Dark
    };
    let place_app = app.clone();
    let place = Box::new(move |width: i32, height: i32| {
        crate::panel::place_native(&place_app, tray, f64::from(width), f64::from(height))
    });
    let activate_app = app.clone();
    let activate_services = services.clone();
    let on_activate = Box::new(move |activation: Activation| {
        handle_activation(&activate_app, &activate_services, activation);
    });
    let on_closed = Box::new(|| {
        with_slot(|slot| slot.closed_at = Some(Instant::now()));
    });
    let request = PanelRequest {
        theme,
        data: PanelData::default(),
        anchor: (
            to_i32(tray.x + tray.width / 2.0),
            to_i32(tray.y + tray.height / 2.0),
        ),
        place,
        on_activate,
        on_closed,
    };
    match quick_panel::open(request) {
        Ok(handle) => Some((handle, services)),
        Err(error) => {
            tracing::error!(%error, "could not open the native quick panel; using the web panel");
            None
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn to_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn handle_activation(app: &AppHandle, services: &Services, activation: Activation) {
    match activation {
        Activation::OpenMain | Activation::ViewAll | Activation::Recent(_) => {
            open_main(app, "/history");
        }
        Activation::Settings => open_main(app, "/settings/general"),
        Activation::Capture(action) => {
            let services = services.clone();
            std::thread::spawn(move || run_native_panel_action(&services, action));
        }
        Activation::CopyLink => {
            let services = services.clone();
            std::thread::spawn(move || {
                if let Err(error) = snapline_services::shell_actions::run(
                    snapline_services::shell_actions::COPY_LAST_LINK,
                    &services,
                ) {
                    tracing::warn!(%error, "could not copy the last link");
                }
            });
        }
    }
}

fn open_main(app: &AppHandle, route: &str) {
    let manager = app.state::<WindowManager>();
    if let Err(error) = manager.open_main(app, route) {
        tracing::error!(%error, "could not open the main window");
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

fn utc_offset_minutes() -> i64 {
    time::UtcOffset::current_local_offset()
        .map(|offset| i64::from(offset.whole_minutes()))
        .unwrap_or(0)
}

fn populate(handle: &PanelHandle, services: &Services) {
    if !handle.is_open() {
        return;
    }
    let mut data = load_data(services);
    handle.update(data.clone());
    if data.recent.is_empty() || !handle.is_open() {
        return;
    }
    let thumbs = services.thumbs();
    let mut codec = None;
    for recent in &mut data.recent {
        if !handle.is_open() {
            return;
        }
        let Ok(thumbnail) = thumbs.get_stored(recent.id) else {
            continue;
        };
        let Ok(codec) = codec.get_or_insert_with(snapline_win::wic::WicCodec::new) else {
            return;
        };
        recent.thumb = codec.decode(&thumbnail.bytes).ok();
    }
    if data.recent.iter().any(|recent| recent.thumb.is_some()) {
        handle.update(data);
    }
}

fn load_data(services: &Services) -> PanelData {
    let now = now_ms();
    let offset = utc_offset_minutes();
    let captures = match services.app().handle(AppReq::Recent) {
        Ok(AppResp::Recent { captures }) => captures,
        _ => Vec::new(),
    };
    let recent = captures
        .iter()
        .map(|capture| recent_view(capture, now, offset))
        .collect();
    let uploads = match services.upload().handle(UploadReq::List) {
        Ok(UploadResp::States(states)) => states
            .iter()
            .filter(|state| is_active(state))
            .map(upload_view)
            .collect(),
        _ => Vec::new(),
    };
    let link = snapline_services::uploads::last_link(services)
        .ok()
        .flatten()
        .filter(|link| !link.is_empty());
    PanelData {
        recent,
        loaded: true,
        uploads,
        link,
    }
}

fn recent_view(capture: &RecentCaptureDto, now: i64, offset: i64) -> RecentView {
    RecentView {
        id: capture.id,
        name: capture.name.clone(),
        time: relative_time(capture.created_at, now, offset),
        duration: duration_label(capture),
        uploaded: capture.status == RecentStatusDto::Uploaded,
        thumb: None,
    }
}

fn duration_label(capture: &RecentCaptureDto) -> Option<String> {
    if capture.kind == CaptureKindDto::Image {
        return None;
    }
    capture.duration_ms.map(format_duration)
}

fn is_active(state: &UploadState) -> bool {
    matches!(
        state.status,
        UploadStatus::Queued | UploadStatus::Uploading | UploadStatus::BackingOff
    )
}

fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{} KB", (bytes + KIB / 2) / KIB)
    } else {
        #[allow(clippy::cast_precision_loss)]
        let megabytes = bytes as f64 / MIB as f64;
        format!("{megabytes:.1} MB")
    }
}

#[allow(clippy::cast_precision_loss)]
fn upload_view(state: &UploadState) -> UploadView {
    let name = if state.file_name.is_empty() {
        String::from("File upload")
    } else {
        state.file_name.clone()
    };
    let known = state.progress_available && state.total > 0;
    UploadView {
        name,
        detail: if known {
            format!(
                "{} of {}",
                format_size(state.sent),
                format_size(state.total)
            )
        } else {
            String::from("Uploading")
        },
        fraction: known.then(|| state.sent as f32 / state.total as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_native_panel_is_the_default_and_the_web_panel_is_opt_in() {
        assert!(use_native(None));
        assert!(use_native(Some("")));
        assert!(use_native(Some("0")));
        assert!(!use_native(Some("1")));
        assert!(!use_native(Some("true")));
        assert!(!use_native(Some(" YES ")));
    }

    #[test]
    fn a_click_closes_an_open_panel() {
        assert_eq!(toggle(true, false, None), Toggle::Close);
        assert_eq!(toggle(true, true, None), Toggle::Close);
    }

    #[test]
    fn the_click_that_blurred_the_panel_does_not_reopen_it() {
        assert_eq!(
            toggle(false, false, Some(Duration::from_millis(40))),
            Toggle::Ignore
        );
        assert_eq!(
            toggle(false, false, Some(Duration::from_millis(299))),
            Toggle::Ignore
        );
        assert_eq!(
            toggle(false, false, Some(Duration::from_millis(300))),
            Toggle::Open
        );
        assert_eq!(toggle(false, false, None), Toggle::Open);
    }

    #[test]
    fn a_second_click_while_opening_is_ignored() {
        assert_eq!(toggle(false, true, None), Toggle::Ignore);
    }

    #[test]
    fn upload_sizes_are_human() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1536), "2 KB");
        assert_eq!(format_size(5 * 1024 * 1024 + 524_288), "5.5 MB");
    }

    fn state(sent: u64, total: u64, available: bool, name: &str) -> UploadState {
        UploadState {
            id: 1,
            capture_id: None,
            file_name: name.to_owned(),
            dest: String::new(),
            run_id: None,
            status: UploadStatus::Uploading,
            cancellable: true,
            progress_available: available,
            sent,
            total,
            link_batch: None,
            attempts: 1,
            link: None,
            page_link: None,
            deletion_link: None,
            error: None,
            retry_in_ms: None,
        }
    }

    #[test]
    fn an_upload_row_shows_progress_only_when_it_is_known() {
        let known = upload_view(&state(1024, 4096, true, "shot.png"));
        assert_eq!(known.name, "shot.png");
        assert_eq!(known.detail, "1 KB of 4 KB");
        assert_eq!(known.fraction, Some(0.25));
        let unknown = upload_view(&state(0, 0, false, ""));
        assert_eq!(unknown.name, "File upload");
        assert_eq!(unknown.detail, "Uploading");
        assert_eq!(unknown.fraction, None);
    }

    #[test]
    fn only_queued_uploading_and_backing_off_uploads_are_shown() {
        let mut item = state(0, 0, false, "a");
        for status in [
            UploadStatus::Queued,
            UploadStatus::Uploading,
            UploadStatus::BackingOff,
        ] {
            item.status = status;
            assert!(is_active(&item));
        }
        for status in [
            UploadStatus::Done,
            UploadStatus::Failed,
            UploadStatus::Cancelled,
        ] {
            item.status = status;
            assert!(!is_active(&item));
        }
    }
}
