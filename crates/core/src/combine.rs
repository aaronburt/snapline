use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::geometry::{Px, Rect};
use crate::image::{Image, ImageError};

pub const DEFAULT_SPACING: u32 = 8;
pub const MAX_SPACING: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    Start,
    Centre,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Background {
    Transparent,
    White,
    Black,
    Custom(String),
}

impl Default for Background {
    fn default() -> Self {
        Self::Transparent
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombineOptions {
    pub layout: Layout,
    pub spacing: u32,
    pub align: Align,
    pub background: Background,
}

impl Default for CombineOptions {
    fn default() -> Self {
        Self {
            layout: Layout::SideBySide,
            spacing: DEFAULT_SPACING,
            align: Align::Start,
            background: Background::Transparent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub index: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombineLayout {
    pub width: u32,
    pub height: u32,
    pub placements: Vec<Placement>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CombineError {
    #[error("Choose at least one image.")]
    Empty,
    #[error("Spacing must be between 0 and 64 px.")]
    InvalidSpacing,
    #[error("Enter the colour as a hex value, for example #1A2B3C.")]
    InvalidHex,
    #[error("The combined image is too large.")]
    Overflow,
    #[error("The combined image could not be created.")]
    Allocate,
}

pub fn parse_hex(value: &str) -> Option<[u8; 4]> {
    let digits = value.strip_prefix('#').unwrap_or(value);
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let red = u8::from_str_radix(&digits[0..2], 16).ok()?;
    let green = u8::from_str_radix(&digits[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&digits[4..6], 16).ok()?;
    Some([blue, green, red, 255])
}

pub fn background_colour(background: &Background) -> Result<[u8; 4], CombineError> {
    match background {
        Background::Transparent => Ok([0, 0, 0, 0]),
        Background::White => Ok([255, 255, 255, 255]),
        Background::Black => Ok([0, 0, 0, 255]),
        Background::Custom(value) => parse_hex(value).ok_or(CombineError::InvalidHex),
    }
}

pub fn layout(images: &[Image], options: &CombineOptions) -> Result<CombineLayout, CombineError> {
    let sizes: Vec<(u32, u32)> = images
        .iter()
        .map(|image| (image.width(), image.height()))
        .collect();
    layout_sizes(&sizes, options)
}

pub fn layout_sizes(
    sizes: &[(u32, u32)],
    options: &CombineOptions,
) -> Result<CombineLayout, CombineError> {
    if sizes.is_empty()
        || sizes
            .iter()
            .any(|(width, height)| *width == 0 || *height == 0)
    {
        return Err(CombineError::Empty);
    }
    if options.spacing > MAX_SPACING {
        return Err(CombineError::InvalidSpacing);
    }
    let count = u64::try_from(sizes.len()).map_err(|_| CombineError::Overflow)?;
    let spacing = u64::from(options.spacing);
    let gaps = count.saturating_sub(1) * spacing;
    let cross_max = sizes
        .iter()
        .map(|(width, height)| match options.layout {
            Layout::SideBySide => u64::from(*height),
            Layout::Stacked => u64::from(*width),
        })
        .max()
        .unwrap_or(0);
    let main_total = sizes.iter().try_fold(0_u64, |total, (width, height)| {
        let value = match options.layout {
            Layout::SideBySide => u64::from(*width),
            Layout::Stacked => u64::from(*height),
        };
        total.checked_add(value).ok_or(CombineError::Overflow)
    })?;
    let (width, height) = match options.layout {
        Layout::SideBySide => (
            main_total.checked_add(gaps).ok_or(CombineError::Overflow)?,
            cross_max,
        ),
        Layout::Stacked => (
            cross_max,
            main_total.checked_add(gaps).ok_or(CombineError::Overflow)?,
        ),
    };
    if width == 0 || height == 0 {
        return Err(CombineError::Empty);
    }
    let limit = u64::try_from(i32::MAX).unwrap_or(u64::MAX);
    if width > limit || height > limit {
        return Err(CombineError::Overflow);
    }
    let width = u32::try_from(width).map_err(|_| CombineError::Overflow)?;
    let height = u32::try_from(height).map_err(|_| CombineError::Overflow)?;
    let mut placements = Vec::with_capacity(sizes.len());
    let mut cursor = 0_u64;
    for (index, (image_width, image_height)) in sizes.iter().copied().enumerate() {
        let (x, y) = match options.layout {
            Layout::SideBySide => {
                let free = u64::from(height).saturating_sub(u64::from(image_height));
                let offset = match options.align {
                    Align::Start => 0,
                    Align::Centre => free / 2,
                    Align::End => free,
                };
                (cursor, offset)
            }
            Layout::Stacked => {
                let free = u64::from(width).saturating_sub(u64::from(image_width));
                let offset = match options.align {
                    Align::Start => 0,
                    Align::Centre => free / 2,
                    Align::End => free,
                };
                (offset, cursor)
            }
        };
        placements.push(Placement {
            index,
            rect: Rect::new(
                Px(i32::try_from(x).map_err(|_| CombineError::Overflow)?),
                Px(i32::try_from(y).map_err(|_| CombineError::Overflow)?),
                Px(i32::try_from(image_width).map_err(|_| CombineError::Overflow)?),
                Px(i32::try_from(image_height).map_err(|_| CombineError::Overflow)?),
            ),
        });
        let advance = match options.layout {
            Layout::SideBySide => u64::from(image_width),
            Layout::Stacked => u64::from(image_height),
        };
        cursor = cursor
            .checked_add(advance)
            .and_then(|value| value.checked_add(spacing))
            .ok_or(CombineError::Overflow)?;
    }
    Ok(CombineLayout {
        width,
        height,
        placements,
    })
}

pub fn compose(images: &[Image], options: &CombineOptions) -> Result<Image, CombineError> {
    let combined = layout(images, options)?;
    let colour = background_colour(&options.background)?;
    let mut output =
        Image::bgra(combined.width, combined.height).map_err(|_| CombineError::Allocate)?;
    fill(&mut output, colour)?;
    for placement in &combined.placements {
        let source = images.get(placement.index).ok_or(CombineError::Allocate)?;
        copy_into(&mut output, source, placement.rect)?;
    }
    Ok(output)
}

fn fill(image: &mut Image, colour: [u8; 4]) -> Result<(), CombineError> {
    for y in 0..image.height() {
        let row = image.row_mut(y).ok_or(CombineError::Allocate)?;
        for pixel in row.chunks_exact_mut(4) {
            pixel.copy_from_slice(&colour);
        }
    }
    Ok(())
}

fn copy_into(target: &mut Image, source: &Image, rect: Rect) -> Result<(), CombineError> {
    let x = u32::try_from(rect.x.get()).map_err(|_| CombineError::Overflow)?;
    let y = u32::try_from(rect.y.get()).map_err(|_| CombineError::Overflow)?;
    let span = usize::try_from(source.width())
        .map_err(|_| CombineError::Overflow)?
        .checked_mul(4)
        .ok_or(CombineError::Overflow)?;
    for source_y in 0..source.height() {
        let source_row = source.row(source_y).ok_or(CombineError::Allocate)?;
        let target_y = y.checked_add(source_y).ok_or(CombineError::Overflow)?;
        let target_row = target.row_mut(target_y).ok_or(CombineError::Allocate)?;
        let start = usize::try_from(x)
            .map_err(|_| CombineError::Overflow)?
            .checked_mul(4)
            .ok_or(CombineError::Overflow)?;
        let end = start.checked_add(span).ok_or(CombineError::Overflow)?;
        let destination = target_row
            .get_mut(start..end)
            .ok_or(CombineError::Allocate)?;
        let source_pixels = source_row.get(..span).ok_or(CombineError::Allocate)?;
        destination.copy_from_slice(source_pixels);
    }
    Ok(())
}

impl From<ImageError> for CombineError {
    fn from(_: ImageError) -> Self {
        Self::Allocate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixel: [u8; 4]) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                image.set_pixel(x, y, pixel).expect("pixel");
            }
        }
        image
    }

    fn options(
        layout: Layout,
        spacing: u32,
        align: Align,
        background: Background,
    ) -> CombineOptions {
        CombineOptions {
            layout,
            spacing,
            align,
            background,
        }
    }

    #[test]
    fn defaults_match_the_design() {
        let default = CombineOptions::default();
        assert_eq!(default.layout, Layout::SideBySide);
        assert_eq!(default.spacing, DEFAULT_SPACING);
        assert_eq!(default.spacing, 8);
        assert_eq!(default.align, Align::Start);
        assert_eq!(default.background, Background::Transparent);
    }

    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(
            layout(&[], &CombineOptions::default()),
            Err(CombineError::Empty)
        );
        assert_eq!(
            layout_sizes(&[(0, 10)], &CombineOptions::default()),
            Err(CombineError::Empty)
        );
    }

    #[test]
    fn spacing_is_limited_to_sixty_four() {
        let sizes = [(10, 10)];
        assert!(layout_sizes(
            &sizes,
            &options(Layout::Stacked, 64, Align::Start, Background::Transparent)
        )
        .is_ok());
        assert_eq!(
            layout_sizes(
                &sizes,
                &options(Layout::Stacked, 65, Align::Start, Background::Transparent)
            ),
            Err(CombineError::InvalidSpacing)
        );
    }

    #[test]
    fn side_by_side_offsets_use_start_alignment() {
        let sizes = [(100, 50), (40, 80), (30, 30)];
        let combined = layout_sizes(
            &sizes,
            &options(Layout::SideBySide, 8, Align::Start, Background::Transparent),
        )
        .expect("layout");
        assert_eq!(combined.width, 100 + 40 + 30 + 16);
        assert_eq!(combined.height, 80);
        assert_eq!(
            combined.placements[0].rect,
            Rect::new(Px(0), Px(0), Px(100), Px(50))
        );
        assert_eq!(
            combined.placements[1].rect,
            Rect::new(Px(108), Px(0), Px(40), Px(80))
        );
        assert_eq!(
            combined.placements[2].rect,
            Rect::new(Px(156), Px(0), Px(30), Px(30))
        );
    }

    #[test]
    fn side_by_side_centres_and_ends_shorter_images() {
        let sizes = [(100, 50), (40, 80), (30, 31)];
        let centred = layout_sizes(
            &sizes,
            &options(
                Layout::SideBySide,
                0,
                Align::Centre,
                Background::Transparent,
            ),
        )
        .expect("layout");
        assert_eq!(centred.placements[0].rect.y, Px(15));
        assert_eq!(centred.placements[1].rect.y, Px(0));
        assert_eq!(centred.placements[2].rect.y, Px(24));
        let ended = layout_sizes(
            &sizes,
            &options(Layout::SideBySide, 0, Align::End, Background::Transparent),
        )
        .expect("layout");
        assert_eq!(ended.placements[0].rect.y, Px(30));
        assert_eq!(ended.placements[1].rect.y, Px(0));
        assert_eq!(ended.placements[2].rect.y, Px(49));
    }

    #[test]
    fn stacked_offsets_use_each_alignment() {
        let sizes = [(100, 50), (40, 80), (30, 30)];
        let start = layout_sizes(
            &sizes,
            &options(Layout::Stacked, 8, Align::Start, Background::Transparent),
        )
        .expect("layout");
        assert_eq!(start.width, 100);
        assert_eq!(start.height, 50 + 80 + 30 + 16);
        assert_eq!(
            start.placements[0].rect,
            Rect::new(Px(0), Px(0), Px(100), Px(50))
        );
        assert_eq!(
            start.placements[1].rect,
            Rect::new(Px(0), Px(58), Px(40), Px(80))
        );
        assert_eq!(
            start.placements[2].rect,
            Rect::new(Px(0), Px(146), Px(30), Px(30))
        );
        let centre = layout_sizes(
            &sizes,
            &options(Layout::Stacked, 0, Align::Centre, Background::Transparent),
        )
        .expect("layout");
        assert_eq!(centre.placements[0].rect.x, Px(0));
        assert_eq!(centre.placements[1].rect.x, Px(30));
        assert_eq!(centre.placements[2].rect.x, Px(35));
        let end = layout_sizes(
            &sizes,
            &options(Layout::Stacked, 0, Align::End, Background::Transparent),
        )
        .expect("layout");
        assert_eq!(end.placements[0].rect.x, Px(0));
        assert_eq!(end.placements[1].rect.x, Px(60));
        assert_eq!(end.placements[2].rect.x, Px(70));
    }

    #[test]
    fn every_spacing_value_changes_the_gap() {
        for spacing in [0_u32, 1, 8, 32, 64] {
            let combined = layout_sizes(
                &[(10, 10), (10, 10), (10, 10)],
                &options(
                    Layout::SideBySide,
                    spacing,
                    Align::Start,
                    Background::Transparent,
                ),
            )
            .expect("layout");
            assert_eq!(combined.width, 30 + spacing * 2);
            assert_eq!(combined.placements[1].rect.x, Px(10 + spacing as i32));
        }
    }

    #[test]
    fn hex_colours_are_read_as_bgra_and_reject_bad_values() {
        assert_eq!(parse_hex("#1A2B3C"), Some([0x3c, 0x2b, 0x1a, 255]));
        assert_eq!(parse_hex("a1b2c3"), Some([0xc3, 0xb2, 0xa1, 255]));
        for value in ["", "#", "#12345", "#1234567", "#GGGGGG", "red", "#12 345"] {
            assert_eq!(parse_hex(value), None, "{value}");
        }
        assert_eq!(
            background_colour(&Background::Custom("#010203".to_owned())),
            Ok([3, 2, 1, 255])
        );
        assert_eq!(
            background_colour(&Background::Custom("nope".to_owned())),
            Err(CombineError::InvalidHex)
        );
    }

    #[test]
    fn transparent_background_leaves_alpha_zero() {
        let composed = compose(
            &[image(2, 2, [10, 20, 30, 255])],
            &options(Layout::SideBySide, 0, Align::Start, Background::Transparent),
        )
        .expect("compose");
        assert_eq!(composed.pixel(0, 0), Ok([10, 20, 30, 255]));
        let spaced = compose(
            &[
                image(1, 1, [10, 20, 30, 255]),
                image(1, 1, [40, 50, 60, 255]),
            ],
            &options(Layout::SideBySide, 4, Align::Start, Background::Transparent),
        )
        .expect("compose");
        assert_eq!(spaced.width(), 6);
        for x in 1..5 {
            assert_eq!(spaced.pixel(x, 0), Ok([0, 0, 0, 0]));
        }
    }

    #[test]
    fn white_black_and_hex_backgrounds_fill_the_gaps() {
        for (background, expected) in [
            (Background::White, [255, 255, 255, 255]),
            (Background::Black, [0, 0, 0, 255]),
            (
                Background::Custom("#102030".to_owned()),
                [0x30, 0x20, 0x10, 255],
            ),
        ] {
            let composed = compose(
                &[image(1, 1, [9, 9, 9, 255]), image(1, 1, [9, 9, 9, 255])],
                &options(Layout::Stacked, 2, Align::Start, background),
            )
            .expect("compose");
            assert_eq!(composed.width(), 1);
            assert_eq!(composed.height(), 4);
            assert_eq!(composed.pixel(0, 1), Ok(expected));
            assert_eq!(composed.pixel(0, 2), Ok(expected));
        }
    }

    #[test]
    fn composition_copies_each_image_without_scaling() {
        let red = image(2, 2, [0, 0, 255, 255]);
        let green = image(1, 3, [0, 255, 0, 255]);
        let composed = compose(
            &[red, green],
            &options(Layout::SideBySide, 1, Align::Start, Background::Transparent),
        )
        .expect("compose");
        assert_eq!((composed.width(), composed.height()), (4, 3));
        assert_eq!(composed.pixel(0, 0), Ok([0, 0, 255, 255]));
        assert_eq!(composed.pixel(1, 1), Ok([0, 0, 255, 255]));
        assert_eq!(composed.pixel(3, 0), Ok([0, 255, 0, 255]));
        assert_eq!(composed.pixel(3, 2), Ok([0, 255, 0, 255]));
        assert_eq!(composed.pixel(2, 0), Ok([0, 0, 0, 0]));
    }

    #[test]
    fn composition_aligns_stacked_images() {
        let composed = compose(
            &[image(4, 1, [1, 2, 3, 255]), image(2, 1, [4, 5, 6, 255])],
            &options(Layout::Stacked, 0, Align::End, Background::Transparent),
        )
        .expect("compose");
        assert_eq!((composed.width(), composed.height()), (4, 2));
        assert_eq!(composed.pixel(0, 1), Ok([0, 0, 0, 0]));
        assert_eq!(composed.pixel(2, 1), Ok([4, 5, 6, 255]));
        assert_eq!(composed.pixel(3, 1), Ok([4, 5, 6, 255]));
    }

    #[test]
    fn huge_dimensions_are_reported_not_wrapped() {
        assert_eq!(
            layout_sizes(
                &[(u32::MAX, 1), (2, 1)],
                &options(Layout::SideBySide, 0, Align::Start, Background::Transparent)
            ),
            Err(CombineError::Overflow)
        );
        assert_eq!(
            layout_sizes(
                &[(1, u32::MAX), (1, 2)],
                &options(Layout::Stacked, 0, Align::Start, Background::Transparent)
            ),
            Err(CombineError::Overflow)
        );
    }

    #[test]
    fn a_bad_hex_value_fails_before_composition() {
        assert_eq!(
            compose(
                &[image(1, 1, [0, 0, 0, 255])],
                &options(
                    Layout::SideBySide,
                    0,
                    Align::Start,
                    Background::Custom("#xyzxyz".to_owned())
                )
            ),
            Err(CombineError::InvalidHex)
        );
    }
}
