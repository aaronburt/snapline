use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::overlay::{State, Tool};

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::ID2D1Image;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::Win32::Graphics::Dxgi::DXGI_PRESENT;

use crate::d2d::{self, Gpu};
use crate::icons::{self, Icon, IconCache};
use crate::magnifier;
use crate::readout;
use crate::tokens::Palette;
use crate::window::{Scene, WindowState};
use crate::{AnnotationColour, StrokeSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    Tool(Tool),
    Colour(AnnotationColour),
    Stroke(StrokeSize),
    Undo,
    Redo,
    Copy,
    Save,
    Pin,
    Upload,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Button {
    pub hit: Hit,
    pub rect: Rect,
}

pub(crate) struct Toolbar {
    pub monitor: usize,
    pub rect: Rect,
    pub buttons: Vec<Button>,
    pub separators: Vec<Rect>,
}

pub(crate) struct HintPill {
    pub monitor: usize,
    pub rect: Rect,
    pub tool: Tool,
}

pub(crate) struct KeyboardPill {
    pub monitor: usize,
    pub rect: Rect,
    pub annotating: bool,
}

#[derive(Default)]
pub(crate) struct Layout {
    pub toolbar: Option<Toolbar>,
    pub hint: Option<HintPill>,
    pub keyboard: Option<KeyboardPill>,
}

pub(crate) struct ToolDef {
    pub tool: Tool,
    pub icon: Icon,
    #[allow(dead_code)]
    pub label: &'static str,
    #[allow(dead_code)]
    pub key: &'static str,
    pub hint: &'static str,
}

pub(crate) const TOOLS: [ToolDef; 11] = [
    ToolDef {
        tool: Tool::Move,
        icon: Icon::Move,
        label: "Select and move",
        key: "V",
        hint: "Select: click an annotation to move or resize it",
    },
    ToolDef {
        tool: Tool::Rect,
        icon: Icon::Rect,
        label: "Rectangle",
        key: "R",
        hint: "Rectangle: drag to draw · Shift for a square",
    },
    ToolDef {
        tool: Tool::Ellipse,
        icon: Icon::Ellipse,
        label: "Ellipse",
        key: "E",
        hint: "Ellipse: drag to draw · Shift for a circle",
    },
    ToolDef {
        tool: Tool::Arrow,
        icon: Icon::Arrow,
        label: "Arrow",
        key: "A",
        hint: "Arrow: drag from tail to tip · drag the middle to curve it",
    },
    ToolDef {
        tool: Tool::Pen,
        icon: Icon::Pen,
        label: "Pen",
        key: "P",
        hint: "Pen: draw freehand",
    },
    ToolDef {
        tool: Tool::Highlighter,
        icon: Icon::Highlighter,
        label: "Highlighter",
        key: "H",
        hint: "Highlighter: drag over text",
    },
    ToolDef {
        tool: Tool::Text,
        icon: Icon::Text,
        label: "Text",
        key: "T",
        hint: "Text: click to type a label",
    },
    ToolDef {
        tool: Tool::Step,
        icon: Icon::Step,
        label: "Step number",
        key: "N",
        hint: "Step: click to place 1 next",
    },
    ToolDef {
        tool: Tool::Blur,
        icon: Icon::Blur,
        label: "Blur or pixelate",
        key: "B",
        hint: "Blur: drag over anything private",
    },
    ToolDef {
        tool: Tool::Crop,
        icon: Icon::Crop,
        label: "Crop",
        key: "C",
        hint: "Crop: drag the selection edges",
    },
    ToolDef {
        tool: Tool::Colour,
        icon: Icon::Colour,
        label: "Pick colour from screen",
        key: "I",
        hint: "Colour picker: click to copy a pixel’s hex value",
    },
];

pub(crate) fn tool_hint(tool: Tool) -> &'static str {
    TOOLS
        .iter()
        .find(|definition| definition.tool == tool)
        .map(|definition| definition.hint)
        .unwrap_or("Select: click an annotation to move or resize it")
}

