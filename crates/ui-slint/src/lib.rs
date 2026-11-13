pub mod backend;
mod destination_form;
mod history;
pub mod host;
mod labels;
mod panel_logic;
mod quick_panel;
pub mod runner;
pub mod secrets;
mod tools;
pub mod upload_files;
mod uploads;
mod workflows;
mod workflows_page;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::{json, Value};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

pub use backend::Backend;
use backend::{patch, report_failure, spawn_bg};
pub use host::{Host, MonitorInfo, PanelMonitor, QrScanResult};
pub use panel_logic::TrayRect;
pub use runner::{OpenFailed, UiHandle};
use snapline_core::chord::{Chord, ChordError, Key, Modifiers};
use snapline_services::dto::hotkeys::HotkeysResp;
use snapline_services::dto::settings::SettingsDto;
use uploads::{DestinationItem, KEEP_HISTORY, PROXY, RETRY, TIMEOUT};
use workflows_page::{Editor, SharedEditor};

#[allow(
    clippy::all,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    unused_qualifications
)]
mod generated {
    slint::include_modules!();
}
pub use generated::SnaplineWindow;
use generated::*;

static DEV_HOOKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn dev_var(name: &str) -> Result<String, std::env::VarError> {
    if DEV_HOOKS.load(std::sync::atomic::Ordering::Relaxed) {
        std::env::var(name)
    } else {
        Err(std::env::VarError::NotPresent)
    }
}

const HOTKEY_ROWS: &[(&str, &str, &str, &str)] = &[
    (
        "capture",
        "capture_region",
        "Capture region",
        "Runs the default workflow",
    ),
    ("capture", "capture_window", "Capture window", ""),
    ("capture", "capture_screen", "Capture full screen", ""),
    ("capture", "capture_last_region", "Last region again", ""),
    ("recording", "record_video", "Record video", ""),
    ("recording", "record_gif", "Record GIF", ""),
    ("tools", "tool_ocr", "Text capture (OCR)", ""),
    ("tools", "tool_colour", "Colour picker", ""),
    ("tools", "tool_ruler", "Screen ruler", ""),
    ("tools", "upload_clipboard", "Upload from clipboard", ""),
];

const EXTERNAL_OFF: &str = "Off: another app is using this shortcut.";
const PRTSC_HINT: &str = "Sign out and back in for Windows to release the PrtSc key.";

fn model(items: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
}

fn string_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn bool_of(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
}

fn choice_label(pairs: &[(&str, impl AsRef<str>)], value: &Value, fallback: &str) -> String {
    labels::label(pairs, value.as_str().unwrap_or_default(), fallback)
}

pub(crate) fn text_matches(title: &str, description: &str, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    needle.is_empty()
        || title.to_lowercase().contains(&needle)
        || description.to_lowercase().contains(&needle)
}

pub(crate) fn hotkey_group_visible(group: &str, query: &str) -> bool {
    let heading = match group {
        "capture" => "Capture",
        "recording" => "Recording",
        "tools" => "Tools",
        _ => "",
    };
    text_matches(heading, "", query)
        || HOTKEY_ROWS
            .iter()
            .filter(|(row_group, _, _, _)| *row_group == group)
            .any(|(_, _, label, description)| text_matches(label, description, query))
}

fn key_name_from_slint(text: &str, shift: bool) -> Option<&'static str> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(match ch {
        '\u{0008}'
        | '\t'
        | '\n'
        | '\r'
        | '\u{0010}'..='\u{0019}'
        | '\u{001b}'
        | ' '
        | '\u{F72F}'
        | '\u{F734}'
        | '\u{F735}'
        | '\u{F748}' => return None,
        '\u{F731}' | '\u{F72E}' | '\u{F738}' => "PrtSc",
        '\u{F730}' => "Pause",
        '\u{007F}' => "Delete",
        '\u{F727}' => "Insert",
        '\u{F729}' => "Home",
        '\u{F72B}' => "End",
        '\u{F72C}' => "PageUp",
        '\u{F72D}' => "PageDown",
        '\u{F700}' => "Up",
        '\u{F701}' => "Down",
        '\u{F702}' => "Left",
        '\u{F703}' => "Right",
        '\u{F704}'..='\u{F71B}' => {
            const F_KEYS: [&str; 24] = [
                "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13",
                "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
            ];
            return (ch as u32)
                .checked_sub(0xF704)
                .and_then(|idx| F_KEYS.get(idx as usize).copied());
        }
        'a' | 'A' => "A",
        'b' | 'B' => "B",
        'c' | 'C' => "C",
        'd' | 'D' => "D",
        'e' | 'E' => "E",
        'f' | 'F' => "F",
        'g' | 'G' => "G",
        'h' | 'H' => "H",
        'i' | 'I' => "I",
        'j' | 'J' => "J",
        'k' | 'K' => "K",
        'l' | 'L' => "L",
        'm' | 'M' => "M",
        'n' | 'N' => "N",
        'o' | 'O' => "O",
        'p' | 'P' => "P",
        'q' | 'Q' => "Q",
        'r' | 'R' => "R",
        's' | 'S' => "S",
        't' | 'T' => "T",
        'u' | 'U' => "U",
        'v' | 'V' => "V",
        'w' | 'W' => "W",
        'x' | 'X' => "X",
        'y' | 'Y' => "Y",
        'z' | 'Z' => "Z",
        '0' => "0",
        '1' => "1",
        '2' => "2",
        '3' => "3",
        '4' => "4",
        '5' => "5",
        '6' => "6",
        '7' => "7",
        '8' => "8",
        '9' => "9",
        ')' if shift => "0",
        '!' if shift => "1",
        '@' if shift => "2",
        '#' if shift => "3",
        '$' if shift => "4",
        '%' if shift => "5",
        '^' if shift => "6",
        '&' if shift => "7",
        '*' if shift => "8",
        '(' if shift => "9",
        _ => return None,
    })
}

