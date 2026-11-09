use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::overlay::Tool;

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext5;

use crate::d2d::{self, Gpu};
use crate::icons::{self, Icon, IconCache};
use crate::readout;
use crate::tokens::Palette;
use crate::{AnnotationColour, StrokeSize};

pub(crate) const TOOLBAR_HEIGHT: f32 = 34.0;
pub(crate) const TOOLBAR_PADDING: f32 = 5.0;
pub(crate) const TOOLBAR_GAP: f32 = 2.0;
pub(crate) const TOOLBAR_RADIUS: f32 = 8.0;
pub(crate) const TOOLBAR_TOP: f32 = 14.0;
pub(crate) const TOOL_SIZE: f32 = 34.0;
pub(crate) const COLOUR_SIZE: f32 = 28.0;
pub(crate) const SWATCH_SIZE: f32 = 18.0;
pub(crate) const SEPARATOR_HEIGHT: f32 = 22.0;
pub(crate) const SEPARATOR_MARGIN: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolbarAction {
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

pub(crate) const fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Move => "Select and move",
        Tool::Rect => "Rectangle",
        Tool::Ellipse => "Ellipse",
        Tool::Arrow => "Arrow",
        Tool::Pen => "Pen",
        Tool::Highlighter => "Highlighter",
        Tool::Text => "Text",
        Tool::Step => "Step number",
        Tool::Blur => "Blur or pixelate",
        Tool::Crop => "Crop",
        Tool::Colour => "Pick colour from screen",
    }
}

