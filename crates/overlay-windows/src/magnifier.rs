use snapline_core::geometry::{Point, Px, Rect, Size};

use crate::d2d::{self, Gpu};
use crate::readout;
use crate::tokens::Palette;
use crate::window::WindowState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Geometry {
    pub destination: Rect,
    pub source: Rect,
    pub readout: Rect,
    pub zoom: i32,
    pub readout_above: bool,
}

pub(crate) fn geometry(
    cursor: Point,
    window: Size,
    image: Size,
    scale: f32,
    zoom: i32,
) -> Geometry {
    let size = (120.0 * scale).round().max(1.0) as i32;
    let zoom = zoom.max(1);
    let span = (size / zoom)
        .max(1)
        .min(image.width.get().max(1))
        .min(image.height.get().max(1));
    let source_x = (cursor.x.get() - span / 2).clamp(0, (image.width.get() - span).max(0));
    let source_y = (cursor.y.get() - span / 2).clamp(0, (image.height.get() - span).max(0));
    let gap = (20.0 * scale).round() as i32;
    let readout_height = (24.0 * scale).round() as i32;
    let readout_gap = (4.0 * scale).round() as i32;
    let block_height = size + readout_gap + readout_height;
    let mut block_x = cursor.x.get() + gap;
    if block_x + size > window.width.get() {
        block_x = cursor.x.get() - gap - size;
    }
    block_x = block_x.clamp(0, (window.width.get() - size).max(0));
    let mut block_y = cursor.y.get() + gap;
    let mut readout_above = false;
    if block_y + block_height > window.height.get() {
        block_y = cursor.y.get() - gap - block_height;
        readout_above = true;
    }
    block_y = block_y.clamp(0, (window.height.get() - block_height).max(0));
    let (destination, readout) = if readout_above {
        (
            Rect::new(
                Px(block_x),
                Px(block_y + readout_height + readout_gap),
                Px(size),
                Px(size),
            ),
            Rect::new(Px(block_x), Px(block_y), Px(size), Px(readout_height)),
        )
    } else {
        (
            Rect::new(Px(block_x), Px(block_y), Px(size), Px(size)),
            Rect::new(
                Px(block_x),
                Px(block_y + size + readout_gap),
                Px(size),
                Px(readout_height),
            ),
        )
    };
    Geometry {
        destination,
        source: Rect::new(Px(source_x), Px(source_y), Px(span), Px(span)),
        readout,
        zoom,
        readout_above,
    }
}

pub(crate) fn draw(
    gpu: &mut Gpu,
    window: &WindowState,
    palette: &Palette,
    cursor: Point,
    zoom: i32,
) {
    let scale = window.source.scale;
    let geometry = geometry(
        cursor,
        window.source.size,
        Size::new(
            Px(window.source.image.width() as i32),
            Px(window.source.image.height() as i32),
        ),
        scale,
        zoom,
    );
    let context = &gpu.context;
    let destination = geometry.destination;
    let dx = destination.x.get() as f32;
    let dy = destination.y.get() as f32;
    let size = destination.width.get() as f32;
    d2d::draw_bitmap(
        context,
        &window.frozen,
        d2d::rect(dx, dy, size, size),
        d2d::rect(
            geometry.source.x.get() as f32,
            geometry.source.y.get() as f32,
            geometry.source.width.get() as f32,
            geometry.source.height.get() as f32,
        ),
        true,
    );
    d2d::stroke_rect(
        context,
        d2d::rect(dx + 0.5, dy + 0.5, size - 1.0, size - 1.0),
        1.0,
        palette.stroke_hi,
    );
    if geometry.zoom >= 8 {
        let step = geometry.zoom as f32;
        let mut offset = step;
        while offset < size {
            d2d::fill_rect(
                context,
                d2d::rect(dx + offset, dy, 1.0, size),
                palette.stroke_hi,
            );
            d2d::fill_rect(
                context,
                d2d::rect(dx, dy + offset, size, 1.0),
                palette.stroke_hi,
            );
            offset += step;
        }
    }
    let centre_x = geometry.source.x.get() + geometry.source.width.get() / 2;
    let centre_y = geometry.source.y.get() + geometry.source.height.get() / 2;
    let pixel = window
        .source
        .image
        .pixel(centre_x.max(0) as u32, centre_y.max(0) as u32)
        .unwrap_or([0, 0, 0, 255]);
    let hex = format!("#{:02X}{:02X}{:02X}", pixel[2], pixel[1], pixel[0]);
    let cursor_global = Point::new(
        Px(cursor.x.get() + window.source.origin.x.get()),
        Px(cursor.y.get() + window.source.origin.y.get()),
    );
    let coordinates = format!("X {} · Y {}", cursor_global.x.get(), cursor_global.y.get());
    readout::draw_magnifier_readout(
        gpu,
        palette,
        scale,
        geometry.readout,
        &hex,
        &coordinates,
        window.source.size.width.get() as f32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: i32, height: i32) -> Size {
        Size::new(Px(width), Px(height))
    }

    #[test]
    fn magnifier_is_120_logical_and_keeps_the_pixel_zoom() {
        for (scale, expected) in [(1.0f32, 120), (1.25, 150), (1.5, 180), (2.0, 240)] {
            let geometry = geometry(
                Point::new(Px(960), Px(540)),
                size(1920, 1080),
                size(1920, 1080),
                scale,
                4,
            );
            assert_eq!(geometry.destination.width.get(), expected);
            assert_eq!(geometry.destination.height.get(), expected);
            assert_eq!(geometry.source.width.get(), expected / 4);
            assert_eq!(geometry.source.height.get(), expected / 4);
            assert_eq!(geometry.zoom, 4);
            assert!(!geometry.readout_above);
        }
        let geometry = geometry(
            Point::new(Px(960), Px(540)),
            size(1920, 1080),
            size(1920, 1080),
            1.0,
            8,
        );
        assert_eq!(geometry.zoom, 8);
        assert_eq!(geometry.source.width.get(), 15);
    }

    #[test]
    fn magnifier_flips_to_stay_on_screen() {
        let flipped = geometry(
            Point::new(Px(1900), Px(1070)),
            size(1920, 1080),
            size(1920, 1080),
            1.0,
            4,
        );
        assert!(flipped.destination.right().get() <= 1920);
        assert!(flipped.readout.bottom().get() <= 1080);
        assert!(flipped.readout_above);
        let near = geometry(
            Point::new(Px(5), Px(5)),
            size(1920, 1080),
            size(1920, 1080),
            1.0,
            4,
        );
        assert!(near.destination.x.get() >= 0);
        assert!(near.readout.x.get() >= 0);
        assert!(!near.readout_above);
    }

    #[test]
    fn magnifier_source_is_clamped_to_the_image() {
        let near = geometry(
            Point::new(Px(2), Px(2)),
            size(1920, 1080),
            size(1920, 1080),
            1.0,
            8,
        );
        assert_eq!(near.source.x.get(), 0);
        assert_eq!(near.source.y.get(), 0);
        let far = geometry(
            Point::new(Px(1918), Px(1078)),
            size(1920, 1080),
            size(1920, 1080),
            1.0,
            8,
        );
        assert_eq!(far.source.right().get(), 1920);
        assert_eq!(far.source.bottom().get(), 1080);
    }
}