pub(crate) fn keyboard_segments(annotating: bool) -> &'static [(&'static str, &'static str)] {
    if annotating {
        &[
            ("Enter", " upload"),
            ("Ctrl+C", " copy"),
            ("Drag edges", " resize"),
            ("Right-click", " reselect"),
            ("Esc", " cancel"),
        ]
    } else {
        &[
            ("Drag", " select"),
            ("Tab", " windows or controls"),
            ("Right-click", " or Esc cancel"),
        ]
    }
}

fn scaled(value: f32, scale: f32) -> f32 {
    value * scale
}

fn push_item(buttons: &mut Vec<(Hit, f32, f32)>, x: &mut f32, hit: Hit, width: f32, gap: f32) {
    buttons.push((hit, *x, width));
    *x += width + gap;
}

fn separator(x: &mut f32, scale: f32) -> f32 {
    *x += scaled(6.0, scale);
    let position = *x;
    *x += scaled(1.0, scale);
    *x += scaled(8.0, scale);
    position
}

fn rect_f(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
    d2d::rect(x, y, width, height)
}

fn local(area: Rect, origin: Point) -> Rect {
    area.translated(Point::new(Px(-origin.x.get()), Px(-origin.y.get())))
}

fn overlap(area: Rect, bounds: Rect) -> Option<Rect> {
    area.intersection(bounds)
}

fn make_rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(
        Px(x.round() as i32),
        Px(y.round() as i32),
        Px(width.round() as i32),
        Px(height.round() as i32),
    )
}

fn item_width(hit: Hit, scale: f32) -> f32 {
    match hit {
        Hit::Tool(_) => scaled(34.0, scale),
        Hit::Colour(_) | Hit::Stroke(_) => scaled(28.0, scale),
        Hit::Upload => 0.0,
        _ => scaled(34.0, scale),
    }
}

fn place_toolbar(monitor: Rect, scale: f32, upload_label: f32) -> (Rect, Vec<Button>, Vec<Rect>) {
    let pad = scaled(5.0, scale);
    let gap = scaled(2.0, scale);
    let height = scaled(34.0, scale);
    let mut buttons: Vec<(Hit, f32, f32)> = Vec::new();
    let mut separators = Vec::new();
    let mut x = 0.0f32;
    for definition in TOOLS {
        push_item(
            &mut buttons,
            &mut x,
            Hit::Tool(definition.tool),
            item_width(Hit::Tool(definition.tool), scale),
            gap,
        );
    }
    separators.push(separator(&mut x, scale));
    for colour in AnnotationColour::ALL {
        push_item(
            &mut buttons,
            &mut x,
            Hit::Colour(colour),
            item_width(Hit::Colour(colour), scale),
            gap,
        );
    }
    separators.push(separator(&mut x, scale));
    for stroke in StrokeSize::ALL {
        push_item(
            &mut buttons,
            &mut x,
            Hit::Stroke(stroke),
            item_width(Hit::Stroke(stroke), scale),
            gap,
        );
    }
    separators.push(separator(&mut x, scale));
    push_item(&mut buttons, &mut x, Hit::Undo, scaled(34.0, scale), gap);
    push_item(&mut buttons, &mut x, Hit::Redo, scaled(34.0, scale), gap);
    separators.push(separator(&mut x, scale));
    push_item(&mut buttons, &mut x, Hit::Copy, scaled(34.0, scale), gap);
    push_item(&mut buttons, &mut x, Hit::Save, scaled(34.0, scale), gap);
    push_item(&mut buttons, &mut x, Hit::Pin, scaled(34.0, scale), gap);
    x += scaled(4.0, scale);
    let upload_width =
        scaled(12.0, scale) * 2.0 + scaled(16.0, scale) + scaled(6.0, scale) + upload_label;
    push_item(&mut buttons, &mut x, Hit::Upload, upload_width, gap);
    x += scaled(2.0, scale);
    push_item(&mut buttons, &mut x, Hit::Cancel, scaled(34.0, scale), gap);
    let inner = (x - gap).max(0.0);
    let width = inner + pad * 2.0;
    let left = (monitor.x.get() as f32 + (monitor.width.get() as f32 - width) / 2.0)
        .max(monitor.x.get() as f32)
        .min((monitor.right().get() as f32 - width).max(monitor.x.get() as f32));
    let top = monitor.y.get() as f32 + scaled(14.0, scale);
    let buttons = buttons
        .into_iter()
        .map(|(hit, offset, button_width)| Button {
            hit,
            rect: make_rect(left + pad + offset, top + pad, button_width, height),
        })
        .collect();
    let separators = separators
        .into_iter()
        .map(|offset| {
            make_rect(
                left + pad + offset,
                top + pad + (height - scaled(22.0, scale)) / 2.0,
                scaled(1.0, scale).max(1.0),
                scaled(22.0, scale),
            )
        })
        .collect();
    (
        make_rect(left, top, width, height + pad * 2.0),
        buttons,
        separators,
    )
}

