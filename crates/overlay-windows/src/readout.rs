use snapline_core::geometry::{Px, Rect};

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD,
};

use crate::d2d::{self, Gpu};
use crate::hints::keyboard_segments;
use crate::tokens::Palette;

fn text_width(gpu: &mut Gpu, size: f32, weight: DWRITE_FONT_WEIGHT, mono: bool, text: &str) -> f32 {
    let Some(format) = gpu.text.format(&gpu.write, size, weight, mono) else {
        return 0.0;
    };
    d2d::measure(&gpu.write, &format, text)
}

pub(crate) fn size_pill_rect(gpu: &mut Gpu, scale: f32, selection: Rect) -> Rect {
    let size_text = format!("{} × {}", selection.width.get(), selection.height.get());
    let coordinate_text = format!("X {} · Y {}", selection.x.get(), selection.y.get());
    let size = 12.0 * scale;
    let pad = 8.0 * scale;
    let gap = 8.0 * scale;
    let width = pad * 2.0
        + text_width(gpu, size, DWRITE_FONT_WEIGHT_SEMI_BOLD, true, &size_text)
        + gap
        + text_width(gpu, size, DWRITE_FONT_WEIGHT_NORMAL, true, &coordinate_text);
    let height = 24.0 * scale;
    let above = selection.y.get() as f32 - 8.0 * scale - height;
    let y = if above < 0.0 {
        selection.y.get() as f32 + 8.0 * scale
    } else {
        above
    };
    Rect::new(
        Px(selection.x.get()),
        Px(y.round() as i32),
        Px(width.round() as i32),
        Px(height.round() as i32),
    )
}

pub(crate) fn draw_size_pill(
    gpu: &mut Gpu,
    palette: &Palette,
    scale: f32,
    area: Rect,
    selection: Rect,
) {
    let size_text = format!("{} × {}", selection.width.get(), selection.height.get());
    let coordinate_text = format!("X {} · Y {}", selection.x.get(), selection.y.get());
    let size = 12.0 * scale;
    let pad = 8.0 * scale;
    let gap = 8.0 * scale;
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    d2d::fill_round(
        &gpu.context,
        d2d::rect(x, y, width, height),
        4.0 * scale,
        palette.layer,
    );
    d2d::stroke_round(
        &gpu.context,
        d2d::rect(x + 0.5, y + 0.5, width - 1.0, height - 1.0),
        4.0 * scale,
        1.0,
        palette.stroke,
    );
    let size_width = text_width(gpu, size, DWRITE_FONT_WEIGHT_SEMI_BOLD, true, &size_text);
    if let Some(format) = gpu
        .text
        .format(&gpu.write, size, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)
    {
        d2d::draw_text(
            &gpu.context,
            &format,
            &size_text,
            d2d::rect(x + pad, y, size_width, height),
            palette.text,
        );
    }
    let coordinate_width = text_width(gpu, size, DWRITE_FONT_WEIGHT_NORMAL, true, &coordinate_text);
    if let Some(format) = gpu
        .text
        .format(&gpu.write, size, DWRITE_FONT_WEIGHT_NORMAL, true)
    {
        d2d::draw_text(
            &gpu.context,
            &format,
            &coordinate_text,
            d2d::rect(x + pad + size_width + gap, y, coordinate_width, height),
            palette.text3,
        );
    }
}

pub(crate) fn draw_hint_pill(gpu: &mut Gpu, palette: &Palette, scale: f32, area: Rect, text: &str) {
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    d2d::fill_round(
        &gpu.context,
        d2d::rect(x, y, width, height),
        13.0 * scale,
        palette.layer,
    );
    d2d::stroke_round(
        &gpu.context,
        d2d::rect(x + 0.5, y + 0.5, width - 1.0, height - 1.0),
        13.0 * scale,
        1.0,
        palette.stroke,
    );
    let size = 12.0 * scale;
    let Some(format) = gpu
        .text
        .format(&gpu.write, size, DWRITE_FONT_WEIGHT_NORMAL, false)
    else {
        return;
    };
    let measured = d2d::measure(&gpu.write, &format, text);
    d2d::draw_text(
        &gpu.context,
        &format,
        text,
        d2d::rect(x + (width - measured) / 2.0, y, measured, height),
        palette.text2,
    );
}

