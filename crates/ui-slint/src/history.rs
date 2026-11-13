use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{bail, Result};
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel, Weak};
use snapline_services::dto::history::{
    CaptureDto, CaptureKindDto, Cursor, HistoryFilter, HistoryPage, HistoryReq, HistoryResp,
    HistoryStats, UploadDto, UploadStateDto,
};
use snapline_services::dto::tool::{ToolReq, ToolResp};
use snapline_services::dto::upload::{
    DestinationViewDto, UploadReq, UploadResp, UploadState, UploadStatus,
};

use crate::backend::{decode_rgba, spawn_bg, Backend};
use crate::{
    tools, HistoryCellData, HistoryDestOption, HistoryRowData, HistoryState, SnaplineWindow,
};

const FILTER_LABELS: [&str; 5] = ["All", "Images", "Videos", "GIFs", "Files"];

#[derive(Clone)]
pub struct CachedThumb(SharedPixelBuffer<Rgba8Pixel>);

#[derive(Default)]
pub struct ThumbCache {
    entries: Mutex<HashMap<i64, Option<CachedThumb>>>,
}

impl ThumbCache {
    pub fn load_or_get(&self, backend: &Backend, id: i64) -> Option<CachedThumb> {
        if let Some(cached) = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&id)
        {
            return cached.clone();
        }
        let decoded = backend
            .thumbnail(id)
            .ok()
            .and_then(|(bytes, _)| decode_rgba(&bytes))
            .map(|(width, height, rgba)| {
                CachedThumb(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                    &rgba, width, height,
                ))
            });
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(id, decoded.clone());
        decoded
    }

    pub fn get_cached(&self, id: i64) -> Option<slint::Image> {
        let cached = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&id)
            .cloned()
            .flatten()?;
        Some(slint::Image::from_rgba8(cached.0))
    }

    pub fn remove(&self, id: i64) {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&id);
    }
}

pub fn format_bytes(bytes: i64) -> String {
    if bytes < 0 {
        return "0 bytes".into();
    }
    format_bytes_u64(bytes as u64)
}

pub fn format_bytes_u64(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", (bytes as f64 / 1024.0).round())
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn format_duration(ms: i64) -> String {
    if ms < 0 {
        return "0:00".into();
    }
    let total = (ms as f64 / 1000.0).round() as i64;
    format!("{}:{:02}", total / 60, total % 60)
}

pub(crate) fn filter_for(index: i32) -> HistoryFilter {
    match index {
        1 => HistoryFilter::Images,
        2 => HistoryFilter::Videos,
        3 => HistoryFilter::Gifs,
        4 => HistoryFilter::Files,
        _ => HistoryFilter::All,
    }
}

pub(crate) fn filter_label_for(index: i32) -> &'static str {
    FILTER_LABELS[index.clamp(0, 4) as usize]
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_lowercase())
        .unwrap_or_default()
}

pub(crate) fn tag_for(kind: CaptureKindDto, name: &str) -> String {
    let ext = extension(name).to_uppercase();
    match kind {
        CaptureKindDto::Gif => "GIF".into(),
        CaptureKindDto::File => {
            if ext.is_empty() {
                "FILE".into()
            } else {
                ext
            }
        }
        CaptureKindDto::Text => {
            if ext.is_empty() {
                "TXT".into()
            } else {
                ext
            }
        }
        CaptureKindDto::Video => {
            if ext.is_empty() {
                "VIDEO".into()
            } else {
                ext
            }
        }
        CaptureKindDto::Image => {
            if ext.is_empty() {
                "IMAGE".into()
            } else {
                ext
            }
        }
    }
}

pub(crate) fn type_label(kind: CaptureKindDto, name: &str) -> String {
    let ext = extension(name);
    match kind {
        CaptureKindDto::Gif => "GIF animation".into(),
        CaptureKindDto::Video => format!("{} video", ext.to_uppercase()),
        CaptureKindDto::Text if ext.is_empty() => "Text file".into(),
        CaptureKindDto::Text => format!("{} text", ext.to_uppercase()),
        CaptureKindDto::File if ext.is_empty() => "File".into(),
        CaptureKindDto::File => format!("{} file", ext.to_uppercase()),
        CaptureKindDto::Image => {
            let image = match ext.as_str() {
                "jpg" => "JPEG".to_owned(),
                "webp" => "WebP".to_owned(),
                other => other.to_uppercase(),
            };
            format!("{} image", if image.is_empty() { "Image" } else { &image })
        }
    }
}

fn local_offset() -> time::UtcOffset {
    time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC)
}

fn local(ms: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(ms.div_euclid(1000))
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .to_offset(local_offset())
}

fn clock(ms: i64) -> String {
    let t = local(ms);
    format!("{:02}:{:02}", t.hour(), t.minute())
}

fn short_month(month: time::Month) -> String {
    let name = month.to_string();
    name.get(..3).unwrap_or(&name).to_owned()
}

fn captured(ms: i64) -> String {
    let t = local(ms);
    let today = time::OffsetDateTime::now_utc()
        .to_offset(local_offset())
        .date();
    if t.date() == today {
        return format!("Today, {}", clock(ms));
    }
    let month = short_month(t.month());
    if t.year() == today.year() {
        format!("{} {}, {}", t.day(), month, clock(ms))
    } else {
        format!("{} {} {}, {}", t.day(), month, t.year(), clock(ms))
    }
}

fn short_day(ms: i64) -> String {
    let date = local(ms).date();
    let today = time::OffsetDateTime::now_utc()
        .to_offset(local_offset())
        .date();
    if date == today {
        "Today".into()
    } else if today.previous_day() == Some(date) {
        "Yesterday".into()
    } else {
        format!("{} {}", date.day(), short_month(date.month()))
    }
}

fn day_label(ms: i64) -> String {
    let date = local(ms).date();
    let today = time::OffsetDateTime::now_utc()
        .to_offset(local_offset())
        .date();
    if date == today {
        "Today".into()
    } else if today.previous_day() == Some(date) {
        "Yesterday".into()
    } else if date.year() == today.year() {
        format!("{} {} {}", date.weekday(), date.day(), date.month())
    } else {
        format!(
            "{} {} {} {}",
            date.weekday(),
            date.day(),
            date.month(),
            date.year()
        )
    }
}

fn count_label(count: usize) -> String {
    if count == 1 {
        "1 capture".into()
    } else {
        format!("{count} captures")
    }
}

pub(crate) fn host_label(dest_id: &str, destinations: &[DestinationViewDto]) -> String {
    match dest_id {
        "imgur" => "Imgur".into(),
        "s3" => "Amazon S3".into(),
        other => destinations
            .iter()
            .find(|dest| dest.id == other)
            .map(|dest| dest.name.clone())
            .unwrap_or_else(|| other.to_owned()),
    }
}

fn card_badge_for(
    capture: &CaptureDto,
    active: Option<&UploadState>,
) -> (&'static str, &'static str, bool, bool, f32) {
    if let Some(state) = active {
        let progress = if state.progress_available && state.total > 0 {
            state.sent as f32 / state.total as f32
        } else {
            -1.0
        };
        return match state.status {
            UploadStatus::Queued => ("Queued", "uploading", false, state.cancellable, -1.0),
            UploadStatus::Uploading => {
                ("Uploading", "uploading", true, state.cancellable, progress)
            }
            UploadStatus::BackingOff => (
                "Waiting to retry",
                "uploading",
                false,
                state.cancellable,
                -1.0,
            ),
            UploadStatus::Done => ("Link", "ok", false, false, -1.0),
            UploadStatus::Failed => ("Failed", "failed", false, false, -1.0),
            UploadStatus::Cancelled => ("Local", "local", false, false, -1.0),
        };
    }
    match capture.upload.as_ref().map(|u| u.state) {
        Some(UploadStateDto::Done) => ("Link", "ok", false, false, -1.0),
        Some(UploadStateDto::Failed) => ("Failed", "failed", false, false, -1.0),
        Some(UploadStateDto::Queued) => ("Queued", "uploading", false, false, -1.0),
        Some(UploadStateDto::Uploading) => {
            let progress = capture
                .upload
                .as_ref()
                .filter(|u| u.total > 0)
                .map_or(-1.0, |u| u.sent as f32 / u.total as f32);
            ("Uploading", "uploading", true, false, progress)
        }
        None | Some(UploadStateDto::Cancelled) => ("Local", "local", false, false, -1.0),
    }
}

fn is_timed(kind: CaptureKindDto) -> bool {
    matches!(kind, CaptureKindDto::Video | CaptureKindDto::Gif)
}