fn measure_pill(gpu: &mut Gpu, scale: f32, text: &str) -> f32 {
    let Some(format) = gpu.text.format(
        &gpu.write,
        scaled(12.0, scale),
        DWRITE_FONT_WEIGHT_NORMAL,
        false,
    ) else {
        return scaled(80.0, scale);
    };
    d2d::measure(&gpu.write, &format, text)
}

fn keyboard_width(gpu: &mut Gpu, scale: f32, annotating: bool) -> f32 {
    let mut width = scaled(14.0, scale) * 2.0;
    let segments = keyboard_segments(annotating);
    for (index, (key, label)) in segments.iter().enumerate() {
        if index > 0 {
            width += scaled(16.0, scale);
        }
        width += key_width(gpu, scale, key);
        width += measure_pill(gpu, scale, label);
    }
    width
}

fn key_width(gpu: &mut Gpu, scale: f32, key: &str) -> f32 {
    let Some(format) = gpu.text.format(
        &gpu.write,
        scaled(12.0, scale),
        DWRITE_FONT_WEIGHT_BOLD,
        false,
    ) else {
        return scaled(30.0, scale);
    };
    d2d::measure(&gpu.write, &format, key)
}

pub(crate) fn layout(windows: &[WindowState], scene: &Scene, gpu: &mut Gpu) -> Layout {
    let Some(window) = windows.get(scene.pointer_monitor) else {
        return Layout::default();
    };
    let selection = scene.live_rect();
    let Some(selection) = selection else {
        let scale = window.source.scale;
        let width = keyboard_width(gpu, scale, false);
        return Layout {
            toolbar: None,
            hint: None,
            keyboard: Some(keyboard_pill(window, scene.pointer_monitor, width, false)),
        };
    };
    let monitor_index = windows
        .iter()
        .position(|candidate| candidate.source.monitor.bounds.contains(selection.center()))
        .unwrap_or(scene.pointer_monitor);
    let monitor = &windows[monitor_index].source;
    let scale = monitor.scale;
    let show_toolbar = scene.annotate
        && !matches!(
            &scene.overlay.state,
            State::Hovering { .. } | State::Dragging { .. }
        );
    let mut toolbar = None;
    let mut hint = None;
    if show_toolbar {
        let upload_label = {
            let Some(format) = gpu.text.format(
                &gpu.write,
                scaled(13.0, scale),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                false,
            ) else {
                return Layout::default();
            };
            d2d::measure(&gpu.write, &format, "Upload")
        };
        let (rect, buttons, separators) =
            place_toolbar(monitor.monitor.bounds, scale, upload_label);
        toolbar = Some(Toolbar {
            monitor: monitor_index,
            rect,
            buttons,
            separators,
        });
        let text = tool_hint(scene.tool);
        let width = measure_pill(gpu, scale, text) + scaled(24.0, scale);
        hint = Some(HintPill {
            monitor: monitor_index,
            rect: make_rect(
                monitor.monitor.bounds.x.get() as f32
                    + (monitor.monitor.bounds.width.get() as f32 - width) / 2.0,
                monitor.monitor.bounds.y.get() as f32 + scaled(66.0, scale),
                width,
                scaled(26.0, scale),
            ),
            tool: scene.tool,
        });
    }
    let width = keyboard_width(gpu, scale, show_toolbar);
    let keyboard = Some(keyboard_pill(
        &windows[monitor_index],
        monitor_index,
        width,
        show_toolbar,
    ));
    Layout {
        toolbar,
        hint,
        keyboard,
    }
}

