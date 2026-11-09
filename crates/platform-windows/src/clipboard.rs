use std::mem::{forget, size_of};
use std::ptr;
use std::time::Duration;

use snapline_core::image::{Image, ImageFormat};
use thiserror::Error;
use windows::core::{w, Error};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{BITMAPV5HEADER, BI_BITFIELDS, BI_RGB};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{CF_DIBV5, CF_UNICODETEXT};

pub const PNG_FORMAT_NAME: &str = "PNG";
pub const OPEN_ATTEMPTS: u32 = 10;
pub const OPEN_RETRY_DELAY: Duration = Duration::from_millis(10);

const DIBV5_RED_MASK: u32 = 0x00ff_0000;
const DIBV5_GREEN_MASK: u32 = 0x0000_ff00;
const DIBV5_BLUE_MASK: u32 = 0x0000_00ff;
const DIBV5_ALPHA_MASK: u32 = 0xff00_0000;
const LCS_SRGB: u32 = 0x7352_4742;
const LCS_GM_IMAGES: u32 = 4;

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("The clipboard is busy. Try again.")]
    Busy,
    #[error("The clipboard could not be cleared.")]
    Empty(#[source] Error),
    #[error("The clipboard format could not be registered.")]
    Format(#[source] Error),
    #[error("The clipboard memory could not be allocated.")]
    Allocate(#[source] Error),
    #[error("The clipboard data could not be stored.")]
    Store(#[source] Error),
    #[error("The clipboard data could not be read.")]
    Read(#[source] Error),
    #[error("The clipboard image is not in a supported format.")]
    Unsupported,
    #[error("The clipboard image is malformed.")]
    Malformed,
}

pub type PngEncode = fn(&Image) -> Result<Vec<u8>, ClipboardError>;
pub type PngDecode = fn(&[u8]) -> Result<Image, ClipboardError>;

#[derive(Clone, Copy, Debug)]
pub struct PngCodec {
    encode: PngEncode,
    decode: PngDecode,
}

impl PngCodec {
    pub const fn new(encode: PngEncode, decode: PngDecode) -> Self {
        Self { encode, decode }
    }

    fn encode(self, image: &Image) -> Result<Vec<u8>, ClipboardError> {
        (self.encode)(image)
    }

    fn decode(self, bytes: &[u8]) -> Result<Image, ClipboardError> {
        (self.decode)(bytes)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Clipboard {
    png: Option<PngCodec>,
}

impl Clipboard {
    pub const fn new() -> Self {
        Self {
            png: Some(PngCodec::new(wic_png_encode, wic_png_decode)),
        }
    }

    pub const fn without_png() -> Self {
        Self { png: None }
    }

    pub const fn with_png_codec(codec: PngCodec) -> Self {
        Self { png: Some(codec) }
    }

    pub fn set_png_codec(&mut self, codec: PngCodec) {
        self.png = Some(codec);
    }

    pub const fn has_png_codec(&self) -> bool {
        self.png.is_some()
    }

    pub fn set_image(&self, image: &Image) -> Result<(), ClipboardError> {
        let dib = dibv5_bytes(image)?;
        let png = match self.png {
            Some(codec) => match codec.encode(image) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    tracing::warn!(error = %error, "clipboard PNG encoding failed");
                    None
                }
            },
            None => None,
        };
        let png_format = match &png {
            Some(_) => Some(png_format_id()?),
            None => None,
        };
        let _session = ClipboardSession::open()?;
        // SAFETY: the clipboard is open on this thread and is cleared before any data is stored.
        unsafe { EmptyClipboard() }.map_err(ClipboardError::Empty)?;
        let result = (|| {
            if let (Some(format), Some(bytes)) = (png_format, png.as_deref()) {
                store_blob(format, bytes)?;
            }
            store_blob(dib_format(), &dib)
        })();
        if result.is_err() {
            // SAFETY: the clipboard is still open on this thread; clearing removes any partial data.
            unsafe {
                let _ = EmptyClipboard();
            }
        }
        result
    }

    pub fn set_text(&self, text: &str) -> Result<(), ClipboardError> {
        let bytes = utf16_bytes(text);
        let _session = ClipboardSession::open()?;
        // SAFETY: the clipboard is open on this thread and is cleared before the text is stored.
        unsafe { EmptyClipboard() }.map_err(ClipboardError::Empty)?;
        let result = store_blob(text_format(), &bytes);
        if result.is_err() {
            // SAFETY: the clipboard is still open on this thread; clearing removes any partial data.
            unsafe {
                let _ = EmptyClipboard();
            }
        }
        result
    }

    pub fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
        let _session = ClipboardSession::open()?;
        let mut png_error = None;
        if let Ok(format) = png_format_id() {
            if available(format) {
                match self.png {
                    Some(codec) => match read_blob(format).and_then(|bytes| codec.decode(&bytes)) {
                        Ok(image) => return Ok(Some(image)),
                        Err(error) => png_error = Some(error),
                    },
                    None => png_error = Some(ClipboardError::Unsupported),
                }
            }
        }
        if available(dib_format()) {
            return read_blob(dib_format())
                .and_then(|bytes| image_from_dibv5(&bytes))
                .map(Some);
        }
        match png_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

fn wic_png_encode(image: &Image) -> Result<Vec<u8>, ClipboardError> {
    let codec = crate::wic::WicCodec::new().map_err(|_| ClipboardError::Unsupported)?;
    codec
        .encode(image, ImageFormat::Png, 100)
        .map_err(|_| ClipboardError::Unsupported)
}

fn wic_png_decode(bytes: &[u8]) -> Result<Image, ClipboardError> {
    let codec = crate::wic::WicCodec::new().map_err(|_| ClipboardError::Unsupported)?;
    codec.decode(bytes).map_err(|_| ClipboardError::Malformed)
}

struct ClipboardSession;

impl ClipboardSession {
    fn open() -> Result<Self, ClipboardError> {
        open_with_retry(|| {
            // SAFETY: a null owner opens the clipboard for the calling task and has no pointer arguments.
            unsafe { OpenClipboard(None) }.is_ok()
        })
        .map(|()| Self)
    }
}

impl Drop for ClipboardSession {
    fn drop(&mut self) {
        // SAFETY: this session opened the clipboard on the current thread and closes it exactly once.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn open_with_retry<F>(mut open: F) -> Result<(), ClipboardError>
where
    F: FnMut() -> bool,
{
    for attempt in 0..OPEN_ATTEMPTS {
        if open() {
            return Ok(());
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            std::thread::sleep(OPEN_RETRY_DELAY);
        }
    }
    Err(ClipboardError::Busy)
}

struct GlobalBuffer {
    handle: HGLOBAL,
}

impl GlobalBuffer {
    fn from_bytes(bytes: &[u8]) -> Result<Self, ClipboardError> {
        let length = bytes.len().max(1);
        // SAFETY: the size is non-zero and the returned handle is owned by the buffer until it is dropped or transferred.
        let handle =
            unsafe { GlobalAlloc(GMEM_MOVEABLE, length) }.map_err(ClipboardError::Allocate)?;
        let buffer = Self { handle };
        // SAFETY: the handle is live and owned by the buffer, which releases it if this call fails.
        let pointer = unsafe { GlobalLock(buffer.handle) };
        if pointer.is_null() {
            return Err(ClipboardError::Allocate(Error::from_thread()));
        }
        // SAFETY: the locked block is at least `length` bytes and `bytes` is a live slice of the same length.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(buffer.handle);
        }
        Ok(buffer)
    }
}

impl Drop for GlobalBuffer {
    fn drop(&mut self) {
        // SAFETY: the buffer still owns this allocation, which has not been transferred to the clipboard, so it is freed once here.
        unsafe {
            let _ = GlobalFree(Some(self.handle));
        }
    }
}

fn store_blob(format: u32, bytes: &[u8]) -> Result<(), ClipboardError> {
    let buffer = GlobalBuffer::from_bytes(bytes)?;
    // SAFETY: the clipboard is open and the allocation is unlocked; on success the clipboard takes ownership of the handle.
    unsafe { SetClipboardData(format, Some(HANDLE(buffer.handle.0))) }
        .map_err(ClipboardError::Store)?;
    forget(buffer);
    Ok(())
}

fn read_blob(format: u32) -> Result<Vec<u8>, ClipboardError> {
    // SAFETY: the clipboard is open on this thread and the format is a valid clipboard format.
    let handle = unsafe { GetClipboardData(format) }.map_err(ClipboardError::Read)?;
    let global = HGLOBAL(handle.0);
    // SAFETY: the handle belongs to the clipboard for the duration of the open session and is locked once here.
    let pointer = unsafe { GlobalLock(global) };
    if pointer.is_null() {
        return Err(ClipboardError::Read(Error::from_thread()));
    }
    // SAFETY: the pointer refers to a locked clipboard allocation whose size is reported by GlobalSize.
    let size = unsafe { GlobalSize(global) };
    let mut bytes = vec![0_u8; size];
    if size > 0 {
        // SAFETY: the locked block holds `size` readable bytes and the vector holds `size` writable bytes.
        unsafe {
            ptr::copy_nonoverlapping(pointer.cast::<u8>(), bytes.as_mut_ptr(), size);
        }
    }
    // SAFETY: the block was locked exactly once above and is released exactly once here; it stays owned by the clipboard.
    unsafe {
        let _ = GlobalUnlock(global);
    }
    Ok(bytes)
}

fn available(format: u32) -> bool {
    // SAFETY: the format is a valid clipboard format and the call has no pointer arguments.
    unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

fn png_format_id() -> Result<u32, ClipboardError> {
    // SAFETY: the format name is a static null-terminated UTF-16 string.
    let format = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    if format == 0 {
        Err(ClipboardError::Format(Error::from_thread()))
    } else {
        Ok(format)
    }
}

fn dib_format() -> u32 {
    u32::from(CF_DIBV5.0)
}

fn text_format() -> u32 {
    u32::from(CF_UNICODETEXT.0)
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((text.len() + 1) * 2);
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn dibv5_bytes(image: &Image) -> Result<Vec<u8>, ClipboardError> {
    let width = i32::try_from(image.width()).map_err(|_| ClipboardError::Malformed)?;
    let height = i32::try_from(image.height()).map_err(|_| ClipboardError::Malformed)?;
    let stride = image
        .width()
        .checked_mul(4)
        .ok_or(ClipboardError::Malformed)?;
    let pixel_bytes = usize::try_from(stride)
        .ok()
        .and_then(|stride| {
            usize::try_from(image.height())
                .ok()
                .and_then(|height| stride.checked_mul(height))
        })
        .ok_or(ClipboardError::Malformed)?;
    let header_size =
        u32::try_from(size_of::<BITMAPV5HEADER>()).map_err(|_| ClipboardError::Malformed)?;
    let total = size_of::<BITMAPV5HEADER>()
        .checked_add(pixel_bytes)
        .ok_or(ClipboardError::Malformed)?;
    let header = BITMAPV5HEADER {
        bV5Size: header_size,
        bV5Width: width,
        bV5Height: -height,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: u32::try_from(pixel_bytes).map_err(|_| ClipboardError::Malformed)?,
        bV5XPelsPerMeter: 0,
        bV5YPelsPerMeter: 0,
        bV5ClrUsed: 0,
        bV5ClrImportant: 0,
        bV5RedMask: DIBV5_RED_MASK,
        bV5GreenMask: DIBV5_GREEN_MASK,
        bV5BlueMask: DIBV5_BLUE_MASK,
        bV5AlphaMask: DIBV5_ALPHA_MASK,
        bV5CSType: LCS_SRGB,
        bV5Endpoints: Default::default(),
        bV5GammaRed: 0,
        bV5GammaGreen: 0,
        bV5GammaBlue: 0,
        bV5Intent: LCS_GM_IMAGES,
        bV5ProfileData: 0,
        bV5ProfileSize: 0,
        bV5Reserved: 0,
    };
    let mut bytes = vec![0_u8; total];
    bytes[..size_of::<BITMAPV5HEADER>()].copy_from_slice(header_bytes(&header));
    let stride = usize::try_from(stride).map_err(|_| ClipboardError::Malformed)?;
    for y in 0..image.height() {
        let row = image.row(y).ok_or(ClipboardError::Malformed)?;
        let offset = size_of::<BITMAPV5HEADER>()
            + usize::try_from(y)
                .map_err(|_| ClipboardError::Malformed)?
                .checked_mul(stride)
                .ok_or(ClipboardError::Malformed)?;
        bytes[offset..offset + stride].copy_from_slice(row);
    }
    Ok(bytes)
}

fn header_bytes(header: &BITMAPV5HEADER) -> &[u8] {
    // SAFETY: BITMAPV5HEADER is repr(C) and the slice covers exactly one initialised value.
    unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(header).cast::<u8>(),
            size_of::<BITMAPV5HEADER>(),
        )
    }
}

fn image_from_dibv5(bytes: &[u8]) -> Result<Image, ClipboardError> {
    let header_size = usize::try_from(read_u32(bytes, 0).ok_or(ClipboardError::Malformed)?)
        .map_err(|_| ClipboardError::Malformed)?;
    if header_size < 40 || header_size > bytes.len() {
        return Err(ClipboardError::Malformed);
    }
    let width = read_i32(bytes, 4).ok_or(ClipboardError::Malformed)?;
    let height = read_i32(bytes, 8).ok_or(ClipboardError::Malformed)?;
    let planes = read_u16(bytes, 12).ok_or(ClipboardError::Malformed)?;
    let bit_count = read_u16(bytes, 14).ok_or(ClipboardError::Malformed)?;
    let compression = read_u32(bytes, 16).ok_or(ClipboardError::Malformed)?;
    if width <= 0 || height == 0 || planes != 1 || bit_count != 32 {
        return Err(ClipboardError::Malformed);
    }
    if compression != BI_RGB.0 && compression != BI_BITFIELDS.0 {
        return Err(ClipboardError::Unsupported);
    }
    let (red, green, blue, alpha) = if header_size >= 108 {
        (
            read_u32(bytes, 40).ok_or(ClipboardError::Malformed)?,
            read_u32(bytes, 44).ok_or(ClipboardError::Malformed)?,
            read_u32(bytes, 48).ok_or(ClipboardError::Malformed)?,
            read_u32(bytes, 52).ok_or(ClipboardError::Malformed)?,
        )
    } else if compression == BI_BITFIELDS.0 {
        (
            read_u32(bytes, 40).ok_or(ClipboardError::Malformed)?,
            read_u32(bytes, 44).ok_or(ClipboardError::Malformed)?,
            read_u32(bytes, 48).ok_or(ClipboardError::Malformed)?,
            0,
        )
    } else {
        (DIBV5_RED_MASK, DIBV5_GREEN_MASK, DIBV5_BLUE_MASK, 0)
    };
    if compression == BI_BITFIELDS.0
        && (red != DIBV5_RED_MASK || green != DIBV5_GREEN_MASK || blue != DIBV5_BLUE_MASK)
    {
        return Err(ClipboardError::Unsupported);
    }
    let pixel_offset = if header_size >= 108 {
        header_size
    } else if compression == BI_BITFIELDS.0 {
        header_size + 12
    } else {
        header_size
    };
    let rows =
        usize::try_from(i64::from(height).unsigned_abs()).map_err(|_| ClipboardError::Malformed)?;
    let columns = usize::try_from(width).map_err(|_| ClipboardError::Malformed)?;
    let stride = columns.checked_mul(4).ok_or(ClipboardError::Malformed)?;
    let pixel_bytes = stride.checked_mul(rows).ok_or(ClipboardError::Malformed)?;
    let end = pixel_offset
        .checked_add(pixel_bytes)
        .ok_or(ClipboardError::Malformed)?;
    if end > bytes.len() {
        return Err(ClipboardError::Malformed);
    }
    let mut image = Image::bgra(
        u32::try_from(columns).map_err(|_| ClipboardError::Malformed)?,
        u32::try_from(rows).map_err(|_| ClipboardError::Malformed)?,
    )
    .map_err(|_| ClipboardError::Malformed)?;
    let top_down = height < 0;
    for y in 0..rows {
        let source_row = if top_down { y } else { rows - 1 - y };
        let source = pixel_offset + source_row * stride;
        let target = image
            .row_mut(u32::try_from(y).map_err(|_| ClipboardError::Malformed)?)
            .ok_or(ClipboardError::Malformed)?;
        target.copy_from_slice(&bytes[source..source + stride]);
    }
    if alpha != DIBV5_ALPHA_MASK {
        for pixel in image.pixels_mut().chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    Ok(image)
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let slice: [u8; 2] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u16::from_le_bytes(slice))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let slice: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(slice))
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let end = offset.checked_add(4)?;
    let slice: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(i32::from_le_bytes(slice))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Mutex;

    use super::*;

    static DESKTOP_LOCK: Mutex<()> = Mutex::new(());

    fn sample_image() -> Image {
        let Ok(mut image) = Image::bgra(3, 2) else {
            panic!("the sample image could not be created");
        };
        let pixels = [
            (0, 0, [1, 2, 3, 255]),
            (1, 0, [4, 5, 6, 128]),
            (2, 0, [7, 8, 9, 0]),
            (0, 1, [10, 11, 12, 255]),
            (1, 1, [13, 14, 15, 64]),
            (2, 1, [16, 17, 18, 200]),
        ];
        for (x, y, value) in pixels {
            if let Err(error) = image.set_pixel(x, y, value) {
                panic!("the sample pixel could not be set: {error:?}");
            }
        }
        image
    }

    fn padded_image() -> Image {
        let mut pixels = vec![0_u8; 32];
        pixels[0..4].copy_from_slice(&[1, 2, 3, 255]);
        pixels[4..8].copy_from_slice(&[4, 5, 6, 128]);
        pixels[8..12].copy_from_slice(&[7, 8, 9, 0]);
        pixels[16..20].copy_from_slice(&[10, 11, 12, 255]);
        pixels[20..24].copy_from_slice(&[13, 14, 15, 64]);
        pixels[24..28].copy_from_slice(&[16, 17, 18, 200]);
        match Image::new(3, 2, 16, pixels) {
            Ok(image) => image,
            Err(error) => panic!("the padded image could not be created: {error:?}"),
        }
    }

    fn encode(image: &Image) -> Vec<u8> {
        match dibv5_bytes(image) {
            Ok(bytes) => bytes,
            Err(error) => panic!("the DIBV5 bytes could not be built: {error}"),
        }
    }

    fn decode(bytes: &[u8]) -> Image {
        match image_from_dibv5(bytes) {
            Ok(image) => image,
            Err(error) => panic!("the DIBV5 bytes could not be read: {error}"),
        }
    }

    fn assert_same_pixels(left: &Image, right: &Image) {
        assert_eq!(left.width(), right.width());
        assert_eq!(left.height(), right.height());
        for y in 0..left.height() {
            for x in 0..left.width() {
                assert_eq!(left.pixel(x, y), right.pixel(x, y));
            }
        }
    }

    #[test]
    fn dibv5_header_matches_the_specified_layout() {
        let image = sample_image();
        let bytes = encode(&image);
        assert_eq!(read_u32(&bytes, 0), Some(124));
        assert_eq!(read_i32(&bytes, 4), Some(3));
        assert_eq!(read_i32(&bytes, 8), Some(-2));
        assert_eq!(read_u16(&bytes, 12), Some(1));
        assert_eq!(read_u16(&bytes, 14), Some(32));
        assert_eq!(read_u32(&bytes, 16), Some(3));
        assert_eq!(read_u32(&bytes, 20), Some(24));
        assert_eq!(read_u32(&bytes, 40), Some(DIBV5_RED_MASK));
        assert_eq!(read_u32(&bytes, 44), Some(DIBV5_GREEN_MASK));
        assert_eq!(read_u32(&bytes, 48), Some(DIBV5_BLUE_MASK));
        assert_eq!(read_u32(&bytes, 52), Some(DIBV5_ALPHA_MASK));
        assert_eq!(read_u32(&bytes, 56), Some(LCS_SRGB));
        assert_eq!(read_u32(&bytes, 108), Some(LCS_GM_IMAGES));
        assert_eq!(bytes.len(), 124 + 24);
        assert_eq!(&bytes[124..128], &[1, 2, 3, 255]);
        assert_eq!(&bytes[136..140], &[10, 11, 12, 255]);
    }

    #[test]
    fn dibv5_round_trip_keeps_every_pixel() {
        let image = sample_image();
        let decoded = decode(&encode(&image));
        assert_eq!(decoded.stride(), 12);
        assert_same_pixels(&decoded, &image);
    }

    #[test]
    fn dibv5_round_trip_ignores_source_padding() {
        let image = padded_image();
        let bytes = encode(&image);
        assert_eq!(bytes.len(), 124 + 24);
        let decoded = decode(&bytes);
        assert_eq!(decoded.stride(), 12);
        assert_same_pixels(&decoded, &image);
    }

    #[test]
    fn bottom_up_dibv5_rows_are_reversed() {
        let image = sample_image();
        let mut bytes = encode(&image);
        let stride = 12;
        let rows = 2;
        let mut flipped = bytes.clone();
        for y in 0..rows {
            let source = 124 + (rows - 1 - y) * stride;
            let target = 124 + y * stride;
            flipped[target..target + stride].copy_from_slice(&bytes[source..source + stride]);
        }
        bytes = flipped;
        bytes[8..12].copy_from_slice(&(rows as i32).to_le_bytes());
        assert_same_pixels(&decode(&bytes), &image);
    }

    #[test]
    fn dibv5_rejects_bad_layouts() {
        assert!(matches!(
            image_from_dibv5(&[]),
            Err(ClipboardError::Malformed)
        ));
        let mut bytes = encode(&sample_image());
        bytes[14..16].copy_from_slice(&24_u16.to_le_bytes());
        assert!(matches!(
            image_from_dibv5(&bytes),
            Err(ClipboardError::Malformed)
        ));
        let mut bytes = encode(&sample_image());
        bytes.truncate(130);
        assert!(matches!(
            image_from_dibv5(&bytes),
            Err(ClipboardError::Malformed)
        ));
        let mut bytes = encode(&sample_image());
        bytes[40..44].copy_from_slice(&DIBV5_BLUE_MASK.to_le_bytes());
        assert!(matches!(
            image_from_dibv5(&bytes),
            Err(ClipboardError::Unsupported)
        ));
    }

    #[test]
    fn text_is_null_terminated_utf16() {
        let bytes = utf16_bytes("Hi \u{2713}");
        let expected: Vec<u8> = "Hi \u{2713}"
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(bytes, expected);
        assert_eq!(bytes.len(), 10);
        assert_eq!(&bytes[8..10], &[0, 0]);
    }

    #[test]
    fn open_retries_until_the_clipboard_is_free() {
        let attempts = Cell::new(0_u32);
        let outcome = open_with_retry(|| {
            attempts.set(attempts.get() + 1);
            attempts.get() >= 3
        });
        assert!(outcome.is_ok());
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn open_gives_up_after_ten_attempts_with_the_busy_message() {
        let attempts = Cell::new(0_u32);
        let outcome = open_with_retry(|| {
            attempts.set(attempts.get() + 1);
            false
        });
        assert_eq!(attempts.get(), OPEN_ATTEMPTS);
        match outcome {
            Err(ClipboardError::Busy) => {}
            Err(error) => panic!("the wrong error was returned: {error}"),
            Ok(()) => panic!("a busy clipboard was reported as free"),
        }
        assert_eq!(
            ClipboardError::Busy.to_string(),
            "The clipboard is busy. Try again."
        );
    }

    #[test]
    fn global_memory_copies_and_releases() {
        let buffer = match GlobalBuffer::from_bytes(b"Snapline") {
            Ok(buffer) => buffer,
            Err(error) => panic!("the global buffer could not be created: {error}"),
        };
        // SAFETY: the buffer owns a live allocation that is locked once here and unlocked once below.
        let pointer = unsafe { GlobalLock(buffer.handle) };
        assert!(!pointer.is_null());
        let mut copy = [0_u8; 8];
        // SAFETY: the locked block holds at least the eight bytes being copied into the local array.
        unsafe {
            std::ptr::copy_nonoverlapping(pointer.cast::<u8>(), copy.as_mut_ptr(), copy.len());
            let _ = GlobalUnlock(buffer.handle);
        }
        assert_eq!(&copy, b"Snapline");
        drop(buffer);
    }

    fn read_png_for_test() -> Option<Vec<u8>> {
        let _session = ClipboardSession::open().ok()?;
        let format = png_format_id().ok()?;
        if !available(format) {
            return None;
        }
        read_blob(format).ok()
    }

    fn read_clipboard_text() -> Option<String> {
        let _session = ClipboardSession::open().ok()?;
        if !available(text_format()) {
            return None;
        }
        let bytes = read_blob(text_format()).ok()?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    struct ClipboardBackup {
        text: Option<String>,
    }

    impl ClipboardBackup {
        fn take() -> Self {
            Self {
                text: read_clipboard_text(),
            }
        }
    }

    impl Drop for ClipboardBackup {
        fn drop(&mut self) {
            let clipboard = Clipboard::new();
            match &self.text {
                Some(text) => {
                    let _ = clipboard.set_text(text);
                }
                None => {
                    if let Ok(_session) = ClipboardSession::open() {
                        // SAFETY: the session has just opened the clipboard on this thread.
                        unsafe {
                            let _ = EmptyClipboard();
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn clipboard_dibv5_round_trip_matches_pixels() {
        let Ok(_guard) = DESKTOP_LOCK.lock() else {
            return;
        };
        let _backup = ClipboardBackup::take();
        let clipboard = Clipboard::without_png();
        let image = sample_image();
        if let Err(error) = clipboard.set_image(&image) {
            panic!("the image could not be placed on the clipboard: {error}");
        }
        match clipboard.get_image() {
            Ok(Some(decoded)) => assert_same_pixels(&decoded, &image),
            Ok(None) => panic!("the clipboard returned no image"),
            Err(error) => panic!("the image could not be read back: {error}"),
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn clipboard_stores_png_and_dibv5_together() {
        let Ok(_guard) = DESKTOP_LOCK.lock() else {
            return;
        };
        let _backup = ClipboardBackup::take();
        let clipboard = Clipboard::new();
        let image = sample_image();
        if let Err(error) = clipboard.set_image(&image) {
            panic!("the image could not be placed on the clipboard: {error}");
        }
        let Ok(_session) = ClipboardSession::open() else {
            panic!("the clipboard could not be opened for inspection");
        };
        let Ok(png) = png_format_id() else {
            panic!("the PNG format could not be registered");
        };
        assert!(available(png));
        assert!(available(dib_format()));
        drop(_session);
        let Some(png_bytes) = read_png_for_test() else {
            panic!("the clipboard PNG could not be read");
        };
        assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        match clipboard.get_image() {
            Ok(Some(decoded)) => assert_same_pixels(&decoded, &image),
            Ok(None) => panic!("the clipboard returned no image"),
            Err(error) => panic!("the image could not be read back: {error}"),
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn clipboard_text_round_trip() {
        let Ok(_guard) = DESKTOP_LOCK.lock() else {
            return;
        };
        let _backup = ClipboardBackup::take();
        let text = "Snapline clipboard \u{2713}";
        if let Err(error) = Clipboard::new().set_text(text) {
            panic!("the text could not be placed on the clipboard: {error}");
        }
        assert_eq!(read_clipboard_text().as_deref(), Some(text));
    }

    #[test]
    #[ignore = "desktop"]
    fn empty_clipboard_returns_none() {
        let Ok(_guard) = DESKTOP_LOCK.lock() else {
            return;
        };
        let _backup = ClipboardBackup::take();
        match ClipboardSession::open() {
            Ok(_session) => {
                // SAFETY: the session has just opened the clipboard on this thread.
                if let Err(error) = unsafe { EmptyClipboard() } {
                    panic!("the clipboard could not be cleared: {error}");
                }
            }
            Err(error) => panic!("the clipboard could not be opened: {error}"),
        }
        match Clipboard::new().get_image() {
            Ok(None) => {}
            Ok(Some(_)) => panic!("the empty clipboard returned an image"),
            Err(error) => panic!("the empty clipboard returned an error: {error}"),
        }
    }
}