fn cell_for(
    item: &CaptureDto,
    selected: bool,
    active: Option<&UploadState>,
    thumb: Option<slint::Image>,
) -> HistoryCellData {
    let (badge, tone, show_progress, cancellable, progress) = card_badge_for(item, active);
    let tag = tag_for(item.kind, &item.name);
    let timed = if is_timed(item.kind) {
        item.duration_ms.map(format_duration).unwrap_or_default()
    } else {
        String::new()
    };
    let duration = if timed.is_empty() {
        String::new()
    } else {
        format!("{tag} · {timed}")
    };
    HistoryCellData {
        id: item.id as i32,
        name: item.name.clone().into(),
        meta: format!("{} · {}", clock(item.created_at), format_bytes(item.bytes)).into(),
        tag: tag.into(),
        has_thumb: thumb.is_some(),
        thumb: thumb.unwrap_or_default(),
        selected,
        index: 0,
        day: day_label(item.created_at).into(),
        duration: duration.into(),
        badge: badge.into(),
        badge_tone: tone.into(),
        type_label: type_label(item.kind, &item.name).into(),
        captured: captured(item.created_at).into(),
        size: format_bytes(item.bytes).into(),
        timed: timed.into(),
        progress,
        show_progress,
        cancellable,
    }
}

fn push_day(days: &mut Vec<HistoryRowData>, cells: Vec<HistoryCellData>) {
    days.push(HistoryRowData {
        label: cells[0].day.clone(),
        count: count_label(cells.len()).into(),
        size: cells.len() as i32,
        cells: ModelRc::new(VecModel::from(cells)),
    });
}