fn keyboard_pill(
    window: &WindowState,
    monitor: usize,
    width: f32,
    annotating: bool,
) -> KeyboardPill {
    let scale = window.source.scale;
    let bounds = window.source.monitor.bounds;
    let height = scaled(32.0, scale);
    KeyboardPill {
        monitor,
        rect: make_rect(
            bounds.x.get() as f32 + (bounds.width.get() as f32 - width) / 2.0,
            bounds.bottom().get() as f32 - scaled(64.0, scale) - height,
            width,
            height,
        ),
        annotating,
    }
}

pub(crate) fn draw_window(
    gpu: &mut Gpu,
    icons: &mut IconCache,
    window: &WindowState,
    index: usize,
    scene: &Scene,
    layout: &Layout,
) {
    let palette = scene.theme.palette();
    let origin = window.source.origin;
    let width = window.source.size.width.get() as f32;
    let height = window.source.size.height.get() as f32;
    let scale = window.source.scale;
    let context = gpu.context.clone();
    let context = &context;

    // SAFETY: the swap chain target and bitmap belong to this render pass.
    unsafe {
        context.SetTarget(&window.target);
    }
    gpu.begin();
    let black = [0.0, 0.0, 0.0, 1.0];
    // SAFETY: clears the whole render target before any drawing.
    unsafe {
        let clear = d2d::colour(black);
        let _ = context.Clear(Some(&clear));
    }
    d2d::draw_bitmap(
        context,
        &window.frozen,
        rect_f(0.0, 0.0, width, height),
        rect_f(
            0.0,
            0.0,
            window.source.image.width() as f32,
            window.source.image.height() as f32,
        ),
        false,
    );

    let selection = scene.live_rect();

    if let Some(selection) = selection {
        draw_dim(context, palette, local(selection, origin), width, height);
        draw_selection(context, palette, local(selection, origin), scale);
        if scene.shown_selection().is_some() {
            draw_handles(context, palette, local(selection, origin), scale);
        }
        let pill_local = readout::size_pill_rect(gpu, scale, local(selection, origin));
        if overlap(
            pill_local,
            Rect::new(Px(0), Px(0), Px(width as i32), Px(height as i32)),
        )
        .is_some()
        {
            readout::draw_size_pill(gpu, palette, scale, pill_local, selection);
        }
    } else {
        if let Some(hovered) = scene.overlay.snap.hovered {
            let local_rect = local(hovered, origin);
            if let Some(visible) = overlap(
                local_rect,
                Rect::new(Px(0), Px(0), Px(width as i32), Px(height as i32)),
            ) {
                d2d::stroke_rect(
                    context,
                    rect_f(
                        visible.x.get() as f32 + 0.5,
                        visible.y.get() as f32 + 0.5,
                        (visible.width.get() as f32 - 1.0).max(0.0),
                        (visible.height.get() as f32 - 1.0).max(0.0),
                    ),
                    scaled(2.0, scale),
                    palette.accent,
                );
            }
        }
        if let Some(cursor) = scene.cursor {
            let cursor_local = Point::new(
                Px(cursor.x.get() - origin.x.get()),
                Px(cursor.y.get() - origin.y.get()),
            );
            if scene.full_screen_crosshair {
                if (0..width as i32).contains(&cursor_local.x.get()) {
                    d2d::fill_rect(
                        context,
                        rect_f(cursor_local.x.get() as f32, 0.0, 1.0, height),
                        palette.accent,
                    );
                }
                if (0..height as i32).contains(&cursor_local.y.get()) {
                    d2d::fill_rect(
                        context,
                        rect_f(0.0, cursor_local.y.get() as f32, width, 1.0),
                        palette.accent,
                    );
                }
            }
            if scene.magnifier.enabled && !scene.editing {
                magnifier::draw(
                    gpu,
                    window,
                    palette,
                    cursor_local,
                    scene.magnifier.zoom.factor(),
                );
            }
        }
    }

    if let Some(toolbar) = &layout.toolbar {
        if toolbar.monitor == index {
            draw_toolbar(gpu, icons, palette, scale, origin, toolbar, scene);
        }
    }
    if let Some(hint) = &layout.hint {
        if hint.monitor == index {
            let area = local(hint.rect, origin);
            readout::draw_hint_pill(gpu, palette, scale, area, tool_hint(hint.tool));
        }
    }
    if let Some(keyboard) = &layout.keyboard {
        if keyboard.monitor == index {
            let area = local(keyboard.rect, origin);
            readout::draw_keyboard_pill(gpu, palette, scale, area, keyboard.annotating);
        }
    }

    gpu.end();
    // SAFETY: the render pass is finished and the buffer can be presented.
    unsafe {
        let _ = context.SetTarget(None::<&ID2D1Image>);
    }
    // SAFETY: the swap chain belongs to the live window.
    unsafe {
        let _ = window.swap_chain.Present(1, DXGI_PRESENT(0));
    }
}

