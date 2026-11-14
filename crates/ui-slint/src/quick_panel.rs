use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use slint::winit_030::winit::event::WindowEvent as WinitWindowEvent;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{
    ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel, Weak,
};
use snapline_services::dto::app::{AppReq, AppResp, RecentCaptureDto, RecentStatusDto};
use snapline_services::dto::history::CaptureKindDto;
use snapline_services::dto::upload::{UploadReq, UploadResp, UploadState, UploadStatus};
use snapline_services::Services;

use crate::backend::decode_rgba;
use crate::host::{Host, PanelMonitor};
use crate::panel_logic::{
    format_duration, format_size, link_without_scheme, placement, relative_time, shrink_first,
    toggle, Bounds, Toggle, TrayRect, COPIED_FOR, MAX_RECENT,
};
use crate::{PanelRecent, PanelState, PanelUpload, QuickPanelWindow, Tokens};

const REFRESH_EVERY: Duration = Duration::from_millis(400);

pub(crate) type OpenMain = Rc<dyn Fn(&str)>;

pub(crate) struct PanelContext {
    pub services: Services,
    pub host: Arc<dyn Host>,
    pub open_main: OpenMain,
}

struct Open {
    window: QuickPanelWindow,
    anchor: TrayRect,
    monitor: Option<PanelMonitor>,
    stop: Arc<AtomicBool>,
}