fn build_rows(cells: &[HistoryCellData]) -> ModelRc<HistoryRowData> {
    let mut days: Vec<HistoryRowData> = Vec::new();
    let mut day: Vec<HistoryCellData> = Vec::new();
    for cell in cells {
        let mut cell = cell.clone();
        if day.first().is_some_and(|first| first.day != cell.day) {
            push_day(&mut days, std::mem::take(&mut day));
        }
        cell.index = day.len() as i32;
        day.push(cell);
    }
    if !day.is_empty() {
        push_day(&mut days, day);
    }
    ModelRc::new(VecModel::from(days))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShareView {
    status: &'static str,
    label: &'static str,
    url: String,
    deletion_url: Option<String>,
    error: Option<String>,
    dest_id: Option<String>,
    queue_id: Option<u64>,
    cancellable: bool,
    uploading_phase: bool,
    progress_available: bool,
    sent: u64,
    total: u64,
}

fn share_status_from_live(status: UploadStatus) -> (&'static str, &'static str) {
    match status {
        UploadStatus::Done => ("uploaded", "Uploaded"),
        UploadStatus::Failed => ("failed", "Upload failed"),
        UploadStatus::Queued => ("uploading", "Queued"),
        UploadStatus::Uploading => ("uploading", "Uploading"),
        UploadStatus::BackingOff => ("uploading", "Waiting to retry"),
        UploadStatus::Cancelled => ("local", "Not uploaded"),
    }
}

fn share_status_from_dto(state: UploadStateDto) -> (&'static str, &'static str) {
    match state {
        UploadStateDto::Done => ("uploaded", "Uploaded"),
        UploadStateDto::Failed => ("failed", "Upload failed"),
        UploadStateDto::Queued => ("uploading", "Queued"),
        UploadStateDto::Uploading => ("uploading", "Uploading"),
        UploadStateDto::Cancelled => ("local", "Not uploaded"),
    }
}

fn live_matches_capture(capture: &CaptureDto, queue_id: Option<u64>, state: &UploadState) -> bool {
    if queue_id.is_some_and(|known| known == state.id) {
        return true;
    }
    if state.capture_id.is_some_and(|id| id == capture.id) {
        return true;
    }
    if state
        .run_id
        .as_deref()
        .is_some_and(|run| run == capture.id.to_string())
    {
        return true;
    }
    capture
        .upload
        .as_ref()
        .and_then(|upload| upload.url.as_deref())
        .is_some_and(|url| state.link.as_deref() == Some(url))
}

fn compute_share_view(
    capture: &CaptureDto,
    known_queue_id: Option<u64>,
    live: Option<&UploadState>,
) -> ShareView {
    let upload = capture.upload.as_ref();
    if let Some(state) = live {
        if live_matches_capture(capture, known_queue_id, state) {
            let (status, label) = share_status_from_live(state.status);
            return ShareView {
                status,
                label,
                url: state
                    .link
                    .clone()
                    .or_else(|| upload.and_then(|u| u.url.clone()))
                    .unwrap_or_default(),
                deletion_url: state
                    .deletion_link
                    .clone()
                    .or_else(|| upload.and_then(|u| u.deletion_url.clone())),
                error: state
                    .error
                    .clone()
                    .or_else(|| upload.and_then(|u| u.error.clone())),
                dest_id: if state.dest.is_empty() {
                    upload.map(|u| u.dest_id.clone())
                } else {
                    Some(state.dest.clone())
                },
                queue_id: Some(state.id),
                cancellable: state.cancellable,
                uploading_phase: matches!(state.status, UploadStatus::Uploading),
                progress_available: state.progress_available,
                sent: state.sent,
                total: state.total,
            };
        }
    }
    if let Some(upload) = upload {
        let (status, label) = share_status_from_dto(upload.state);
        return ShareView {
            status,
            label,
            url: upload.url.clone().unwrap_or_default(),
            deletion_url: upload.deletion_url.clone(),
            error: upload.error.clone(),
            dest_id: Some(upload.dest_id.clone()),
            queue_id: known_queue_id,
            cancellable: upload.state == UploadStateDto::Queued && known_queue_id.is_some(),
            uploading_phase: matches!(upload.state, UploadStateDto::Uploading),
            progress_available: false,
            sent: upload.sent.max(0) as u64,
            total: upload.total.max(0) as u64,
        };
    }
    ShareView {
        status: "local",
        label: "Not uploaded",
        url: String::new(),
        deletion_url: None,
        error: None,
        dest_id: None,
        queue_id: None,
        cancellable: false,
        uploading_phase: false,
        progress_available: false,
        sent: 0,
        total: 0,
    }
}

fn upload_dto_from_state(state: &UploadState) -> Option<UploadDto> {
    let dto_state = match state.status {
        UploadStatus::Queued => UploadStateDto::Queued,
        UploadStatus::Uploading | UploadStatus::BackingOff => UploadStateDto::Uploading,
        UploadStatus::Done => UploadStateDto::Done,
        UploadStatus::Failed => UploadStateDto::Failed,
        UploadStatus::Cancelled => UploadStateDto::Cancelled,
    };
    Some(UploadDto {
        dest_id: state.dest.clone(),
        state: dto_state,
        sent: state.sent.min(i64::MAX as u64) as i64,
        total: state.total.min(i64::MAX as u64) as i64,
        url: state.link.clone(),
        deletion_url: state.deletion_link.clone(),
        error: state.error.clone(),
        attempts: i64::from(state.attempts),
        updated_at: 0,
    })
}

fn active_upload_for(
    active_uploads: &HashMap<u64, UploadState>,
    capture_id: i64,
) -> Option<&UploadState> {
    let mut chosen: Option<&UploadState> = None;
    for upload in active_uploads.values() {
        if upload.capture_id != Some(capture_id) {
            continue;
        }
        let replace = match chosen {
            None => true,
            Some(prev) => {
                prev.status == UploadStatus::Queued && upload.status != UploadStatus::Queued
            }
        };
        if replace {
            chosen = Some(upload);
        }
    }
    chosen
}

fn is_active_upload(status: UploadStatus) -> bool {
    matches!(
        status,
        UploadStatus::Queued | UploadStatus::Uploading | UploadStatus::BackingOff
    )
}

pub(crate) fn compute_selection(
    items: &[CaptureDto],
    selected_ids: &HashSet<i64>,
    anchor_id: Option<i64>,
    id: i64,
    additive: bool,
    range: bool,
) -> (HashSet<i64>, Option<i64>) {
    let mut ids = if additive {
        selected_ids.clone()
    } else {
        HashSet::new()
    };
    let mut next_anchor = anchor_id;
    if range {
        if let Some(anchor) = anchor_id {
            let from = items.iter().position(|item| item.id == anchor);
            let to = items.iter().position(|item| item.id == id);
            if let (Some(from), Some(to)) = (from, to) {
                let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
                for item in &items[lo..=hi] {
                    ids.insert(item.id);
                }
                return (ids, next_anchor);
            }
        }
        next_anchor = Some(id);
        ids.insert(id);
        return (ids, next_anchor);
    }
    next_anchor = Some(id);
    if !ids.remove(&id) {
        ids.insert(id);
    }
    (ids, next_anchor)
}

struct LoadedPage {
    stats: Result<HistoryStats>,
    page: Result<HistoryPage>,
}

struct HistoryController {
    app: Weak<SnaplineWindow>,
    cache: Arc<ThumbCache>,
    items: Vec<CaptureDto>,
    next: Option<Cursor>,
    stats: HistoryStats,
    selected_ids: HashSet<i64>,
    anchor_id: Option<i64>,
    focus_id: Option<i64>,
    shown_detail_id: Option<i64>,
    pending_select: Option<i64>,
    filter_index: i32,
    query: String,
    grid_view: bool,
    active_uploads: HashMap<u64, UploadState>,
    observed_upload_ids: HashSet<u64>,
    detail_live: Option<UploadState>,
    queue_ids: HashMap<i64, u64>,
    destinations: Vec<DestinationViewDto>,
    destinations_loaded: bool,
    default_destination: String,
    generation: u64,
    loading: bool,
    loaded: bool,
    deleting: bool,
    upload_busy: bool,
    error_message: String,
    delete_error: String,
    share_error: String,
}

impl HistoryController {
    fn new(app: Weak<SnaplineWindow>, cache: Arc<ThumbCache>) -> Self {
        Self {
            app,
            cache,
            items: Vec::new(),
            next: None,
            stats: HistoryStats {
                count: 0,
                bytes: 0,
                uploaded_this_week: 0,
            },
            selected_ids: HashSet::new(),
            anchor_id: None,
            focus_id: None,
            shown_detail_id: None,
            pending_select: None,
            filter_index: 0,
            query: String::new(),
            grid_view: true,
            active_uploads: HashMap::new(),
            observed_upload_ids: HashSet::new(),
            detail_live: None,
            queue_ids: HashMap::new(),
            destinations: Vec::new(),
            destinations_loaded: false,
            default_destination: String::new(),
            generation: 0,
            loading: false,
            loaded: false,
            deleting: false,
            upload_busy: false,
            error_message: String::new(),
            delete_error: String::new(),
            share_error: String::new(),
        }
    }

    fn current_banner(&self) -> &str {
        if !self.delete_error.is_empty() {
            &self.delete_error
        } else {
            &self.error_message
        }
    }

    fn sync_ui(&mut self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if self.loaded {
            app.set_history_count(self.stats.count.to_string().into());
        }
        let s = app.global::<HistoryState>();
        s.set_stats_line(
            format!(
                "{} captures · {} on disk · {} uploaded this week",
                self.stats.count,
                format_bytes(self.stats.bytes),
                self.stats.uploaded_this_week
            )
            .into(),
        );
        s.set_filter_index(self.filter_index);
        s.set_filter_label(filter_label_for(self.filter_index).into());
        if s.get_query().as_str() != self.query {
            s.set_query(self.query.clone().into());
        }
        s.set_grid_view(self.grid_view);
        s.set_loading(self.loading);
        s.set_loaded(self.loaded);
        s.set_deleting(self.deleting);
        s.set_upload_busy(self.upload_busy);
        s.set_has_more(self.next.is_some());
        s.set_message(self.current_banner().into());

        let searching = !self.query.trim().is_empty();
        let filtered = searching || self.filter_index != 0;
        let is_empty =
            self.loaded && !self.loading && self.error_message.is_empty() && self.items.is_empty();
        s.set_is_empty(is_empty);
        s.set_empty_filtered(filtered);
        s.set_can_clear_search(searching);
        s.set_can_reset_filter(self.filter_index != 0);

        let thumbs: Vec<Option<slint::Image>> = self
            .items
            .iter()
            .map(|item| {
                if is_timed(item.kind) || matches!(item.kind, CaptureKindDto::Image) {
                    self.cache.get_cached(item.id)
                } else {
                    None
                }
            })
            .collect();

        let cells: Vec<HistoryCellData> = self
            .items
            .iter()
            .zip(thumbs)
            .map(|(item, thumb)| {
                let selected = self.selected_ids.contains(&item.id);
                let active = active_upload_for(&self.active_uploads, item.id);
                cell_for(item, selected, active, thumb)
            })
            .collect();
        let rows = build_rows(&cells);
        s.set_cells(ModelRc::new(VecModel::from(cells)));
        s.set_rows(rows);

        self.sync_destinations_to_ui(&s);
        self.sync_details_to_ui(&s);
    }

    fn sync_destinations_to_ui(&self, s: &HistoryState) {
        let configured: Vec<HistoryDestOption> = self
            .destinations
            .iter()
            .filter(|dest| dest.configured)
            .map(|dest| HistoryDestOption {
                id: dest.id.clone().into(),
                name: dest.name.clone().into(),
            })
            .collect();
        s.set_destinations(ModelRc::new(VecModel::from(configured)));
    }

    fn sync_details_to_ui(&mut self, s: &HistoryState) {
        let selected_count = self.selected_ids.len() as i32;
        s.set_selected_count(selected_count);
        if self.selected_ids.len() != 1 {
            self.shown_detail_id = None;
            self.detail_live = None;
            self.share_error.clear();
            s.set_selected_id(-1);
            s.set_has_selection(false);
            s.set_confirm_delete(false);
            return;
        }
        let selected_id = match self.selected_ids.iter().next().copied() {
            Some(id) => id,
            None => return,
        };
        let Some(capture_index) = self.items.iter().position(|item| item.id == selected_id) else {
            self.shown_detail_id = None;
            self.detail_live = None;
            s.set_selected_id(-1);
            s.set_has_selection(false);
            return;
        };
        let thumb_needed = {
            let capture = &self.items[capture_index];
            is_timed(capture.kind) || matches!(capture.kind, CaptureKindDto::Image)
        };
        let thumb = if thumb_needed {
            self.cache.get_cached(selected_id)
        } else {
            None
        };
        let capture = &self.items[capture_index];
        if self.shown_detail_id != Some(selected_id) {
            self.shown_detail_id = Some(selected_id);
            self.detail_live = None;
            self.share_error.clear();
        }
        s.set_selected_id(capture.id as i32);
        s.set_has_selection(true);
        s.set_confirm_delete(false);
        s.set_detail_name(capture.name.clone().into());
        s.set_detail_kind(
            format!(
                "{} · {} at {}",
                type_label(capture.kind, &capture.name),
                short_day(capture.created_at),
                clock(capture.created_at)
            )
            .into(),
        );
        s.set_detail_tag(tag_for(capture.kind, &capture.name).into());
        s.set_detail_has_thumb(thumb.is_some());
        s.set_detail_thumb(thumb.unwrap_or_default());
        s.set_detail_is_image(matches!(capture.kind, CaptureKindDto::Image));
        s.set_detail_owned(capture.owned);
        s.set_detail_missing(capture.missing);
        s.set_detail_dimensions(match (capture.width, capture.height) {
            (Some(w), Some(h)) => format!("{w} × {h}").into(),
            _ => "—".into(),
        });
        s.set_detail_size(format_bytes(capture.bytes).into());
        let timed = if is_timed(capture.kind) {
            capture.duration_ms.map(format_duration).unwrap_or_default()
        } else {
            String::new()
        };
        s.set_detail_timed(if timed.is_empty() {
            "".into()
        } else {
            format!("{} · {}", tag_for(capture.kind, &capture.name), timed).into()
        });
        s.set_detail_duration(
            capture
                .duration_ms
                .map(format_duration)
                .unwrap_or_default()
                .into(),
        );
        s.set_detail_source(capture.source_app.as_deref().unwrap_or("Desktop").into());
        s.set_detail_folder(if capture.folder.is_empty() {
            "—".into()
        } else {
            capture.folder.clone().into()
        });

        let known_queue_id = self.queue_ids.get(&capture.id).copied();
        let active_for_capture = active_upload_for(&self.active_uploads, capture.id);
        let current_live = self
            .detail_live
            .as_ref()
            .filter(|state| live_matches_capture(capture, known_queue_id, state))
            .or(active_for_capture);
        let share = compute_share_view(capture, known_queue_id, current_live);

        let dest_label = match share.dest_id.as_deref() {
            Some(id) if !id.is_empty() => host_label(id, &self.destinations),
            _ => "—".into(),
        };
        s.set_detail_destination(dest_label.clone().into());
        s.set_share_status(share.status.into());
        s.set_share_label(share.label.into());
        s.set_share_url(share.url.into());
        let has_deletion_url = share
            .deletion_url
            .as_deref()
            .is_some_and(|url| !url.is_empty());
        s.set_share_has_deletion_url(has_deletion_url);
        s.set_share_deletion_url(share.deletion_url.clone().unwrap_or_default().into());
        s.set_share_deletion(if has_deletion_url {
            "Deletion link saved".into()
        } else if share.status == "uploaded"
            && matches!(share.dest_id.as_deref(), Some("imgur" | "s3"))
        {
            format!("Delete from {dest_label}").into()
        } else {
            "".into()
        });
        s.set_share_uploading(share.uploading_phase);
        s.set_share_progress(if share.progress_available && share.total > 0 {
            share.sent as f32 / share.total as f32
        } else {
            -1.0
        });
        s.set_share_cancellable(share.cancellable);
        s.set_share_note(
            match share.status {
                "failed" => format!(
                    "Upload failed: {}",
                    share
                        .error
                        .as_deref()
                        .unwrap_or("the destination did not accept the file.")
                ),
                "uploading" if share.uploading_phase => {
                    if share.progress_available && share.total > 0 {
                        format!(
                            "{} of {}",
                            format_bytes_u64(share.sent),
                            format_bytes_u64(share.total)
                        )
                    } else {
                        "Sending file…".into()
                    }
                }
                "local" if capture.missing => {
                    "The original file is unavailable. Add it again before uploading.".into()
                }
                "local" => "This item is saved locally.".into(),
                _ => String::new(),
            }
            .into(),
        );
        s.set_share_error(self.share_error.clone().into());
    }
}

#[derive(Clone)]
pub struct HistoryHandle(Arc<Mutex<HistoryController>>);

impl HistoryHandle {
    pub fn new(app: Weak<SnaplineWindow>, cache: Arc<ThumbCache>) -> Self {
        Self(Arc::new(Mutex::new(HistoryController::new(app, cache))))
    }

    fn lock(&self) -> MutexGuard<'_, HistoryController> {
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn start_initial(&self, backend: &Backend, select_id: Option<i64>) {
        if let Some(id) = select_id {
            self.lock().pending_select = Some(id);
        }
        self.refresh_destinations(backend);
        self.load_active_uploads(backend);
        self.fetch_page(backend, true);
    }

    pub fn reload_reset(&self, backend: &Backend) {
        {
            let mut ctrl = self.lock();
            ctrl.items.clear();
            ctrl.next = None;
            ctrl.selected_ids.clear();
            ctrl.anchor_id = None;
            ctrl.focus_id = None;
            ctrl.shown_detail_id = None;
            ctrl.detail_live = None;
            ctrl.delete_error.clear();
            ctrl.share_error.clear();
            ctrl.loaded = false;
        }
        self.fetch_page(backend, true);
    }

    pub fn refresh_keep(&self, backend: &Backend) {
        self.fetch_page(backend, true);
    }

    pub fn load_more(&self, backend: &Backend) {
        let should_load = {
            let ctrl = self.lock();
            !ctrl.loading && ctrl.next.is_some()
        };
        if should_load {
            self.fetch_page(backend, false);
        }
    }

    fn fetch_page(&self, backend: &Backend, reset: bool) {
        let (gen, filter, query, cursor, minimum, cache) = {
            let mut ctrl = self.lock();
            let gen = if reset {
                ctrl.generation += 1;
                ctrl.generation
            } else {
                ctrl.generation
            };
            ctrl.loading = true;
            let cursor = if reset { None } else { ctrl.next };
            let minimum = if reset { ctrl.items.len() } else { 0 };
            let filter = filter_for(ctrl.filter_index);
            let query = ctrl.query.trim().to_owned();
            let cache = ctrl.cache.clone();
            ctrl.sync_ui();
            (gen, filter, query, cursor, minimum, cache)
        };
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| {
                let stats = match bg.history_request(HistoryReq::Stats) {
                    Ok(HistoryResp::Stats { stats }) => Ok(stats),
                    Ok(other) => Err(anyhow::anyhow!("unexpected stats response: {other:?}")),
                    Err(error) => Err(error),
                };
                let read = |at: Option<Cursor>| -> Result<HistoryPage> {
                    if query.is_empty() {
                        bg.history_page_dto(filter, at)
                    } else {
                        bg.history_search_dto(&query, filter, at)
                    }
                };
                let page = (|| -> Result<HistoryPage> {
                    let mut page = read(cursor)?;
                    if reset {
                        while let Some(next_cursor) = page.next {
                            if page.items.len() >= minimum {
                                break;
                            }
                            let more = read(Some(next_cursor))?;
                            page.items.extend(more.items);
                            page.next = more.next;
                        }
                    }
                    for item in &page.items {
                        if is_timed(item.kind) || matches!(item.kind, CaptureKindDto::Image) {
                            let _ = cache.load_or_get(&bg, item.id);
                        }
                    }
                    Ok(page)
                })();
                LoadedPage { stats, page }
            },
            move |_, loaded| {
                let mut ctrl = handle.lock();
                if ctrl.generation != gen {
                    return;
                }
                ctrl.loading = false;
                ctrl.loaded = true;
                if let Ok(stats) = loaded.stats {
                    ctrl.stats = stats;
                }
                match loaded.page {
                    Ok(page) => {
                        ctrl.error_message.clear();
                        if reset {
                            ctrl.items = page.items;
                            let available: HashSet<i64> =
                                ctrl.items.iter().map(|item| item.id).collect();
                            ctrl.selected_ids.retain(|id| available.contains(id));
                            if ctrl.focus_id.is_some_and(|id| !available.contains(&id)) {
                                ctrl.focus_id = None;
                            }
                            if ctrl.anchor_id.is_some_and(|id| !available.contains(&id)) {
                                ctrl.anchor_id = None;
                            }
                        } else {
                            let known: HashSet<i64> =
                                ctrl.items.iter().map(|item| item.id).collect();
                            for item in page.items {
                                if !known.contains(&item.id) {
                                    ctrl.items.push(item);
                                }
                            }
                        }
                        ctrl.next = page.next;
                        if let Some(target_id) = ctrl.pending_select.take() {
                            if ctrl.items.iter().any(|item| item.id == target_id) {
                                ctrl.selected_ids.clear();
                                ctrl.selected_ids.insert(target_id);
                                ctrl.anchor_id = Some(target_id);
                                ctrl.focus_id = Some(target_id);
                            }
                        }
                    }
                    Err(error) => {
                        ctrl.error_message = format!("{error:#}");
                    }
                }
                ctrl.sync_ui();
            },
        );
    }

    pub fn refresh_destinations(&self, backend: &Backend) {
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| {
                let dests = bg.destinations().ok();
                let default_dest = bg.settings().ok().map(|settings| settings.uploads.images);
                (dests, default_dest)
            },
            move |_, (dests, default_dest)| {
                let mut ctrl = handle.lock();
                if let Some(list) = dests {
                    ctrl.destinations = list;
                    ctrl.destinations_loaded = true;
                }
                if let Some(default_dest) = default_dest {
                    ctrl.default_destination = default_dest;
                }
                if let Some(app) = ctrl.app.upgrade() {
                    let s = app.global::<HistoryState>();
                    ctrl.sync_destinations_to_ui(&s);
                    ctrl.sync_details_to_ui(&s);
                }
            },
        );
    }

    pub fn ensure_destinations(&self, backend: &Backend) {
        if !self.lock().destinations_loaded {
            self.refresh_destinations(backend);
        }
    }

    fn load_active_uploads(&self, backend: &Backend) {
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| match bg.upload_request(UploadReq::List) {
                Ok(UploadResp::States(list)) => Ok(list),
                Ok(other) => Err(anyhow::anyhow!(
                    "unexpected upload list response: {other:?}"
                )),
                Err(error) => Err(error),
            },
            move |_, result| {
                if let Ok(states) = result {
                    let mut ctrl = handle.lock();
                    let mut changed = false;
                    for state in states {
                        if state.capture_id.is_some()
                            && !ctrl.observed_upload_ids.contains(&state.id)
                            && is_active_upload(state.status)
                        {
                            ctrl.active_uploads.insert(state.id, state);
                            changed = true;
                        }
                    }
                    if changed {
                        ctrl.sync_ui();
                    }
                }
            },
        );
    }

    pub fn handle_history_event(
        &self,
        backend: &Backend,
        event: &snapline_services::history::Event,
    ) {
        let weak = {
            let mut ctrl = self.lock();
            match event {
                snapline_services::history::Event::Removed { id } => {
                    ctrl.cache.remove(*id);
                    ctrl.selected_ids.remove(id);
                    if ctrl.anchor_id == Some(*id) {
                        ctrl.anchor_id = None;
                    }
                    if ctrl.focus_id == Some(*id) {
                        ctrl.focus_id = None;
                    }
                }
                snapline_services::history::Event::RemovedMany { ids } => {
                    for id in ids {
                        ctrl.cache.remove(*id);
                        ctrl.selected_ids.remove(id);
                        if ctrl.anchor_id == Some(*id) {
                            ctrl.anchor_id = None;
                        }
                        if ctrl.focus_id == Some(*id) {
                            ctrl.focus_id = None;
                        }
                    }
                }
                snapline_services::history::Event::Added { .. }
                | snapline_services::history::Event::Ping => {}
            }
            ctrl.app.clone()
        };
        self.refresh_keep(backend);
        tools::refresh_history_choices(&weak, backend);
    }

    pub fn apply_upload_state(&self, backend: &Backend, state: &UploadState) {
        let should_reload = matches!(
            state.status,
            UploadStatus::Queued
                | UploadStatus::Done
                | UploadStatus::Failed
                | UploadStatus::Cancelled
        );
        {
            let mut ctrl = self.lock();
            ctrl.observed_upload_ids.insert(state.id);
            if let Some(capture_id) = state.capture_id {
                ctrl.queue_ids.insert(capture_id, state.id);
                if is_active_upload(state.status) {
                    ctrl.active_uploads.insert(state.id, state.clone());
                } else {
                    ctrl.active_uploads.remove(&state.id);
                    if let Some(item) = ctrl.items.iter_mut().find(|item| item.id == capture_id) {
                        item.upload = if matches!(state.status, UploadStatus::Cancelled) {
                            None
                        } else {
                            upload_dto_from_state(state)
                        };
                    }
                }
            }
            let selected_single = if ctrl.selected_ids.len() == 1 {
                ctrl.selected_ids.iter().next().copied()
            } else {
                None
            };
            if let Some(selected_id) = selected_single {
                if let Some(capture) = ctrl.items.iter().find(|item| item.id == selected_id) {
                    let known_queue_id = ctrl.queue_ids.get(&capture.id).copied();
                    if live_matches_capture(capture, known_queue_id, state) {
                        ctrl.detail_live = Some(state.clone());
                        if !is_active_upload(state.status) {
                            ctrl.share_error.clear();
                        }
                    }
                }
            }
            ctrl.sync_ui();
        }
        if should_reload {
            self.refresh_keep(backend);
        }
    }

    pub fn select(&self, id: i64, additive: bool, range: bool) {
        let mut ctrl = self.lock();
        let (next_ids, next_anchor) = compute_selection(
            &ctrl.items,
            &ctrl.selected_ids,
            ctrl.anchor_id,
            id,
            additive,
            range,
        );
        ctrl.selected_ids = next_ids;
        ctrl.anchor_id = next_anchor;
        ctrl.focus_id = Some(id);
        ctrl.sync_ui();
    }

    pub fn select_exclusive(&self, backend: &Backend, id: i64) {
        enum Outcome {
            Selected,
            Refresh,
            ResetAndReload,
        }
        let outcome = {
            let mut ctrl = self.lock();
            if ctrl.items.iter().any(|item| item.id == id) {
                ctrl.selected_ids.clear();
                ctrl.selected_ids.insert(id);
                ctrl.anchor_id = Some(id);
                ctrl.focus_id = Some(id);
                ctrl.sync_ui();
                Outcome::Selected
            } else {
                ctrl.pending_select = Some(id);
                if ctrl.filter_index != 0 || !ctrl.query.trim().is_empty() {
                    ctrl.filter_index = 0;
                    ctrl.query.clear();
                    if let Some(app) = ctrl.app.upgrade() {
                        app.set_history_find("".into());
                        if app.get_page() == 3 {
                            app.set_find("".into());
                        }
                    }
                    Outcome::ResetAndReload
                } else {
                    Outcome::Refresh
                }
            }
        };
        match outcome {
            Outcome::Selected => {}
            Outcome::Refresh => self.refresh_keep(backend),
            Outcome::ResetAndReload => self.reload_reset(backend),
        }
    }

    pub fn clear_selection(&self) {
        let mut ctrl = self.lock();
        if ctrl.selected_ids.is_empty() {
            return;
        }
        ctrl.selected_ids.clear();
        ctrl.anchor_id = None;
        ctrl.focus_id = None;
        ctrl.shown_detail_id = None;
        ctrl.detail_live = None;
        ctrl.share_error.clear();
        ctrl.sync_ui();
    }

    pub fn move_selection(&self, delta: i32) {
        let mut ctrl = self.lock();
        if ctrl.items.is_empty() {
            return;
        }
        let current_id = ctrl.focus_id.or(ctrl.anchor_id);
        let index = current_id
            .and_then(|id| ctrl.items.iter().position(|item| item.id == id))
            .map_or(0, |pos| {
                (pos as isize + delta as isize).clamp(0, ctrl.items.len() as isize - 1) as usize
            });
        let id = ctrl.items[index].id;
        ctrl.selected_ids.clear();
        ctrl.selected_ids.insert(id);
        ctrl.anchor_id = Some(id);
        ctrl.focus_id = Some(id);
        ctrl.sync_ui();
    }

    pub fn set_query(&self, backend: &Backend, query: String) {
        {
            let mut ctrl = self.lock();
            if ctrl.query == query {
                return;
            }
            ctrl.query = query;
        }
        self.reload_reset(backend);
    }

    pub fn set_filter(&self, backend: &Backend, index: i32) {
        {
            let mut ctrl = self.lock();
            if ctrl.filter_index == index {
                return;
            }
            ctrl.filter_index = index;
        }
        self.reload_reset(backend);
    }

    pub fn set_grid(&self, grid: bool) {
        let mut ctrl = self.lock();
        ctrl.grid_view = grid;
        ctrl.sync_ui();
    }

    pub fn open_file(&self, backend: &Backend, id: i64) {
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| bg.history_request(HistoryReq::OpenFile { id }),
            move |_, result| {
                let mut ctrl = handle.lock();
                match result {
                    Ok(_) => ctrl.error_message.clear(),
                    Err(error) => ctrl.error_message = format!("{error:#}"),
                }
                ctrl.sync_ui();
            },
        );
    }

    pub fn copy_image(&self, backend: &Backend, id: i64) {
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| bg.history_request(HistoryReq::CopyImage { id }),
            move |_, result| {
                let mut ctrl = handle.lock();
                match result {
                    Ok(_) => ctrl.error_message.clear(),
                    Err(error) => ctrl.error_message = format!("{error:#}"),
                }
                ctrl.sync_ui();
            },
        );
    }

    pub fn show_in_folder(&self, backend: &Backend, id: i64) {
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| bg.history_request(HistoryReq::ShowInFolder { id }),
            move |_, result| {
                let mut ctrl = handle.lock();
                match result {
                    Ok(_) => ctrl.error_message.clear(),
                    Err(error) => ctrl.error_message = format!("{error:#}"),
                }
                ctrl.sync_ui();
            },
        );
    }

    pub fn annotate(&self, backend: &Backend, id: i64) {
        let handle = self.clone();
        let cache = self.lock().cache.clone();
        spawn_bg(
            backend,
            move |bg| bg.tool_request(ToolReq::EditorOpenHistory { capture_id: id }),
            move |ui_bg, result| {
                let (edited_id, weak) = {
                    let mut ctrl = handle.lock();
                    let mut edited_id = None;
                    match result {
                        Ok(ToolResp::EditorOpened { outcome }) => {
                            ctrl.error_message.clear();
                            if !outcome.cancelled {
                                if let Some(cid) = outcome.capture_id {
                                    cache.remove(id);
                                    cache.remove(cid);
                                    edited_id = Some(cid);
                                }
                            }
                        }
                        Ok(_) => ctrl.error_message.clear(),
                        Err(error) => ctrl.error_message = format!("{error:#}"),
                    }
                    ctrl.sync_ui();
                    (edited_id, ctrl.app.clone())
                };
                if edited_id.is_some() {
                    handle.refresh_keep(&ui_bg);
                    tools::refresh_history_choices(&weak, &ui_bg);
                }
            },
        );
    }

    pub fn pin_capture(&self, backend: &Backend, id: i64) {
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| bg.tool_request(ToolReq::PinHistory { capture_id: id }),
            move |_, result| {
                let mut ctrl = handle.lock();
                match result {
                    Ok(_) => ctrl.error_message.clear(),
                    Err(error) => ctrl.error_message = format!("{error:#}"),
                }
                ctrl.sync_ui();
            },
        );
    }

    pub fn upload_files(&self, backend: &Backend) {
        {
            let ctrl = self.lock();
            if ctrl.upload_busy {
                return;
            }
        }
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| bg.upload_request(UploadReq::PickAndUpload),
            move |ui_bg, result| {
                let (uploaded, weak) = {
                    let mut ctrl = handle.lock();
                    let mut uploaded = false;
                    match result {
                        Ok(UploadResp::AcceptedMany { ids }) => {
                            ctrl.error_message.clear();
                            uploaded = !ids.is_empty();
                        }
                        Ok(_) => ctrl.error_message.clear(),
                        Err(error) => ctrl.error_message = format!("{error:#}"),
                    }
                    ctrl.sync_ui();
                    (uploaded, ctrl.app.clone())
                };
                if uploaded {
                    handle.refresh_keep(&ui_bg);
                    tools::refresh_history_choices(&weak, &ui_bg);
                }
            },
        );
    }

    pub fn delete_selected(&self, backend: &Backend) {
        let ids: Vec<i64> = {
            let ctrl = self.lock();
            ctrl.items
                .iter()
                .filter(|item| ctrl.selected_ids.contains(&item.id))
                .map(|item| item.id)
                .collect()
        };
        self.delete_ids(backend, ids);
    }

    pub fn delete_ids(&self, backend: &Backend, ids: Vec<i64>) {
        if ids.is_empty() {
            return;
        }
        let cache = {
            let mut ctrl = self.lock();
            if ctrl.deleting {
                return;
            }
            ctrl.deleting = true;
            ctrl.delete_error.clear();
            ctrl.sync_ui();
            ctrl.cache.clone()
        };
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| -> Result<(Vec<i64>, usize)> {
                if ids.len() == 1 {
                    let id = ids[0];
                    let confirmed = match bg.history_request(HistoryReq::ConfirmDelete { id })? {
                        HistoryResp::ConfirmDelete { confirmed } => confirmed,
                        other => bail!("unexpected confirm response: {other:?}"),
                    };
                    if !confirmed {
                        return Ok((Vec::new(), 0));
                    }
                    bg.history_request(HistoryReq::Delete { id })?;
                    return Ok((vec![id], 0));
                }
                match bg.history_request(HistoryReq::DeleteMany { ids })? {
                    HistoryResp::DeleteMany { removed, failed } => Ok((removed, failed)),
                    other => bail!("unexpected batch delete response: {other:?}"),
                }
            },
            move |ui_bg, result| {
                let (any_removed, weak) = {
                    let mut ctrl = handle.lock();
                    let any_removed;
                    ctrl.deleting = false;
                    match result {
                        Ok((removed, failed)) => {
                            if failed > 0 {
                                let noun = if failed == 1 { "item" } else { "items" };
                                ctrl.delete_error =
                                    format!("Could not delete {failed} {noun}. Try again.");
                            }
                            any_removed = !removed.is_empty();
                            if any_removed {
                                let deleted: HashSet<i64> = removed.into_iter().collect();
                                for id in &deleted {
                                    cache.remove(*id);
                                }
                                ctrl.items.retain(|item| !deleted.contains(&item.id));
                                ctrl.selected_ids.retain(|id| !deleted.contains(id));
                                if ctrl.focus_id.is_some_and(|id| deleted.contains(&id)) {
                                    ctrl.focus_id = None;
                                }
                                if ctrl.anchor_id.is_some_and(|id| deleted.contains(&id)) {
                                    ctrl.anchor_id = None;
                                }
                            }
                        }
                        Err(error) => {
                            any_removed = false;
                            ctrl.delete_error = format!("{error:#}");
                        }
                    }
                    ctrl.sync_ui();
                    (any_removed, ctrl.app.clone())
                };
                if any_removed {
                    handle.refresh_stats_only(&ui_bg);
                    tools::refresh_history_choices(&weak, &ui_bg);
                }
            },
        );
    }

    fn refresh_stats_only(&self, backend: &Backend) {
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| match bg.history_request(HistoryReq::Stats) {
                Ok(HistoryResp::Stats { stats }) => Ok(stats),
                Ok(other) => Err(anyhow::anyhow!("unexpected stats response: {other:?}")),
                Err(error) => Err(error),
            },
            move |_, result| {
                let mut ctrl = handle.lock();
                if let Ok(stats) = result {
                    ctrl.stats = stats;
                    ctrl.sync_ui();
                }
            },
        );
    }

    pub fn activate_selected(&self, backend: &Backend) {
        let target = {
            let ctrl = self.lock();
            if ctrl.selected_ids.len() == 1 {
                ctrl.selected_ids.iter().next().copied()
            } else {
                None
            }
        };
        if let Some(id) = target {
            self.open_file(backend, id);
        }
    }

    pub fn copy_selected(&self, backend: &Backend) {
        let target = {
            let ctrl = self.lock();
            if ctrl.selected_ids.len() == 1 {
                ctrl.selected_ids.iter().next().copied()
            } else {
                None
            }
        };
        if let Some(id) = target {
            self.copy_image(backend, id);
        }
    }

    pub fn copy_share_text(&self, backend: &Backend, text: String) {
        if text.is_empty() {
            return;
        }
        let mut ctrl = self.lock();
        match backend.host().copy_text(&text) {
            Ok(()) => ctrl.share_error.clear(),
            Err(_) => ctrl.share_error = "The link could not be copied.".into(),
        }
        ctrl.sync_ui();
    }

    pub fn upload_to(&self, backend: &Backend, dest: String) {
        let (capture_id, file_name) = {
            let mut ctrl = self.lock();
            if ctrl.upload_busy || ctrl.selected_ids.len() != 1 {
                return;
            }
            let Some(id) = ctrl.selected_ids.iter().next().copied() else {
                return;
            };
            let Some(capture) = ctrl.items.iter().find(|item| item.id == id) else {
                return;
            };
            let name = capture.name.clone();
            ctrl.upload_busy = true;
            ctrl.share_error.clear();
            ctrl.sync_ui();
            (id, name)
        };
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| {
                bg.upload_request(UploadReq::UploadElsewhere {
                    capture_id,
                    dest: dest.clone(),
                })
                .map(|resp| (resp, dest))
            },
            move |_, result| {
                let mut ctrl = handle.lock();
                ctrl.upload_busy = false;
                match result {
                    Ok((UploadResp::Accepted { id: queue_id }, dest)) => {
                        let skip = ctrl.detail_live.as_ref().is_some_and(|live| {
                            live.capture_id == Some(capture_id) && live.id >= queue_id
                        });
                        if !skip {
                            let queued = UploadState {
                                id: queue_id,
                                capture_id: Some(capture_id),
                                file_name,
                                dest,
                                run_id: None,
                                status: UploadStatus::Queued,
                                cancellable: true,
                                progress_available: false,
                                sent: 0,
                                total: 0,
                                link_batch: None,
                                attempts: 0,
                                link: None,
                                page_link: None,
                                deletion_link: None,
                                error: None,
                                retry_in_ms: None,
                            };
                            ctrl.queue_ids.insert(capture_id, queue_id);
                            ctrl.active_uploads.insert(queue_id, queued.clone());
                            ctrl.detail_live = Some(queued);
                        }
                    }
                    Ok((other, _)) => {
                        ctrl.share_error = format!("unexpected upload response: {other:?}");
                    }
                    Err(error) => {
                        ctrl.share_error = format!("{error:#}");
                    }
                }
                ctrl.sync_ui();
            },
        );
    }

    pub fn retry_upload(&self, backend: &Backend) {
        enum RetryAction {
            Queue(u64),
            Enqueue(String),
        }
        let action = {
            let mut ctrl = self.lock();
            if ctrl.upload_busy || ctrl.selected_ids.len() != 1 {
                return;
            }
            let Some(capture_id) = ctrl.selected_ids.iter().next().copied() else {
                return;
            };
            let Some(capture) = ctrl.items.iter().find(|item| item.id == capture_id) else {
                return;
            };
            let known_queue_id = ctrl.queue_ids.get(&capture_id).copied();
            let active_for_capture = active_upload_for(&ctrl.active_uploads, capture_id);
            let current_live = ctrl
                .detail_live
                .as_ref()
                .filter(|state| live_matches_capture(capture, known_queue_id, state))
                .or(active_for_capture);
            let share = compute_share_view(capture, known_queue_id, current_live);
            let live_queue_id = share
                .queue_id
                .filter(|qid| ctrl.detail_live.as_ref().is_some_and(|l| l.id == *qid));
            if let Some(queue_id) = live_queue_id {
                ctrl.upload_busy = true;
                ctrl.share_error.clear();
                ctrl.sync_ui();
                RetryAction::Queue(queue_id)
            } else {
                let dest = share
                    .dest_id
                    .filter(|id| !id.is_empty())
                    .or_else(|| {
                        (!ctrl.default_destination.is_empty()
                            && ctrl.default_destination != "none"
                            && ctrl.default_destination != "save_locally_only")
                            .then(|| ctrl.default_destination.clone())
                    })
                    .or_else(|| {
                        ctrl.destinations
                            .iter()
                            .find(|dest| dest.configured)
                            .map(|dest| dest.id.clone())
                    });
                match dest {
                    Some(dest) => RetryAction::Enqueue(dest),
                    None => {
                        ctrl.share_error = "No upload destination configured.".into();
                        ctrl.sync_ui();
                        return;
                    }
                }
            }
        };
        match action {
            RetryAction::Enqueue(dest) => self.upload_to(backend, dest),
            RetryAction::Queue(queue_id) => {
                let handle = self.clone();
                spawn_bg(
                    backend,
                    move |bg| bg.upload_request(UploadReq::Retry { id: queue_id }),
                    move |_, result| {
                        let mut ctrl = handle.lock();
                        ctrl.upload_busy = false;
                        if let Err(error) = result {
                            ctrl.share_error = format!("{error:#}");
                        }
                        ctrl.sync_ui();
                    },
                );
            }
        }
    }

    pub fn cancel_upload(&self, backend: &Backend) {
        let queue_id = {
            let mut ctrl = self.lock();
            if ctrl.upload_busy || ctrl.selected_ids.len() != 1 {
                return;
            }
            let Some(capture_id) = ctrl.selected_ids.iter().next().copied() else {
                return;
            };
            let Some(capture) = ctrl.items.iter().find(|item| item.id == capture_id) else {
                return;
            };
            let known_queue_id = ctrl.queue_ids.get(&capture_id).copied();
            let active_for_capture = active_upload_for(&ctrl.active_uploads, capture_id);
            let current_live = ctrl
                .detail_live
                .as_ref()
                .filter(|state| live_matches_capture(capture, known_queue_id, state))
                .or(active_for_capture);
            let share = compute_share_view(capture, known_queue_id, current_live);
            let Some(queue_id) = share.queue_id else {
                return;
            };
            ctrl.upload_busy = true;
            ctrl.share_error.clear();
            ctrl.sync_ui();
            queue_id
        };
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| bg.upload_request(UploadReq::Cancel { id: queue_id }),
            move |_, result| {
                let mut ctrl = handle.lock();
                ctrl.upload_busy = false;
                if let Err(error) = result {
                    ctrl.share_error = format!("{error:#}");
                }
                ctrl.sync_ui();
            },
        );
    }

    pub fn cancel_row_upload(&self, backend: &Backend, capture_id: i64) {
        let queue_id = {
            let ctrl = self.lock();
            active_upload_for(&ctrl.active_uploads, capture_id).map(|u| u.id)
        };
        let Some(queue_id) = queue_id else {
            return;
        };
        let handle = self.clone();
        spawn_bg(
            backend,
            move |bg| bg.upload_request(UploadReq::Cancel { id: queue_id }),
            move |_, result| {
                if let Err(error) = result {
                    let mut ctrl = handle.lock();
                    ctrl.error_message = format!("{error:#}");
                    ctrl.sync_ui();
                }
            },
        );
    }
}

