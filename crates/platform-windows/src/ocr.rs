use std::time::Duration;

use snapline_core::image::Image;
use snapline_core::ports::{Ocr, OcrError, OcrText};
use windows::core::{Error as WindowsError, Interface, HSTRING};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::Streams::Buffer;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::WinRT::{
    IBufferByteAccess, RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
};

const STATUS_STARTED: i32 = 0;
const STATUS_COMPLETED: i32 = 1;
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WindowsOcr;

impl Ocr for WindowsOcr {
    fn recognise(&self, image: &Image, language: Option<&str>) -> Result<OcrText, OcrError> {
        let _apartment = Apartment::enter()?;
        let engine = engine(language)?;
        let bitmap = software_bitmap(image)?;
        let result = recognise(&engine, &bitmap)?;
        let lines = result.Lines().map_err(failed)?;
        let count = lines.Size().map_err(failed)?;
        let mut collected = Vec::with_capacity(count as usize);
        for index in 0..count {
            let line = lines.GetAt(index).map_err(failed)?;
            collected.push(line.Text().map_err(failed)?.to_string());
        }
        Ok(OcrText { lines: collected })
    }
}

fn recognise(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Result<OcrResult, OcrError> {
    let operation = engine.RecognizeAsync(bitmap).map_err(failed)?;
    loop {
        let status = operation.Status().map_err(failed)?;
        if status.0 == STATUS_COMPLETED {
            return operation.GetResults().map_err(failed);
        }
        if status.0 != STATUS_STARTED {
            return Err(failed(WindowsError::from_hresult(
                operation.ErrorCode().map_err(failed)?,
            )));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn engine(language: Option<&str>) -> Result<OcrEngine, OcrError> {
    match language {
        Some(tag) if !tag.trim().is_empty() => {
            let tag = Language::CreateLanguage(&HSTRING::from(tag)).map_err(failed)?;
            OcrEngine::TryCreateFromLanguage(&tag).map_err(engine_error)
        }
        _ => OcrEngine::TryCreateFromUserProfileLanguages().map_err(engine_error),
    }
}

fn engine_error(error: WindowsError) -> OcrError {
    let available = OcrEngine::AvailableRecognizerLanguages()
        .ok()
        .and_then(|languages| languages.Size().ok());
    match available {
        Some(0) => OcrError::NoLanguage,
        _ => OcrError::Failed(error.message()),
    }
}

fn software_bitmap(image: &Image) -> Result<SoftwareBitmap, OcrError> {
    let packed = packed_bgra(image)?;
    let width = i32::try_from(image.width()).map_err(|_| too_large())?;
    let height = i32::try_from(image.height()).map_err(|_| too_large())?;
    let length = u32::try_from(packed.len()).map_err(|_| too_large())?;
    let buffer = Buffer::Create(length).map_err(failed)?;
    buffer.SetLength(length).map_err(failed)?;
    let access = buffer.cast::<IBufferByteAccess>().map_err(failed)?;
    // SAFETY: buffer is a live IBuffer whose length was set to packed.len(), so the returned pointer covers that many writable bytes.
    let destination = unsafe { access.Buffer().map_err(failed)? };
    // SAFETY: packed holds packed.len() readable bytes and destination is the buffer's writable region of the same length.
    unsafe { std::ptr::copy_nonoverlapping(packed.as_ptr(), destination, packed.len()) };
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width,
        height,
        BitmapAlphaMode::Ignore,
    )
    .map_err(failed)
}

fn packed_bgra(image: &Image) -> Result<Vec<u8>, OcrError> {
    let row = usize::try_from(image.width())
        .map_err(|_| too_large())?
        .checked_mul(4)
        .ok_or_else(too_large)?;
    let mut packed = Vec::with_capacity(row.saturating_mul(image.height() as usize));
    for y in 0..image.height() {
        let source = image.row(y).ok_or_else(too_large)?;
        packed.extend_from_slice(source);
    }
    Ok(packed)
}

fn too_large() -> OcrError {
    OcrError::Failed("The image is too large for text recognition.".to_owned())
}

fn failed(error: WindowsError) -> OcrError {
    OcrError::Failed(error.message())
}

struct Apartment {
    owned: bool,
}

impl Apartment {
    fn enter() -> Result<Self, OcrError> {
        // SAFETY: RoInitialize takes no pointers and only initialises a WinRT apartment on this thread.
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Ok(Self { owned: true }),
            Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(Self { owned: false }),
            Err(error) => Err(failed(error)),
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: this thread was initialised by the matching RoInitialize above, so it is uninitialised exactly once here.
            unsafe { RoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_bgra_rebuilds_tight_rows() {
        let mut pixels = vec![0_u8; 16];
        pixels[0..4].copy_from_slice(&[1, 2, 3, 4]);
        pixels[8..12].copy_from_slice(&[5, 6, 7, 8]);
        let image = Image::new(1, 2, 8, pixels).expect("image");
        assert_eq!(packed_bgra(&image), Ok(vec![1, 2, 3, 4, 5, 6, 7, 8]));
    }

    #[test]
    fn packed_bgra_keeps_every_row() {
        let mut image = Image::bgra(2, 2).expect("image");
        image.set_pixel(0, 0, [9, 8, 7, 6]).expect("pixel");
        image.set_pixel(1, 1, [5, 4, 3, 2]).expect("pixel");
        let packed = packed_bgra(&image).expect("packed");
        assert_eq!(packed.len(), 16);
        assert_eq!(&packed[0..4], &[9, 8, 7, 6]);
        assert_eq!(&packed[12..16], &[5, 4, 3, 2]);
    }

    #[test]
    #[ignore = "desktop"]
    fn recognises_a_blank_image() {
        let image = Image::bgra(400, 200).expect("image");
        let result = WindowsOcr.recognise(&image, None);
        assert!(matches!(result, Ok(_) | Err(OcrError::NoLanguage)));
    }
}