impl ToolbarAction {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Tool(tool) => tool_name(tool),
            Self::Colour(colour) => colour.name(),
            Self::Stroke(stroke) => stroke.name(),
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Copy => "Copy to clipboard",
            Self::Save => "Save",
            Self::Pin => "Pin to screen",
            Self::Upload => "Upload",
            Self::Cancel => "Cancel",
        }
    }

    pub(crate) fn announcement(self, state: &ToolbarState) -> String {
        match self {
            Self::Tool(tool) if state.tool == tool => format!("{}, selected", tool_name(tool)),
            Self::Colour(colour) if state.colour == colour => {
                format!("{} colour, selected", colour.name())
            }
            Self::Stroke(stroke) if state.stroke == stroke => {
                format!("{} stroke, selected", stroke.name())
            }
            Self::Undo if !state.can_undo => String::from("Undo, unavailable"),
            Self::Redo if !state.can_redo => String::from("Redo, unavailable"),
            _ => String::from(self.label()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolbarHit {
    Outside,
    Padding,
    Action(ToolbarAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Button {
    pub action: ToolbarAction,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toolbar {
    pub monitor: usize,
    pub rect: Rect,
    pub buttons: Vec<Button>,
    pub separators: Vec<Rect>,
}

impl Toolbar {
    pub(crate) fn hit(&self, point: Point) -> ToolbarHit {
        if !self.rect.contains(point) {
            return ToolbarHit::Outside;
        }
        match self
            .buttons
            .iter()
            .find(|button| button.rect.contains(point))
        {
            Some(button) => ToolbarHit::Action(button.action),
            None => ToolbarHit::Padding,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolbarState {
    pub tool: Tool,
    pub colour: AnnotationColour,
    pub stroke: StrokeSize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub focus: Option<usize>,
}

pub(crate) const TOOLS: [(Tool, Icon); 11] = [
    (Tool::Move, Icon::Move),
    (Tool::Rect, Icon::Rect),
    (Tool::Ellipse, Icon::Ellipse),
    (Tool::Arrow, Icon::Arrow),
    (Tool::Pen, Icon::Pen),
    (Tool::Highlighter, Icon::Highlighter),
    (Tool::Text, Icon::Text),
    (Tool::Step, Icon::Step),
    (Tool::Blur, Icon::Blur),
    (Tool::Crop, Icon::Crop),
    (Tool::Colour, Icon::Colour),
];

fn signed(scale: f32) -> f32 {
    scale.max(0.01)
}

pub(crate) fn next_focus(current: Option<usize>, count: usize, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current, forward) {
        (Some(index), true) => (index + 1) % count,
        (Some(index), false) => (index + count - 1) % count,
        (None, true) => 0,
        (None, false) => count - 1,
    })
}

fn item(
    buttons: &mut Vec<(ToolbarAction, f32, f32)>,
    x: &mut f32,
    action: ToolbarAction,
    width: f32,
    gap: f32,
    placed: &mut bool,
) {
    if *placed {
        *x += gap;
    }
    buttons.push((action, *x, width));
    *x += width;
    *placed = true;
}

fn separator(rects: &mut Vec<f32>, x: &mut f32, scale: f32, gap: f32, placed: &mut bool) {
    if *placed {
        *x += gap;
    }
    *x += SEPARATOR_MARGIN * scale;
    rects.push(*x);
    *x += scale.max(1.0);
    *x += SEPARATOR_MARGIN * scale;
    *placed = true;
}

pub(crate) fn layout(
    monitor: usize,
    bounds: Rect,
    scale: f32,
    upload_label: f32,
    editing: bool,
    upload_on_finish: bool,
) -> Toolbar {
    let scale = signed(scale);
    let pad = TOOLBAR_PADDING * scale;
    let gap = TOOLBAR_GAP * scale;
    let height = TOOLBAR_HEIGHT * scale;
    let mut buttons: Vec<(ToolbarAction, f32, f32)> = Vec::new();
    let mut separators: Vec<f32> = Vec::new();
    let mut x = 0.0_f32;
    let mut placed = false;
    for (tool, _) in TOOLS {
        item(
            &mut buttons,
            &mut x,
            ToolbarAction::Tool(tool),
            TOOL_SIZE * scale,
            gap,
            &mut placed,
        );
    }
    separator(&mut separators, &mut x, scale, gap, &mut placed);
    for colour in AnnotationColour::ALL {
        item(
            &mut buttons,
            &mut x,
            ToolbarAction::Colour(colour),
            COLOUR_SIZE * scale,
            gap,
            &mut placed,
        );
    }
    separator(&mut separators, &mut x, scale, gap, &mut placed);
    for stroke in StrokeSize::ALL {
        item(
            &mut buttons,
            &mut x,
            ToolbarAction::Stroke(stroke),
            COLOUR_SIZE * scale,
            gap,
            &mut placed,
        );
    }
    separator(&mut separators, &mut x, scale, gap, &mut placed);
    item(
        &mut buttons,
        &mut x,
        ToolbarAction::Undo,
        TOOL_SIZE * scale,
        gap,
        &mut placed,
    );
    item(
        &mut buttons,
        &mut x,
        ToolbarAction::Redo,
        TOOL_SIZE * scale,
        gap,
        &mut placed,
    );
    separator(&mut separators, &mut x, scale, gap, &mut placed);
    item(
        &mut buttons,
        &mut x,
        ToolbarAction::Copy,
        TOOL_SIZE * scale,
        gap,
        &mut placed,
    );
    if editing {
        item(
            &mut buttons,
            &mut x,
            ToolbarAction::Save,
            TOOL_SIZE * scale,
            gap,
            &mut placed,
        );
        item(
            &mut buttons,
            &mut x,
            ToolbarAction::Pin,
            TOOL_SIZE * scale,
            gap,
            &mut placed,
        );
    }
    x += 4.0 * scale;
    let upload_width = 24.0 * scale + 16.0 * scale + 6.0 * scale + upload_label;
    item(
        &mut buttons,
        &mut x,
        if editing || upload_on_finish {
            ToolbarAction::Upload
        } else {
            ToolbarAction::Save
        },
        upload_width,
        gap,
        &mut placed,
    );
    x += 2.0 * scale;
    item(
        &mut buttons,
        &mut x,
        ToolbarAction::Cancel,
        TOOL_SIZE * scale,
        gap,
        &mut placed,
    );
    let width = x + pad * 2.0;
    let left = (bounds.x.get() as f32 + (bounds.width.get() as f32 - width) / 2.0)
        .max(bounds.x.get() as f32)
        .min((bounds.right().get() as f32 - width).max(bounds.x.get() as f32));
    let top = bounds.y.get() as f32 + TOOLBAR_TOP * scale;
    let buttons = buttons
        .into_iter()
        .map(|(action, offset, button_width)| Button {
            action,
            rect: make_rect(left + pad + offset, top + pad, button_width, height),
        })
        .collect();
    let separators = separators
        .into_iter()
        .map(|offset| {
            make_rect(
                left + pad + offset,
                top + pad + (height - SEPARATOR_HEIGHT * scale) / 2.0,
                scale.max(1.0),
                SEPARATOR_HEIGHT * scale,
            )
        })
        .collect();
    Toolbar {
        monitor,
        rect: make_rect(left, top, width, height + pad * 2.0),
        buttons,
        separators,
    }
}

fn make_rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(
        Px(x.round() as i32),
        Px(y.round() as i32),
        Px(width.round() as i32),
        Px(height.round() as i32),
    )
}

fn rect_f(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
    d2d::rect(x, y, width, height)
}

fn shadow(context: &ID2D1DeviceContext5, area: Rect, scale: f32) {
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    let radius = TOOLBAR_RADIUS * scale;
    let spread = 32.0 * scale;
    let offset = 12.0 * scale;
    let steps = 16;
    for step in 0..steps {
        let progress = step as f32 / steps as f32;
        let expand = spread * (1.0 - progress);
        d2d::fill_round(
            context,
            rect_f(
                x - expand,
                y - expand + offset,
                width + expand * 2.0,
                height + expand * 2.0,
            ),
            radius + expand,
            [0.0, 0.0, 0.0, 0.45 * 0.03],
        );
    }
}

pub(crate) fn draw(
    gpu: &mut Gpu,
    icons: &mut IconCache,
    palette: &Palette,
    scale: f32,
    origin: Point,
    toolbar: &Toolbar,
    state: &ToolbarState,
) {
    let context = gpu.context.clone();
    let context = &context;
    let area = local(toolbar.rect, origin);
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    let radius = TOOLBAR_RADIUS * scale;

    shadow(context, area, scale);
    d2d::fill_round(context, rect_f(x, y, width, height), radius, palette.layer);
    d2d::stroke_round(
        context,
        rect_f(x + 0.5, y + 0.5, width - 1.0, height - 1.0),
        radius,
        1.0,
        palette.stroke,
    );
    for separator in &toolbar.separators {
        let local = local(*separator, origin);
        d2d::fill_rect(
            context,
            rect_f(
                local.x.get() as f32,
                local.y.get() as f32,
                local.width.get() as f32,
                local.height.get() as f32,
            ),
            palette.stroke,
        );
    }
    for (index, button) in toolbar.buttons.iter().enumerate() {
        draw_button(gpu, icons, palette, scale, origin, button, state);
        if state.focus == Some(index) {
            draw_focus(context, palette, scale, local(button.rect, origin));
        }
    }
}

fn draw_focus(context: &ID2D1DeviceContext5, palette: &Palette, scale: f32, area: Rect) {
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    let inset = scale.max(1.0);
    d2d::stroke_round(
        context,
        rect_f(
            x + inset,
            y + inset,
            width - inset * 2.0,
            height - inset * 2.0,
        ),
        5.0 * scale,
        2.0 * scale,
        palette.text,
    );
}

fn draw_button(
    gpu: &mut Gpu,
    icons: &mut IconCache,
    palette: &Palette,
    scale: f32,
    origin: Point,
    button: &Button,
    state: &ToolbarState,
) {
    let context = gpu.context.clone();
    let context = &context;
    let area = local(button.rect, origin);
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    let centre_x = x + width / 2.0;
    let centre_y = y + height / 2.0;
    match button.action {
        ToolbarAction::Tool(tool) => {
            let active = state.tool == tool;
            if active {
                d2d::fill_round(
                    context,
                    rect_f(x, y, width, height),
                    5.0 * scale,
                    palette.accent_soft,
                );
            }
            if let Some((_, icon)) = TOOLS.iter().find(|(candidate, _)| *candidate == tool) {
                icons::draw(
                    icons,
                    gpu,
                    *icon,
                    if active { palette.accent } else { palette.text },
                    centre_x,
                    centre_y,
                    18.0 * scale,
                );
            }
        }
        ToolbarAction::Colour(colour) => {
            let radius = SWATCH_SIZE * scale / 2.0;
            d2d::fill_circle(context, centre_x, centre_y, radius, colour.rgba(palette));
            if state.colour == colour {
                d2d::stroke_circle(
                    context,
                    centre_x,
                    centre_y,
                    radius + 1.0 * scale,
                    2.0 * scale,
                    palette.layer,
                );
                d2d::stroke_circle(
                    context,
                    centre_x,
                    centre_y,
                    radius + 3.0 * scale,
                    2.0 * scale,
                    palette.text,
                );
            } else {
                d2d::stroke_circle(
                    context,
                    centre_x,
                    centre_y,
                    radius - 0.5,
                    scale.max(1.0),
                    [0.0, 0.0, 0.0, 0.25],
                );
            }
        }
        ToolbarAction::Stroke(stroke) => {
            let active = state.stroke == stroke;
            if active {
                d2d::fill_round(
                    context,
                    rect_f(x, y, width, height),
                    5.0 * scale,
                    palette.accent_soft,
                );
            }
            d2d::fill_circle(
                context,
                centre_x,
                centre_y,
                stroke.dot() as f32 * 0.5 * scale,
                if active {
                    palette.accent
                } else {
                    palette.text2
                },
            );
        }
        ToolbarAction::Undo => {
            icons::draw(
                icons,
                gpu,
                Icon::Undo,
                if state.can_undo {
                    palette.text
                } else {
                    palette.text3
                },
                centre_x,
                centre_y,
                18.0 * scale,
            );
        }
        ToolbarAction::Redo => {
            icons::draw(
                icons,
                gpu,
                Icon::Redo,
                if state.can_redo {
                    palette.text
                } else {
                    palette.text3
                },
                centre_x,
                centre_y,
                18.0 * scale,
            );
        }
        ToolbarAction::Copy => {
            icons::draw(
                icons,
                gpu,
                Icon::Copy,
                palette.text,
                centre_x,
                centre_y,
                18.0 * scale,
            );
        }
        ToolbarAction::Save if width <= TOOL_SIZE * scale + 1.0 => {
            icons::draw(
                icons,
                gpu,
                Icon::Save,
                palette.text,
                centre_x,
                centre_y,
                18.0 * scale,
            );
        }
        ToolbarAction::Pin => {
            icons::draw(
                icons,
                gpu,
                Icon::Pin,
                palette.text,
                centre_x,
                centre_y,
                18.0 * scale,
            );
        }
        ToolbarAction::Save | ToolbarAction::Upload => {
            d2d::fill_round(
                context,
                rect_f(x, y, width, height),
                5.0 * scale,
                palette.accent,
            );
            let label_width = readout::measure_upload_label(gpu, scale);
            let content = 16.0 * scale + 6.0 * scale + label_width;
            let icon_x = x + (width - content) / 2.0;
            icons::draw(
                icons,
                gpu,
                if button.action == ToolbarAction::Save {
                    Icon::Save
                } else {
                    Icon::Upload
                },
                palette.on_accent,
                icon_x,
                centre_y,
                16.0 * scale,
            );
            readout::draw_upload_label(
                gpu,
                palette,
                scale,
                x + (width - content) / 2.0 + 16.0 * scale + 6.0 * scale,
                y,
                height,
                if button.action == ToolbarAction::Save {
                    "Save"
                } else {
                    "Upload"
                },
            );
        }
        ToolbarAction::Cancel => {
            icons::draw(
                icons,
                gpu,
                Icon::Cancel,
                palette.text,
                centre_x,
                centre_y,
                16.0 * scale,
            );
        }
    }
}

fn local(area: Rect, origin: Point) -> Rect {
    area.translated(Point::new(Px(-origin.x.get()), Px(-origin.y.get())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> Rect {
        Rect::new(Px(1920), Px(0), Px(2560), Px(1440))
    }

    fn actions(toolbar: &Toolbar) -> Vec<ToolbarAction> {
        toolbar.buttons.iter().map(|button| button.action).collect()
    }

    #[test]
    fn toolbar_order_matches_the_plan() {
        let toolbar = layout(1, monitor(), 1.0, 47.0, true, true);
        let actions = actions(&toolbar);
        let expected: Vec<ToolbarAction> = TOOLS
            .iter()
            .map(|(tool, _)| ToolbarAction::Tool(*tool))
            .chain(AnnotationColour::ALL.map(ToolbarAction::Colour))
            .chain(StrokeSize::ALL.map(ToolbarAction::Stroke))
            .chain([
                ToolbarAction::Undo,
                ToolbarAction::Redo,
                ToolbarAction::Copy,
                ToolbarAction::Save,
                ToolbarAction::Pin,
                ToolbarAction::Upload,
                ToolbarAction::Cancel,
            ])
            .collect();
        assert_eq!(actions, expected);
        assert_eq!(actions.len(), 11 + 6 + 3 + 2 + 3 + 2);
        assert_eq!(toolbar.separators.len(), 4);
    }

    #[test]
    fn capture_toolbar_offers_annotation_tools_and_completion() {
        let toolbar = layout(0, monitor(), 1.0, 47.0, false, false);
        let actions = actions(&toolbar);
        assert!(actions.contains(&ToolbarAction::Tool(Tool::Pen)));
        assert!(actions.contains(&ToolbarAction::Tool(Tool::Highlighter)));
        assert!(actions.contains(&ToolbarAction::Copy));
        assert!(actions.contains(&ToolbarAction::Cancel));
        assert!(actions.contains(&ToolbarAction::Save));
        assert!(!actions.contains(&ToolbarAction::Upload));
        assert!(!actions.contains(&ToolbarAction::Pin));
        let upload: Vec<_> = layout(0, monitor(), 1.0, 47.0, false, true)
            .buttons
            .iter()
            .map(|button| button.action)
            .collect();
        assert!(upload.contains(&ToolbarAction::Upload));
        assert!(!upload.contains(&ToolbarAction::Save));
    }

    #[test]
    fn toolbar_is_centred_fourteen_pixels_from_the_top() {
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            let bounds = monitor();
            let toolbar = layout(0, bounds, scale, 47.0 * scale, true, true);
            let centre = toolbar.rect.x.get() as f32 + toolbar.rect.width.get() as f32 / 2.0;
            let monitor_centre = bounds.x.get() as f32 + bounds.width.get() as f32 / 2.0;
            assert!((centre - monitor_centre).abs() <= 1.0);
            assert!(toolbar.rect.x.get() >= bounds.x.get());
            assert!(toolbar.rect.right().get() <= bounds.right().get());
            assert_eq!(
                toolbar.rect.y.get(),
                bounds.y.get() + (TOOLBAR_TOP * scale).round() as i32
            );
            for button in &toolbar.buttons {
                assert!(toolbar.rect.contains_rect(button.rect));
            }
        }
    }

    #[test]
    fn toolbar_buttons_are_the_documented_sizes() {
        let toolbar = layout(0, monitor(), 1.0, 47.0, true, true);
        for button in &toolbar.buttons {
            match button.action {
                ToolbarAction::Tool(_) => {
                    assert_eq!(button.rect.width, Px(34));
                    assert_eq!(button.rect.height, Px(34));
                }
                ToolbarAction::Colour(_) | ToolbarAction::Stroke(_) => {
                    assert_eq!(button.rect.width, Px(28));
                    assert_eq!(button.rect.height, Px(34));
                }
                _ => assert_eq!(button.rect.height, Px(34)),
            }
        }
        assert_eq!(
            toolbar.rect.height,
            Px((TOOLBAR_HEIGHT as i32) + (TOOLBAR_PADDING as i32) * 2)
        );
        for separator in &toolbar.separators {
            assert_eq!(separator.width, Px(1));
            assert_eq!(separator.height, Px(22));
        }
    }

    #[test]
    fn toolbar_buttons_never_overlap() {
        let toolbar = layout(0, monitor(), 1.5, 70.0, true, true);
        for (index, button) in toolbar.buttons.iter().enumerate() {
            for other in toolbar.buttons.iter().skip(index + 1) {
                assert_eq!(button.rect.intersection(other.rect), None);
            }
        }
    }

    #[test]
    fn hit_testing_separates_buttons_padding_and_outside() {
        let toolbar = layout(0, monitor(), 1.0, 47.0, true, true);
        let Some(first) = toolbar.buttons.first() else {
            return;
        };
        assert_eq!(
            toolbar.hit(first.rect.center()),
            ToolbarHit::Action(first.action)
        );
        assert_eq!(toolbar.hit(Point::new(Px(0), Px(0))), ToolbarHit::Outside);
        let padding = Point::new(Px(toolbar.rect.x.get() + 1), Px(toolbar.rect.y.get() + 1));
        assert_eq!(toolbar.hit(padding), ToolbarHit::Padding);
    }

    fn state() -> ToolbarState {
        ToolbarState {
            tool: Tool::Highlighter,
            colour: AnnotationColour::Yellow,
            stroke: StrokeSize::Medium,
            can_undo: false,
            can_redo: true,
            focus: None,
        }
    }

    #[test]
    fn every_toolbar_button_has_a_screen_reader_name() {
        let state = state();
        let toolbar = layout(0, monitor(), 1.0, 47.0, true, true);
        for button in &toolbar.buttons {
            assert!(!button.action.label().is_empty());
            assert!(!button.action.announcement(&state).is_empty());
        }
        assert_eq!(ToolbarAction::Tool(Tool::Move).label(), "Select and move");
        assert_eq!(
            ToolbarAction::Tool(Tool::Colour).label(),
            "Pick colour from screen"
        );
        assert_eq!(ToolbarAction::Colour(AnnotationColour::Red).label(), "Red");
        assert_eq!(ToolbarAction::Stroke(StrokeSize::Thin).label(), "Thin");
        assert_eq!(ToolbarAction::Upload.label(), "Upload");
        assert_eq!(ToolbarAction::Cancel.label(), "Cancel");
    }

    #[test]
    fn tab_order_moves_through_buttons_in_visual_order_and_wraps() {
        assert_eq!(next_focus(None, 4, true), Some(0));
        assert_eq!(next_focus(None, 4, false), Some(3));
        assert_eq!(next_focus(Some(0), 4, true), Some(1));
        assert_eq!(next_focus(Some(2), 4, true), Some(3));
        assert_eq!(next_focus(Some(3), 4, true), Some(0));
        assert_eq!(next_focus(Some(0), 4, false), Some(3));
        assert_eq!(next_focus(Some(1), 4, false), Some(0));
        assert_eq!(next_focus(Some(0), 0, true), None);
    }

    #[test]
    fn announcements_include_selection_and_unavailable_state() {
        let state = state();
        assert_eq!(
            ToolbarAction::Tool(Tool::Highlighter).announcement(&state),
            "Highlighter, selected"
        );
        assert_eq!(
            ToolbarAction::Colour(AnnotationColour::Yellow).announcement(&state),
            "Yellow colour, selected"
        );
        assert_eq!(
            ToolbarAction::Stroke(StrokeSize::Medium).announcement(&state),
            "Medium stroke, selected"
        );
        assert_eq!(
            ToolbarAction::Undo.announcement(&state),
            "Undo, unavailable"
        );
        assert_eq!(ToolbarAction::Redo.announcement(&state), "Redo");
        assert_eq!(
            ToolbarAction::Colour(AnnotationColour::Red).announcement(&state),
            "Red"
        );
    }
}
