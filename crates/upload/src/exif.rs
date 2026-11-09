use std::fmt;

use img_parts::{
    jpeg::{markers, Jpeg, JpegSegment},
    png::Png,
    Bytes, ImageEXIF,
};
use thiserror::Error;

const JPEG_SIGNATURE: &[u8] = &[0xFF, 0xD8];
const PNG_SIGNATURE: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

const EXIF_PREFIX: &[u8] = b"Exif\0\0";
const XMP_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_EXTENSION_PREFIX: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

const PNG_EXIF: [u8; 4] = *b"eXIf";
const PNG_TEXT: [u8; 4] = *b"tEXt";
const PNG_COMPRESSED_TEXT: [u8; 4] = *b"zTXt";
const PNG_INTERNATIONAL_TEXT: [u8; 4] = *b"iTXt";

const ORIENTATION_TAG: u16 = 0x0112;
const SHORT_TYPE: u16 = 3;
const TIFF_MAGIC: u16 = 42;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
}

impl fmt::Display for ImageKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExifError {
    #[error("unsupported image format")]
    UnsupportedFormat,
    #[error("malformed {0} image")]
    Malformed(ImageKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Normal,
    MirrorHorizontal,
    Rotate180,
    MirrorVertical,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    #[must_use]
    pub const fn from_value(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Normal),
            2 => Some(Self::MirrorHorizontal),
            3 => Some(Self::Rotate180),
            4 => Some(Self::MirrorVertical),
            5 => Some(Self::Transpose),
            6 => Some(Self::Rotate90),
            7 => Some(Self::Transverse),
            8 => Some(Self::Rotate270),
            _ => None,
        }
    }

    #[must_use]
    pub const fn value(self) -> u16 {
        match self {
            Self::Normal => 1,
            Self::MirrorHorizontal => 2,
            Self::Rotate180 => 3,
            Self::MirrorVertical => 4,
            Self::Transpose => 5,
            Self::Rotate90 => 6,
            Self::Transverse => 7,
            Self::Rotate270 => 8,
        }
    }

    #[must_use]
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripReport {
    pub bytes: Vec<u8>,
    pub format: ImageKind,
    pub orientation: Option<Orientation>,
}

pub fn strip(bytes: &[u8]) -> Result<Vec<u8>, ExifError> {
    strip_with_report(bytes).map(|report| report.bytes)
}

pub fn strip_with_report(bytes: &[u8]) -> Result<StripReport, ExifError> {
    if bytes.starts_with(PNG_SIGNATURE) {
        strip_png(bytes)
    } else if bytes.starts_with(JPEG_SIGNATURE) {
        strip_jpeg(bytes)
    } else {
        Err(ExifError::UnsupportedFormat)
    }
}

fn strip_jpeg(bytes: &[u8]) -> Result<StripReport, ExifError> {
    let mut jpeg = Jpeg::from_bytes(Bytes::copy_from_slice(bytes))
        .map_err(|_| ExifError::Malformed(ImageKind::Jpeg))?;
    let orientation = jpeg.exif().and_then(|exif| tiff_orientation(&exif));
    jpeg.set_exif(None);
    jpeg.segments_mut()
        .retain(|segment| !is_private_app1(segment));
    Ok(StripReport {
        bytes: jpeg.encoder().bytes().to_vec(),
        format: ImageKind::Jpeg,
        orientation,
    })
}

fn is_private_app1(segment: &JpegSegment) -> bool {
    segment.marker() == markers::APP1
        && (segment.contents().starts_with(XMP_PREFIX)
            || segment.contents().starts_with(XMP_EXTENSION_PREFIX)
            || segment.contents().starts_with(EXIF_PREFIX))
}