pub(crate) fn draw_keyboard_pill(
    gpu: &mut Gpu,
    palette: &Palette,
    scale: f32,
    area: Rect,
    annotating: bool,
) {
    let x = area.x.get() as f32;
    let y = area.y.get() as f32;
    let width = area.width.get() as f32;
    let height = area.height.get() as f32;
    d2d::fill_round(
        &gpu.context,
        d2d::rect(x, y, width, height),
        16.0 * scale,
        palette.hint_pill,
    );
    let size = 12.0 * scale;
    let gap = 16.0 * scale;
    let pad = 14.0 * scale;
    let segments = keyboard_segments(annotating);
    let mut total = 0.0;
    for (index, (key, label)) in segments.iter().enumerate() {
        if index > 0 {
            total += gap;
        }
        total += text_width(gpu, size, DWRITE_FONT_WEIGHT_BOLD, false, key);
        total += text_width(gpu, size, DWRITE_FONT_WEIGHT_NORMAL, false, label);
    }
    let mut cursor = x + ((width - total) / 2.0).max(0.0);
    if cursor < x + pad {
        cursor = x + pad;
    }
    for (key, label) in segments {
        let key_width = text_width(gpu, size, DWRITE_FONT_WEIGHT_BOLD, false, key);
        let label_width = text_width(gpu, size, DWRITE_FONT_WEIGHT_NORMAL, false, label);
        if let Some(format) = gpu
            .text
            .format(&gpu.write, size, DWRITE_FONT_WEIGHT_BOLD, false)
        {
            d2d::draw_text(
                &gpu.context,
                &format,
                key,
                d2d::rect(cursor, y, key_width, height),
                palette.white,
            );
        }
        cursor += key_width;
        if let Some(format) = gpu
            .text
            .format(&gpu.write, size, DWRITE_FONT_WEIGHT_NORMAL, false)
        {
            d2d::draw_text(
                &gpu.context,
                &format,
                label,
                d2d::rect(cursor, y, label_width, height),
                palette.hint_text,
            );
        }
        cursor += label_width + gap;
    }
}

pub(crate) fn draw_magnifier_readout(
    gpu: &mut Gpu,
    palette: &Palette,
    scale: f32,
    area: Rect,
    hex: &str,
    coordinates: &str,
    window_width: f32,
) {
    let size = 12.0 * scale;
    let pad = 8.0 * scale;
    let gap = 8.0 * scale;
    let hex_width = text_width(gpu, size, DWRITE_FONT_WEIGHT_SEMI_BOLD, true, hex);
    let coordinate_width = text_width(gpu, size, DWRITE_FONT_WEIGHT_NORMAL, true, coordinates);
    let width = (pad * 2.0 + hex_width + gap + coordinate_width)
        .max(area.width.get() as f32)
        .min(4000.0);
    let x = (area.x.get() as f32).clamp(0.0, (window_width - width).max(0.0));
    let y = area.y.get() as f32;
    let height = area.height.get() as f32;
    d2d::fill_round(
        &gpu.context,
        d2d::rect(x, y, width, height),
        4.0 * scale,
        palette.layer,
    );
    d2d::stroke_round(
        &gpu.context,
        d2d::rect(x + 0.5, y + 0.5, width - 1.0, height - 1.0),
        4.0 * scale,
        1.0,
        palette.stroke,
    );
    if let Some(format) = gpu
        .text
        .format(&gpu.write, size, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)
    {
        d2d::draw_text(
            &gpu.context,
            &format,
            hex,
            d2d::rect(x + pad, y, hex_width, height),
            palette.text,
        );
    }
    if let Some(format) = gpu
        .text
        .format(&gpu.write, size, DWRITE_FONT_WEIGHT_NORMAL, true)
    {
        d2d::draw_text(
            &gpu.context,
            &format,
            coordinates,
            d2d::rect(x + pad + hex_width + gap, y, coordinate_width, height),
            palette.text3,
        );
    }
}

pub(crate) fn measure_upload_label(gpu: &mut Gpu, scale: f32) -> f32 {
    text_width(
        gpu,
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        false,
        "Upload",
    )
}

pub(crate) fn draw_upload_label(
    gpu: &mut Gpu,
    palette: &Palette,
    scale: f32,
    x: f32,
    y: f32,
    height: f32,
    label: &str,
) {
    let Some(format) = gpu.text.format(
        &gpu.write,
        13.0 * scale,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        false,
    ) else {
        return;
    };
    let width = d2d::measure(&gpu.write, &format, label);
    d2d::draw_text(
        &gpu.context,
        &format,
        label,
        d2d::rect(x, y, width, height),
        palette.on_accent,
    );
}
