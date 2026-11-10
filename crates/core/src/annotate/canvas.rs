use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Px, Rect};
use crate::image::Image;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const TRANSPARENT: Self = Self::new(0, 0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255, 255);
    pub const BLACK: Self = Self::new(0, 0, 0, 255);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn alpha(self, a: u8) -> Self {
        Self::new(self.r, self.g, self.b, a)
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        let channel = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();
        Some(Self::new(channel(0)?, channel(2)?, channel(4)?, 255))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tolerance {
    per_channel: u8,
}

impl Tolerance {
    pub const EXACT: Self = Self { per_channel: 0 };
    pub const TWO: Self = Self { per_channel: 2 };

    pub const fn new(per_channel: u8) -> Self {
        Self { per_channel }
    }

    pub const fn per_channel(self) -> u8 {
        self.per_channel
    }

    pub fn within(self, expected: [u8; 4], actual: [u8; 4]) -> bool {
        expected
            .iter()
            .zip(actual.iter())
            .all(|(a, b)| a.abs_diff(*b) <= self.per_channel)
    }

    pub fn first_difference(self, expected: &Image, actual: &Image) -> Option<(u32, u32)> {
        if expected.width() != actual.width() || expected.height() != actual.height() {
            return Some((0, 0));
        }
        for y in 0..expected.height() {
            for x in 0..expected.width() {
                let left = expected.pixel(x, y).ok()?;
                let right = actual.pixel(x, y).ok()?;
                if !self.within(left, right) {
                    return Some((x, y));
                }
            }
        }
        None
    }

    pub fn matches(self, expected: &Image, actual: &Image) -> bool {
        self.first_difference(expected, actual).is_none()
    }
}

pub trait Canvas {
    fn fill_rect(&mut self, rect: Rect, radius: Px, colour: Rgba);
    fn stroke_rect(&mut self, rect: Rect, radius: Px, width: Px, colour: Rgba);
    fn stroke_ellipse(&mut self, rect: Rect, width: Px, colour: Rgba);
    fn line(&mut self, from: Point, to: Point, width: Px, colour: Rgba);
    fn polyline(&mut self, points: &[Point], width: Px, colour: Rgba);
    fn text(&mut self, origin: Point, text: &str, size: Px, bold: bool, colour: Rgba);
    fn pixelate(&mut self, rect: Rect, block: Px, source: &Image);
    fn image(&mut self, image: &Image, rect: Rect);
}

pub fn fingerprint(image: &Image) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in image.pixels() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
