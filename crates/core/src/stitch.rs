use thiserror::Error;

use crate::image::{Image, ImageError};

pub const MIN_OVERLAP: u32 = 20;
pub const DEFAULT_MAX_HEIGHT: u32 = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StitchOptions {
    pub remove_sticky: bool,
    pub min_overlap: u32,
    pub max_height: u32,
}

impl Default for StitchOptions {
    fn default() -> Self {
        Self {
            remove_sticky: true,
            min_overlap: MIN_OVERLAP,
            max_height: DEFAULT_MAX_HEIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    CapturesEnded,
    Duplicate,
    MaxHeight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StitchMeta {
    pub offsets: Vec<u32>,
    pub stop: StopReason,
    pub sticky_top: u32,
    pub sticky_bottom: u32,
    pub frames_used: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stitched {
    pub image: Image,
    pub meta: StitchMeta,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StitchError {
    #[error("There is nothing to stitch.")]
    NoCaptures,
    #[error("The captures must all be the same width.")]
    WidthMismatch,
    #[error("The stitched image could not be built.")]
    Image,
}

impl From<ImageError> for StitchError {
    fn from(_: ImageError) -> Self {
        Self::Image
    }
}

struct Content {
    rows: Vec<u8>,
    hashes: Vec<u64>,
}

impl Content {
    fn from_image(image: &Image, top: u32, bottom: u32) -> Self {
        let end = image.height().saturating_sub(bottom);
        let mut rows = Vec::new();
        let mut hashes = Vec::new();
        for y in top..end {
            if let Some(row) = image.row(y) {
                hashes.push(row_hash(row));
                rows.extend_from_slice(row);
            }
        }
        Self { rows, hashes }
    }

    fn height(&self) -> u32 {
        u32::try_from(self.hashes.len()).unwrap_or(0)
    }
}

pub fn stitch(frames: &[Image], options: &StitchOptions) -> Result<Stitched, StitchError> {
    let first = frames.first().ok_or(StitchError::NoCaptures)?;
    let width = first.width();
    if frames.iter().any(|frame| frame.width() != width) {
        return Err(StitchError::WidthMismatch);
    }
    let row_len = width as usize * 4;
    let max_height = options.max_height.max(1);
    let (sticky_top, sticky_bottom) = if options.remove_sticky && frames.len() > 1 {
        sticky_bands(frames)
    } else {
        (0, 0)
    };

    let mut stop = StopReason::CapturesEnded;
    let mut used = 1_usize;
    let first_content = Content::from_image(first, sticky_top, sticky_bottom);
    let mut rows = first_content.rows;
    let mut hashes = first_content.hashes;
    let mut offsets = vec![sticky_top];

    for (index, frame) in frames.iter().enumerate().skip(1) {
        if frame == &frames[index - 1] {
            stop = StopReason::Duplicate;
            break;
        }
        let content = Content::from_image(frame, sticky_top, sticky_bottom);
        let canvas_height = u32::try_from(hashes.len()).unwrap_or(0);
        let content_height = content.height();
        let max_overlap = content_height
            .saturating_sub(1)
            .min(canvas_height);
        let overlap = find_overlap(
            &rows,
            &hashes,
            &content.rows,
            &content.hashes,
            row_len,
            options.min_overlap,
            max_overlap,
        );
        offsets.push(sticky_top + canvas_height.saturating_sub(overlap));
        let start = overlap as usize * row_len;
        rows.extend_from_slice(&content.rows[start..]);
        hashes.extend_from_slice(&content.hashes[overlap as usize..]);
        used += 1;
        let total = sticky_top
            .saturating_add(u32::try_from(hashes.len()).unwrap_or(0))
            .saturating_add(sticky_bottom);
        if total >= max_height {
            stop = StopReason::MaxHeight;
            break;
        }
    }

    let mut output = Vec::new();
    for y in 0..sticky_top {
        if let Some(row) = first.row(y) {
            output.extend_from_slice(row);
        }
    }
    output.extend_from_slice(&rows);
    let last = &frames[used - 1];
    if sticky_bottom > 0 {
        let start = last.height().saturating_sub(sticky_bottom);
        for y in start..last.height() {
            if let Some(row) = last.row(y) {
                output.extend_from_slice(row);
            }
        }
    }

    let mut height = u32::try_from(output.len() / row_len).unwrap_or(0);
    if height > max_height {
        output.truncate(max_height as usize * row_len);
        height = max_height;
        stop = StopReason::MaxHeight;
    }
    if height == 0 || output.len() < height as usize * row_len {
        return Err(StitchError::Image);
    }

    let image = Image::new(width, height, row_len, output)?;
    Ok(Stitched {
        image,
        meta: StitchMeta {
            offsets,
            stop,
            sticky_top,
            sticky_bottom,
            frames_used: used,
        },
    })
}

fn sticky_bands(frames: &[Image]) -> (u32, u32) {
    let min_height = frames
        .iter()
        .map(Image::height)
        .min()
        .unwrap_or_default();
    let mut top = 0_u32;
    while top < min_height {
        let Some(row) = frames[0].row(top) else {
            break;
        };
        if frames[1..]
            .iter()
            .all(|frame| frame.row(top) == Some(row))
        {
            top += 1;
        } else {
            break;
        }
    }
    let mut bottom = 0_u32;
    while top.saturating_add(bottom) < min_height {
        let y = min_height - 1 - bottom;
        let Some(row) = frames[0].row(y) else {
            break;
        };
        if frames[1..]
            .iter()
            .all(|frame| frame.row(y) == Some(row))
        {
            bottom += 1;
        } else {
            break;
        }
    }
    (top, bottom)
}

fn find_overlap(
    canvas: &[u8],
    canvas_hashes: &[u64],
    content: &[u8],
    content_hashes: &[u64],
    row_len: usize,
    min_overlap: u32,
    max_overlap: u32,
) -> u32 {
    if min_overlap == 0 || max_overlap < min_overlap {
        return 0;
    }
    let mut overlap = max_overlap;
    loop {
        let canvas_start = canvas_hashes.len().saturating_sub(overlap as usize);
        if canvas_hashes[canvas_start..] == content_hashes[..overlap as usize]
            && canvas[canvas_start * row_len..] == content[..overlap as usize * row_len]
        {
            return overlap;
        }
        if overlap == min_overlap {
            return 0;
        }
        overlap -= 1;
    }
}

fn row_hash(row: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in row {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(width: u32, height: u32) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                let pixel = [
                    u8::try_from(y % 251).unwrap_or(0),
                    u8::try_from(x % 241).unwrap_or(0),
                    u8::try_from((y / 251) % 239).unwrap_or(0),
                    255,
                ];
                image.set_pixel(x, y, pixel).expect("pixel");
            }
        }
        image
    }

    fn viewport(source: &Image, top: u32, height: u32) -> Image {
        let mut image = Image::bgra(source.width(), height).expect("viewport");
        for y in 0..height {
            if let Some(row) = source.row(top + y) {
                if let Some(target) = image.row_mut(y) {
                    target.copy_from_slice(row);
                }
            }
        }
        image
    }

    fn band_pixel(y: u32, x: u32) -> [u8; 4] {
        [u8::try_from(y % 7).unwrap_or(0), 0, u8::try_from(x % 5).unwrap_or(0), 255]
    }

    fn frame(
        source: &Image,
        top: u32,
        height: u32,
        header: u32,
        footer: u32,
    ) -> Image {
        let mut image = Image::bgra(source.width(), height).expect("frame");
        for y in 0..height {
            for x in 0..source.width() {
                let pixel = if y < header {
                    band_pixel(0, x)
                } else if y >= height.saturating_sub(footer) {
                    band_pixel(1, x)
                } else {
                    source.pixel(x, top + y).expect("pixel")
                };
                image.set_pixel(x, y, pixel).expect("pixel");
            }
        }
        image
    }

    fn no_sticky(frames: Vec<Image>) -> Stitched {
        stitch(
            &frames,
            &StitchOptions {
                remove_sticky: false,
                min_overlap: 5,
                max_height: DEFAULT_MAX_HEIGHT,
            },
        )
        .expect("stitch")
    }

    #[test]
    fn overlapping_rows_are_appended_once() {
        let source = page(8, 120);
        let frames: Vec<Image> = [0_u32, 10, 20]
            .iter()
            .map(|top| viewport(&source, *top, 40))
            .collect();
        let stitched = no_sticky(frames);
        assert_eq!(stitched.image.height(), 60);
        assert_eq!(stitched.meta.offsets, vec![0, 10, 20]);
        assert_eq!(stitched.meta.stop, StopReason::CapturesEnded);
        assert_eq!(stitched.meta.frames_used, 3);
        for y in 0..60 {
            assert_eq!(
                stitched.image.row(y),
                source.row(y),
                "row {y} does not match the page"
            );
        }
    }

    #[test]
    fn no_overlap_appends_the_whole_capture() {
        let source = page(8, 160);
        let frames = vec![
            viewport(&source, 0, 40),
            viewport(&source, 80, 40),
        ];
        let stitched = no_sticky(frames);
        assert_eq!(stitched.image.height(), 80);
        assert_eq!(stitched.meta.offsets, vec![0, 40]);
        assert_eq!(stitched.meta.stop, StopReason::CapturesEnded);
    }

    #[test]
    fn a_duplicate_frame_stops_the_stitch() {
        let source = page(8, 80);
        let repeated = viewport(&source, 0, 40);
        let frames = vec![repeated.clone(), repeated];
        let stitched = no_sticky(frames);
        assert_eq!(stitched.meta.stop, StopReason::Duplicate);
        assert_eq!(stitched.meta.frames_used, 1);
        assert_eq!(stitched.meta.offsets, vec![0]);
        assert_eq!(stitched.image, viewport(&source, 0, 40));
    }

    #[test]
    fn width_mismatch_is_rejected() {
        let frames = vec![page(8, 40), page(9, 40)];
        assert_eq!(
            stitch(&frames, &StitchOptions::default()),
            Err(StitchError::WidthMismatch)
        );
    }

    #[test]
    fn an_empty_capture_list_is_rejected() {
        let frames: Vec<Image> = Vec::new();
        assert_eq!(
            stitch(&frames, &StitchOptions::default()),
            Err(StitchError::NoCaptures)
        );
    }

    #[test]
    fn sticky_headers_and_footers_are_kept_once() {
        let source = page(8, 140);
        let frames: Vec<Image> = [0_u32, 20, 40]
            .iter()
            .map(|top| frame(&source, *top, 40, 4, 3))
            .collect();
        let stitched = stitch(
            &frames,
            &StitchOptions {
                remove_sticky: true,
                min_overlap: 5,
                max_height: DEFAULT_MAX_HEIGHT,
            },
        )
        .expect("stitch");
        assert_eq!(stitched.meta.sticky_top, 4);
        assert_eq!(stitched.meta.sticky_bottom, 3);
        assert_eq!(stitched.meta.frames_used, 3);
        let page_height = 73;
        assert_eq!(stitched.image.height(), 4 + page_height + 3);
        for x in 0..8 {
            assert_eq!(
                stitched.image.pixel(x, 0),
                Ok(band_pixel(0, x)),
                "the header is missing at the top"
            );
            let last = stitched.image.height() - 1;
            assert_eq!(
                stitched.image.pixel(x, last),
                Ok(band_pixel(1, x)),
                "the footer is missing at the bottom"
            );
        }
        for y in 0..page_height {
            assert_eq!(
                stitched.image.row(4 + y),
                source.row(4 + y),
                "content row {y} is wrong"
            );
        }
    }

    #[test]
    fn sticky_removal_off_keeps_every_band() {
        let source = page(8, 140);
        let frames: Vec<Image> = [0_u32, 20]
            .iter()
            .map(|top| frame(&source, *top, 40, 4, 3))
            .collect();
        let stitched = no_sticky(frames);
        assert_eq!(stitched.meta.sticky_top, 0);
        assert_eq!(stitched.meta.sticky_bottom, 0);
        assert_eq!(stitched.image.height(), 80);
    }

    #[test]
    fn max_height_truncates_and_reports_the_stop() {
        let source = page(8, 200);
        let frames: Vec<Image> = [0_u32, 20]
            .iter()
            .map(|top| viewport(&source, *top, 40))
            .collect();
        let stitched = stitch(
            &frames,
            &StitchOptions {
                remove_sticky: false,
                min_overlap: 5,
                max_height: 50,
            },
        )
        .expect("stitch");
        assert_eq!(stitched.image.height(), 50);
        assert_eq!(stitched.meta.stop, StopReason::MaxHeight);
        for y in 0..50 {
            assert_eq!(stitched.image.row(y), source.row(y));
        }
    }

    #[test]
    fn a_single_capture_is_returned_unchanged() {
        let source = page(8, 60);
        let stitched = no_sticky(vec![source.clone()]);
        assert_eq!(stitched.image, source);
        assert_eq!(stitched.meta.offsets, vec![0]);
        assert_eq!(stitched.meta.frames_used, 1);
        assert_eq!(stitched.meta.stop, StopReason::CapturesEnded);
    }

    #[test]
    fn an_overlap_shorter_than_the_minimum_is_not_used() {
        let source = page(8, 100);
        let frames = vec![
            viewport(&source, 20, 40),
            viewport(&source, 56, 40),
        ];
        let stitched = stitch(
            &frames,
            &StitchOptions {
                remove_sticky: false,
                min_overlap: 20,
                max_height: DEFAULT_MAX_HEIGHT,
            },
        )
        .expect("stitch");
        assert_eq!(stitched.image.height(), 80);
        assert_eq!(stitched.meta.offsets, vec![0, 40]);
    }

    #[test]
    fn stitching_is_exact_and_reproducible() {
        let source = page(8, 140);
        let frames: Vec<Image> = [0_u32, 20, 40]
            .iter()
            .map(|top| frame(&source, *top, 40, 4, 3))
            .collect();
        let options = StitchOptions {
            remove_sticky: true,
            min_overlap: 5,
            max_height: DEFAULT_MAX_HEIGHT,
        };
        let first = stitch(&frames, &options).expect("first");
        let second = stitch(&frames, &options).expect("second");
        assert_eq!(first, second);
    }
}
