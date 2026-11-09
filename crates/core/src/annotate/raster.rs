use crate::geometry::{Point, Px, Rect};
use crate::image::{Image, ImageError};

use super::canvas::{Canvas, Rgba};
use super::shape::{
    half, label_rect, label_text_origin, points_bounds, round, smooth, step_rect, text_advance,
    text_scale, Shape, STEP_RADIUS, STEP_SIZE, TEXT_SIZE,
};

const SUBSAMPLES: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    image: Image,
}

impl Raster {
    pub fn new(width: u32, height: u32) -> Result<Self, ImageError> {
        Ok(Self {
            image: Image::bgra(width, height)?,
        })
    }

    pub const fn from_image(image: Image) -> Self {
        Self { image }
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    pub fn into_image(self) -> Image {
        self.image
    }

    pub fn to_png(&self) -> Vec<u8> {
        encode_png(&self.image)
    }

    pub fn draw(&mut self, shape: &Shape, frozen: &Image) {
        match shape {
            Shape::Rect { rect, style } => {
                self.stroke_rect(
                    *rect,
                    super::shape::CORNER_RADIUS,
                    style.width(),
                    style.rgba(),
                );
            }
            Shape::Ellipse { rect, style } => {
                self.stroke_ellipse(*rect, style.width(), style.rgba());
            }
            Shape::Arrow {
                tail,
                tip,
                curve,
                style,
            } => self.draw_arrow(*tail, *tip, *curve, style.width(), style.rgba()),
            Shape::Pen { points, style } => {
                self.polyline(&smooth(points), style.width(), style.rgba());
            }
            Shape::Highlight { points, style } => {
                self.polyline(points, style.width(), style.highlight());
            }
            Shape::Text {
                origin,
                text,
                colour,
            } => {
                self.fill_rect(
                    label_rect(*origin, text),
                    super::shape::LABEL_RADIUS,
                    colour.label_background(),
                );
                self.text(
                    label_text_origin(*origin),
                    text,
                    TEXT_SIZE,
                    true,
                    colour.rgba(),
                );
            }
            Shape::Step {
                center,
                number,
                colour,
            } => {
                self.fill_rect(step_rect(*center), STEP_RADIUS, colour.rgba());
                let value = number.to_string();
                let advance = text_advance(STEP_SIZE);
                let width = value.chars().count() as i32 * advance;
                let height = 7 * text_scale(STEP_SIZE);
                let origin = Point::new(Px(center.x.0 - width / 2), Px(center.y.0 - height / 2));
                self.text(origin, &value, STEP_SIZE, true, Rgba::WHITE);
            }
            Shape::Blur { rect } => {
                self.pixelate(*rect, super::shape::BLUR_BLOCK, frozen);
            }
        }
    }

    fn draw_arrow(
        &mut self,
        tail: Point,
        tip: Point,
        curve: Option<Point>,
        width: Px,
        colour: Rgba,
    ) {
        let shaft = match curve {
            Some(control) => quadratic(tail, control, tip),
            None => vec![tail, tip],
        };
        let direction = match curve {
            Some(control) => (
                f64::from(tip.x.0 - control.x.0),
                f64::from(tip.y.0 - control.y.0),
            ),
            None => (f64::from(tip.x.0 - tail.x.0), f64::from(tip.y.0 - tail.y.0)),
        };
        let length = direction.0.hypot(direction.1);
        if length == 0.0 {
            return;
        }
        let ux = direction.0 / length;
        let uy = direction.1 / length;
        let head = f64::from(width.0 * 2 + 12).max(14.0);
        let back_x = f64::from(tip.x.0) - ux * head;
        let back_y = f64::from(tip.y.0) - uy * head;
        let wing = head * 0.5;
        let first = Point::new(Px(round(back_x - uy * wing)), Px(round(back_y + ux * wing)));
        let second = Point::new(Px(round(back_x + uy * wing)), Px(round(back_y - ux * wing)));
        let paths = vec![shaft, vec![tip, first], vec![tip, second]];
        self.stroke_paths(&paths, width, colour);
    }

    fn stroke_paths(&mut self, paths: &[Vec<Point>], width: Px, colour: Rgba) {
        let reach = half(width);
        let mut bounds = Rect::ZERO;
        let mut first = true;
        for path in paths {
            if path.is_empty() {
                continue;
            }
            let path_bounds = points_bounds(path, reach + Px::ONE);
            bounds = if first {
                first = false;
                path_bounds
            } else {
                bounds.union(path_bounds)
            };
        }
        if first {
            return;
        }
        let limit = f64::from(reach.0) * f64::from(reach.0);
        self.shade(bounds, colour, |x, y| {
            paths.iter().any(|path| {
                if path.len() == 1 {
                    segment_distance2(x, y, path[0], path[0]) <= limit
                } else {
                    path.windows(2)
                        .any(|pair| segment_distance2(x, y, pair[0], pair[1]) <= limit)
                }
            })
        });
    }

    fn shade<F>(&mut self, bounds: Rect, colour: Rgba, inside: F)
    where
        F: Fn(f64, f64) -> bool,
    {
        let Some((left, top, right, bottom)) = self.clip(bounds) else {
            return;
        };
        for y in top..bottom {
            for x in left..right {
                let mut count = 0;
                for sy in 0..SUBSAMPLES {
                    for sx in 0..SUBSAMPLES {
                        let px = f64::from(x) + (f64::from(sx) + 0.5) / f64::from(SUBSAMPLES);
                        let py = f64::from(y) + (f64::from(sy) + 0.5) / f64::from(SUBSAMPLES);
                        if inside(px, py) {
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    self.blend(x, y, colour, count);
                }
            }
        }
    }

    fn clip(&self, rect: Rect) -> Option<(i32, i32, i32, i32)> {
        let left = rect.left().0.max(0);
        let top = rect.top().0.max(0);
        let right = rect.right().0.min(self.image.width() as i32);
        let bottom = rect.bottom().0.min(self.image.height() as i32);
        if right <= left || bottom <= top {
            None
        } else {
            Some((left, top, right, bottom))
        }
    }

    fn blend(&mut self, x: i32, y: i32, colour: Rgba, coverage: u32) {
        if x < 0 || y < 0 || x >= self.image.width() as i32 || y >= self.image.height() as i32 {
            return;
        }
        let Ok(destination) = self.image.pixel(x as u32, y as u32) else {
            return;
        };
        let source_alpha = (u32::from(colour.a) * coverage + SUBSAMPLES * SUBSAMPLES / 2)
            / (SUBSAMPLES * SUBSAMPLES);
        if source_alpha == 0 {
            return;
        }
        let inverse = 255 - source_alpha;
        let destination_alpha = u32::from(destination[3]);
        let denominator = source_alpha * 255 + destination_alpha * inverse;
        if denominator == 0 {
            return;
        }
        let channel = |source: u8, target: u8| {
            let numerator = u32::from(source) * source_alpha * 255
                + u32::from(target) * destination_alpha * inverse;
            ((numerator + denominator / 2) / denominator) as u8
        };
        let alpha = (denominator + 127) / 255;
        let pixel = [
            channel(colour.b, destination[0]),
            channel(colour.g, destination[1]),
            channel(colour.r, destination[2]),
            alpha as u8,
        ];
        let _ = self.image.set_pixel(x as u32, y as u32, pixel);
    }

    fn solid(&mut self, rect: Rect, colour: Rgba) {
        let Some((left, top, right, bottom)) = self.clip(rect) else {
            return;
        };
        for y in top..bottom {
            for x in left..right {
                self.blend(x, y, colour, SUBSAMPLES * SUBSAMPLES);
            }
        }
    }

    fn draw_text(&mut self, origin: Point, text: &str, size: Px, bold: bool, colour: Rgba) {
        let scale = text_scale(size);
        let advance = text_advance(size);
        for (index, character) in text.chars().enumerate() {
            let rows = glyph(character);
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) == 0 {
                        continue;
                    }
                    let x = origin.x.0 + index as i32 * advance + column * scale;
                    let y = origin.y.0 + row as i32 * scale;
                    let width = scale + if bold { 1 } else { 0 };
                    self.solid(Rect::new(Px(x), Px(y), Px(width), Px(scale)), colour);
                }
            }
        }
    }