fn draw_dim(
    context: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext5,
    palette: &Palette,
    selection: Rect,
    width: f32,
    height: f32,
) {
    let left = selection.x.get() as f32;
    let top = selection.y.get() as f32;
    let right = selection.right().get() as f32;
    let bottom = selection.bottom().get() as f32;
    if top > 0.0 {
        d2d::fill_rect(
            context,
            rect_f(0.0, 0.0, width, top.min(height)),
            palette.dim,
        );
    }
    if bottom < height {
        d2d::fill_rect(
            context,
            rect_f(0.0, bottom.max(0.0), width, height - bottom.max(0.0)),
            palette.dim,
        );
    }
    let band_top = top.max(0.0);
    let band_height = (bottom.min(height) - band_top).max(0.0);
    if left > 0.0 {
        d2d::fill_rect(
            context,
            rect_f(0.0, band_top, left.min(width), band_height),
            palette.dim,
        );
    }
    if right < width {
        d2d::fill_rect(
            context,
            rect_f(
                right.max(0.0),
                band_top,
                width - right.max(0.0),
                band_height,
            ),
            palette.dim,
        );
    }
}

fn draw_selection(
    context: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext5,
    palette: &Palette,
    selection: Rect,
    scale: f32,
) {
    d2d::stroke_rect(
        context,
        rect_f(
            selection.x.get() as f32 + 0.5,
            selection.y.get() as f32 + 0.5,
            (selection.width.get() as f32 - 1.0).max(0.0),
            (selection.height.get() as f32 - 1.0).max(0.0),
        ),
        scaled(1.0, scale).max(1.0),
        palette.accent,
    );
}

fn draw_handles(
    context: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext5,
    palette: &Palette,
    selection: Rect,
    scale: f32,
) {
    let size = scaled(9.0, scale).max(9.0);
    let border = scaled(1.0, scale).max(1.0);
    for handle in selection.handles() {
        let left = handle.x.get() as f32 - size / 2.0;
        let top = handle.y.get() as f32 - size / 2.0;
        d2d::fill_rect(context, rect_f(left, top, size, size), palette.white);
        d2d::stroke_rect(
            context,
            rect_f(
                left + border / 2.0,
                top + border / 2.0,
                size - border,
                size - border,
            ),
            border,
            palette.accent,
        );
    }
}

