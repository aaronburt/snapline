use std::fmt::Write as _;

use qrcode::types::QrError as EncodeError;
use qrcode::{Color, EcLevel, QrCode};
use rqrr::PreparedImage;
use thiserror::Error;

use crate::image::Image;

pub const QUIET_ZONE: u32 = 4;
pub const DEFAULT_SCALE: u32 = 8;
pub const MAX_SCALE: u32 = 64;
pub const MAX_QUIET_ZONE: u32 = 16;
pub const MIN_SCAN_SIDE: u32 = 21;
pub const SVG_MODULE_PIXELS: u32 = 4;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum QrError {
    #[error("Enter some text to make a QR code.")]
    EmptyText,
    #[error("The text is too long for a QR code. Shorten it and try again.")]
    TooLong,
    #[error("No QR code was found in the selected area.")]
    NotFound,
    #[error("The selected image is too small to scan. Select a larger area.")]
    UnsupportedImage,
    #[error("The text cannot be encoded as a QR code.")]
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrScan {
    pub text: String,
    pub link: Option<String>,
}

impl QrScan {
    pub fn is_url(&self) -> bool {
        self.link.is_some()
    }
}

pub fn make_svg(text: &str) -> Result<String, QrError> {
    let code = encode(text)?;
    let modules = code.width();
    let side = u32::try_from(modules)
        .map_err(|_| QrError::Invalid)?
        .checked_add(QUIET_ZONE * 2)
        .ok_or(QrError::TooLong)?;
    let pixels = side
        .checked_mul(SVG_MODULE_PIXELS)
        .ok_or(QrError::TooLong)?;
    let colors = code.to_colors();
    let mut path = String::new();
    for y in 0..modules {
        let mut x = 0;
        while x < modules {
            if colors[y * modules + x] == Color::Dark {
                let start = x;
                while x < modules && colors[y * modules + x] == Color::Dark {
                    x += 1;
                }
                let run = x - start;
                let left = u32::try_from(start).map_err(|_| QrError::Invalid)? + QUIET_ZONE;
                let top = u32::try_from(y).map_err(|_| QrError::Invalid)? + QUIET_ZONE;
                write!(path, "M{left} {top}h{run}v1h-{run}z").map_err(|_| QrError::Invalid)?;
            } else {
                x += 1;
            }
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" width=\"{pixels}\" height=\"{pixels}\" shape-rendering=\"crispEdges\" role=\"img\" aria-label=\"QR code\"><rect width=\"{side}\" height=\"{side}\" fill=\"#ffffff\"/><path d=\"{path}\" fill=\"#000000\"/></svg>"
    ))
}

pub fn make_image(text: &str) -> Result<Image, QrError> {
    make_image_with(text, DEFAULT_SCALE, QUIET_ZONE)
}

pub fn make_image_with(text: &str, scale: u32, quiet_zone: u32) -> Result<Image, QrError> {
    let code = encode(text)?;
    let modules = code.width();
    let scale = scale.clamp(1, MAX_SCALE);
    let quiet_zone = quiet_zone.min(MAX_QUIET_ZONE);
    let side = u32::try_from(modules)
        .map_err(|_| QrError::Invalid)?
        .checked_add(quiet_zone * 2)
        .and_then(|value| value.checked_mul(scale))
        .ok_or(QrError::TooLong)?;
    let mut image = Image::bgra(side, side).map_err(|_| QrError::UnsupportedImage)?;
    for pixel in image.pixels_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&[255, 255, 255, 255]);
    }
    let colors = code.to_colors();
    for y in 0..modules {
        for x in 0..modules {
            if colors[y * modules + x] == Color::Dark {
                let left = (u32::try_from(x).map_err(|_| QrError::Invalid)? + quiet_zone) * scale;
                let top = (u32::try_from(y).map_err(|_| QrError::Invalid)? + quiet_zone) * scale;
                fill_block(&mut image, left, top, scale);
            }
        }
    }
    Ok(image)
}

pub fn scan(image: &Image) -> Result<QrScan, QrError> {
    if image.width() < MIN_SCAN_SIDE || image.height() < MIN_SCAN_SIDE {
        return Err(QrError::UnsupportedImage);
    }
    let width = usize::try_from(image.width()).map_err(|_| QrError::UnsupportedImage)?;
    let height = usize::try_from(image.height()).map_err(|_| QrError::UnsupportedImage)?;
    let stride = image.stride();
    let pixels = image.pixels();
    let mut prepared = PreparedImage::prepare_from_greyscale(width, height, |x, y| {
        let offset = y * stride + x * 4;
        let blue = u32::from(pixels[offset]);
        let green = u32::from(pixels[offset + 1]);
        let red = u32::from(pixels[offset + 2]);
        let alpha = u32::from(pixels[offset + 3]);
        let luma = (77 * red + 150 * green + 29 * blue + 128) >> 8;
        let blended = (luma * alpha + 255 * (255 - alpha)) / 255;
        u8::try_from(blended).unwrap_or(255)
    });
    for grid in prepared.detect_grids() {
        if let Ok((_meta, text)) = grid.decode() {
            if !text.is_empty() {
                return Ok(QrScan {
                    link: link_of(&text),
                    text,
                });
            }
        }
    }
    Err(QrError::NotFound)
}