    fn draw_image(&mut self, source: &Image, rect: Rect) {
        let Some((left, top, right, bottom)) = self.clip(rect) else {
            return;
        };
        if rect.width.0 <= 0 || rect.height.0 <= 0 {
            return;
        }
        for y in top..bottom {
            for x in left..right {
                let source_x = ((x - rect.left().0) as u64 * u64::from(source.width())
                    / rect.width.0 as u64) as u32;
                let source_y = ((y - rect.top().0) as u64 * u64::from(source.height())
                    / rect.height.0 as u64) as u32;
                let Ok(pixel) = source.pixel(source_x, source_y) else {
                    continue;
                };
                let colour = Rgba::new(pixel[2], pixel[1], pixel[0], pixel[3]);
                self.blend(x, y, colour, SUBSAMPLES * SUBSAMPLES);
            }
        }
    }
}

impl Canvas for Raster {
    fn fill_rect(&mut self, rect: Rect, radius: Px, colour: Rgba) {
        let radius = f64::from(radius.0.max(0));
        self.shade(rect, colour, |x, y| inside_rounded(x, y, rect, radius));
    }

    fn stroke_rect(&mut self, rect: Rect, radius: Px, width: Px, colour: Rgba) {
        let radius = f64::from(radius.0.max(0));
        let width = width.0.max(1);
        let inner = if rect.width.0 > width * 2 && rect.height.0 > width * 2 {
            Some(Rect::new(
                Px(rect.x.0 + width),
                Px(rect.y.0 + width),
                Px(rect.width.0 - width * 2),
                Px(rect.height.0 - width * 2),
            ))
        } else {
            None
        };
        let inner_radius = (radius - f64::from(width)).max(0.0);
        self.shade(rect, colour, |x, y| {
            inside_rounded(x, y, rect, radius)
                && !inner.is_some_and(|inner| inside_rounded(x, y, inner, inner_radius))
        });
    }