fn draw_toolbar(
    gpu: &mut Gpu,
    icons: &mut IconCache,
    palette: &Palette,
    scale: f32,
    origin: Point,
    toolbar: &Toolbar,
    scene: &Scene,
) {
    let context = gpu.context.clone();
    let context = &context;
    let area = local(toolbar.rect, origin);
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    let radius = scaled(8.0, scale);

    draw_shadow(context, x, y, width, height, radius, scale);
    d2d::fill_round(context, rect_f(x, y, width, height), radius, palette.layer);
    d2d::stroke_round(
        context,
        rect_f(x + 0.5, y + 0.5, width - 1.0, height - 1.0),
        radius,
        scaled(1.0, scale).max(1.0),
        palette.stroke,
    );

    for separator in &toolbar.separators {
        let area = local(*separator, origin);
        d2d::fill_rect(
            context,
            rect_f(
                area.x.get() as f32,
                area.y.get() as f32,
                area.width.get() as f32,
                area.height.get() as f32,
            ),
            palette.stroke,
        );
    }

    for button in &toolbar.buttons {
        let area = local(button.rect, origin);
        let bx = area.x.get() as f32;
        let by = area.y.get() as f32;
        let bw = area.width.get() as f32;
        let bh = area.height.get() as f32;
        let cx = bx + bw / 2.0;
        let cy = by + bh / 2.0;
        match button.hit {
            Hit::Tool(tool) => {
                let active = scene.tool == tool;
                if active {
                    d2d::fill_round(
                        context,
                        rect_f(bx, by, bw, bh),
                        scaled(5.0, scale),
                        palette.accent_soft,
                    );
                }
                let definition = TOOLS.iter().find(|definition| definition.tool == tool);
                if let Some(definition) = definition {
                    icons::draw(
                        icons,
                        gpu,
                        definition.icon,
                        if active { palette.accent } else { palette.text },
                        cx,
                        cy,
                        scaled(18.0, scale),
                    );
                }
            }
            Hit::Colour(colour) => {
                let radius = scaled(9.0, scale);
                d2d::fill_circle(context, cx, cy, radius, colour.rgba(palette));
                if scene.colour == colour {
                    d2d::stroke_circle(
                        context,
                        cx,
                        cy,
                        radius + scaled(1.0, scale),
                        scaled(2.0, scale),
                        palette.layer,
                    );
                    d2d::stroke_circle(
                        context,
                        cx,
                        cy,
                        radius + scaled(3.0, scale),
                        scaled(2.0, scale),
                        palette.text,
                    );
                } else {
                    d2d::stroke_circle(
                        context,
                        cx,
                        cy,
                        radius - 0.5,
                        scaled(1.0, scale),
                        [0.0, 0.0, 0.0, 0.25],
                    );
                }
            }
            Hit::Stroke(stroke) => {
                let active = scene.stroke == stroke;
                if active {
                    d2d::fill_round(
                        context,
                        rect_f(bx, by, bw, bh),
                        scaled(5.0, scale),
                        palette.accent_soft,
                    );
                }
                let dot = stroke.dot() as f32 * 0.5 * scale;
                d2d::fill_circle(
                    context,
                    cx,
                    cy,
                    dot,
                    if active {
                        palette.accent
                    } else {
                        palette.text2
                    },
                );
            }
            Hit::Undo => {
                icons::draw(
                    icons,
                    gpu,
                    Icon::Undo,
                    palette.text,
                    cx,
                    cy,
                    scaled(18.0, scale),
                );
            }
            Hit::Redo => {
                icons::draw(
                    icons,
                    gpu,
                    Icon::Redo,
                    palette.text3,
                    cx,
                    cy,
                    scaled(18.0, scale),
                );
            }
            Hit::Copy => {
                icons::draw(
                    icons,
                    gpu,
                    Icon::Copy,
                    palette.text,
                    cx,
                    cy,
                    scaled(18.0, scale),
                );
            }
            Hit::Save => {
                icons::draw(
                    icons,
                    gpu,
                    Icon::Save,
                    palette.text,
                    cx,
                    cy,
                    scaled(18.0, scale),
                );
            }
            Hit::Pin => {
                icons::draw(
                    icons,
                    gpu,
                    Icon::Pin,
                    palette.text,
                    cx,
                    cy,
                    scaled(18.0, scale),
                );
            }
            Hit::Upload => {
                d2d::fill_round(
                    context,
                    rect_f(bx, by, bw, bh),
                    scaled(5.0, scale),
                    palette.accent,
                );
                d2d::stroke_round(
                    context,
                    rect_f(bx + 0.5, by + 0.5, bw - 1.0, bh - 1.0),
                    scaled(5.0, scale),
                    scaled(1.0, scale).max(1.0),
                    palette.accent,
                );
                let label = "Upload";
                let label_width = readout::measure_upload_label(gpu, scale);
                let content = scaled(16.0, scale) + scaled(6.0, scale) + label_width;
                let icon_x = bx + (bw - content) / 2.0 + scaled(8.0, scale);
                icons::draw(
                    icons,
                    gpu,
                    Icon::Upload,
                    palette.on_accent,
                    icon_x,
                    cy,
                    scaled(16.0, scale),
                );
                readout::draw_upload_label(
                    gpu,
                    palette,
                    scale,
                    bx + (bw - content) / 2.0 + scaled(16.0, scale) + scaled(6.0, scale),
                    by,
                    bh,
                    label,
                );
            }
            Hit::Cancel => {
                icons::draw(
                    icons,
                    gpu,
                    Icon::Cancel,
                    palette.text,
                    cx,
                    cy,
                    scaled(16.0, scale),
                );
            }
        }
    }
}