pub(crate) fn format_hotkey(
    text: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> HotkeyFormatResult {
    let Some(name) = key_name_from_slint(text, shift) else {
        return HotkeyFormatResult {
            status: "ignore".into(),
            value: "".into(),
        };
    };
    let Ok(key) = Key::parse(name) else {
        return HotkeyFormatResult {
            status: "ignore".into(),
            value: "".into(),
        };
    };
    let mut modifiers = Modifiers::NONE;
    if ctrl {
        modifiers.insert(Modifiers::CTRL);
    }
    if alt {
        modifiers.insert(Modifiers::ALT);
    }
    if shift {
        modifiers.insert(Modifiers::SHIFT);
    }
    if meta {
        modifiers.insert(Modifiers::WIN);
    }
    match Chord::new(modifiers, key) {
        Ok(chord) => HotkeyFormatResult {
            status: "ok".into(),
            value: chord.to_string().into(),
        },
        Err(ChordError::WindowsReserved) => HotkeyFormatResult {
            status: "error".into(),
            value: "Windows uses this shortcut.".into(),
        },
        Err(_) => HotkeyFormatResult {
            status: "ignore".into(),
            value: "".into(),
        },
    }
}

pub(crate) fn monitor_number(name: &str, index: usize) -> String {
    let lower = name.to_ascii_lowercase();
    if let Some(pos) = lower.rfind("display") {
        let rest = &name[pos + "display".len()..];
        if !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.to_string();
        }
    }
    (index + 1).to_string()
}

pub(crate) fn compute_monitor_topology(
    monitors: &[MonitorInfo],
    monitor_mode: &str,
) -> (Vec<MonitorItemData>, i32, i32, String) {
    let visible: Vec<&MonitorInfo> = monitors
        .iter()
        .filter(|m| m.width > 0 && m.height > 0)
        .collect();
    if visible.is_empty() {
        return (Vec::new(), 0, 0, "Monitor layout is unavailable.".into());
    }
    let min_x = visible.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = visible.iter().map(|m| m.y).min().unwrap_or(0);
    let max_r = visible.iter().map(|m| m.x + m.width).max().unwrap_or(0);
    let max_b = visible.iter().map(|m| m.y + m.height).max().unwrap_or(0);
    let total_w = (max_r - min_x).max(1);
    let total_h = (max_b - min_y).max(1);
    let items: Vec<MonitorItemData> = visible
        .iter()
        .enumerate()
        .map(|(index, m)| MonitorItemData {
            number: monitor_number(&m.name, index).into(),
            resolution: if m.primary {
                format!("{} × {} · primary", m.width, m.height)
            } else {
                format!("{} × {}", m.width, m.height)
            }
            .into(),
            left: (m.x - min_x) as f32 / total_w as f32,
            top: (m.y - min_y) as f32 / total_h as f32,
            width: m.width as f32 / total_w as f32,
            height: m.height as f32 / total_h as f32,
            primary: m.primary,
        })
        .collect();
    let primary_num = items
        .iter()
        .find(|m| m.primary)
        .map(|m| m.number.to_string());
    let caption = match monitor_mode {
        "All monitors" => format!("All monitors are captured as one {total_w} × {total_h} image"),
        "Primary monitor" => match primary_num {
            Some(num) => format!("Monitor {num} (primary) is captured"),
            None => "The primary monitor is captured".to_string(),
        },
        _ => "The monitor with the cursor is captured".to_string(),
    };
    (items, total_w, total_h, caption)
}

fn apply_monitors(s: &SettingsState, monitors: &[MonitorInfo], monitor_mode: &str) {
    let (items, total_w, total_h, caption) = compute_monitor_topology(monitors, monitor_mode);
    s.set_monitors(ModelRc::new(VecModel::from(items)));
    s.set_monitors_total_w(total_w);
    s.set_monitors_total_h(total_h);
    s.set_monitor_caption(caption.into());
}

fn apply_settings(s: &SettingsState, v: &Value, mics: Vec<String>, monitors: &[MonitorInfo]) {
    let general = &v["general"];
    let capture = &v["capture"];
    let hotkeys = &v["hotkeys"];

    s.set_launch_at_sign_in(bool_of(&general["launchAtSignin"]));
    s.set_start_minimised(bool_of(&general["startMinimised"]));
    s.set_tray_click_capture(bool_of(&general["trayClickCapture"]));
    s.set_notifications(bool_of(&general["notifications"]));
    let save_folder = string_of(&general["saveFolder"]);
    if s.get_save_folder().as_str() != save_folder {
        s.set_save_folder(save_folder.into());
    }
    let subfolders = labels::subfolder_pairs();
    s.set_subfolder_options(model(subfolders.iter().map(|(_, l)| l.clone()).collect()));
    s.set_subfolders(choice_label(&subfolders, &general["subfolders"], "").into());
    s.set_image_format_options(model(labels::labels(labels::IMAGE_FORMAT)));
    s.set_image_format(choice_label(labels::IMAGE_FORMAT, &general["imageFormat"], "PNG").into());
    s.set_lossy_quality(general["lossyQuality"].as_i64().unwrap_or(90) as i32);
    s.set_app_version(env!("CARGO_PKG_VERSION").into());
    s.set_zoom_options(model(labels::zoom_labels()));
    s.set_zoom(i32::from(labels::nearest_zoom(
        general["zoom"].as_i64().unwrap_or(100),
    )));
    let name_pattern = string_of(&general["namePattern"]);
    if s.get_name_pattern().as_str() != name_pattern {
        s.set_name_pattern(name_pattern.into());
    }
    s.set_copied_link_format(
        choice_label(
            labels::LINK_FORMAT,
            &general["copiedLinkFormat"],
            "Direct link",
        )
        .into(),
    );
    s.set_link_format_options(model(labels::labels(labels::LINK_FORMAT)));

    s.set_include_cursor(bool_of(&capture["includeCursor"]));
    s.set_capture_delay_options(model(labels::labels(labels::CAPTURE_DELAY)));
    s.set_capture_delay(
        choice_label(labels::CAPTURE_DELAY, &capture["captureDelay"], "Off").into(),
    );
    s.set_shutter_sound(bool_of(&capture["shutterSound"]));
    s.set_window_shadow(bool_of(&capture["keepWindowShadow"]));
    s.set_freeze_screen(bool_of(&capture["freezeScreen"]));
    s.set_magnifier(bool_of(&capture["magnifier"]));
    s.set_magnifier_zoom_options(model(labels::labels(labels::MAGNIFIER_ZOOM)));
    s.set_magnifier_zoom(
        choice_label(labels::MAGNIFIER_ZOOM, &capture["magnifierZoom"], "4×").into(),
    );
    s.set_snap_windows(bool_of(&capture["snapToWindows"]));
    s.set_show_size(bool_of(&capture["showSizeAndPosition"]));
    s.set_crosshair(bool_of(&capture["fullscreenCrosshair"]));
    s.set_monitor_options(model(labels::labels(labels::MONITOR)));
    let monitor_mode = choice_label(
        labels::MONITOR,
        &capture["fullscreenCaptureUses"],
        "Monitor under cursor",
    );
    s.set_monitor_mode(monitor_mode.clone().into());
    apply_monitors(s, monitors, &monitor_mode);
    s.set_video_format_options(model(labels::labels(labels::VIDEO_FORMAT)));
    s.set_video_format(
        choice_label(labels::VIDEO_FORMAT, &capture["videoFormat"], "MP4 · H.264").into(),
    );
    s.set_frame_rate_options(model(labels::labels(labels::FRAME_RATE)));
    s.set_frame_rate(choice_label(labels::FRAME_RATE, &capture["frameRate"], "30").into());
    s.set_quality_options(model(labels::labels(labels::QUALITY)));
    s.set_quality(
        choice_label(
            labels::QUALITY,
            &capture["videoQuality"],
            "High (recommended)",
        )
        .into(),
    );
    s.set_system_audio(bool_of(&capture["recordSystemAudio"]));
    s.set_microphone(bool_of(&capture["recordMicrophone"]));
    let mic_device = string_of(&capture["microphoneDevice"]);
    let mut mic_options = mics;
    if !mic_device.is_empty() && !mic_options.iter().any(|opt| opt == &mic_device) {
        mic_options.push(mic_device.clone());
    }
    s.set_microphone_options(model(mic_options));
    s.set_microphone_device(mic_device.into());
    s.set_recording_border(bool_of(&capture["recordingBorder"]));
    s.set_ffmpeg_path(capture["ffmpegPath"].as_str().unwrap_or_default().into());
    s.set_gif_frame_rate_options(model(labels::labels(labels::GIF_FRAME_RATE)));
    s.set_gif_frame_rate(
        choice_label(labels::GIF_FRAME_RATE, &capture["gifFrameRate"], "15").into(),
    );
    s.set_gif_width_options(model(labels::labels(labels::GIF_WIDTH)));
    s.set_gif_width(choice_label(labels::GIF_WIDTH, &capture["gifMaxWidth"], "800 px wide").into());
    s.set_optimise_palette(bool_of(&capture["gifOptimisePalette"]));

    s.set_take_over_prtsc(bool_of(&hotkeys["takeOverPrtSc"]));
    s.set_warn_clashes(bool_of(&hotkeys["warnAboutClashes"]));
}