    fn stroke_ellipse(&mut self, rect: Rect, width: Px, colour: Rgba) {
        let width = width.0.max(1);
        self.shade(rect, colour, |x, y| {
            inside_ellipse(x, y, rect, 0.0) && !inside_ellipse(x, y, rect, f64::from(width))
        });
    }

    fn line(&mut self, from: Point, to: Point, width: Px, colour: Rgba) {
        self.stroke_paths(&[vec![from, to]], width, colour);
    }

    fn polyline(&mut self, points: &[Point], width: Px, colour: Rgba) {
        self.stroke_paths(&[points.to_vec()], width, colour);
    }

    fn text(&mut self, origin: Point, text: &str, size: Px, bold: bool, colour: Rgba) {
        self.draw_text(origin, text, size, bold, colour);
    }

    fn pixelate(&mut self, rect: Rect, block: Px, source: &Image) {
        let block = block.0.max(1);
        let Some((left, top, right, bottom)) = self.clip(rect) else {
            return;
        };
        let source_width = source.width() as i32;
        let source_height = source.height() as i32;
        let mut y = top;
        while y < bottom {
            let mut x = left;
            while x < right {
                let end_x = (x + block).min(right).min(source_width);
                let end_y = (y + block).min(bottom).min(source_height);
                let start_x = x.max(0);
                let start_y = y.max(0);
                if end_x > start_x && end_y > start_y {
                    let count = ((end_x - start_x) * (end_y - start_y)) as u32;
                    let mut total = [0_u32; 4];
                    for sample_y in start_y..end_y {
                        for sample_x in start_x..end_x {
                            let Ok(pixel) = source.pixel(sample_x as u32, sample_y as u32) else {
                                continue;
                            };
                            for channel in 0..4 {
                                total[channel] += u32::from(pixel[channel]);
                            }
                        }
                    }
                    let average = |value: u32| ((value + count / 2) / count) as u8;
                    let colour = Rgba::new(
                        average(total[2]),
                        average(total[1]),
                        average(total[0]),
                        average(total[3]),
                    );
                    self.solid(
                        Rect::new(Px(x), Px(y), Px(end_x - x), Px(end_y - y)),
                        colour,
                    );
                }
                x += block;
            }
            y += block;
        }
    }

    fn image(&mut self, image: &Image, rect: Rect) {
        self.draw_image(image, rect);
    }
}

fn inside_rounded(x: f64, y: f64, rect: Rect, radius: f64) -> bool {
    let left = f64::from(rect.left().0);
    let top = f64::from(rect.top().0);
    let right = f64::from(rect.right().0);
    let bottom = f64::from(rect.bottom().0);
    if x < left || x > right || y < top || y > bottom {
        return false;
    }
    let radius = radius
        .min((right - left) / 2.0)
        .min((bottom - top) / 2.0)
        .max(0.0);
    let center_x = x.clamp(left + radius, right - radius);
    let center_y = y.clamp(top + radius, bottom - radius);
    let dx = x - center_x;
    let dy = y - center_y;
    dx * dx + dy * dy <= radius * radius
}

fn inside_ellipse(x: f64, y: f64, rect: Rect, inset: f64) -> bool {
    let radius_x = f64::from(rect.width.0) / 2.0 - inset;
    let radius_y = f64::from(rect.height.0) / 2.0 - inset;
    if radius_x <= 0.0 || radius_y <= 0.0 {
        return false;
    }
    let center_x = f64::from(rect.left().0) + f64::from(rect.width.0) / 2.0;
    let center_y = f64::from(rect.top().0) + f64::from(rect.height.0) / 2.0;
    let dx = (x - center_x) / radius_x;
    let dy = (y - center_y) / radius_y;
    dx * dx + dy * dy <= 1.0
}