fn tiff_orientation(tiff: &[u8]) -> Option<Orientation> {
    let little_endian = if tiff.starts_with(b"II") {
        true
    } else if tiff.starts_with(b"MM") {
        false
    } else {
        return None;
    };
    if read_u16(tiff, 2, little_endian)? != TIFF_MAGIC {
        return None;
    }
    let ifd = usize::try_from(read_u32(tiff, 4, little_endian)?).ok()?;
    let count = usize::from(read_u16(tiff, ifd, little_endian)?);
    for index in 0..count {
        let entry = ifd.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
        if read_u16(tiff, entry, little_endian)? != ORIENTATION_TAG {
            continue;
        }
        if read_u16(tiff, entry.checked_add(2)?, little_endian)? != SHORT_TYPE {
            return None;
        }
        if read_u32(tiff, entry.checked_add(4)?, little_endian)? != 1 {
            return None;
        }
        let raw = read_u16(tiff, entry.checked_add(8)?, little_endian)?;
        return Orientation::from_value(raw);
    }
    None
}

fn strip_png(bytes: &[u8]) -> Result<StripReport, ExifError> {
    let mut png = Png::from_bytes(Bytes::copy_from_slice(bytes))
        .map_err(|_| ExifError::Malformed(ImageKind::Png))?;
    let orientation = png
        .chunk_by_type(PNG_EXIF)
        .and_then(|chunk| tiff_orientation(chunk.contents()));
    for kind in [
        PNG_EXIF,
        PNG_TEXT,
        PNG_COMPRESSED_TEXT,
        PNG_INTERNATIONAL_TEXT,
    ] {
        png.remove_chunks_by_type(kind);
    }
    Ok(StripReport {
        bytes: png.encoder().bytes().to_vec(),
        format: ImageKind::Png,
        orientation,
    })
}

fn read_u16(bytes: &[u8], offset: usize, little_endian: bool) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let pair: [u8; 2] = bytes.get(offset..end)?.try_into().ok()?;
    Some(if little_endian {
        u16::from_le_bytes(pair)
    } else {
        u16::from_be_bytes(pair)
    })
}

