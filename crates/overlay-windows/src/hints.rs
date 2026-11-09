use snapline_core::geometry::{Px, Rect};
use snapline_core::overlay::Tool;

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
};

use crate::d2d::{self, Gpu};
use crate::tokens::Palette;

pub(crate) const HINT_TOP: f32 = 66.0;
pub(crate) const HINT_HEIGHT: f32 = 26.0;
pub(crate) const HINT_PADDING: f32 = 12.0;
pub(crate) const KEYBOARD_BOTTOM: f32 = 64.0;
pub(crate) const KEYBOARD_HEIGHT: f32 = 32.0;
pub(crate) const KEYBOARD_GAP: f32 = 16.0;
pub(crate) const KEYBOARD_PADDING: f32 = 14.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HintPill {
    pub monitor: usize,
    pub rect: Rect,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyboardPill {
    pub monitor: usize,
    pub rect: Rect,
    pub annotating: bool,
}

pub(crate) fn hint_text(tool: Tool, next_step: u32) -> String {
    match tool {
        Tool::Move => "Select: click an annotation to move or resize it".to_string(),
        Tool::Rect => "Rectangle: drag to draw · Shift for a square".to_string(),
        Tool::Ellipse => "Ellipse: drag to draw · Shift for a circle".to_string(),
        Tool::Arrow => {
            "Arrow: drag from tail to tip · drag the middle to curve it".to_string()
        }
        Tool::Pen => "Pen: draw freehand".to_string(),
        Tool::Highlighter => "Highlighter: drag over text".to_string(),
        Tool::Text => "Text: click to type a label".to_string(),
        Tool::Step => format!("Step: click to place {next_step} next"),
        Tool::Blur => "Blur: drag over anything private".to_string(),
        Tool::Crop => "Crop: drag the selection edges".to_string(),
        Tool::Colour => "Colour picker: click to copy a pixel’s hex value".to_string(),
    }
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

fn text_width(gpu: &mut Gpu, size: f32, weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT, text: &str) -> f32 {
    let Some(format) = gpu.text.format(&gpu.write, size, weight, false) else {
        return 0.0;
    };
    d2d::measure(&gpu.write, &format, text)
}

pub(crate) fn hint_width(gpu: &mut Gpu, scale: f32, text: &str) -> f32 {
    text_width(gpu, 12.0 * scale, DWRITE_FONT_WEIGHT_NORMAL, text) + HINT_PADDING * 2.0 * scale
}

pub(crate) fn keyboard_width(gpu: &mut Gpu, scale: f32, annotating: bool) -> f32 {
    let segments = keyboard_segments(annotating);
    let mut width = KEYBOARD_PADDING * 2.0 * scale;
    for (index, (key, label)) in segments.iter().enumerate() {
        if index > 0 {
            width += KEYBOARD_GAP * scale;
        }
        width += text_width(gpu, 12.0 * scale, DWRITE_FONT_WEIGHT_BOLD, key);
        width += text_width(gpu, 12.0 * scale, DWRITE_FONT_WEIGHT_NORMAL, label);
    }
    width
}

pub(crate) fn hint_pill(monitor: Rect, scale: f32, width: f32) -> Rect {
    Rect::new(
        Px((monitor.x.get() as f32 + (monitor.width.get() as f32 - width) / 2.0).round() as i32),
        Px((monitor.y.get() as f32 + HINT_TOP * scale).round() as i32),
        Px(width.round() as i32),
        Px((HINT_HEIGHT * scale).round() as i32),
    )
}

pub(crate) fn keyboard_pill(monitor: Rect, scale: f32, width: f32) -> Rect {
    let height = KEYBOARD_HEIGHT * scale;
    Rect::new(
        Px((monitor.x.get() as f32 + (monitor.width.get() as f32 - width) / 2.0).round() as i32),
        Px((monitor.bottom().get() as f32 - KEYBOARD_BOTTOM * scale - height).round() as i32),
        Px(width.round() as i32),
        Px(height.round() as i32),
    )
}

pub(crate) fn draw_hint(
    gpu: &mut Gpu,
    palette: &Palette,
    scale: f32,
    area: Rect,
    text: &str,
) {
    crate::readout::draw_hint_pill(gpu, palette, scale, area, text);
}

pub(crate) fn draw_keyboard(
    gpu: &mut Gpu,
    palette: &Palette,
    scale: f32,
    area: Rect,
    annotating: bool,
) {
    crate::readout::draw_keyboard_pill(gpu, palette, scale, area, annotating);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_hints_match_the_plan() {
        assert_eq!(
            hint_text(Tool::Move, 1),
            "Select: click an annotation to move or resize it"
        );
        assert_eq!(
            hint_text(Tool::Rect, 1),
            "Rectangle: drag to draw · Shift for a square"
        );
        assert_eq!(
            hint_text(Tool::Ellipse, 1),
            "Ellipse: drag to draw · Shift for a circle"
        );
        assert_eq!(
            hint_text(Tool::Arrow, 1),
            "Arrow: drag from tail to tip · drag the middle to curve it"
        );
        assert_eq!(hint_text(Tool::Pen, 1), "Pen: draw freehand");
        assert_eq!(
            hint_text(Tool::Highlighter, 1),
            "Highlighter: drag over text"
        );
        assert_eq!(hint_text(Tool::Text, 1), "Text: click to type a label");
        assert_eq!(hint_text(Tool::Step, 4), "Step: click to place 4 next");
        assert_eq!(
            hint_text(Tool::Blur, 1),
            "Blur: drag over anything private"
        );
        assert_eq!(hint_text(Tool::Crop, 1), "Crop: drag the selection edges");
        assert_eq!(
            hint_text(Tool::Colour, 1),
            "Colour picker: click to copy a pixel’s hex value"
        );
    }

    #[test]
    fn keyboard_hints_match_the_plan() {
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

    #[test]
    fn pills_use_the_documented_geometry() {
        let monitor = Rect::new(Px(0), Px(0), Px(1920), Px(1080));
        let hint = hint_pill(monitor, 1.0, 300.0);
        assert_eq!(hint.y, Px(66));
        assert_eq!(hint.height, Px(26));
        assert_eq!(hint.x, Px(810));
        assert_eq!(hint.width, Px(300));
        let keyboard = keyboard_pill(monitor, 1.0, 400.0);
        assert_eq!(keyboard.height, Px(32));
        assert_eq!(keyboard.bottom(), Px(1080 - 64));
        assert_eq!(keyboard.x, Px(760));
    }
}