pub fn wire(app: &SnaplineWindow, backend: Rc<Backend>, cache: Arc<ThumbCache>) -> HistoryHandle {
    let weak = app.as_weak();
    let handle = HistoryHandle::new(weak.clone(), cache);
    let s = app.global::<HistoryState>();

    let h = handle.clone();
    s.on_select(move |id, additive, range| {
        h.select(i64::from(id), additive, range);
    });

    let h = handle.clone();
    s.on_clear_selection(move || {
        h.clear_selection();
    });

    let h = handle.clone();
    s.on_close_details(move || {
        h.clear_selection();
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_open_file(move |id| {
        h.open_file(&b, i64::from(id));
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_annotate(move |id| {
        h.annotate(&b, i64::from(id));
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_copy_image(move |id| {
        h.copy_image(&b, i64::from(id));
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_show_in_folder(move |id| {
        h.show_in_folder(&b, i64::from(id));
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_pin_capture(move |id| {
        h.pin_capture(&b, i64::from(id));
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_delete_capture(move |id| {
        h.delete_ids(&b, vec![i64::from(id)]);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_delete_selected(move || {
        h.delete_selected(&b);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_upload_files(move || {
        h.upload_files(&b);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_load_more(move || {
        h.load_more(&b);
    });

    let (b, h, w) = (backend.clone(), handle.clone(), weak.clone());
    s.on_clear_search(move || {
        if let Some(app) = w.upgrade() {
            app.set_history_find("".into());
            if app.get_page() == 3 {
                app.set_find("".into());
            }
        }
        h.set_query(&b, String::new());
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_reset_filter(move || {
        h.set_filter(&b, 0);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_copy_share_link(move |url| {
        h.copy_share_text(&b, url.to_string());
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_copy_deletion_link(move |url| {
        h.copy_share_text(&b, url.to_string());
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_retry_upload(move || {
        h.retry_upload(&b);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_cancel_upload(move || {
        h.cancel_upload(&b);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_cancel_row_upload(move |id| {
        h.cancel_row_upload(&b, i64::from(id));
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_upload_to(move |dest| {
        h.upload_to(&b, dest.to_string());
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_ensure_destinations(move || {
        h.ensure_destinations(&b);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_search_changed(move |text| {
        h.set_query(&b, text.to_string());
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_filter_changed(move |index| {
        h.set_filter(&b, index);
    });

    let h = handle.clone();
    s.on_set_grid(move |grid| {
        h.set_grid(grid);
    });

    let h = handle.clone();
    s.on_move_selection(move |delta| {
        h.move_selection(delta);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_activate_selected(move || {
        h.activate_selected(&b);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_copy_selected(move || {
        h.copy_selected(&b);
    });

    let (b, h) = (backend.clone(), handle.clone());
    s.on_delete_hotkey(move || {
        h.delete_selected(&b);
    });

    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_capture(
        id: i64,
        kind: CaptureKindDto,
        name: &str,
        upload: Option<UploadDto>,
    ) -> CaptureDto {
        CaptureDto {
            id,
            kind,
            owned: true,
            missing: false,
            name: name.to_owned(),
            bytes: 1024,
            width: Some(800),
            height: Some(600),
            duration_ms: None,
            source_app: None,
            created_at: 1_760_000_000_000,
            folder: String::new(),
            upload,
        }
    }

    fn sample_upload_state(id: u64, capture_id: Option<i64>, status: UploadStatus) -> UploadState {
        UploadState {
            id,
            capture_id,
            file_name: "shot.png".to_owned(),
            dest: "s3".to_owned(),
            run_id: None,
            status,
            cancellable: matches!(
                status,
                UploadStatus::Queued | UploadStatus::Uploading | UploadStatus::BackingOff
            ),
            progress_available: true,
            sent: 50,
            total: 100,
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
    fn formatting_and_label_helpers() {
        assert_eq!(format_bytes(-1), "0 bytes");
        assert_eq!(format_bytes(512), "512 bytes");
        assert_eq!(format_bytes(1536), "2 KB");
        assert_eq!(format_bytes(1_572_864), "1.5 MB");
        assert_eq!(format_bytes(2_147_483_648), "2.0 GB");

        assert_eq!(format_duration(-5), "0:00");
        assert_eq!(format_duration(500), "0:01");
        assert_eq!(format_duration(65_400), "1:05");

        assert_eq!(filter_for(0), HistoryFilter::All);
        assert_eq!(filter_for(1), HistoryFilter::Images);
        assert_eq!(filter_for(2), HistoryFilter::Videos);
        assert_eq!(filter_for(3), HistoryFilter::Gifs);
        assert_eq!(filter_for(4), HistoryFilter::Files);
        assert_eq!(filter_for(99), HistoryFilter::All);
        assert_eq!(filter_label_for(-1), "All");
        assert_eq!(filter_label_for(2), "Videos");
        assert_eq!(filter_label_for(99), "Files");

        assert_eq!(tag_for(CaptureKindDto::Gif, "a.gif"), "GIF");
        assert_eq!(tag_for(CaptureKindDto::Image, "a.png"), "PNG");
        assert_eq!(tag_for(CaptureKindDto::Image, "noext"), "IMAGE");
        assert_eq!(tag_for(CaptureKindDto::Video, "noext"), "VIDEO");
        assert_eq!(tag_for(CaptureKindDto::Text, "noext"), "TXT");
        assert_eq!(tag_for(CaptureKindDto::File, "archive.tar.gz"), "GZ");

        assert_eq!(type_label(CaptureKindDto::Gif, "a.gif"), "GIF animation");
        assert_eq!(type_label(CaptureKindDto::Image, "a.jpg"), "JPEG image");
        assert_eq!(type_label(CaptureKindDto::Image, "b.webp"), "WebP image");
        assert_eq!(type_label(CaptureKindDto::Image, "noext"), "Image image");
        assert_eq!(type_label(CaptureKindDto::Video, "clip.mp4"), "MP4 video");
        assert_eq!(type_label(CaptureKindDto::Text, "note.md"), "MD text");
        assert_eq!(type_label(CaptureKindDto::Text, "plain"), "Text file");
        assert_eq!(type_label(CaptureKindDto::File, "blob"), "File");

        let dest = DestinationViewDto {
            id: "custom-1".to_owned(),
            kind: snapline_services::dto::upload::DestinationKindDto::Custom,
            name: "My CDN".to_owned(),
            detail: String::new(),
            status: snapline_services::dto::upload::DestinationStatusDto::Configured,
            action: snapline_services::dto::upload::DestinationActionDto::Edit,
            configured: true,
            client_id_saved: false,
            endpoint: None,
            region: None,
            bucket: None,
            path_prefix: None,
            public_url_pattern: None,
            acl_public_read: false,
            access_key_id_saved: false,
            secret_access_key_saved: false,
            method: None,
            request_url: None,
            body: None,
            file_form_field: None,
            link: None,
            thumbnail: None,
            deletion: None,
            error_message: None,
            headers: Vec::new(),
            imported_from_sxcu: false,
            host: None,
            port: None,
            username: None,
            auth_method: None,
            private_key_path: None,
            remote_folder: None,
            password_saved: false,
            passphrase_saved: false,
            warnings: Vec::new(),
        };
        assert_eq!(host_label("imgur", &[]), "Imgur");
        assert_eq!(host_label("s3", &[]), "Amazon S3");
        assert_eq!(host_label("custom-1", &[dest]), "My CDN");
        assert_eq!(host_label("unknown-dest", &[]), "unknown-dest");
    }

    #[test]
    fn compute_selection_covers_single_additive_and_ranges() {
        let items: Vec<CaptureDto> = [10, 20, 30, 40]
            .into_iter()
            .map(|id| sample_capture(id, CaptureKindDto::Image, "shot.png", None))
            .collect();

        let (selected, anchor) = compute_selection(&items, &HashSet::new(), None, 20, false, false);
        assert_eq!(selected, HashSet::from([20]));
        assert_eq!(anchor, Some(20));

        let (selected_toggle, anchor_toggle) =
            compute_selection(&items, &selected, anchor, 20, false, false);
        assert_eq!(selected_toggle, HashSet::from([20]));
        assert_eq!(anchor_toggle, Some(20));

        let (selected_replace, anchor_replace) =
            compute_selection(&items, &HashSet::from([10, 20]), Some(10), 20, false, false);
        assert_eq!(selected_replace, HashSet::from([20]));
        assert_eq!(anchor_replace, Some(20));

        let (add_selected, add_anchor) =
            compute_selection(&items, &HashSet::from([10]), Some(10), 30, true, false);
        assert_eq!(add_selected, HashSet::from([10, 30]));
        assert_eq!(add_anchor, Some(30));

        let (rem_selected, rem_anchor) =
            compute_selection(&items, &add_selected, add_anchor, 10, true, false);
        assert_eq!(rem_selected, HashSet::from([30]));
        assert_eq!(rem_anchor, Some(10));

        let (fwd_range, fwd_anchor) =
            compute_selection(&items, &HashSet::from([10]), Some(20), 40, false, true);
        assert_eq!(fwd_range, HashSet::from([20, 30, 40]));
        assert_eq!(fwd_anchor, Some(20));

        let (bwd_range, bwd_anchor) =
            compute_selection(&items, &HashSet::new(), Some(30), 10, false, true);
        assert_eq!(bwd_range, HashSet::from([10, 20, 30]));
        assert_eq!(bwd_anchor, Some(30));

        let (add_range, _) =
            compute_selection(&items, &HashSet::from([40]), Some(10), 20, true, true);
        assert_eq!(add_range, HashSet::from([10, 20, 40]));

        let (missing_anchor, next_anchor) =
            compute_selection(&items, &HashSet::from([40]), Some(999), 20, false, true);
        assert_eq!(missing_anchor, HashSet::from([20]));
        assert_eq!(next_anchor, Some(20));
    }

    #[test]
    fn live_matches_and_share_view_prioritise_live_over_dto_and_fall_back_cleanly() {
        let dto = UploadDto {
            dest_id: "imgur".to_owned(),
            state: UploadStateDto::Queued,
            sent: 0,
            total: 100,
            url: Some("https://img.example/old.png".to_owned()),
            deletion_url: Some("https://img.example/del".to_owned()),
            error: Some("dto err".to_owned()),
            attempts: 1,
            updated_at: 10,
        };
        let capture = sample_capture(42, CaptureKindDto::Image, "shot.png", Some(dto));

        let mut by_run = sample_upload_state(7, None, UploadStatus::Done);
        by_run.run_id = Some("42".to_owned());
        assert!(live_matches_capture(&capture, None, &by_run));

        let mut by_url = sample_upload_state(8, None, UploadStatus::Done);
        by_url.link = Some("https://img.example/old.png".to_owned());
        assert!(live_matches_capture(&capture, None, &by_url));

        let by_queue = sample_upload_state(99, None, UploadStatus::Uploading);
        assert!(live_matches_capture(&capture, Some(99), &by_queue));
        assert!(!live_matches_capture(&capture, Some(5), &by_queue));

        let local = sample_capture(1, CaptureKindDto::Image, "local.png", None);
        let local_view = compute_share_view(&local, None, None);
        assert_eq!(local_view.status, "local");
        assert_eq!(local_view.label, "Not uploaded");
        assert_eq!(local_view.queue_id, None);

        let dto_view = compute_share_view(&capture, Some(55), Some(&by_queue));
        assert_eq!(dto_view.status, "uploading");
        assert_eq!(dto_view.label, "Queued");
        assert_eq!(dto_view.queue_id, Some(55));
        assert!(dto_view.cancellable);
        assert_eq!(dto_view.dest_id.as_deref(), Some("imgur"));

        let mut live = sample_upload_state(55, Some(42), UploadStatus::Uploading);
        live.dest = String::new();
        live.link = Some("https://cdn.example/new.png".to_owned());
        let live_view = compute_share_view(&capture, Some(55), Some(&live));
        assert_eq!(live_view.status, "uploading");
        assert_eq!(live_view.label, "Uploading");
        assert!(live_view.uploading_phase);
        assert!(live_view.progress_available);
        assert_eq!(live_view.sent, 50);
        assert_eq!(live_view.total, 100);
        assert_eq!(live_view.url, "https://cdn.example/new.png");
        assert_eq!(
            live_view.deletion_url.as_deref(),
            Some("https://img.example/del")
        );
        assert_eq!(live_view.dest_id.as_deref(), Some("imgur"));
        assert_eq!(live_view.queue_id, Some(55));
    }

    #[test]
    fn card_badge_for_and_upload_dto_from_state_cover_all_statuses() {
        let plain = sample_capture(1, CaptureKindDto::Image, "shot.png", None);
        assert_eq!(
            card_badge_for(&plain, None),
            ("Local", "local", false, false, -1.0)
        );

        let queued = sample_upload_state(1, Some(1), UploadStatus::Queued);
        assert_eq!(
            card_badge_for(&plain, Some(&queued)),
            ("Queued", "uploading", false, true, -1.0)
        );

        let uploading = sample_upload_state(2, Some(1), UploadStatus::Uploading);
        assert_eq!(
            card_badge_for(&plain, Some(&uploading)),
            ("Uploading", "uploading", true, true, 0.5)
        );

        let mut no_progress = sample_upload_state(3, Some(1), UploadStatus::Uploading);
        no_progress.progress_available = false;
        assert_eq!(
            card_badge_for(&plain, Some(&no_progress)),
            ("Uploading", "uploading", true, true, -1.0)
        );

        no_progress.cancellable = false;
        assert_eq!(
            card_badge_for(&plain, Some(&no_progress)),
            ("Uploading", "uploading", true, false, -1.0)
        );

        let backoff = sample_upload_state(4, Some(1), UploadStatus::BackingOff);
        assert_eq!(
            card_badge_for(&plain, Some(&backoff)),
            ("Waiting to retry", "uploading", false, true, -1.0)
        );

        let mut big = backoff.clone();
        big.sent = u64::MAX;
        big.total = u64::MAX;
        big.link = Some("https://cdn.example/a.png".to_owned());
        big.deletion_link = Some("https://cdn.example/del".to_owned());
        let Some(mapped) = upload_dto_from_state(&big) else {
            panic!("expected mapped upload dto");
        };
        assert_eq!(mapped.state, UploadStateDto::Uploading);
        assert_eq!(mapped.sent, i64::MAX);
        assert_eq!(mapped.total, i64::MAX);
        assert_eq!(mapped.url.as_deref(), Some("https://cdn.example/a.png"));
    }
}
