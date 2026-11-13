use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;
use slint::{Color, ComponentHandle, Model, ModelRc, VecModel, Weak};
use snapline_core::combine::parse_hex;
use snapline_services::dto::history::{CaptureDto, HistoryFilter, HistoryReq};
use snapline_services::dto::hotkeys::HotkeysResp;
use snapline_services::dto::tool::{
    CombineAlignDto, CombineBackgroundDto, CombineLayoutDto, CombineOptionsDto, CombineRequestDto,
    CombineSourceDto, ConvertFormatDto, ConvertPhaseDto, ConvertProgressDto, ConvertSource,
    ToolReq, ToolResp,
};
use snapline_services::ServiceError;

use crate::backend::{rgba_to_slint, spawn_bg, Backend};
use crate::history::{format_bytes, format_duration, type_label, HistoryHandle};
use crate::{
    CombineHistoryItem, CombineSourceItem, ConvertHistoryItem, SnaplineWindow, ToolCardData,
    ToolsState,
};

const HISTORY_CHOICE_LIMIT: usize = 50;
const CONVERT_POLL_MS: u64 = 80;

static NEXT_TIMER_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CONVERT_TIMERS: RefCell<HashMap<u64, slint::Timer>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy)]
pub(crate) struct ToolCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    pub action: &'static str,
    pub keywords: &'static str,
}

pub(crate) const TOOL_CATALOGUE: &[ToolCatalogEntry] = &[
    ToolCatalogEntry {
        id: "tool_colour",
        name: "Colour picker",
        description: "Pick any pixel on screen and copy its hex, RGB or HSL value.",
        icon: "colour",
        action: "tool_colour",
        keywords: "eyedropper color palette pixel hex rgb hsl",
    },
    ToolCatalogEntry {
        id: "tool_ocr",
        name: "Text capture (OCR)",
        description: "Select an area and copy the text in it.",
        icon: "ocr",
        action: "tool_ocr",
        keywords: "ocr text copy extract read screen",
    },
    ToolCatalogEntry {
        id: "tool_ruler",
        name: "Screen ruler",
        description: "Measure distances and sizes in pixels.",
        icon: "ruler",
        action: "tool_ruler",
        keywords: "measure pixels distance dimensions size",
    },
    ToolCatalogEntry {
        id: "tool_qr",
        name: "QR code",
        description: "Scan a QR code on screen, or make one from a link.",
        icon: "qr",
        action: "panel:qr",
        keywords: "barcode scan generate link url",
    },
    ToolCatalogEntry {
        id: "tool_editor",
        name: "Image editor",
        description: "Open any image to annotate, crop or blur.",
        icon: "edit",
        action: "tool_editor",
        keywords: "annotate crop blur draw markup",
    },
    ToolCatalogEntry {
        id: "tool_pin",
        name: "Pin to screen",
        description: "Keep an image floating above other windows.",
        icon: "pin",
        action: "tool_pin",
        keywords: "float always on top window image",
    },
    ToolCatalogEntry {
        id: "tool_combine",
        name: "Image combiner",
        description: "Join several captures side by side or stacked.",
        icon: "combine",
        action: "panel:combine",
        keywords: "combine stitch merge join side stacked",
    },
    ToolCatalogEntry {
        id: "tool_convert",
        name: "Video converter",
        description: "Turn recordings into GIF, WebM or a smaller MP4.",
        icon: "gif",
        action: "panel:convert",
        keywords: "convert gif webm mp4 encode ffmpeg",
    },
];

pub(crate) fn matches_tool_query(entry: &ToolCatalogEntry, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    needle.is_empty()
        || entry.name.to_lowercase().contains(&needle)
        || entry.description.to_lowercase().contains(&needle)
        || entry.keywords.to_lowercase().contains(&needle)
        || entry.id.to_lowercase().contains(&needle)
}