fn segment_distance2(x: f64, y: f64, a: Point, b: Point) -> f64 {
    let ax = f64::from(a.x.0);
    let ay = f64::from(a.y.0);
    let dx = f64::from(b.x.0) - ax;
    let dy = f64::from(b.y.0) - ay;
    let length = dx * dx + dy * dy;
    let t = if length == 0.0 {
        0.0
    } else {
        (((x - ax) * dx + (y - ay) * dy) / length).clamp(0.0, 1.0)
    };
    let px = ax + t * dx;
    let py = ay + t * dy;
    (x - px) * (x - px) + (y - py) * (y - py)
}

fn quadratic(tail: Point, control: Point, tip: Point) -> Vec<Point> {
    let mut points = Vec::with_capacity(17);
    for step in 0..=16 {
        let t = f64::from(step) / 16.0;
        let inverse = 1.0 - t;
        let x = inverse * inverse * f64::from(tail.x.0)
            + 2.0 * inverse * t * f64::from(control.x.0)
            + t * t * f64::from(tip.x.0);
        let y = inverse * inverse * f64::from(tail.y.0)
            + 2.0 * inverse * t * f64::from(control.y.0)
            + t * t * f64::from(tip.y.0);
        let point = Point::new(Px(round(x)), Px(round(y)));
        if points.last() != Some(&point) {
            points.push(point);
        }
    }
    points
}

pub fn crop_image(image: &Image, rect: Rect) -> Result<Image, ImageError> {
    if rect.is_empty() {
        return Err(ImageError::Empty);
    }
    let width = rect.width.0 as u32;
    let height = rect.height.0 as u32;
    let mut output = Image::bgra(width, height)?;
    for y in 0..height {
        for x in 0..width {
            let source_x = rect.x.0 as u32 + x;
            let source_y = rect.y.0 as u32 + y;
            if let Ok(pixel) = image.pixel(source_x, source_y) {
                output.set_pixel(x, y, pixel)?;
            }
        }
    }
    Ok(output)
}

pub fn encode_png(image: &Image) -> Vec<u8> {
    let mut raw = Vec::with_capacity((image.width() as usize * 4 + 1) * image.height() as usize);
    for y in 0..image.height() {
        raw.push(0);
        for x in 0..image.width() {
            if let Ok(pixel) = image.pixel(x, y) {
                raw.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let mut offset = 0;
    while offset < raw.len() {
        let remaining = raw.len() - offset;
        let length = remaining.min(65_535);
        let last = offset + length == raw.len();
        zlib.push(u8::from(last));
        zlib.extend_from_slice(&(length as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(length as u16)).to_le_bytes());
        zlib.extend_from_slice(&raw[offset..offset + length]);
        offset += length;
    }
    if raw.is_empty() {
        zlib.push(1);
        zlib.extend_from_slice(&0_u16.to_le_bytes());
        zlib.extend_from_slice(&u16::MAX.to_le_bytes());
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut output = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    let mut header = Vec::new();
    header.extend_from_slice(&image.width().to_be_bytes());
    header.extend_from_slice(&image.height().to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    output.extend_from_slice(&chunk(b"IHDR", &header));
    output.extend_from_slice(&chunk(b"IDAT", &zlib));
    output.extend_from_slice(&chunk(b"IEND", &[]));
    output
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + 12);
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc = Crc::new();
    crc.update(kind);
    crc.update(data);
    output.extend_from_slice(&crc.finish().to_be_bytes());
    output
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

struct Crc {
    value: u32,
}

impl Crc {
    const fn new() -> Self {
        Self { value: 0xffff_ffff }
    }

    fn update(&mut self, data: &[u8]) {
        for byte in data {
            self.value ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = (self.value & 1).wrapping_neg();
                self.value = (self.value >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }

    const fn finish(self) -> u32 {
        self.value ^ 0xffff_ffff
    }
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        ' ' => [0; 7],
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        ',' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000,
        ],
        ':' => [
            0b00000, 0b00110, 0b00110, 0b00000, 0b00110, 0b00110, 0b00000,
        ],
        ';' => [
            0b00000, 0b00110, 0b00110, 0b00000, 0b00110, 0b00110, 0b01000,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '+' => [
            0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
        ],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '!' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
        '\'' => [
            0b00100, 0b00100, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000,
        ],
        '"' => [
            0b01010, 0b01010, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000,
        ],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '=' => [
            0b00000, 0b00000, 0b11111, 0b00000, 0b11111, 0b00000, 0b00000,
        ],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        '%' => [
            0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011,
        ],
        '&' => [
            0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
        ],
        '*' => [
            0b00000, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0b00000,
        ],
        '@' => [
            0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01110,
        ],
        _ => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
    }
}