pub fn link_of(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.chars().any(|character| character.is_whitespace()) {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let https = lower
        .strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty());
    let http = lower
        .strip_prefix("http://")
        .is_some_and(|rest| !rest.is_empty());
    if https || http {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

fn encode(text: &str) -> Result<QrCode, QrError> {
    if text.is_empty() {
        return Err(QrError::EmptyText);
    }
    QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).map_err(|error| match error {
        EncodeError::DataTooLong => QrError::TooLong,
        _ => QrError::Invalid,
    })
}

fn fill_block(image: &mut Image, left: u32, top: u32, size: u32) {
    for y in top..top.saturating_add(size) {
        if let Some(row) = image.row_mut(y) {
            for x in left..left.saturating_add(size) {
                let offset = usize::try_from(x).unwrap_or(0) * 4;
                if let Some(pixel) = row.get_mut(offset..offset + 4) {
                    pixel.copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(width: u32, height: u32) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for pixel in image.pixels_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
        image
    }

    #[test]
    fn svg_has_a_quiet_zone_white_background_and_black_modules() {
        let svg = make_svg("hello").expect("svg");
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains("viewBox=\"0 0 29 29\""));
        assert!(svg.contains("<rect width=\"29\" height=\"29\" fill=\"#ffffff\"/>"));
        assert!(svg.contains("fill=\"#000000\""));
        assert!(svg.contains("aria-label=\"QR code\""));
    }

    #[test]
    fn empty_text_is_rejected() {
        assert_eq!(make_svg(""), Err(QrError::EmptyText));
        assert_eq!(make_image(""), Err(QrError::EmptyText));
    }

    #[test]
    fn oversized_text_is_rejected() {
        let text = "a".repeat(5_000);
        assert_eq!(make_svg(&text), Err(QrError::TooLong));
    }

    #[test]
    fn source_image_is_black_and_white_bgra() {
        let image = make_image("hello").expect("image");
        assert_eq!(image.width(), image.height());
        assert_eq!(image.pixel(0, 0), Ok([255, 255, 255, 255]));
        let dark = image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel == [0, 0, 0, 255]);
        assert!(dark);
    }

    #[test]
    fn make_and_scan_round_trip_text() {
        let image = make_image("Snapline QR").expect("image");
        let scanned = scan(&image).expect("scan");
        assert_eq!(scanned.text, "Snapline QR");
        assert_eq!(scanned.link, None);
        assert!(!scanned.is_url());
    }

    #[test]
    fn make_and_scan_round_trip_a_url() {
        let image = make_image("https://example.com/snapline").expect("image");
        let scanned = scan(&image).expect("scan");
        assert_eq!(scanned.text, "https://example.com/snapline");
        assert_eq!(
            scanned.link.as_deref(),
            Some("https://example.com/snapline")
        );
        assert!(scanned.is_url());
    }

    #[test]
    fn scaled_images_still_scan() {
        let image = make_image_with("scale test", 4, 4).expect("image");
        assert_eq!(scan(&image).expect("scan").text, "scale test");
    }

    #[test]
    fn a_blank_image_has_no_code() {
        let image = blank(120, 120);
        assert_eq!(scan(&image), Err(QrError::NotFound));
    }

    #[test]
    fn a_tiny_image_is_unsupported() {
        let image = blank(10, 10);
        assert_eq!(scan(&image), Err(QrError::UnsupportedImage));
    }

    #[test]
    fn links_must_be_http_or_https() {
        assert_eq!(
            link_of("https://example.com").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            link_of("HTTP://EXAMPLE.COM").as_deref(),
            Some("HTTP://EXAMPLE.COM")
        );
        assert_eq!(link_of("https://"), None);
        assert_eq!(link_of("ftp://example.com"), None);
        assert_eq!(link_of("example.com"), None);
        assert_eq!(link_of("https://example.com/a b"), None);
    }

    #[test]
    fn links_are_trimmed() {
        assert_eq!(
            link_of("  https://example.com  ").as_deref(),
            Some("https://example.com")
        );
    }
}