pub(crate) fn hotkey_for(owner: &str, settings: &Value, hotkeys: &HotkeysResp) -> String {
    if let HotkeysResp::State { bindings, .. } = hotkeys {
        if let Some(binding) = bindings.iter().find(|binding| binding.owner == owner) {
            return binding.chord.clone();
        }
    }
    settings["hotkeys"]["bindings"][owner]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn build_cards(
    query: &str,
    settings: &Value,
    hotkeys: &HotkeysResp,
) -> Vec<ToolCardData> {
    TOOL_CATALOGUE
        .iter()
        .filter(|entry| matches_tool_query(entry, query))
        .map(|entry| ToolCardData {
            id: entry.id.into(),
            name: entry.name.into(),
            description: entry.description.into(),
            icon: entry.icon.into(),
            hotkey: hotkey_for(entry.id, settings, hotkeys).into(),
            action: entry.action.into(),
        })
        .collect()
}

pub(crate) fn custom_swatch(hex: &str) -> Option<Color> {
    let [blue, green, red, _] = parse_hex(hex.trim())?;
    Some(Color::from_rgb_u8(red, green, blue))
}

pub(crate) fn combine_layout_dto(label: &str) -> CombineLayoutDto {
    if label == "Stacked" {
        CombineLayoutDto::Stacked
    } else {
        CombineLayoutDto::SideBySide
    }
}

pub(crate) fn combine_align_dto(label: &str) -> CombineAlignDto {
    match label {
        "Centre" => CombineAlignDto::Centre,
        "End" => CombineAlignDto::End,
        _ => CombineAlignDto::Start,
    }
}

pub(crate) fn combine_background_dto(kind: &str, custom_hex: &str) -> CombineBackgroundDto {
    match kind {
        "White" => CombineBackgroundDto::White,
        "Black" => CombineBackgroundDto::Black,
        "Custom" => CombineBackgroundDto::Custom(custom_hex.trim().to_owned()),
        _ => CombineBackgroundDto::Transparent,
    }
}

pub(crate) fn convert_format_dto(label: &str) -> ConvertFormatDto {
    match label {
        "WebM VP9" => ConvertFormatDto::WebmVp9,
        "MP4 Small" => ConvertFormatDto::Mp4Small,
        _ => ConvertFormatDto::Gif,
    }
}

pub(crate) fn format_convert_status(progress: &ConvertProgressDto) -> (f32, String) {
    let bar = progress
        .percent
        .map_or(-1.0, |p| (p as f32 / 100.0).clamp(0.0, 1.0));
    let status = match progress.phase {
        ConvertPhaseDto::Converting => {
            let mut parts = Vec::new();
            if let Some(pct) = progress.percent {
                parts.push(format!("Converting… {:.0}%", pct.clamp(0.0, 100.0)));
            } else {
                parts.push("Converting…".to_owned());
            }
            if let Some(ms) = progress.out_time_ms.filter(|ms| *ms > 0) {
                parts.push(format_duration(ms as i64));
            }
            if let Some(speed) = progress.speed.filter(|s| *s > 0.0 && s.is_finite()) {
                parts.push(format!("{speed:.1}×"));
            }
            parts.join(" · ")
        }
        ConvertPhaseDto::Saving => "Saving to History…".to_owned(),
        ConvertPhaseDto::Done => "Saved to History.".to_owned(),
        ConvertPhaseDto::Cancelled => "Conversion cancelled.".to_owned(),
        ConvertPhaseDto::Failed => progress
            .message
            .clone()
            .filter(|msg| !msg.trim().is_empty())
            .unwrap_or_else(|| "The video could not be converted.".to_owned()),
    };
    (bar, status)
}

pub(crate) fn convert_item_meta(item: &CaptureDto) -> String {
    let mut parts = vec![type_label(item.kind, &item.name)];
    if let Some(ms) = item.duration_ms.filter(|ms| *ms > 0) {
        parts.push(format_duration(ms));
    }
    parts.push(format_bytes(item.bytes));
    parts.join(" · ")
}

pub(crate) fn display_filename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

pub(crate) fn merge_convert_captures(
    videos: Vec<CaptureDto>,
    gifs: Vec<CaptureDto>,
) -> Vec<CaptureDto> {
    let mut merged: HashMap<i64, CaptureDto> = HashMap::new();
    for item in videos.into_iter().chain(gifs) {
        if !item.missing {
            merged.insert(item.id, item);
        }
    }
    let mut list: Vec<CaptureDto> = merged.into_values().collect();
    list.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    list.truncate(HISTORY_CHOICE_LIMIT);
    list
}

pub(crate) fn filter_combine_captures(images: Vec<CaptureDto>) -> Vec<CaptureDto> {
    let mut list: Vec<CaptureDto> = images.into_iter().filter(|item| !item.missing).collect();
    list.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    list.truncate(HISTORY_CHOICE_LIMIT);
    list
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CombineEntry {
    source: CombineSourceDto,
    kind: &'static str,
    id: i32,
    name: String,
}

struct ToolsController {
    app: Weak<SnaplineWindow>,
    tool_busy: bool,
    combine_entries: Vec<CombineEntry>,
    combine_preview_gen: u64,
    combine_preview_inflight: bool,
    combine_preview_dirty: bool,
    combine_busy: bool,
    convert_running: bool,
    convert_job_id: Option<u64>,
    convert_poll_gen: u64,
    convert_poll_inflight: bool,
}

impl ToolsController {
    fn new(app: Weak<SnaplineWindow>) -> Self {
        Self {
            app,
            tool_busy: false,
            combine_entries: Vec::new(),
            combine_preview_gen: 0,
            combine_preview_inflight: false,
            combine_preview_dirty: false,
            combine_busy: false,
            convert_running: false,
            convert_job_id: None,
            convert_poll_gen: 0,
            convert_poll_inflight: false,
        }
    }

    fn sync_combine_sources(&self, s: &ToolsState) {
        let model: Vec<CombineSourceItem> = self
            .combine_entries
            .iter()
            .enumerate()
            .map(|(index, entry)| CombineSourceItem {
                kind: entry.kind.into(),
                id: entry.id,
                name: entry.name.clone().into(),
                index: index as i32,
            })
            .collect();
        s.set_combine_sources(ModelRc::new(VecModel::from(model)));
    }
}

#[derive(Clone)]
struct ToolsHandle(Arc<Mutex<ToolsController>>);

impl ToolsHandle {
    fn lock(&self) -> MutexGuard<'_, ToolsController> {
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn read_combine_request(
    ctrl: &ToolsController,
    s: &ToolsState,
) -> Result<CombineRequestDto, String> {
    let layout = combine_layout_dto(s.get_combine_layout().as_str());
    let align = combine_align_dto(s.get_combine_align().as_str());
    let spacing = s.get_combine_spacing().clamp(0, 64) as u32;
    let background_kind = s.get_combine_background().to_string();
    let custom_hex = s.get_combine_custom_hex().to_string();
    if background_kind == "Custom" && parse_hex(custom_hex.trim()).is_none() {
        return Err("Enter the background colour as a hex value, for example #1A2B3C.".to_owned());
    }
    Ok(CombineRequestDto {
        sources: ctrl
            .combine_entries
            .iter()
            .map(|entry| entry.source.clone())
            .collect(),
        options: CombineOptionsDto {
            layout,
            spacing,
            align,
            background: combine_background_dto(&background_kind, &custom_hex),
        },
    })
}

fn schedule_combine_preview(handle: &ToolsHandle, backend: &Backend) {
    let weak = {
        let ctrl = handle.lock();
        ctrl.app.clone()
    };
    let Some(app) = weak.upgrade() else {
        return;
    };
    let s = app.global::<ToolsState>();
    let hex = s.get_combine_custom_hex().to_string();
    if let Some(color) = custom_swatch(&hex) {
        s.set_combine_custom_swatch(color);
    }

    let request = {
        let mut ctrl = handle.lock();
        if ctrl.combine_entries.is_empty() {
            ctrl.combine_preview_gen = ctrl.combine_preview_gen.wrapping_add(1);
            ctrl.combine_preview_dirty = false;
            s.set_combine_has_preview(false);
            s.set_combine_preview(slint::Image::default());
            s.set_combine_error("".into());
            s.set_combine_busy(ctrl.combine_busy);
            return;
        }
        match read_combine_request(&ctrl, &s) {
            Ok(req) => req,
            Err(message) => {
                ctrl.combine_preview_gen = ctrl.combine_preview_gen.wrapping_add(1);
                ctrl.combine_preview_dirty = false;
                s.set_combine_error(message.into());
                s.set_combine_busy(ctrl.combine_busy);
                return;
            }
        }
    };

    let generation = {
        let mut ctrl = handle.lock();
        ctrl.combine_preview_gen = ctrl.combine_preview_gen.wrapping_add(1);
        if ctrl.combine_preview_inflight {
            ctrl.combine_preview_dirty = true;
            return;
        }
        ctrl.combine_preview_inflight = true;
        ctrl.combine_preview_dirty = false;
        s.set_combine_busy(true);
        ctrl.combine_preview_gen
    };

    let h = handle.clone();
    spawn_bg(
        backend,
        move |bg| bg.combine_preview_rgba(request),
        move |ui_bg, result| {
            let rerun = {
                let mut ctrl = h.lock();
                ctrl.combine_preview_inflight = false;
                let rerun = ctrl.combine_preview_dirty && !ctrl.combine_entries.is_empty();
                ctrl.combine_preview_dirty = false;
                if let Some(app) = ctrl.app.upgrade() {
                    let s = app.global::<ToolsState>();
                    if generation == ctrl.combine_preview_gen {
                        match result {
                            Ok((width, height, rgba)) => {
                                s.set_combine_preview(rgba_to_slint(width, height, &rgba));
                                s.set_combine_has_preview(true);
                                s.set_combine_error("".into());
                            }
                            Err(error) => {
                                s.set_combine_error(format!("{error:#}").into());
                            }
                        }
                    }
                    s.set_combine_busy(ctrl.combine_busy || rerun);
                }
                rerun
            };
            if rerun {
                schedule_combine_preview(&h, &ui_bg);
            }
        },
    );
}

fn start_convert_timer(
    handle: ToolsHandle,
    backend: Rc<Backend>,
    history: HistoryHandle,
    timer_id: u64,
) {
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(CONVERT_POLL_MS),
        move || {
            tick_convert_poll(&handle, &backend, &history, timer_id);
        },
    );
    let previous = CONVERT_TIMERS.with(|slot| slot.borrow_mut().insert(timer_id, timer));
    drop(previous);
}

fn stop_convert_timer(timer_id: u64) {
    let removed = CONVERT_TIMERS.with(|slot| slot.borrow_mut().remove(&timer_id));
    drop(removed);
}

fn tick_convert_poll(
    handle: &ToolsHandle,
    backend: &Backend,
    history: &HistoryHandle,
    timer_id: u64,
) {
    let (job_id, gen, alive) = {
        let mut ctrl = handle.lock();
        if ctrl.app.upgrade().is_none() || !ctrl.convert_running {
            ctrl.convert_poll_inflight = false;
            drop(ctrl);
            stop_convert_timer(timer_id);
            return;
        }
        if ctrl.convert_poll_inflight {
            return;
        }
        let Some(job_id) = ctrl.convert_job_id else {
            return;
        };
        ctrl.convert_poll_inflight = true;
        (job_id, ctrl.convert_poll_gen, true)
    };
    if !alive {
        return;
    }
    let h = handle.clone();
    let hist = history.clone();
    spawn_bg(
        backend,
        move |bg| {
            let prog = bg.tool_request(ToolReq::ConvertProgress { job_id });
            let finished = matches!(
                &prog,
                Ok(ToolResp::ConvertProgressed { progress, .. })
                    if matches!(
                        progress.phase,
                        ConvertPhaseDto::Done | ConvertPhaseDto::Failed | ConvertPhaseDto::Cancelled
                    )
            );
            let outcome = if finished {
                Some(bg.tool_request(ToolReq::ConvertResult { job_id }))
            } else {
                None
            };
            (prog, outcome)
        },
        move |ui_bg, (prog, outcome)| {
            let mut should_refresh = false;
            let mut stop_timer = false;
            let weak = {
                let mut ctrl = h.lock();
                ctrl.convert_poll_inflight = false;
                if gen != ctrl.convert_poll_gen || ctrl.convert_job_id != Some(job_id) {
                    if !ctrl.convert_running {
                        stop_timer = true;
                    }
                    ctrl.app.clone()
                } else {
                    if let Some(app) = ctrl.app.upgrade() {
                        let s = app.global::<ToolsState>();
                        match prog {
                            Ok(ToolResp::ConvertProgressed { progress, .. }) => {
                                let (bar, status) = format_convert_status(&progress);
                                s.set_convert_progress(bar);
                                s.set_convert_status(status.into());
                                if let Some(result) = outcome {
                                    stop_timer = true;
                                    ctrl.convert_running = false;
                                    ctrl.convert_job_id = None;
                                    s.set_convert_running(false);
                                    match result {
                                        Ok(ToolResp::Converted { converted }) => {
                                            s.set_convert_progress(1.0);
                                            s.set_convert_status("Saved to History.".into());
                                            s.set_convert_done_capture_id(
                                                converted.capture_id as i32,
                                            );
                                            s.set_convert_done_name(converted.name.into());
                                            s.set_convert_error("".into());
                                            should_refresh = true;
                                        }
                                        Err(error) => {
                                            if progress.phase == ConvertPhaseDto::Cancelled {
                                                s.set_convert_progress(-1.0);
                                                s.set_convert_status(
                                                    "Conversion cancelled.".into(),
                                                );
                                                s.set_convert_error("".into());
                                            } else {
                                                s.set_convert_progress(-1.0);
                                                s.set_convert_status("".into());
                                                s.set_convert_error(format!("{error:#}").into());
                                            }
                                        }
                                        Ok(_) => {
                                            s.set_convert_progress(-1.0);
                                        }
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                stop_timer = true;
                                ctrl.convert_running = false;
                                ctrl.convert_job_id = None;
                                s.set_convert_running(false);
                                s.set_convert_progress(-1.0);
                                s.set_convert_status("".into());
                                s.set_convert_error(format!("{error:#}").into());
                            }
                        }
                    } else {
                        stop_timer = true;
                    }
                    ctrl.app.clone()
                }
            };
            if stop_timer {
                stop_convert_timer(timer_id);
            }
            if should_refresh {
                hist.refresh_keep(&ui_bg);
                refresh_history_choices(&weak, &ui_bg);
                crate::update_history_count(&weak, &ui_bg);
            }
        },
    );
}

fn apply_qr_preview(s: &ToolsState, backend: &Backend, text: &str) {
    if text.is_empty() {
        s.set_qr_preview(slint::Image::default());
        s.set_qr_has_preview(false);
        s.set_qr_link("".into());
        s.set_qr_error("".into());
        return;
    }
    match backend.qr_make_preview(text) {
        Ok((preview, link)) => {
            s.set_qr_preview(preview);
            s.set_qr_has_preview(true);
            s.set_qr_link(link.unwrap_or_default().into());
            s.set_qr_error("".into());
        }
        Err(error) => {
            s.set_qr_preview(slint::Image::default());
            s.set_qr_has_preview(false);
            s.set_qr_link("".into());
            s.set_qr_error(format!("{error:#}").into());
        }
    }
}

pub fn wire(app: &SnaplineWindow, backend: Rc<Backend>, history: HistoryHandle) {
    let weak = app.as_weak();
    let handle = ToolsHandle(Arc::new(Mutex::new(ToolsController::new(weak.clone()))));
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::Relaxed);
    let s = app.global::<ToolsState>();

    if let (Ok(settings), Ok(hotkeys)) = (backend.settings(), backend.hotkeys()) {
        if let Ok(settings_value) = serde_json::to_value(settings) {
            refresh_cards_from(&weak, &settings_value, &hotkeys);
        }
    }
    refresh_history_choices(&weak, &backend);

    let (b, w) = (backend.clone(), weak.clone());
    s.on_search_changed(move |query| {
        if let Some(app) = w.upgrade() {
            app.global::<ToolsState>().set_query(query);
        }
        refresh_cards_from_backend(&w, &b);
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_clear_search(move || {
        if let Some(app) = w.upgrade() {
            app.set_tools_find("".into());
            if app.get_page() == 6 {
                app.set_find("".into());
            }
            let ts = app.global::<ToolsState>();
            ts.set_query("".into());
            ts.set_message("".into());
        }
        refresh_cards_from_backend(&w, &b);
    });

    let (h, b, w) = (handle.clone(), backend.clone(), weak.clone());
    s.on_open_panel(move |panel| {
        open_panel_inner(&h, &w, &b, panel.as_str());
    });

    let w = weak.clone();
    s.on_close_panel(move || {
        if let Some(app) = w.upgrade() {
            let ts = app.global::<ToolsState>();
            ts.set_active_panel("".into());
            ts.set_message("".into());
        }
    });

    let (h, b, w, hist) = (
        handle.clone(),
        backend.clone(),
        weak.clone(),
        history.clone(),
    );
    s.on_run_tool(move |action| {
        let action = action.to_string();
        if let Some(panel) = action.strip_prefix("panel:") {
            open_panel_inner(&h, &w, &b, panel);
            return;
        }
        {
            let mut ctrl = h.lock();
            if ctrl.tool_busy {
                return;
            }
            ctrl.tool_busy = true;
            if let Some(app) = w.upgrade() {
                let ts = app.global::<ToolsState>();
                ts.set_tool_busy(true);
                ts.set_message("".into());
            }
        }
        let h2 = h.clone();
        let w2 = w.clone();
        let hist2 = hist.clone();
        spawn_bg(
            &b,
            move |bg| match action.as_str() {
                "tool_editor" => match bg.tool_request(ToolReq::EditorOpenFile) {
                    Ok(ToolResp::EditorOpened { outcome }) => {
                        let edited = !outcome.cancelled && outcome.capture_id.is_some();
                        Ok(edited)
                    }
                    Ok(_) => Ok(false),
                    Err(error)
                        if error.downcast_ref::<ServiceError>() == Some(&ServiceError::NotYet) =>
                    {
                        bg.run_action("tool_editor").map(|()| false)
                    }
                    Err(error) => Err(error),
                },
                "tool_pin" => match bg.tool_request(ToolReq::PinMenu) {
                    Ok(ToolResp::Pinned { .. } | ToolResp::PinCancelled) | Ok(_) => Ok(false),
                    Err(error)
                        if error.downcast_ref::<ServiceError>() == Some(&ServiceError::NotYet) =>
                    {
                        bg.run_action("tool_pin").map(|()| false)
                    }
                    Err(error) => Err(error),
                },
                other => bg.run_action(other).map(|()| false),
            },
            move |ui_bg, result| {
                let mut refreshed = false;
                {
                    let mut ctrl = h2.lock();
                    ctrl.tool_busy = false;
                }
                if let Some(app) = w2.upgrade() {
                    let ts = app.global::<ToolsState>();
                    ts.set_tool_busy(false);
                    match result {
                        Ok(edited) => {
                            ts.set_message("".into());
                            refreshed = edited;
                        }
                        Err(error) => ts.set_message(format!("{error:#}").into()),
                    }
                }
                if refreshed {
                    hist2.refresh_keep(&ui_bg);
                    refresh_history_choices(&w2, &ui_bg);
                    crate::update_history_count(&w2, &ui_bg);
                }
            },
        );
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_qr_text_edited(move |text| {
        if let Some(app) = w.upgrade() {
            let ts = app.global::<ToolsState>();
            ts.set_qr_notice("".into());
            apply_qr_preview(&ts, &b, text.as_str());
        }
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_qr_copy_text(move |text| {
        let Some(app) = w.upgrade() else {
            return;
        };
        let ts = app.global::<ToolsState>();
        if text.is_empty() {
            return;
        }
        let outcome = match b.tool_request(ToolReq::QrCopyText {
            text: text.to_string(),
        }) {
            Ok(_) => Ok(()),
            Err(error) if error.downcast_ref::<ServiceError>() == Some(&ServiceError::NotYet) => b
                .host()
                .copy_text(text.as_str())
                .map_err(anyhow::Error::msg),
            Err(error) => Err(error),
        };
        match outcome {
            Ok(()) => {
                ts.set_qr_error("".into());
                ts.set_qr_notice("Copied text to clipboard.".into());
            }
            Err(error) => {
                ts.set_qr_notice("".into());
                ts.set_qr_error(format!("{error:#}").into());
            }
        }
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_qr_copy_image(move || {
        let Some(app) = w.upgrade() else {
            return;
        };
        let ts = app.global::<ToolsState>();
        if ts.get_qr_busy() {
            return;
        }
        let text = ts.get_qr_text().to_string();
        if text.is_empty() {
            return;
        }
        ts.set_qr_busy(true);
        ts.set_qr_error("".into());
        ts.set_qr_notice("".into());
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| bg.copy_qr_image(&text),
            move |_, result| {
                if let Some(app) = w2.upgrade() {
                    let ts = app.global::<ToolsState>();
                    ts.set_qr_busy(false);
                    match result {
                        Ok(()) => ts.set_qr_notice("Copied QR code image to clipboard.".into()),
                        Err(error) => ts.set_qr_error(format!("{error:#}").into()),
                    }
                }
            },
        );
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_qr_save_png(move || {
        let Some(app) = w.upgrade() else {
            return;
        };
        let ts = app.global::<ToolsState>();
        if ts.get_qr_busy() {
            return;
        }
        let text = ts.get_qr_text().to_string();
        if text.is_empty() {
            return;
        }
        ts.set_qr_busy(true);
        ts.set_qr_error("".into());
        ts.set_qr_notice("".into());
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| bg.save_qr_png(&text),
            move |_, result| {
                if let Some(app) = w2.upgrade() {
                    let ts = app.global::<ToolsState>();
                    ts.set_qr_busy(false);
                    match result {
                        Ok(Some(path)) => ts.set_qr_notice(format!("Saved to {path}.").into()),
                        Ok(None) => {}
                        Err(error) => ts.set_qr_error(format!("{error:#}").into()),
                    }
                }
            },
        );
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_qr_open_link(move |link| {
        let Some(app) = w.upgrade() else {
            return;
        };
        let ts = app.global::<ToolsState>();
        let link = link.to_string();
        if link.is_empty() {
            return;
        }
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| bg.open_qr_link(&link),
            move |_, result| {
                if let Some(app) = w2.upgrade() {
                    let ts = app.global::<ToolsState>();
                    match result {
                        Ok(()) => ts.set_qr_error("".into()),
                        Err(error) => ts.set_qr_error(format!("{error:#}").into()),
                    }
                }
            },
        );
        ts.set_qr_notice("".into());
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_qr_scan_screen(move || {
        let Some(app) = w.upgrade() else {
            return;
        };
        let ts = app.global::<ToolsState>();
        if ts.get_qr_busy() {
            return;
        }
        ts.set_qr_busy(true);
        ts.set_qr_error("".into());
        ts.set_qr_notice("".into());
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| bg.scan_qr_screen(),
            move |_, result| {
                if let Some(app) = w2.upgrade() {
                    let ts = app.global::<ToolsState>();
                    ts.set_qr_busy(false);
                    match result {
                        Ok(None) => {}
                        Ok(Some(scan)) => {
                            ts.set_qr_scan_text(scan.text.into());
                            ts.set_qr_scan_link(scan.link.unwrap_or_default().into());
                            ts.set_qr_notice("QR code scanned.".into());
                        }
                        Err(error) => ts.set_qr_error(format!("{error:#}").into()),
                    }
                }
            },
        );
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_qr_use_scanned(move || {
        let Some(app) = w.upgrade() else {
            return;
        };
        let ts = app.global::<ToolsState>();
        let scanned = ts.get_qr_scan_text().to_string();
        if scanned.is_empty() {
            return;
        }
        ts.set_qr_text(scanned.clone().into());
        ts.set_qr_notice("".into());
        apply_qr_preview(&ts, &b, &scanned);
    });

    let (h, b) = (handle.clone(), backend.clone());
    s.on_combine_pick_files(move || {
        let busy = h.lock().combine_busy;
        if busy {
            return;
        }
        let h2 = h.clone();
        spawn_bg(
            &b,
            move |bg| bg.tool_request(ToolReq::CombinePickFiles),
            move |ui_bg, result| {
                let mut added = false;
                {
                    let mut ctrl = h2.lock();
                    if let Some(app) = ctrl.app.upgrade() {
                        let ts = app.global::<ToolsState>();
                        match result {
                            Ok(ToolResp::CombinePickedFiles { files }) => {
                                ts.set_combine_error("".into());
                                ts.set_combine_notice("".into());
                                ts.set_combine_done_capture_id(-1);
                                for file in files {
                                    ctrl.combine_entries.push(CombineEntry {
                                        source: CombineSourceDto::Picked { id: file.id },
                                        kind: "picked",
                                        id: file.id as i32,
                                        name: file.name,
                                    });
                                    added = true;
                                }
                                ctrl.sync_combine_sources(&ts);
                            }
                            Ok(_) => {}
                            Err(error) => {
                                ts.set_combine_error(format!("{error:#}").into());
                            }
                        }
                    }
                }
                if added {
                    schedule_combine_preview(&h2, &ui_bg);
                }
            },
        );
    });

    let (h, b) = (handle.clone(), backend.clone());
    s.on_combine_add_history(move |id| {
        let added = {
            let mut ctrl = h.lock();
            if ctrl.combine_busy {
                return;
            }
            let Some(app) = ctrl.app.upgrade() else {
                return;
            };
            let ts = app.global::<ToolsState>();
            let name = ts
                .get_combine_history_choices()
                .iter()
                .find(|item| item.id == id)
                .map(|item| item.name.to_string())
                .unwrap_or_else(|| format!("Capture {id}"));
            ctrl.combine_entries.push(CombineEntry {
                source: CombineSourceDto::History { id: i64::from(id) },
                kind: "history",
                id,
                name,
            });
            ts.set_combine_error("".into());
            ts.set_combine_notice("".into());
            ts.set_combine_done_capture_id(-1);
            ctrl.sync_combine_sources(&ts);
            true
        };
        if added {
            schedule_combine_preview(&h, &b);
        }
    });

    let (h, b) = (handle.clone(), backend.clone());
    s.on_combine_move_source(move |index, delta| {
        let moved = {
            let mut ctrl = h.lock();
            if ctrl.combine_busy || index < 0 {
                return;
            }
            let len = ctrl.combine_entries.len();
            let idx = index as usize;
            let target = index + delta;
            if idx >= len || target < 0 || (target as usize) >= len || target == index {
                return;
            }
            ctrl.combine_entries.swap(idx, target as usize);
            if let Some(app) = ctrl.app.upgrade() {
                let ts = app.global::<ToolsState>();
                ts.set_combine_notice("".into());
                ts.set_combine_done_capture_id(-1);
                ctrl.sync_combine_sources(&ts);
            }
            true
        };
        if moved {
            schedule_combine_preview(&h, &b);
        }
    });

    let (h, b) = (handle.clone(), backend.clone());
    s.on_combine_remove_source(move |index| {
        let removed = {
            let mut ctrl = h.lock();
            if ctrl.combine_busy || index < 0 {
                return;
            }
            let idx = index as usize;
            if idx >= ctrl.combine_entries.len() {
                return;
            }
            ctrl.combine_entries.remove(idx);
            if let Some(app) = ctrl.app.upgrade() {
                let ts = app.global::<ToolsState>();
                ts.set_combine_notice("".into());
                ts.set_combine_done_capture_id(-1);
                ctrl.sync_combine_sources(&ts);
            }
            true
        };
        if removed {
            schedule_combine_preview(&h, &b);
        }
    });

    let (h, b) = (handle.clone(), backend.clone());
    s.on_combine_options_changed(move || {
        if let Some(app) = h.lock().app.upgrade() {
            let ts = app.global::<ToolsState>();
            ts.set_combine_notice("".into());
            ts.set_combine_done_capture_id(-1);
        }
        schedule_combine_preview(&h, &b);
    });

    let (h, b, hist) = (handle.clone(), backend.clone(), history.clone());
    s.on_combine_save(move || {
        let request = {
            let mut ctrl = h.lock();
            if ctrl.combine_busy {
                return;
            }
            let Some(app) = ctrl.app.upgrade() else {
                return;
            };
            let ts = app.global::<ToolsState>();
            if ctrl.combine_entries.is_empty() {
                ts.set_combine_error("Choose at least one image.".into());
                return;
            }
            match read_combine_request(&ctrl, &ts) {
                Ok(req) => {
                    ctrl.combine_busy = true;
                    ts.set_combine_busy(true);
                    ts.set_combine_error("".into());
                    ts.set_combine_notice("".into());
                    ts.set_combine_done_capture_id(-1);
                    req
                }
                Err(message) => {
                    ts.set_combine_error(message.into());
                    return;
                }
            }
        };
        let h2 = h.clone();
        let hist2 = hist.clone();
        spawn_bg(
            &b,
            move |bg| bg.tool_request(ToolReq::CombineSave { request }),
            move |ui_bg, result| {
                let mut saved_id = None;
                let weak = {
                    let mut ctrl = h2.lock();
                    ctrl.combine_busy = false;
                    let still_previewing = ctrl.combine_preview_inflight;
                    if let Some(app) = ctrl.app.upgrade() {
                        let ts = app.global::<ToolsState>();
                        ts.set_combine_busy(still_previewing);
                        match result {
                            Ok(ToolResp::CombineSaved { capture_id }) => {
                                saved_id = Some(capture_id);
                                ts.set_combine_done_capture_id(capture_id as i32);
                                ts.set_combine_error("".into());
                                ts.set_combine_notice("Saved combined image to History.".into());
                            }
                            Ok(_) => {}
                            Err(error) => {
                                ts.set_combine_error(format!("{error:#}").into());
                            }
                        }
                    }
                    ctrl.app.clone()
                };
                if saved_id.is_some() {
                    hist2.refresh_keep(&ui_bg);
                    refresh_history_choices(&weak, &ui_bg);
                    crate::update_history_count(&weak, &ui_bg);
                }
            },
        );
    });

    let (b, w, hist) = (backend.clone(), weak.clone(), history.clone());
    s.on_combine_show_in_history(move || {
        let Some(app) = w.upgrade() else {
            return;
        };
        let id = app.global::<ToolsState>().get_combine_done_capture_id();
        if id <= 0 {
            return;
        }
        app.set_page(3);
        hist.select_exclusive(&b, i64::from(id));
    });

    let (h, b) = (handle.clone(), backend.clone());
    s.on_convert_pick_file(move || {
        if h.lock().convert_running {
            return;
        }
        let h2 = h.clone();
        spawn_bg(
            &b,
            move |bg| bg.tool_request(ToolReq::ConvertPickFile),
            move |_, result| {
                let ctrl = h2.lock();
                let Some(app) = ctrl.app.upgrade() else {
                    return;
                };
                let ts = app.global::<ToolsState>();
                match result {
                    Ok(ToolResp::ConvertFilePicked { path: Some(path) }) => {
                        ts.set_convert_source_kind("file".into());
                        ts.set_convert_capture_id(-1);
                        ts.set_convert_source_label(display_filename(&path).into());
                        ts.set_convert_file_path(path.into());
                        ts.set_convert_error("".into());
                        ts.set_convert_status("".into());
                        ts.set_convert_progress(-1.0);
                        ts.set_convert_done_capture_id(-1);
                        ts.set_convert_done_name("".into());
                    }
                    Ok(_) => {}
                    Err(error) => {
                        ts.set_convert_error(format!("{error:#}").into());
                    }
                }
            },
        );
    });

    let h_choose = handle.clone();
    s.on_convert_choose_history(move |id| {
        let ctrl = h_choose.lock();
        if ctrl.convert_running {
            return;
        }
        let Some(app) = ctrl.app.upgrade() else {
            return;
        };
        let ts = app.global::<ToolsState>();
        let name = ts
            .get_convert_history_choices()
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.name.to_string())
            .unwrap_or_else(|| format!("Capture {id}"));
        ts.set_convert_source_kind("history".into());
        ts.set_convert_capture_id(id);
        ts.set_convert_file_path("".into());
        ts.set_convert_source_label(name.into());
        ts.set_convert_error("".into());
        ts.set_convert_status("".into());
        ts.set_convert_progress(-1.0);
        ts.set_convert_done_capture_id(-1);
        ts.set_convert_done_name("".into());
    });

    let (h, b, hist) = (handle.clone(), backend.clone(), history.clone());
    s.on_convert_start(move || {
        let (source, format, gen) = {
            let mut ctrl = h.lock();
            if ctrl.convert_running {
                return;
            }
            let Some(app) = ctrl.app.upgrade() else {
                return;
            };
            let ts = app.global::<ToolsState>();
            let kind = ts.get_convert_source_kind().to_string();
            let capture_id = i64::from(ts.get_convert_capture_id());
            let file_path = ts.get_convert_file_path().to_string();
            let source = match kind.as_str() {
                "history" if capture_id > 0 => ConvertSource::Capture { capture_id },
                "file" if !file_path.trim().is_empty() => ConvertSource::File { path: file_path },
                _ => {
                    ts.set_convert_error("Choose a video to convert.".into());
                    return;
                }
            };
            let format = convert_format_dto(ts.get_convert_format().as_str());
            ctrl.convert_poll_gen = ctrl.convert_poll_gen.wrapping_add(1);
            ctrl.convert_running = true;
            ctrl.convert_job_id = None;
            ctrl.convert_poll_inflight = false;
            ts.set_convert_running(true);
            ts.set_convert_progress(-1.0);
            ts.set_convert_status("Starting conversion…".into());
            ts.set_convert_error("".into());
            ts.set_convert_done_capture_id(-1);
            ts.set_convert_done_name("".into());
            (source, format, ctrl.convert_poll_gen)
        };
        stop_convert_timer(timer_id);
        let h2 = h.clone();
        let hist2 = hist.clone();
        spawn_bg(
            &b,
            move |bg| bg.tool_request(ToolReq::ConvertStart { source, format }),
            move |ui_bg, result| {
                let started = {
                    let mut ctrl = h2.lock();
                    if gen != ctrl.convert_poll_gen {
                        return;
                    }
                    match result {
                        Ok(ToolResp::ConvertStarted { job_id }) => {
                            ctrl.convert_job_id = Some(job_id);
                            if let Some(app) = ctrl.app.upgrade() {
                                app.global::<ToolsState>()
                                    .set_convert_status("Converting…".into());
                            }
                            true
                        }
                        Ok(_) => {
                            ctrl.convert_running = false;
                            ctrl.convert_job_id = None;
                            if let Some(app) = ctrl.app.upgrade() {
                                let ts = app.global::<ToolsState>();
                                ts.set_convert_running(false);
                                ts.set_convert_status("".into());
                            }
                            false
                        }
                        Err(error) => {
                            ctrl.convert_running = false;
                            ctrl.convert_job_id = None;
                            if let Some(app) = ctrl.app.upgrade() {
                                let ts = app.global::<ToolsState>();
                                ts.set_convert_running(false);
                                ts.set_convert_progress(-1.0);
                                ts.set_convert_status("".into());
                                ts.set_convert_error(format!("{error:#}").into());
                            }
                            false
                        }
                    }
                };
                if started {
                    start_convert_timer(h2.clone(), Rc::new(ui_bg), hist2.clone(), timer_id);
                }
            },
        );
    });

    let (h, b) = (handle.clone(), backend.clone());
    s.on_convert_cancel(move || {
        let job_id = {
            let ctrl = h.lock();
            if !ctrl.convert_running {
                return;
            }
            if let Some(app) = ctrl.app.upgrade() {
                app.global::<ToolsState>()
                    .set_convert_status("Cancelling…".into());
            }
            ctrl.convert_job_id
        };
        let Some(job_id) = job_id else {
            return;
        };
        let h2 = h.clone();
        spawn_bg(
            &b,
            move |bg| bg.tool_request(ToolReq::ConvertCancel { job_id }),
            move |_, result| {
                if let Err(error) = result {
                    let ctrl = h2.lock();
                    if ctrl.convert_running {
                        if let Some(app) = ctrl.app.upgrade() {
                            app.global::<ToolsState>()
                                .set_convert_error(format!("{error:#}").into());
                        }
                    }
                }
            },
        );
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_convert_open_result(move || {
        let Some(app) = w.upgrade() else {
            return;
        };
        let id = app.global::<ToolsState>().get_convert_done_capture_id();
        if id <= 0 {
            return;
        }
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| bg.history_request(HistoryReq::OpenFile { id: i64::from(id) }),
            move |_, result| {
                if let Err(error) = result {
                    if let Some(app) = w2.upgrade() {
                        app.global::<ToolsState>()
                            .set_convert_error(format!("{error:#}").into());
                    }
                }
            },
        );
    });

    let (b, w, hist) = (backend, weak, history);
    s.on_convert_show_in_history(move || {
        let Some(app) = w.upgrade() else {
            return;
        };
        let id = app.global::<ToolsState>().get_convert_done_capture_id();
        if id <= 0 {
            return;
        }
        app.set_page(3);
        hist.select_exclusive(&b, i64::from(id));
    });
}

fn open_panel_inner(
    _handle: &ToolsHandle,
    app: &Weak<SnaplineWindow>,
    backend: &Backend,
    panel: &str,
) {
    let Some(upgraded) = app.upgrade() else {
        return;
    };
    let ts = upgraded.global::<ToolsState>();
    ts.set_message("".into());
    match panel {
        "qr" => {
            ts.set_active_panel("qr".into());
            ts.set_qr_error("".into());
            ts.set_qr_notice("".into());
        }
        "combine" => {
            ts.set_active_panel("combine".into());
            ts.set_combine_error("".into());
            refresh_history_choices(app, backend);
        }
        "convert" => {
            ts.set_active_panel("convert".into());
            ts.set_convert_error("".into());
            refresh_history_choices(app, backend);
        }
        _ => {
            ts.set_active_panel("".into());
        }
    }
}

pub(crate) fn refresh_cards_from(
    app: &Weak<SnaplineWindow>,
    settings: &Value,
    hotkeys: &HotkeysResp,
) {
    let Some(app) = app.upgrade() else {
        return;
    };
    let ts = app.global::<ToolsState>();
    let query = ts.get_query().to_string();
    let cards = build_cards(&query, settings, hotkeys);
    ts.set_cards(ModelRc::new(VecModel::from(cards)));
}

fn refresh_cards_from_backend(app: &Weak<SnaplineWindow>, backend: &Backend) {
    let (Ok(settings), Ok(hotkeys)) = (backend.settings(), backend.hotkeys()) else {
        return;
    };
    let Ok(settings_value) = serde_json::to_value(settings) else {
        return;
    };
    refresh_cards_from(app, &settings_value, &hotkeys);
}

pub fn refresh_history_choices(app: &Weak<SnaplineWindow>, backend: &Backend) {
    let weak = app.clone();
    spawn_bg(
        backend,
        move |bg| {
            let images = bg
                .history_page_dto(HistoryFilter::Images, None)
                .map(|page| page.items)
                .unwrap_or_default();
            let videos = bg
                .history_page_dto(HistoryFilter::Videos, None)
                .map(|page| page.items)
                .unwrap_or_default();
            let gifs = bg
                .history_page_dto(HistoryFilter::Gifs, None)
                .map(|page| page.items)
                .unwrap_or_default();
            (
                filter_combine_captures(images),
                merge_convert_captures(videos, gifs),
            )
        },
        move |_, (combine_list, convert_list)| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let ts = app.global::<ToolsState>();
            let combine_items: Vec<CombineHistoryItem> = combine_list
                .into_iter()
                .map(|item| CombineHistoryItem {
                    id: item.id as i32,
                    name: item.name.into(),
                })
                .collect();
            let convert_items: Vec<ConvertHistoryItem> = convert_list
                .into_iter()
                .map(|item| ConvertHistoryItem {
                    id: item.id as i32,
                    name: item.name.clone().into(),
                    meta: convert_item_meta(&item).into(),
                })
                .collect();
            ts.set_combine_history_choices(ModelRc::new(VecModel::from(combine_items)));
            ts.set_convert_history_choices(ModelRc::new(VecModel::from(convert_items)));
        },
    );
}

pub fn open_qr(app: &Weak<SnaplineWindow>, backend: &Backend, text: &str) {
    let Some(app) = app.upgrade() else {
        return;
    };
    let s = app.global::<ToolsState>();
    s.set_message("".into());
    s.set_active_panel("qr".into());
    s.set_qr_text(text.into());
    s.set_qr_notice("".into());
    apply_qr_preview(&s, backend, text);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use snapline_core::actions::CATALOGUE;
    use snapline_services::dto::history::CaptureKindDto;
    use snapline_services::dto::hotkeys::BindingDto;

    use super::*;

    fn sample_capture(
        id: i64,
        kind: CaptureKindDto,
        name: &str,
        created_at: i64,
        missing: bool,
    ) -> CaptureDto {
        CaptureDto {
            id,
            kind,
            owned: true,
            missing,
            name: name.to_owned(),
            bytes: 2048,
            width: Some(1280),
            height: Some(720),
            duration_ms: Some(65_000),
            source_app: None,
            created_at,
            folder: String::new(),
            upload: None,
        }
    }

    #[test]
    fn tool_catalogue_covers_all_core_tool_actions() {
        let mut ui_ids = HashSet::new();
        for entry in TOOL_CATALOGUE {
            assert!(ui_ids.insert(entry.id), "duplicate id {}", entry.id);
        }
        let core_ids: Vec<&str> = CATALOGUE
            .iter()
            .filter(|def| def.id.starts_with("tool_"))
            .map(|def| def.id)
            .collect();
        assert_eq!(ui_ids.len(), core_ids.len());
        for id in core_ids {
            assert!(ui_ids.contains(id), "missing tool catalogue entry {id}");
        }
    }

    #[test]
    fn matches_tool_query_matches_name_description_keywords_and_id() {
        let colour = &TOOL_CATALOGUE[0];
        assert!(matches_tool_query(colour, ""));
        assert!(matches_tool_query(colour, "  COLOUR "));
        assert!(matches_tool_query(colour, "hsl"));
        assert!(matches_tool_query(colour, "eyedropper"));
        assert!(matches_tool_query(colour, "tool_colour"));
        assert!(!matches_tool_query(colour, "ffmpeg"));
    }

    #[test]
    fn hotkey_for_prefers_live_binding_and_falls_back_to_settings() {
        let settings = serde_json::json!({
            "hotkeys": {
                "bindings": {
                    "tool_colour": "Ctrl+Shift+C",
                    "tool_ocr": "Ctrl+Shift+T"
                }
            }
        });
        let hotkeys = HotkeysResp::State {
            bindings: vec![BindingDto {
                owner: "tool_colour".to_owned(),
                label: "Colour picker".to_owned(),
                chord: "Ctrl+Alt+C".to_owned(),
            }],
            blocked: Vec::new(),
            paused: false,
        };
        assert_eq!(hotkey_for("tool_colour", &settings, &hotkeys), "Ctrl+Alt+C");
        assert_eq!(hotkey_for("tool_ocr", &settings, &hotkeys), "Ctrl+Shift+T");
        assert_eq!(hotkey_for("tool_qr", &settings, &HotkeysResp::Pong), "");
        assert_eq!(
            hotkey_for("tool_colour", &settings, &HotkeysResp::Pong),
            "Ctrl+Shift+C"
        );
    }

    #[test]
    fn build_cards_filters_by_query_and_populates_hotkeys() {
        let settings = serde_json::json!({
            "hotkeys": {
                "bindings": {
                    "tool_colour": "Ctrl+Shift+C"
                }
            }
        });
        let all = build_cards("", &settings, &HotkeysResp::Pong);
        assert_eq!(all.len(), TOOL_CATALOGUE.len());
        assert_eq!(all[0].hotkey.as_str(), "Ctrl+Shift+C");

        let qr_only = build_cards("qr", &settings, &HotkeysResp::Pong);
        assert_eq!(qr_only.len(), 1);
        assert_eq!(qr_only[0].id.as_str(), "tool_qr");
        assert_eq!(qr_only[0].action.as_str(), "panel:qr");
    }

    #[test]
    fn custom_swatch_parses_hex_channels_and_rejects_invalid() {
        let Some(swatch) = custom_swatch(" #1A2B3C ") else {
            panic!("expected valid hex swatch");
        };
        assert_eq!(swatch.red(), 0x1A);
        assert_eq!(swatch.green(), 0x2B);
        assert_eq!(swatch.blue(), 0x3C);
        assert_eq!(custom_swatch("not-hex"), None);
        assert_eq!(custom_swatch(""), None);
    }

    #[test]
    fn dto_mappers_map_all_labels_with_defaults() {
        assert_eq!(combine_layout_dto("Stacked"), CombineLayoutDto::Stacked);
        assert_eq!(
            combine_layout_dto("Side by side"),
            CombineLayoutDto::SideBySide
        );
        assert_eq!(combine_layout_dto("other"), CombineLayoutDto::SideBySide);

        assert_eq!(combine_align_dto("Start"), CombineAlignDto::Start);
        assert_eq!(combine_align_dto("Centre"), CombineAlignDto::Centre);
        assert_eq!(combine_align_dto("End"), CombineAlignDto::End);
        assert_eq!(combine_align_dto("unknown"), CombineAlignDto::Start);

        assert_eq!(
            combine_background_dto("White", ""),
            CombineBackgroundDto::White
        );
        assert_eq!(
            combine_background_dto("Black", ""),
            CombineBackgroundDto::Black
        );
        assert_eq!(
            combine_background_dto("Custom", " #123456 "),
            CombineBackgroundDto::Custom("#123456".to_owned())
        );
        assert_eq!(
            combine_background_dto("Transparent", ""),
            CombineBackgroundDto::Transparent
        );
        assert_eq!(
            combine_background_dto("other", ""),
            CombineBackgroundDto::Transparent
        );

        assert_eq!(convert_format_dto("WebM VP9"), ConvertFormatDto::WebmVp9);
        assert_eq!(convert_format_dto("MP4 Small"), ConvertFormatDto::Mp4Small);
        assert_eq!(convert_format_dto("GIF"), ConvertFormatDto::Gif);
        assert_eq!(convert_format_dto("other"), ConvertFormatDto::Gif);
    }

    #[test]
    fn format_convert_status_covers_all_phases_and_clamps_percent() {
        let converting = ConvertProgressDto {
            phase: ConvertPhaseDto::Converting,
            percent: Some(140.0),
            out_time_ms: Some(12_000),
            speed: Some(1.54),
            message: None,
        };
        assert_eq!(
            format_convert_status(&converting),
            (1.0, "Converting… 100% · 0:12 · 1.5×".to_owned())
        );

        let clamped_low = ConvertProgressDto {
            phase: ConvertPhaseDto::Converting,
            percent: Some(-5.0),
            out_time_ms: Some(0),
            speed: Some(f64::NAN),
            message: None,
        };
        assert_eq!(
            format_convert_status(&clamped_low),
            (0.0, "Converting… 0%".to_owned())
        );

        let indeterminate = ConvertProgressDto {
            phase: ConvertPhaseDto::Converting,
            percent: None,
            out_time_ms: None,
            speed: None,
            message: None,
        };
        assert_eq!(
            format_convert_status(&indeterminate),
            (-1.0, "Converting…".to_owned())
        );

        let saving = ConvertProgressDto {
            phase: ConvertPhaseDto::Saving,
            percent: None,
            out_time_ms: None,
            speed: None,
            message: None,
        };
        assert_eq!(
            format_convert_status(&saving),
            (-1.0, "Saving to History…".to_owned())
        );

        let done = ConvertProgressDto {
            phase: ConvertPhaseDto::Done,
            percent: Some(100.0),
            out_time_ms: None,
            speed: None,
            message: None,
        };
        assert_eq!(
            format_convert_status(&done),
            (1.0, "Saved to History.".to_owned())
        );

        let cancelled = ConvertProgressDto {
            phase: ConvertPhaseDto::Cancelled,
            percent: None,
            out_time_ms: None,
            speed: None,
            message: None,
        };
        assert_eq!(
            format_convert_status(&cancelled),
            (-1.0, "Conversion cancelled.".to_owned())
        );

        let failed_blank = ConvertProgressDto {
            phase: ConvertPhaseDto::Failed,
            percent: None,
            out_time_ms: None,
            speed: None,
            message: Some("   ".to_owned()),
        };
        assert_eq!(
            format_convert_status(&failed_blank),
            (-1.0, "The video could not be converted.".to_owned())
        );

        let failed_msg = ConvertProgressDto {
            phase: ConvertPhaseDto::Failed,
            percent: None,
            out_time_ms: None,
            speed: None,
            message: Some("FFmpeg exited with code 1.".to_owned()),
        };
        assert_eq!(
            format_convert_status(&failed_msg),
            (-1.0, "FFmpeg exited with code 1.".to_owned())
        );
    }

    #[test]
    fn convert_item_meta_and_display_filename_format_cleanly() {
        let video = sample_capture(1, CaptureKindDto::Video, "demo.mp4", 100, false);
        assert_eq!(convert_item_meta(&video), "MP4 video · 1:05 · 2 KB");

        let mut gif = sample_capture(2, CaptureKindDto::Gif, "loop.gif", 200, false);
        gif.duration_ms = None;
        gif.bytes = 512;
        assert_eq!(convert_item_meta(&gif), "GIF animation · 512 bytes");

        let joined = std::path::PathBuf::from("clips")
            .join("recording.mp4")
            .to_string_lossy()
            .into_owned();
        assert_eq!(display_filename(&joined), "recording.mp4");
        assert_eq!(display_filename(""), "");
    }

    #[test]
    fn combine_and_convert_capture_lists_filter_missing_dedup_sort_and_cap() {
        let mut images: Vec<CaptureDto> = (1..=60)
            .map(|i| {
                sample_capture(
                    i,
                    CaptureKindDto::Image,
                    &format!("shot-{i}.png"),
                    1000 + (i % 10),
                    i == 60,
                )
            })
            .collect();
        images.push(sample_capture(
            99,
            CaptureKindDto::Image,
            "newest.png",
            5000,
            false,
        ));
        let combined = filter_combine_captures(images);
        assert_eq!(combined.len(), HISTORY_CHOICE_LIMIT);
        assert_eq!(combined[0].id, 99);
        assert!(!combined.iter().any(|item| item.id == 60 || item.missing));
        assert!(combined
            .windows(2)
            .all(|pair| { (pair[0].created_at, pair[0].id) >= (pair[1].created_at, pair[1].id) }));

        let mut videos: Vec<CaptureDto> = (1..=35)
            .map(|i| sample_capture(i, CaptureKindDto::Video, "v.mp4", 2000 + i, false))
            .collect();
        videos.push(sample_capture(
            500,
            CaptureKindDto::Video,
            "missing.mp4",
            9999,
            true,
        ));
        let mut gifs: Vec<CaptureDto> = (30..=60)
            .map(|i| sample_capture(i, CaptureKindDto::Gif, "g.gif", 2000 + i, false))
            .collect();
        gifs.push(sample_capture(
            1,
            CaptureKindDto::Gif,
            "dup.gif",
            999_999,
            false,
        ));
        let merged = merge_convert_captures(videos, gifs);
        assert_eq!(merged.len(), HISTORY_CHOICE_LIMIT);
        let unique_ids: HashSet<i64> = merged.iter().map(|item| item.id).collect();
        assert_eq!(unique_ids.len(), merged.len());
        assert!(!unique_ids.contains(&500));
        assert!(merged
            .windows(2)
            .all(|pair| { (pair[0].created_at, pair[0].id) >= (pair[1].created_at, pair[1].id) }));
    }
}