fn draw_shadow(
    context: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext5,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    scale: f32,
) {
    let spread = scaled(32.0, scale);
    let offset = scaled(12.0, scale);
    let steps = 16;
    for step in 0..steps {
        let progress = step as f32 / steps as f32;
        let expand = spread * (1.0 - progress);
        let alpha = 0.45 * 0.03;
        d2d::fill_round(
            context,
            rect_f(
                x - expand,
                y - expand + offset,
                width + expand * 2.0,
                height + expand * 2.0,
            ),
            radius + expand,
            [0.0, 0.0, 0.0, alpha],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> Rect {
        Rect::new(Px(1920), Px(0), Px(1920), Px(1080))
    }

    #[test]
    fn toolbar_has_every_control_in_order() {
        let (_, buttons, separators) = place_toolbar(monitor(), 1.0, 47.0);
        let hits: Vec<Hit> = buttons.iter().map(|button| button.hit).collect();
        assert_eq!(hits.len(), 11 + 6 + 3 + 2 + 3 + 2);
        for definition in TOOLS {
            assert!(hits.contains(&Hit::Tool(definition.tool)));
        }
        for colour in AnnotationColour::ALL {
            assert!(hits.contains(&Hit::Colour(colour)));
        }
        for stroke in StrokeSize::ALL {
            assert!(hits.contains(&Hit::Stroke(stroke)));
        }
        for hit in [
            Hit::Undo,
            Hit::Redo,
            Hit::Copy,
            Hit::Save,
            Hit::Pin,
            Hit::Upload,
            Hit::Cancel,
        ] {
            assert!(hits.contains(&hit));
        }
        assert_eq!(separators.len(), 4);
        assert_eq!(hits.first(), Some(&Hit::Tool(Tool::Move)));
        assert_eq!(hits.last(), Some(&Hit::Cancel));
    }

    #[test]
    fn toolbar_is_centred_and_inside_the_monitor() {
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            let bounds = monitor();
            let (rect, buttons, _) = place_toolbar(bounds, scale, 47.0 * scale);
            let centre = rect.x.get() as f32 + rect.width.get() as f32 / 2.0;
            let monitor_centre = bounds.x.get() as f32 + bounds.width.get() as f32 / 2.0;
            assert!((centre - monitor_centre).abs() <= 1.0);
            assert!(rect.x.get() >= bounds.x.get());
            assert!(rect.right().get() <= bounds.right().get());
            assert_eq!(rect.y.get(), bounds.y.get() + (14.0 * scale).round() as i32);
            for button in &buttons {
                assert!(rect.contains_rect(button.rect));
            }
        }
    }

    #[test]
    fn toolbar_buttons_never_overlap() {
        let (_, buttons, _) = place_toolbar(monitor(), 1.5, 70.0);
        for (index, button) in buttons.iter().enumerate() {
            for other in buttons.iter().skip(index + 1) {
                assert_eq!(button.rect.intersection(other.rect), None);
            }
        }
    }

    #[test]
    fn hint_strings_match_the_plan() {
        assert_eq!(tool_hint(Tool::Highlighter), "Highlighter: drag over text");
        assert_eq!(
            tool_hint(Tool::Colour),
            "Colour picker: click to copy a pixel’s hex value"
        );
        assert_eq!(
            tool_hint(Tool::Rect),
            "Rectangle: drag to draw · Shift for a square"
        );
        let annotating = keyboard_segments(true);
        assert_eq!(annotating.len(), 5);
        assert_eq!(annotating[0], ("Enter", " upload"));
        assert_eq!(annotating[1], ("Ctrl+C", " copy"));
        assert_eq!(annotating[2], ("Drag edges", " resize"));
        assert_eq!(annotating[3], ("Right-click", " reselect"));
        assert_eq!(annotating[4], ("Esc", " cancel"));
        let selecting = keyboard_segments(false);
        assert_eq!(selecting.len(), 3);
        assert_eq!(selecting[0], ("Drag", " select"));
        assert_eq!(selecting[1], ("Tab", " windows or controls"));
        assert_eq!(selecting[2], ("Right-click", " or Esc cancel"));
    }
}