thread_local! {
    static SLOT: RefCell<Option<Open>> = const { RefCell::new(None) };
    static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub(crate) fn show(context: PanelContext, anchor: TrayRect) {
    let open = SLOT.with(|slot| slot.borrow().is_some());
    let since_closed = CLOSED_AT.with(Cell::get).map(|closed| closed.elapsed());
    match toggle(open, since_closed) {
        Toggle::Close => close(),
        Toggle::Ignore => {}
        Toggle::Open => {
            if let Err(error) = open_panel(&context, anchor) {
                tracing::error!(%error, "could not open the quick panel");
            }
        }
    }
}

pub(crate) fn close() {
    let taken = SLOT.with(|slot| slot.borrow_mut().take());
    CLOSED_AT.with(|closed| closed.set(Some(Instant::now())));
    if let Some(open) = taken {
        open.stop.store(true, Ordering::SeqCst);
        let _ = open.window.hide();
        slint::Timer::single_shot(Duration::ZERO, move || drop(open));
    }
}

pub(crate) fn set_light(light: bool) {
    SLOT.with(|slot| {
        if let Some(open) = slot.borrow().as_ref() {
            open.window.global::<Tokens>().set_light(light);
        }
    });
}

fn open_panel(context: &PanelContext, anchor: TrayRect) -> Result<(), slint::PlatformError> {
    let monitor = context
        .host
        .panel_monitor_at(round(anchor.centre_x()), round(anchor.centre_y()));
    let window = QuickPanelWindow::new()?;
    let light = context.host.is_light();
    window.global::<Tokens>().set_light(light);
    wire(&window, context);

    let had_focus = Rc::new(Cell::new(false));
    window
        .window()
        .on_winit_window_event(move |_window, event| {
            match event {
                WinitWindowEvent::Focused(true) => had_focus.set(true),
                WinitWindowEvent::Focused(false) if had_focus.get() => {
                    slint::Timer::single_shot(Duration::ZERO, close);
                }
                _ => {}
            }
            EventResult::Propagate
        });

    let stop = Arc::new(AtomicBool::new(false));
    SLOT.with(|slot| {
        *slot.borrow_mut() = Some(Open {
            window: window.clone_strong(),
            anchor,
            monitor,
            stop: stop.clone(),
        });
    });
    window.show()?;
    prepare_native(&window, context.host.as_ref(), light);
    place();
    spawn_loader(context.services.clone(), stop, window.as_weak());
    Ok(())
}

fn prepare_native(window: &QuickPanelWindow, host: &dyn Host, light: bool) {
    window.window().with_winit_window(|winit| {
        #[cfg(windows)]
        {
            use slint::winit_030::winit::platform::windows::WindowExtWindows;
            winit.set_skip_taskbar(true);
        }
        if let Ok(handle) = winit.window_handle() {
            if let RawWindowHandle::Win32(win32) = handle.as_raw() {
                host.configure_panel(win32.hwnd.get(), light);
            }
        }
        winit.focus_window();
    });
}

fn place() {
    SLOT.with(|slot| {
        let slot = slot.borrow();
        let Some(open) = slot.as_ref() else {
            return;
        };
        let window = open.window.window();
        let current = f64::from(window.scale_factor());
        // Size from the laid-out content, not window.size(): the OS resize after
        // the data loads is asynchronous, so the live size can still be the old one.
        let logical_width = f64::from(window.size().width) / current;
        let logical_height = f64::from(open.window.get_content_height());
        let scale = open.monitor.map_or(current, |monitor| monitor.scale);
        let width = logical_width * scale;
        let height = logical_height * scale;
        let target = (
            round(width).max(1).unsigned_abs(),
            round(height).max(1).unsigned_abs(),
        );
        let (x, y) = match open.monitor {
            Some(monitor) => {
                let (bx, by, bw, bh) = monitor.bounds;
                let (wx, wy, ww, wh) = monitor.work;
                placement(
                    open.anchor,
                    Bounds::new(f64::from(wx), f64::from(wy), f64::from(ww), f64::from(wh)),
                    Bounds::new(f64::from(bx), f64::from(by), f64::from(bw), f64::from(bh)),
                    width,
                    height,
                )
            }
            None => (open.anchor.x, open.anchor.y),
        };
        let live = window.size();
        let first = shrink_first((live.width, live.height), target);
        if first != (live.width, live.height) {
            window.set_size(slint::PhysicalSize::new(first.0, first.1));
        }
        window.set_position(slint::PhysicalPosition::new(round(x), round(y)));
        if first != target {
            window.set_size(slint::PhysicalSize::new(target.0, target.1));
        }
    });
}

#[allow(clippy::cast_possible_truncation)]
fn round(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn wire(window: &QuickPanelWindow, context: &PanelContext) {
    let state = window.global::<PanelState>();

    let host = context.host.clone();
    state.on_action(move |action| {
        close();
        let host = host.clone();
        let action = action.to_string();
        std::thread::spawn(move || host.panel_action(&action));
    });

    let open_main = context.open_main.clone();
    state.on_open_main(move |route| {
        open_main(&route);
        close();
    });

    let open_main = context.open_main.clone();
    state.on_open_capture(move |_id| {
        open_main("/history");
        close();
    });

    let host = context.host.clone();
    let weak = window.as_weak();
    state.on_copy_link(move || {
        let host = host.clone();
        let weak = weak.clone();
        std::thread::spawn(move || {
            if !host.copy_last_link() {
                return;
            }
            let _ = slint::invoke_from_event_loop(move || {
                set_copy_label(&weak, "Copied");
                slint::Timer::single_shot(COPIED_FOR, move || set_copy_label(&weak, "Copy"));
            });
        });
    });

    let services = context.services.clone();
    state.on_cancel_upload(move |id| {
        let Ok(id) = u64::try_from(id) else {
            return;
        };
        let services = services.clone();
        std::thread::spawn(move || {
            if let Err(error) = services.upload().handle(UploadReq::Cancel { id }) {
                tracing::warn!(%error, "the upload could not be cancelled");
            }
        });
    });

    state.on_close(close);
}

fn set_copy_label(weak: &Weak<QuickPanelWindow>, label: &str) {
    if let Some(window) = weak.upgrade() {
        window.global::<PanelState>().set_copy_label(label.into());
    }
}

struct RecentData {
    id: i64,
    name: String,
    time: String,
    duration: String,
    uploaded: bool,
    thumb: Option<SharedPixelBuffer<Rgba8Pixel>>,
}

struct UploadData {
    id: i32,
    name: String,
    detail: String,
    fraction: Option<f32>,
    cancellable: bool,
}

struct PanelData {
    recent: Vec<RecentData>,
    uploads: Vec<UploadData>,
    link: String,
}

#[derive(PartialEq)]
struct DataKey {
    recent: Vec<(i64, String, String, String, bool)>,
    uploads: Vec<(i32, String, String, Option<u32>, bool)>,
    link: String,
}

fn data_key(data: &PanelData) -> DataKey {
    DataKey {
        recent: data
            .recent
            .iter()
            .map(|recent| {
                (
                    recent.id,
                    recent.name.clone(),
                    recent.time.clone(),
                    recent.duration.clone(),
                    recent.uploaded,
                )
            })
            .collect(),
        uploads: data
            .uploads
            .iter()
            .map(|upload| {
                (
                    upload.id,
                    upload.name.clone(),
                    upload.detail.clone(),
                    upload.fraction.map(f32::to_bits),
                    upload.cancellable,
                )
            })
            .collect(),
        link: data.link.clone(),
    }
}

fn spawn_loader(services: Services, stop: Arc<AtomicBool>, weak: Weak<QuickPanelWindow>) {
    let spawned = std::thread::Builder::new()
        .name("snapline-quick-panel".to_owned())
        .spawn(move || {
            let mut cache: HashMap<i64, SharedPixelBuffer<Rgba8Pixel>> = HashMap::new();
            let mut last: Option<DataKey> = None;
            while !stop.load(Ordering::SeqCst) {
                let mut data = load(&services);
                let key = data_key(&data);
                if last.as_ref() != Some(&key) {
                    if last.is_none() && !send(&weak, &data) {
                        return;
                    }
                    for recent in &mut data.recent {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        if let Some(buffer) = cache.get(&recent.id) {
                            recent.thumb = Some(buffer.clone());
                        } else if let Ok(thumbnail) = services.thumbs().get_stored(recent.id) {
                            if let Some((width, height, rgba)) = decode_rgba(&thumbnail.bytes) {
                                let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                                    &rgba, width, height,
                                );
                                cache.insert(recent.id, buffer.clone());
                                recent.thumb = Some(buffer);
                            }
                        }
                    }
                    if !send(&weak, &data) {
                        return;
                    }
                    last = Some(key);
                }
                std::thread::sleep(REFRESH_EVERY);
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "the quick panel could not start loading its data");
    }
}

fn send(weak: &Weak<QuickPanelWindow>, data: &PanelData) -> bool {
    let weak = weak.clone();
    let recent: Vec<_> = data
        .recent
        .iter()
        .map(|recent| {
            (
                recent.id,
                recent.name.clone(),
                recent.time.clone(),
                recent.duration.clone(),
                recent.uploaded,
                recent.thumb.clone(),
            )
        })
        .collect();
    let uploads: Vec<_> = data
        .uploads
        .iter()
        .map(|upload| {
            (
                upload.id,
                upload.name.clone(),
                upload.detail.clone(),
                upload.fraction,
                upload.cancellable,
            )
        })
        .collect();
    let link = data.link.clone();
    slint::invoke_from_event_loop(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let state = window.global::<PanelState>();
        let recents: Vec<PanelRecent> = recent
            .into_iter()
            .map(|(id, name, time, duration, uploaded, thumb)| PanelRecent {
                id: i32::try_from(id).unwrap_or(0),
                name: name.into(),
                time: time.into(),
                duration: duration.into(),
                uploaded,
                has_thumb: thumb.is_some(),
                thumb: thumb.map(slint::Image::from_rgba8).unwrap_or_default(),
            })
            .collect();
        let uploads: Vec<PanelUpload> = uploads
            .into_iter()
            .map(|(id, name, detail, fraction, cancellable)| PanelUpload {
                id,
                name: name.into(),
                detail: detail.into(),
                fraction: fraction.unwrap_or(0.0),
                indeterminate: fraction.is_none(),
                cancellable,
            })
            .collect();
        state.set_recents(ModelRc::new(VecModel::from(recents)));
        state.set_uploads(ModelRc::new(VecModel::from(uploads)));
        state.set_link(SharedString::from(link));
        state.set_loaded(true);
        slint::Timer::single_shot(Duration::from_millis(30), place);
    })
    .is_ok()
}

fn load(services: &Services) -> PanelData {
    let now = now_ms();
    let offset = utc_offset_minutes();
    let captures = match services.app().handle(AppReq::Recent) {
        Ok(AppResp::Recent { captures }) => captures,
        _ => Vec::new(),
    };
    let recent = captures
        .iter()
        .take(MAX_RECENT)
        .map(|capture| recent_data(capture, now, offset))
        .collect();
    let uploads = match services.upload().handle(UploadReq::List) {
        Ok(UploadResp::States(states)) => states
            .iter()
            .filter(|state| is_active(state))
            .map(upload_data)
            .collect(),
        _ => Vec::new(),
    };
    let link = snapline_services::uploads::last_link(services)
        .ok()
        .flatten()
        .filter(|link| !link.is_empty())
        .map(|link| link_without_scheme(&link).to_owned())
        .unwrap_or_default();
    PanelData {
        recent,
        uploads,
        link,
    }
}

fn recent_data(capture: &RecentCaptureDto, now: i64, offset: i64) -> RecentData {
    let duration = if capture.kind == CaptureKindDto::Image {
        String::new()
    } else {
        capture.duration_ms.map(format_duration).unwrap_or_default()
    };
    RecentData {
        id: capture.id,
        name: capture.name.clone(),
        time: relative_time(capture.created_at, now, offset),
        duration,
        uploaded: capture.status == RecentStatusDto::Uploaded,
        thumb: None,
    }
}

fn is_active(state: &UploadState) -> bool {
    matches!(
        state.status,
        UploadStatus::Queued | UploadStatus::Uploading | UploadStatus::BackingOff
    )
}

#[allow(clippy::cast_precision_loss)]
fn upload_data(state: &UploadState) -> UploadData {
    let name = if state.file_name.is_empty() {
        String::from("File upload")
    } else {
        state.file_name.clone()
    };
    let known = state.progress_available && state.total > 0;
    UploadData {
        id: i32::try_from(state.id).unwrap_or(0),
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
        fraction: known.then(|| (state.sent as f32 / state.total as f32).clamp(0.0, 1.0)),
        cancellable: state.cancellable,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let known = upload_data(&state(1024, 4096, true, "shot.png"));
        assert_eq!(known.name, "shot.png");
        assert_eq!(known.detail, "1 KB of 4 KB");
        assert_eq!(known.fraction, Some(0.25));
        let unknown = upload_data(&state(0, 0, false, ""));
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