fn read_u32(bytes: &[u8], offset: usize, little_endian: bool) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let quad: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(if little_endian {
        u32::from_le_bytes(quad)
    } else {
        u32::from_be_bytes(quad)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_segment(marker: u8, contents: &[u8]) -> Vec<u8> {
        let mut segment = vec![0xFF, marker];
        let length = u16::try_from(contents.len() + 2).expect("segment fits");
        segment.extend_from_slice(&length.to_be_bytes());
        segment.extend_from_slice(contents);
        segment
    }

    fn tiff_with_orientation(value: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&TIFF_MAGIC.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&ORIENTATION_TAG.to_le_bytes());
        tiff.extend_from_slice(&SHORT_TYPE.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&value.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff
    }

    fn jpeg_fixture(orientation: Option<u16>) -> Vec<u8> {
        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(JPEG_SIGNATURE);
        if let Some(value) = orientation {
            let mut exif = EXIF_PREFIX.to_vec();
            exif.extend_from_slice(&tiff_with_orientation(value));
            jpeg.extend_from_slice(&jpeg_segment(markers::APP1, &exif));
        }
        let mut xmp = XMP_PREFIX.to_vec();
        xmp.extend_from_slice(b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>");
        jpeg.extend_from_slice(&jpeg_segment(markers::APP1, &xmp));
        jpeg.extend_from_slice(&jpeg_segment(markers::APP1, b"Other\0vendor"));
        let mut icc = b"ICC_PROFILE\0".to_vec();
        icc.extend_from_slice(&[1, 1]);
        icc.extend_from_slice(b"profile bytes");
        jpeg.extend_from_slice(&jpeg_segment(markers::APP2, &icc));
        jpeg.extend_from_slice(&jpeg_segment(markers::COM, b"a comment"));
        jpeg.extend_from_slice(&jpeg_segment(markers::DQT, &[0]));
        jpeg.extend_from_slice(&jpeg_segment(
            markers::SOF0,
            &[8, 0, 1, 0, 1, 3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1],
        ));
        jpeg.extend_from_slice(&jpeg_segment(markers::SOS, &[1, 3, 0, 1, 2, 0, 3, 0]));
        jpeg.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0xFF, 0x00, 0x9A, 0xFF, 0xD9]);
        jpeg
    }

    fn scan_from(bytes: &[u8]) -> &[u8] {
        let position = bytes
            .windows(2)
            .position(|window| window[0] == 0xFF && window[1] == markers::SOS)
            .expect("scan marker");
        &bytes[position..]
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn png_chunk(kind: [u8; 4], contents: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        let length = u32::try_from(contents.len()).expect("chunk fits");
        chunk.extend_from_slice(&length.to_be_bytes());
        let mut checked = Vec::new();
        checked.extend_from_slice(&kind);
        checked.extend_from_slice(contents);
        chunk.extend_from_slice(&kind);
        chunk.extend_from_slice(contents);
        chunk.extend_from_slice(&crc32(&checked).to_be_bytes());
        chunk
    }

    fn png_fixture(orientation: Option<u16>) -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(PNG_SIGNATURE);
        png.extend_from_slice(&png_chunk(
            *b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
        ));
        png.extend_from_slice(&png_chunk(*b"gAMA", &[0, 0, 0, 1]));
        png.extend_from_slice(&png_chunk(*b"iCCP", b"icc\0\0profile bytes"));
        if let Some(value) = orientation {
            png.extend_from_slice(&png_chunk(PNG_EXIF, &tiff_with_orientation(value)));
        }
        png.extend_from_slice(&png_chunk(PNG_TEXT, b"Comment\0latitude 51.5"));
        png.extend_from_slice(&png_chunk(
            PNG_COMPRESSED_TEXT,
            b"Raw profile type exif\0\0zz",
        ));
        png.extend_from_slice(&png_chunk(
            PNG_INTERNATIONAL_TEXT,
            b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta/>",
        ));
        png.extend_from_slice(&png_chunk(*b"IDAT", &[0x78, 0x9C, 0x01, 0x02, 0x03]));
        png.extend_from_slice(&png_chunk(*b"IEND", &[]));
        png
    }

    fn chunk_contents(png: &Png, kind: [u8; 4]) -> Vec<u8> {
        let mut contents = Vec::new();
        for chunk in png.chunks_by_type(kind) {
            contents.extend_from_slice(chunk.contents());
        }
        contents
    }

    #[test]
    fn jpeg_removes_exif_and_xmp() {
        let input = jpeg_fixture(Some(6));
        let report = strip_with_report(&input).expect("jpeg strips");
        assert_eq!(report.format, ImageKind::Jpeg);
        let stripped = Jpeg::from_bytes(Bytes::copy_from_slice(&report.bytes)).expect("valid jpeg");
        assert!(stripped.segments_by_marker(markers::APP1).all(|segment| {
            !segment.contents().starts_with(EXIF_PREFIX)
                && !segment.contents().starts_with(XMP_PREFIX)
        }));
        assert_eq!(stripped.segments_by_marker(markers::APP1).count(), 1);
    }

    #[test]
    fn jpeg_keeps_icc_and_comments() {
        let report = strip_with_report(&jpeg_fixture(None)).expect("jpeg strips");
        let stripped = Jpeg::from_bytes(Bytes::copy_from_slice(&report.bytes)).expect("valid jpeg");
        assert!(stripped.segment_by_marker(markers::APP2).is_some());
        assert!(stripped.segment_by_marker(markers::COM).is_some());
    }

    #[test]
    fn jpeg_keeps_scan_data_identical() {
        let input = jpeg_fixture(Some(6));
        let stripped = strip(&input).expect("jpeg strips");
        assert_eq!(scan_from(&stripped), scan_from(&input));
    }

    #[test]
    fn jpeg_reports_orientation() {
        let report = strip_with_report(&jpeg_fixture(Some(6))).expect("jpeg strips");
        assert_eq!(report.orientation, Some(Orientation::Rotate90));
    }

    #[test]
    fn jpeg_reports_normal_orientation() {
        let report = strip_with_report(&jpeg_fixture(Some(1))).expect("jpeg strips");
        assert_eq!(report.orientation, Some(Orientation::Normal));
    }

    #[test]
    fn jpeg_without_exif_has_no_orientation() {
        let report = strip_with_report(&jpeg_fixture(None)).expect("jpeg strips");
        assert_eq!(report.orientation, None);
    }

    #[test]
    fn png_removes_exif_and_text_chunks() {
        let input = png_fixture(Some(6));
        let report = strip_with_report(&input).expect("png strips");
        assert_eq!(report.format, ImageKind::Png);
        let stripped = Png::from_bytes(Bytes::copy_from_slice(&report.bytes)).expect("valid png");
        assert!(stripped.chunk_by_type(PNG_EXIF).is_none());
        assert!(stripped.chunk_by_type(PNG_TEXT).is_none());
        assert!(stripped.chunk_by_type(PNG_COMPRESSED_TEXT).is_none());
        assert!(stripped.chunk_by_type(PNG_INTERNATIONAL_TEXT).is_none());
    }

    #[test]
    fn png_keeps_colour_chunks() {
        let report = strip_with_report(&png_fixture(None)).expect("png strips");
        let stripped = Png::from_bytes(Bytes::copy_from_slice(&report.bytes)).expect("valid png");
        assert!(stripped.chunk_by_type(*b"iCCP").is_some());
        assert!(stripped.chunk_by_type(*b"gAMA").is_some());
    }

    #[test]
    fn png_keeps_pixel_data_identical() {
        let input = png_fixture(Some(6));
        let stripped = strip(&input).expect("png strips");
        let original = Png::from_bytes(Bytes::copy_from_slice(&input)).expect("valid png");
        let result = Png::from_bytes(Bytes::copy_from_slice(&stripped)).expect("valid png");
        assert_eq!(
            chunk_contents(&result, *b"IDAT"),
            chunk_contents(&original, *b"IDAT")
        );
    }

    #[test]
    fn png_reports_orientation() {
        let report = strip_with_report(&png_fixture(Some(8))).expect("png strips");
        assert_eq!(report.orientation, Some(Orientation::Rotate270));
    }

    #[test]
    fn png_without_exif_has_no_orientation() {
        let report = strip_with_report(&png_fixture(None)).expect("png strips");
        assert_eq!(report.orientation, None);
    }

    #[test]
    fn unsupported_formats_are_typed() {
        assert_eq!(
            strip_with_report(b"GIF89a\x01\x00\x01\x00"),
            Err(ExifError::UnsupportedFormat)
        );
        assert_eq!(strip(b""), Err(ExifError::UnsupportedFormat));
    }

    #[test]
    fn truncated_images_are_typed() {
        let mut jpeg = JPEG_SIGNATURE.to_vec();
        jpeg.extend_from_slice(&[0xFF, markers::APP1]);
        assert_eq!(
            strip_with_report(&jpeg),
            Err(ExifError::Malformed(ImageKind::Jpeg))
        );
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(
            strip_with_report(&png),
            Err(ExifError::Malformed(ImageKind::Png))
        );
    }

    #[test]
    fn orientation_values_round_trip() {
        for value in 1..=8 {
            let orientation = Orientation::from_value(value).expect("known orientation");
            assert_eq!(orientation.value(), value);
        }
        assert_eq!(Orientation::from_value(0), None);
        assert_eq!(Orientation::from_value(9), None);
        assert!(Orientation::from_value(1).expect("normal").is_normal());
        assert!(!Orientation::from_value(6).expect("rotated").is_normal());
    }

    #[test]
    fn big_endian_tiff_is_read() {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"MM");
        tiff.extend_from_slice(&TIFF_MAGIC.to_be_bytes());
        tiff.extend_from_slice(&8u32.to_be_bytes());
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend_from_slice(&ORIENTATION_TAG.to_be_bytes());
        tiff.extend_from_slice(&SHORT_TYPE.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        tiff.extend_from_slice(&6u16.to_be_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(tiff_orientation(&tiff), Some(Orientation::Rotate90));
    }
}