pub(crate) fn owner_label(owner: &str, workflow_names: &HashMap<String, String>) -> String {
    if let Some((_, _, label, _)) = HOTKEY_ROWS.iter().find(|(_, key, _, _)| *key == owner) {
        return (*label).to_string();
    }
    if let Some(rest) = owner.strip_prefix("workflow:") {
        if let Some(name) = workflow_names
            .get(rest)
            .filter(|name| !name.trim().is_empty())
        {
            return name.clone();
        }
        return format!("Workflow {rest}");
    }
    owner.to_string()
}

pub(crate) fn format_owners(labels: &[String], fallback: &str) -> String {
    match labels {
        [] => fallback.to_string(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

pub(crate) fn clash_text(
    resp: &HotkeysResp,
    workflow_names: &HashMap<String, String>,
) -> HashMap<String, String> {
    use snapline_services::dto::hotkeys::BlockedReasonDto;
    let mut text = HashMap::new();
    let HotkeysResp::State { blocked, .. } = resp else {
        return text;
    };
    for entry in blocked {
        if entry.reason == BlockedReasonDto::External {
            text.insert(entry.owner.clone(), EXTERNAL_OFF.to_string());
            continue;
        }
        let others: Vec<String> = blocked
            .iter()
            .filter(|other| other.chord == entry.chord && other.owner != entry.owner)
            .map(|other| owner_label(&other.owner, workflow_names))
            .collect();
        text.insert(
            entry.owner.clone(),
            format!(
                "Off: also used by {}.",
                format_owners(&others, &entry.label)
            ),
        );
    }
    text
}

fn hotkey_rows(
    settings: &Value,
    resp: &HotkeysResp,
    workflow_names: &HashMap<String, String>,
) -> Vec<Vec<HotkeyRowData>> {
    let clashes = clash_text(resp, workflow_names);
    let bound = |owner: &str| -> String {
        if let HotkeysResp::State { bindings, .. } = resp {
            if let Some(binding) = bindings.iter().find(|binding| binding.owner == owner) {
                return binding.chord.clone();
            }
        }
        string_of(&settings["hotkeys"]["bindings"][owner])
    };
    ["capture", "recording", "tools"]
        .iter()
        .map(|group| {
            HOTKEY_ROWS
                .iter()
                .filter(|(row_group, _, _, _)| row_group == group)
                .map(|(_, owner, label, description)| HotkeyRowData {
                    owner: (*owner).into(),
                    label: (*label).into(),
                    description: (*description).into(),
                    chord: bound(owner).into(),
                    clash: clashes.get(*owner).cloned().unwrap_or_default().into(),
                })
                .collect()
        })
        .collect()
}

/// Scales the whole window by overriding its scale factor: the OS scale times the zoom.
pub(crate) fn apply_zoom(app: &SnaplineWindow) {
    use slint::platform::WindowEvent;
    use slint::winit_030::WinitWindowAccessor;
    let zoom = app.global::<SettingsState>().get_zoom() as f32 / 100.0;
    let window = app.window();
    let Some(os_scale) = window.with_winit_window(|window| window.scale_factor() as f32) else {
        return;
    };
    let wanted = os_scale * zoom;
    let size = window.size();
    if (window.scale_factor() - wanted).abs() > f32::EPSILON && size.width > 0 && size.height > 0 {
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: wanted,
        });
        window.dispatch_event(WindowEvent::Resized {
            size: size.to_logical(wanted),
        });
    }
}

pub(crate) fn reload(app: &Weak<SnaplineWindow>, backend: &Backend) -> Result<()> {
    let Some(app) = app.upgrade() else {
        return Ok(());
    };
    let settings = serde_json::to_value(backend.settings()?)?;
    let hotkeys = backend.hotkeys()?;
    let workflow_names = workflows_page::name_map(backend);
    let mics = backend.host().microphones();
    let monitors = backend.host().monitors();
    let s = app.global::<SettingsState>();
    apply_settings(&s, &settings, mics, &monitors);
    apply_zoom(&app);
    let capture_hotkey = match &hotkeys {
        HotkeysResp::State { bindings, .. } => bindings
            .iter()
            .find(|binding| binding.owner == "capture_region")
            .map(|binding| binding.chord.clone()),
        _ => None,
    }
    .unwrap_or_else(|| string_of(&settings["hotkeys"]["bindings"]["capture_region"]));
    s.set_capture_hotkey(capture_hotkey.into());
    let format = string_of(&settings["general"]["imageFormat"]);
    let format = if format.is_empty() {
        "png".to_string()
    } else {
        format
    };
    if let Ok(name) = backend.preview_name(&string_of(&settings["general"]["namePattern"]), &format)
    {
        s.set_preview_name(name.into());
    }
    let groups = hotkey_rows(&settings, &hotkeys, &workflow_names);
    let rows = |index: usize| ModelRc::new(VecModel::from(groups[index].clone()));
    s.set_capture_hotkeys(rows(0));
    s.set_recording_hotkeys(rows(1));
    s.set_tool_hotkeys(rows(2));

    let destinations: Vec<DestinationItem> = backend
        .destinations()?
        .iter()
        .map(DestinationItem::from)
        .collect();
    apply_uploads(&app.global::<UploadsState>(), &settings, &destinations);

    let upload_note = uploads::label_for(&string_of(&settings["uploads"]["images"]), &destinations);
    let (workflow, count) = workflows::load(backend)?;
    let rows = workflows::build_rows(&workflow.steps, &upload_note);
    let w = app.global::<WorkflowState>();
    w.set_default_rows(rows_model(&rows));
    w.set_workflow_count(count as i32);
    w.set_message("".into());
    workflows_page::refresh_list(&app.as_weak(), backend)?;
    tools::refresh_cards_from(&app.as_weak(), &settings, &hotkeys);
    Ok(())
}

fn rows_model(rows: &[workflows::Row]) -> ModelRc<WorkflowRowData> {
    ModelRc::new(VecModel::from(
        rows.iter()
            .map(|row| WorkflowRowData {
                id: row.id.clone().into(),
                label: row.label.clone().into(),
                checked: row.checked,
                note: row.note.clone().into(),
            })
            .collect::<Vec<_>>(),
    ))
}

fn read_rows(app: &Weak<SnaplineWindow>) -> Vec<workflows::Row> {
    let Some(app) = app.upgrade() else {
        return Vec::new();
    };
    app.global::<WorkflowState>()
        .get_default_rows()
        .iter()
        .map(|row| workflows::Row {
            id: row.id.to_string(),
            label: row.label.to_string(),
            checked: row.checked,
            note: row.note.to_string(),
        })
        .collect()
}

fn persist_default(app: &Weak<SnaplineWindow>, backend: &Backend, rows: Vec<workflows::Row>) {
    let result = (|| -> Result<()> {
        let (current, _) = workflows::load(backend)?;
        let settings = backend.settings()?;
        let steps =
            workflows::build_steps(&rows, &current.steps, settings.general.copied_link_format);
        workflows::save(backend, &current, steps)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            let _ = reload(app, backend);
        }
        Err(error) => {
            if let Some(app) = app.upgrade() {
                app.global::<WorkflowState>()
                    .set_message(format!("{error:#}").into());
            }
        }
    }
}

fn apply_uploads(u: &UploadsState, settings: &Value, destinations: &[DestinationItem]) {
    let up = &settings["uploads"];
    let targets = [
        ("images", "images", "images"),
        ("videos", "videos_and_gifs", "videos"),
        ("text", "text_and_code", "text"),
        ("other", "other_files", "other_files"),
        ("shortener", "link_shortener", "link_shortener"),
    ];
    for (target, key, _) in targets {
        let stored = string_of(&up[camel(key)]);
        let options = uploads::options(target, destinations);
        let shown = uploads::label_for(&stored, destinations);
        let labels_only: Vec<String> = options.iter().map(|(_, label)| label.clone()).collect();
        match target {
            "images" => {
                u.set_images_options(model(labels_only));
                u.set_images(shown.into());
            }
            "videos" => {
                u.set_videos_options(model(labels_only));
                u.set_videos(shown.into());
            }
            "text" => {
                u.set_text_options(model(labels_only));
                u.set_text(shown.into());
            }
            "other" => {
                u.set_other_options(model(labels_only));
                u.set_other_files(shown.into());
            }
            _ => {
                u.set_shortener_options(model(labels_only));
                u.set_link_shortener(shown.into());
            }
        }
    }
    u.set_strip_exif(bool_of(&up["stripExif"]));
    u.set_ask_clipboard(bool_of(&up["askBeforeClipboardUpload"]));
    u.set_retry_options(model(labels::labels(RETRY)));
    u.set_retry(labels::label(RETRY, up["retry"].as_str().unwrap_or_default(), "Once").into());
    u.set_timeout_options(model(labels::labels(TIMEOUT)));
    u.set_timeout(
        labels::label(TIMEOUT, up["timeout"].as_str().unwrap_or_default(), "60 s").into(),
    );
    u.set_keep_history_options(model(labels::labels(KEEP_HISTORY)));
    u.set_keep_history(
        labels::label(
            KEEP_HISTORY,
            up["keepHistory"].as_str().unwrap_or_default(),
            "Forever",
        )
        .into(),
    );
    u.set_proxy_options(model(labels::labels(PROXY)));
    u.set_proxy(
        labels::label(
            PROXY,
            up["proxy"].as_str().unwrap_or_default(),
            "Use system proxy",
        )
        .into(),
    );
    let custom_proxy = string_of(&up["proxyUrl"]);
    if u.get_custom_proxy().as_str() != custom_proxy {
        u.set_custom_proxy(custom_proxy.into());
    }

    let rows: Vec<DestinationRowData> = destinations
        .iter()
        .enumerate()
        .map(|(index, item)| DestinationRowData {
            id: item.id.clone().into(),
            name: item.name.clone().into(),
            detail: item.detail.clone().into(),
            status: item.status.clone().into(),
            configured: item.configured,
            kind: item.kind.clone().into(),
            action: item.action.clone().into(),
            index: index as i32,
        })
        .collect();
    u.set_destinations(ModelRc::new(VecModel::from(rows)));
}

fn camel(key: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for ch in key.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

const SETTINGS_DEBOUNCE: Duration = Duration::from_millis(500);

/// Text boxes and sliders report every change, so they are saved once the user pauses.
#[derive(Clone)]
struct SettingsDebounce {
    inner: Rc<DebounceInner>,
}

struct DebounceInner {
    backend: Rc<Backend>,
    app: Weak<SnaplineWindow>,
    pending: RefCell<Vec<(String, Value)>>,
    timer: slint::Timer,
}

impl SettingsDebounce {
    fn new(backend: &Rc<Backend>, app: &Weak<SnaplineWindow>) -> Self {
        Self {
            inner: Rc::new(DebounceInner {
                backend: backend.clone(),
                app: app.clone(),
                pending: RefCell::new(Vec::new()),
                timer: slint::Timer::default(),
            }),
        }
    }

    fn queue(&self, path: String, value: Value) {
        {
            let mut pending = self.inner.pending.borrow_mut();
            pending.retain(|(queued, _)| *queued != path);
            pending.push((path, value));
        }
        let weak = Rc::downgrade(&self.inner);
        self.inner
            .timer
            .start(slint::TimerMode::SingleShot, SETTINGS_DEBOUNCE, move || {
                if let Some(inner) = weak.upgrade() {
                    let queued = std::mem::take(&mut *inner.pending.borrow_mut());
                    for (path, value) in queued {
                        send(&inner.app, &inner.backend, patch(&path, value));
                    }
                }
            });
    }
}

fn send(app: &Weak<SnaplineWindow>, backend: &Backend, value: Value) {
    if let Err(error) = backend.update(value).and_then(|_| reload(app, backend)) {
        report_failure(backend, "Settings could not be saved", &error);
    }
}

fn choice_pairs(path: &str) -> Result<Vec<(&'static str, String)>> {
    let fixed = |pairs: labels::Pairs| pairs.iter().map(|(v, l)| (*v, (*l).to_string())).collect();
    Ok(match path {
        "general.subfolders" => labels::subfolder_pairs(),
        "general.image_format" => fixed(labels::IMAGE_FORMAT),
        "general.copied_link_format" => fixed(labels::LINK_FORMAT),
        "capture.capture_delay" => fixed(labels::CAPTURE_DELAY),
        "capture.magnifier_zoom" => fixed(labels::MAGNIFIER_ZOOM),
        "capture.fullscreen_capture_uses" => fixed(labels::MONITOR),
        "capture.video_format" => fixed(labels::VIDEO_FORMAT),
        "capture.frame_rate" => fixed(labels::FRAME_RATE),
        "capture.video_quality" => fixed(labels::QUALITY),
        "capture.gif_frame_rate" => fixed(labels::GIF_FRAME_RATE),
        "capture.gif_max_width" => fixed(labels::GIF_WIDTH),
        "uploads.retry" => fixed(RETRY),
        "uploads.timeout" => fixed(TIMEOUT),
        "uploads.keep_history" => fixed(KEEP_HISTORY),
        "uploads.proxy" => fixed(PROXY),
        other => bail!("no choices for {other}"),
    })
}

fn choice_value(path: &str, shown: &str) -> Result<Value> {
    let pairs = choice_pairs(path)?;
    Ok(json!(labels::value(&pairs, shown, "")))
}

pub(crate) fn update_history_count(app: &Weak<SnaplineWindow>, backend: &Backend) {
    let Some(app) = app.upgrade() else {
        return;
    };
    if let Ok(count) = backend.history_count() {
        app.set_history_count(count.to_string().into());
    }
}

pub fn create_window(backend: Rc<Backend>, dev_hooks: bool) -> Result<SnaplineWindow> {
    create_window_with_cache(backend, Arc::new(history::ThumbCache::default()), dev_hooks)
        .map(|(app, _)| app)
}

pub(crate) fn create_window_with_cache(
    backend: Rc<Backend>,
    cache: Arc<history::ThumbCache>,
    dev_hooks: bool,
) -> Result<(SnaplineWindow, history::HistoryHandle)> {
    DEV_HOOKS.store(dev_hooks, std::sync::atomic::Ordering::Relaxed);
    let app = SnaplineWindow::new()?;
    let weak = app.as_weak();
    let light = backend.host().is_light() || crate::dev_var("SNAPLINE_LIGHT").is_ok();
    app.global::<Tokens>().set_light(light);
    reload(&weak, &backend)?;
    update_history_count(&weak, &backend);

    if let Some(page) = crate::dev_var("SNAPLINE_PAGE")
        .ok()
        .and_then(|p| p.parse().ok())
    {
        app.set_page(page);
    }
    if let Some(y) = crate::dev_var("SNAPLINE_SCROLL")
        .ok()
        .and_then(|h| h.parse::<f32>().ok())
    {
        app.set_scroll_y(y as i32 as f32 * 1.0);
    }
    if crate::dev_var("SNAPLINE_ADD_MENU").is_ok() {
        app.global::<UploadsState>().set_add_open(true);
    }

    let s = app.global::<SettingsState>();

    s.on_text_matches(|title, description, query| text_matches(&title, &description, &query));
    s.on_hotkey_group_visible(|group, query| hotkey_group_visible(&group, &query));
    s.on_format_hotkey(|text, ctrl, alt, shift, meta| format_hotkey(&text, ctrl, alt, shift, meta));

    let b = backend.clone();
    s.on_run_capture(move || {
        spawn_bg(
            &b,
            move |bg| bg.run_action("capture_region"),
            move |ui_bg, result| {
                if let Err(error) = result {
                    report_failure(&ui_bg, "Capture failed", &error);
                }
            },
        );
    });

    let (b, w) = (backend.clone(), weak.clone());
    let prtsc_hint_shown = Rc::new(Cell::new(false));
    let hint = prtsc_hint_shown.clone();
    s.on_update_bool(move |path, value| {
        send(&w, &b, patch(&path, json!(value)));
        if path == "hotkeys.take_over_prtsc" {
            if let Some(app) = w.upgrade() {
                let s = app.global::<SettingsState>();
                if !value {
                    s.set_prtsc_hint("".into());
                } else if !hint.get() {
                    hint.set(true);
                    s.set_prtsc_hint(PRTSC_HINT.into());
                }
            }
        }
    });

    let pending = SettingsDebounce::new(&backend, &weak);
    let queue = pending.clone();
    s.on_update_text(move |path, value| {
        queue.queue(path.to_string(), json!(value.as_str()));
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_update_choice(move |path, shown| match choice_value(&path, &shown) {
        Ok(value) => {
            send(&w, &b, patch(&path, value));
            if path == "general.copied_link_format" {
                persist_default(&w, &b, read_rows(&w));
            }
        }
        Err(error) => tracing::warn!("{error:#}"),
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_update_zoom(move |shown| {
        if let Ok(percent) = shown.trim_end_matches('%').parse::<i64>() {
            send(
                &w,
                &b,
                patch("general.zoom", json!(labels::nearest_zoom(percent))),
            );
        }
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_zoom_step(move |delta| {
        let Some(app) = w.upgrade() else {
            return;
        };
        let current = i64::from(app.global::<SettingsState>().get_zoom());
        let next = labels::step_zoom(current, delta);
        if i64::from(next) != current {
            send(&w, &b, patch("general.zoom", json!(next)));
        }
    });

    let queue = pending.clone();
    s.on_update_number(move |path, value| {
        queue.queue(path.to_string(), json!(value));
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_update_chord(move |owner, chord| {
        send(
            &w,
            &b,
            patch(&format!("hotkeys.bindings.{owner}"), json!(chord.as_str())),
        );
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_preview_name_requested(move |pattern| {
        let format = b
            .settings()
            .map(|settings: SettingsDto| settings.general.image_format)
            .map(|format| serde_json::to_value(format).ok())
            .ok()
            .flatten()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "png".to_string());
        match b.preview_name(&pattern, &format) {
            Ok(name) => {
                if let Some(app) = w.upgrade() {
                    app.global::<SettingsState>().set_preview_name(name.into());
                }
            }
            Err(error) => tracing::warn!("the file name preview failed: {error:#}"),
        }
    });

    let (b, w) = (backend.clone(), weak.clone());
    s.on_browse_folder(move || {
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| {
                let current = bg
                    .settings()
                    .ok()
                    .map(|settings| settings.general.save_folder);
                bg.host()
                    .browse_folder(current.as_deref())
                    .map(|path| bg.update(patch("general.save_folder", json!(path))))
            },
            move |ui_bg, result| {
                if let Some(result) = result {
                    match result {
                        Ok(_) => {
                            if let Err(error) = reload(&w2, &ui_bg) {
                                tracing::error!("reload failed: {error:#}");
                            }
                        }
                        Err(error) => {
                            report_failure(&ui_bg, "The save folder could not be changed", &error);
                        }
                    }
                }
            },
        );
    });
    let (b, w) = (backend.clone(), weak.clone());
    s.on_browse_ffmpeg(move || {
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| bg.pick_ffmpeg(),
            move |ui_bg, result| {
                if let Err(error) = result.and_then(|_| reload(&w2, &ui_bg)) {
                    report_failure(&ui_bg, "The FFmpeg path could not be changed", &error);
                }
            },
        );
    });
    let (b, w) = (backend.clone(), weak.clone());
    s.on_reset_ffmpeg(move || {
        let w2 = w.clone();
        spawn_bg(
            &b,
            move |bg| bg.reset_ffmpeg(),
            move |ui_bg, result| {
                if let Err(error) = result.and_then(|_| reload(&w2, &ui_bg)) {
                    report_failure(&ui_bg, "The FFmpeg path could not be reset", &error);
                }
            },
        );
    });

    wire_uploads(&app, backend.clone());
    wire_workflows(&app, backend.clone());
    let history_handle = history::wire(&app, backend.clone(), cache);
    if let Ok(id) = crate::dev_var("SNAPLINE_EDIT_WORKFLOW") {
        app.global::<WorkflowsState>().invoke_edit(id.into());
    }
    if crate::dev_var("SNAPLINE_HISTORY_LIST").is_ok() {
        history_handle.set_grid(false);
    }
    let initial_select = crate::dev_var("SNAPLINE_SELECT")
        .ok()
        .and_then(|v| v.parse::<i64>().ok());
    history_handle.start_initial(&backend, initial_select);
    tools::wire(&app, backend.clone(), history_handle.clone());

    let w = app.global::<WorkflowState>();
    let (b, wk) = (backend.clone(), weak.clone());
    w.on_toggle(move |index| {
        let mut rows = read_rows(&wk);
        if let Some(row) = rows.get_mut(index as usize) {
            row.checked = !row.checked;
        }
        persist_default(&wk, &b, rows);
    });
    let (b, wk) = (backend.clone(), weak.clone());
    w.on_move(move |index, direction| {
        let mut rows = read_rows(&wk);
        let target = index + direction;
        if index >= 0 && target >= 0 && (target as usize) < rows.len() {
            rows.swap(index as usize, target as usize);
            persist_default(&wk, &b, rows);
        }
    });
    let wk = weak.clone();
    w.on_open_workflows(move || {
        if let Some(app) = wk.upgrade() {
            app.set_page(4);
        }
    });

    Ok((app, history_handle))
}

fn current_destinations(backend: &Backend) -> Vec<DestinationItem> {
    backend
        .destinations()
        .map(|list| list.iter().map(DestinationItem::from).collect())
        .unwrap_or_default()
}

fn wire_uploads(app: &SnaplineWindow, backend: Rc<Backend>) {
    let u = app.global::<UploadsState>();
    let weak = app.as_weak();

    let (b, w) = (backend.clone(), weak.clone());
    u.on_update_choice(move |path, shown| {
        let value = if let Some(target) = path.strip_prefix("uploads.") {
            let options_for = |target: &str| {
                let destinations = current_destinations(&b);
                let options = uploads::options(target, &destinations);
                (options.clone(), uploads::value_for(&shown, &options))
            };
            let (_, value) = match target {
                "images" => options_for("images"),
                "videos_and_gifs" => options_for("videos"),
                "text_and_code" => options_for("text"),
                "other_files" => options_for("other"),
                "link_shortener" => options_for("shortener"),
                _ => (
                    Vec::new(),
                    labels::value(&choice_pairs(&path).unwrap_or_default(), &shown, ""),
                ),
            };
            json!(value)
        } else {
            json!(shown.as_str())
        };
        let key = path
            .strip_prefix("uploads.")
            .map(str::to_string)
            .unwrap_or(path.to_string());
        let full = format!("uploads.{key}");
        send(&w, &b, patch(&full, value));
    });

    let (b, w) = (backend.clone(), weak.clone());
    u.on_update_bool(move |path, value| {
        send(&w, &b, patch(&path, json!(value)));
    });

    let (b, w) = (backend.clone(), weak.clone());
    u.on_update_text(move |path, value| {
        let payload = if path == "uploads.proxy_url" && value.trim().is_empty() {
            Value::Null
        } else {
            json!(value.as_str())
        };
        send(&w, &b, patch(&path, payload));
    });

    let w = weak.clone();
    u.on_accounts_visible(move |query| {
        if text_matches("Accounts and uploaders", "", &query)
            || text_matches("Add destination", "", &query)
        {
            return true;
        }
        let Some(app) = w.upgrade() else {
            return true;
        };
        app.global::<UploadsState>()
            .get_destinations()
            .iter()
            .any(|dest| {
                text_matches(&dest.name, "", &query)
                    || text_matches(&dest.detail, "", &query)
                    || text_matches(&dest.status, "", &query)
            })
    });

    destination_form::wire(app, backend);
}

fn wire_workflows(app: &SnaplineWindow, backend: Rc<Backend>) {
    let ws = app.global::<WorkflowsState>();
    let weak = app.as_weak();
    let editor: SharedEditor = Rc::new(RefCell::new(Editor::default()));

    let report_push = |weak: &Weak<SnaplineWindow>, backend: &Backend, editor: &Editor| {
        if let Err(error) = workflows_page::push_editor(weak, backend, editor) {
            if let Some(app) = weak.upgrade() {
                app.global::<WorkflowsState>()
                    .set_message(format!("{error:#}").into());
            }
        }
    };

    let clear_editor_banner = |weak: &Weak<SnaplineWindow>| {
        if let Some(app) = weak.upgrade() {
            let s = app.global::<WorkflowsState>();
            s.set_confirm_delete(false);
            s.set_message("".into());
        }
    };

    let (b, w) = (backend.clone(), weak.clone());
    ws.on_search_changed(move |query| {
        if let Some(app) = w.upgrade() {
            app.global::<WorkflowsState>().set_query(query);
        }
        if let Err(error) = workflows_page::refresh_list(&w, &b) {
            if let Some(app) = w.upgrade() {
                app.global::<WorkflowsState>()
                    .set_message(format!("{error:#}").into());
            }
        }
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_new_workflow(move || {
        clear_editor_banner(&w);
        *e.borrow_mut() = workflows_page::open_new(&b);
        report_push(&w, &b, &e.borrow());
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_edit(move |id| {
        clear_editor_banner(&w);
        match workflows_page::open_existing(&b, &id) {
            Ok(loaded) => {
                *e.borrow_mut() = loaded;
                report_push(&w, &b, &e.borrow());
            }
            Err(error) => {
                if let Some(app) = w.upgrade() {
                    app.global::<WorkflowsState>()
                        .set_message(format!("{error:#}").into());
                }
            }
        }
    });

    let (b, w) = (backend.clone(), weak.clone());
    ws.on_toggle(move |id, enabled| {
        if let Err(error) = workflows_page::set_enabled(&b, &id, enabled) {
            if let Some(app) = w.upgrade() {
                app.global::<WorkflowsState>()
                    .set_message(format!("{error:#}").into());
            }
        } else if let Some(app) = w.upgrade() {
            app.global::<WorkflowsState>().set_message("".into());
        }
        let _ = reload(&w, &b);
    });

    let (b, w) = (backend.clone(), weak.clone());
    ws.on_back(move || {
        if let Some(app) = w.upgrade() {
            let s = app.global::<WorkflowsState>();
            s.set_editing(false);
            s.set_confirm_delete(false);
            s.set_message("".into());
        }
        let _ = reload(&w, &b);
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_save(move || {
        let current = e.borrow().clone();
        let issues = workflows_page::violations_for(&b, &current);
        if let Some(message) = issues.first() {
            if let Some(app) = w.upgrade() {
                app.global::<WorkflowsState>()
                    .set_message(message.clone().into());
            }
            return;
        }
        match workflows_page::save(&b, &current) {
            Ok(()) => {
                if let Some(app) = w.upgrade() {
                    let s = app.global::<WorkflowsState>();
                    s.set_message("".into());
                    s.set_editing(false);
                    s.set_confirm_delete(false);
                }
                let _ = reload(&w, &b);
            }
            Err(error) => {
                if let Some(app) = w.upgrade() {
                    app.global::<WorkflowsState>()
                        .set_message(format!("{error:#}").into());
                }
            }
        }
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_duplicate(move || {
        clear_editor_banner(&w);
        let id = e.borrow().id.clone();
        if let Some(id) = id {
            match workflows_page::duplicate(&b, &id)
                .and_then(|copy| workflows_page::open_existing(&b, &copy))
            {
                Ok(copy) => {
                    *e.borrow_mut() = copy;
                    report_push(&w, &b, &e.borrow());
                    let _ = reload(&w, &b);
                }
                Err(error) => {
                    if let Some(app) = w.upgrade() {
                        app.global::<WorkflowsState>()
                            .set_message(format!("{error:#}").into());
                    }
                }
            }
        }
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_delete(move || {
        let id = e.borrow().id.clone();
        if let Some(id) = id {
            if let Err(error) = workflows_page::delete(&b, &id) {
                if let Some(app) = w.upgrade() {
                    app.global::<WorkflowsState>()
                        .set_message(format!("{error:#}").into());
                }
                return;
            }
        }
        if let Some(app) = w.upgrade() {
            let s = app.global::<WorkflowsState>();
            s.set_editing(false);
            s.set_confirm_delete(false);
            s.set_message("".into());
        }
        let _ = reload(&w, &b);
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_name_edited(move |name| {
        e.borrow_mut().name = name.to_string();
        report_push(&w, &b, &e.borrow());
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_hotkey_committed(move |chord| {
        let clash = {
            let mut current = e.borrow_mut();
            current.hotkey = chord.to_string();
            workflows_page::hotkey_clash(&b, &current, &chord)
        };
        if let Some(app) = w.upgrade() {
            let s = app.global::<WorkflowsState>();
            s.set_editor_hotkey(chord);
            s.set_editor_clash(clash.into());
        }
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_enabled_changed(move |enabled| {
        e.borrow_mut().enabled = enabled;
        report_push(&w, &b, &e.borrow());
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_add_step(move |label| {
        workflows_page::add_step(&e, &label);
        report_push(&w, &b, &e.borrow());
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_remove_step(move |index| {
        {
            let mut current = e.borrow_mut();
            if index >= 0 && (index as usize) < current.steps.len() {
                current.steps.remove(index as usize);
            }
        }
        report_push(&w, &b, &e.borrow());
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_move_step(move |index, direction| {
        {
            let mut current = e.borrow_mut();
            let target = index + direction;
            if index >= 0 && (index as usize) < current.steps.len() {
                let target = target.clamp(0, current.steps.len() as i32 - 1);
                let step = current.steps.remove(index as usize);
                current.steps.insert(target as usize, step);
            }
        }
        report_push(&w, &b, &e.borrow());
    });

    let (b, w, e) = (backend.clone(), weak.clone(), editor.clone());
    ws.on_step_choice(move |index, label| {
        if index >= 0 {
            workflows_page::step_choice_apply(&e, index as usize, &label);
        }
        report_push(&w, &b, &e.borrow());
    });
}

#[cfg(test)]
mod tests {
    use snapline_services::dto::hotkeys::{BindingDto, BlockedDto, BlockedReasonDto};

    use super::*;

    #[test]
    fn format_hotkey_handles_chords_shifted_digits_ignored_keys_and_windows_reserved() {
        let chord = format_hotkey("c", true, false, true, false);
        assert_eq!(chord.status.as_str(), "ok");
        assert_eq!(chord.value.as_str(), "Ctrl+Shift+C");

        let shifted = format_hotkey("!", false, false, true, false);
        assert_eq!(shifted.status.as_str(), "ok");
        assert_eq!(shifted.value.as_str(), "Shift+1");

        let fn_key = format_hotkey("\u{F70F}", true, false, false, false);
        assert_eq!(fn_key.status.as_str(), "ok");
        assert_eq!(fn_key.value.as_str(), "Ctrl+F12");

        let ignored_space = format_hotkey(" ", true, false, false, false);
        assert_eq!(ignored_space.status.as_str(), "ignore");

        let ignored_multi = format_hotkey("ab", true, false, false, false);
        assert_eq!(ignored_multi.status.as_str(), "ignore");

        let reserved = format_hotkey("l", false, false, false, true);
        assert_eq!(reserved.status.as_str(), "error");
        assert_eq!(reserved.value.as_str(), "Windows uses this shortcut.");
    }

    #[test]
    fn monitor_number_and_topology_compute_bounds_and_captions() {
        assert_eq!(monitor_number(r"\\.\DISPLAY2", 0), "2");
        assert_eq!(monitor_number("Primary", 1), "2");

        let empty = compute_monitor_topology(&[], "All monitors");
        assert!(empty.0.is_empty());
        assert_eq!((empty.1, empty.2), (0, 0));
        assert_eq!(empty.3, "Monitor layout is unavailable.");

        let monitors = vec![
            MonitorInfo {
                name: r"\\.\DISPLAY2".to_owned(),
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
                primary: false,
            },
            MonitorInfo {
                name: r"\\.\DISPLAY1".to_owned(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                primary: true,
            },
        ];

        let (items, total_w, total_h, all_caption) =
            compute_monitor_topology(&monitors, "All monitors");
        assert_eq!((total_w, total_h), (4480, 1440));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].number.as_str(), "2");
        assert_eq!(items[0].resolution.as_str(), "1920 × 1080");
        assert_eq!(items[1].number.as_str(), "1");
        assert_eq!(items[1].resolution.as_str(), "2560 × 1440 · primary");
        assert!((items[0].left - 0.0).abs() < 1e-5);
        assert!((items[1].left - (1920.0 / 4480.0)).abs() < 1e-5);
        assert_eq!(
            all_caption,
            "All monitors are captured as one 4480 × 1440 image"
        );

        let (_, _, _, primary_caption) = compute_monitor_topology(&monitors, "Primary monitor");
        assert_eq!(primary_caption, "Monitor 1 (primary) is captured");

        let (_, _, _, cursor_caption) = compute_monitor_topology(&monitors, "Monitor under cursor");
        assert_eq!(cursor_caption, "The monitor with the cursor is captured");
    }

    #[test]
    fn owner_label_format_owners_and_clash_text_work_together() {
        let names = HashMap::from([
            ("wf1".to_owned(), "Quick Share".to_owned()),
            ("blank".to_owned(), "   ".to_owned()),
        ]);
        assert_eq!(owner_label("tool_colour", &names), "Colour picker");
        assert_eq!(owner_label("workflow:wf1", &names), "Quick Share");
        assert_eq!(owner_label("workflow:blank", &names), "Workflow blank");
        assert_eq!(owner_label("workflow:wf2", &names), "Workflow wf2");
        assert_eq!(owner_label("other", &names), "other");

        assert_eq!(format_owners(&[], "fallback"), "fallback");
        assert_eq!(format_owners(&["A".to_owned()], "fallback"), "A");
        assert_eq!(
            format_owners(&["A".to_owned(), "B".to_owned()], "fallback"),
            "A and B"
        );
        assert_eq!(
            format_owners(
                &["A".to_owned(), "B".to_owned(), "C".to_owned()],
                "fallback"
            ),
            "A, B and C"
        );

        let resp = HotkeysResp::State {
            bindings: vec![BindingDto {
                owner: "capture_region".to_owned(),
                label: "Capture region".to_owned(),
                chord: "PrtSc".to_owned(),
            }],
            blocked: vec![
                BlockedDto {
                    owner: "capture_window".to_owned(),
                    label: "Capture window".to_owned(),
                    chord: "Ctrl+Shift+A".to_owned(),
                    reason: BlockedReasonDto::External,
                },
                BlockedDto {
                    owner: "tool_colour".to_owned(),
                    label: "Colour picker".to_owned(),
                    chord: "Ctrl+Shift+C".to_owned(),
                    reason: BlockedReasonDto::Internal,
                },
                BlockedDto {
                    owner: "workflow:wf1".to_owned(),
                    label: "Quick Share".to_owned(),
                    chord: "Ctrl+Shift+C".to_owned(),
                    reason: BlockedReasonDto::Internal,
                },
            ],
            paused: false,
        };
        let clashes = clash_text(&resp, &names);
        assert_eq!(
            clashes.get("capture_window").map(String::as_str),
            Some(EXTERNAL_OFF)
        );
        assert_eq!(
            clashes.get("tool_colour").map(String::as_str),
            Some("Off: also used by Quick Share.")
        );
        assert_eq!(
            clashes.get("workflow:wf1").map(String::as_str),
            Some("Off: also used by Colour picker.")
        );
    }

    #[test]
    fn text_matches_and_hotkey_group_visible_filter_case_insensitively() {
        assert!(text_matches("Capture region", "Runs default", ""));
        assert!(text_matches("Capture region", "Runs default", "  DEFAULT "));
        assert!(!text_matches("Capture region", "Runs default", "upload"));

        assert!(hotkey_group_visible("capture", "default workflow"));
        assert!(hotkey_group_visible("tools", "colour"));
        assert!(hotkey_group_visible("recording", "RECORD"));
        assert!(!hotkey_group_visible("tools", "nonexistent-xyz"));
    }
}
