use std::mem::ManuallyDrop;

use snapline_core::image::{Image, ImageFormat};
use thiserror::Error;
use windows::core::{GUID, PWSTR};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_ContainerFormatPng,
    GUID_ContainerFormatWebp, GUID_WICPixelFormat32bppBGRA, IWICBitmap, IWICBitmapScaler,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPBAG2};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IStream, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_R4};
use windows::Win32::UI::Shell::SHCreateMemStream;

pub const THUMBNAIL_WIDTH: u32 = 320;

const QUALITY_MIN: u8 = 40;
const QUALITY_MAX: u8 = 100;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WicError {
    #[error("Windows imaging could not start on this thread.")]
    Com,
    #[error("Windows imaging could not create its image codec.")]
    Factory,
    #[error("The image could not be decoded.")]
    Decode,
    #[error("The image could not be encoded.")]
    Encode,
    #[error("Windows does not have a WebP encoder.")]
    WebpUnavailable,
    #[error("The image dimensions are not supported.")]
    Dimensions,
}

struct ComApartment {
    uninit: bool,
}

impl ComApartment {
    fn start() -> Result<Self, WicError> {
        // SAFETY: no pointer is shared and the matching CoUninitialize runs in Drop on this thread.
        let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if status.is_ok() {
            Ok(Self { uninit: true })
        } else if status == RPC_E_CHANGED_MODE {
            Ok(Self { uninit: false })
        } else {
            Err(WicError::Com)
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninit {
            // SAFETY: this balances the successful CoInitializeEx in start on the same thread.
            unsafe { CoUninitialize() };
        }
    }
}

pub struct WicCodec {
    factory: IWICImagingFactory,
    webp: bool,
}

impl WicCodec {
    pub fn new() -> Result<Self, WicError> {
        let _com = ComApartment::start()?;
        let factory: IWICImagingFactory = {
            // SAFETY: the class is registered in-process and the returned interface owns a reference.
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                .map_err(|_| WicError::Factory)?
        };
        let webp = {
            // SAFETY: the factory is live and a null vendor GUID asks for any installed encoder.
            unsafe { factory.CreateEncoder(&GUID_ContainerFormatWebp, std::ptr::null()) }.is_ok()
        };
        Ok(Self { factory, webp })
    }

    pub fn webp_available(&self) -> bool {
        self.webp
    }

    pub fn encode(
        &self,
        image: &Image,
        format: ImageFormat,
        quality: u8,
    ) -> Result<Vec<u8>, WicError> {
        let _com = ComApartment::start()?;
        let container = container_format(format, self.webp)?;
        let quality = clamp_quality(quality);
        // SAFETY: a null initial buffer asks for an empty growable memory stream.
        let stream = unsafe { SHCreateMemStream(None) }.ok_or(WicError::Encode)?;
        self.encode_into(image, format, container, quality, &stream)
    }

    pub fn decode(&self, bytes: &[u8]) -> Result<Image, WicError> {
        let _com = ComApartment::start()?;
        if u32::try_from(bytes.len()).is_err() {
            return Err(WicError::Decode);
        }
        // SAFETY: the factory is live and returns an owned memory stream interface.
        let stream = unsafe { self.factory.CreateStream() }.map_err(|_| WicError::Decode)?;
        // SAFETY: the stream is live and borrows the byte slice for the duration of the call.
        unsafe { stream.InitializeFromMemory(bytes) }.map_err(|_| WicError::Decode)?;
        // SAFETY: the stream is live and a null vendor GUID asks for any installed decoder.
        let decoder = unsafe {
            self.factory.CreateDecoderFromStream(
                &stream,
                std::ptr::null(),
                WICDecodeMetadataCacheOnDemand,
            )
        }
        .map_err(|_| WicError::Decode)?;
        // SAFETY: the decoder is live and frame zero is the first image in the container.
        let frame = unsafe { decoder.GetFrame(0) }.map_err(|_| WicError::Decode)?;
        let mut width = 0_u32;
        let mut height = 0_u32;
        // SAFETY: both out pointers refer to live u32 values for the duration of the call.
        unsafe { frame.GetSize(&mut width, &mut height) }.map_err(|_| WicError::Decode)?;
        if width == 0 || height == 0 {
            return Err(WicError::Decode);
        }
        let converter = {
            // SAFETY: the factory is live and returns an owned format converter interface.
            unsafe { self.factory.CreateFormatConverter() }.map_err(|_| WicError::Decode)?
        };
        // SAFETY: the frame and converter are live and the destination format is a valid WIC GUID.
        unsafe {
            converter.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }
        .map_err(|_| WicError::Decode)?;
        let stride = usize::try_from(width)
            .ok()
            .and_then(|value| value.checked_mul(4))
            .ok_or(WicError::Dimensions)?;
        let length = stride
            .checked_mul(usize::try_from(height).map_err(|_| WicError::Dimensions)?)
            .ok_or(WicError::Dimensions)?;
        let stride_u32 = u32::try_from(stride).map_err(|_| WicError::Dimensions)?;
        let mut pixels = vec![0_u8; length];
        // SAFETY: the converter is live and the buffer is exactly stride times height bytes.
        unsafe { converter.CopyPixels(std::ptr::null(), stride_u32, &mut pixels) }
            .map_err(|_| WicError::Decode)?;
        Image::new(width, height, stride, pixels).map_err(|_| WicError::Decode)
    }

    pub fn encode_thumbnail(
        &self,
        image: &Image,
        width: u32,
        quality: u8,
    ) -> Result<(Vec<u8>, ImageFormat), WicError> {
        let _com = ComApartment::start()?;
        let (target_width, target_height) = thumbnail_size(image.width(), image.height(), width);
        let bitmap = self.bitmap_from_image(image)?;
        // SAFETY: the factory is live and returns an owned scaler interface.
        let scaler = unsafe { self.factory.CreateBitmapScaler() }.map_err(|_| WicError::Encode)?;
        // SAFETY: the bitmap and scaler are live and the target size is derived from the source.
        unsafe {
            scaler.Initialize(
                &bitmap,
                target_width,
                target_height,
                WICBitmapInterpolationModeHighQualityCubic,
            )
        }
        .map_err(|_| WicError::Encode)?;
        let scaled = scale_to_image(&scaler, target_width, target_height)?;
        let format = if self.webp {
            ImageFormat::WebP
        } else {
            ImageFormat::Jpeg
        };
        let bytes = self.encode(&scaled, format, quality)?;
        Ok((bytes, format))
    }

    fn encode_into(
        &self,
        image: &Image,
        format: ImageFormat,
        container: &GUID,
        quality: u8,
        stream: &IStream,
    ) -> Result<Vec<u8>, WicError> {
        // SAFETY: the factory is live, the stream is live and the container names an encoder.
        let encoder = unsafe { self.factory.CreateEncoder(container, std::ptr::null()) }
            .map_err(|_| WicError::Encode)?;
        // SAFETY: the encoder and stream are live and no cache is requested.
        unsafe { encoder.Initialize(stream, WICBitmapEncoderNoCache) }
            .map_err(|_| WicError::Encode)?;
        let mut frame: Option<windows::Win32::Graphics::Imaging::IWICBitmapFrameEncode> = None;
        let mut options: Option<IPropertyBag2> = None;
        // SAFETY: both out pointers refer to live option slots that receive owned interfaces.
        unsafe { encoder.CreateNewFrame(&mut frame, &mut options) }
            .map_err(|_| WicError::Encode)?;
        let frame = frame.ok_or(WicError::Encode)?;
        if matches!(format, ImageFormat::Jpeg | ImageFormat::WebP) {
            if let Some(bag) = options.as_ref() {
                write_quality(bag, quality);
            }
        }
        // SAFETY: the frame is live and the property bag is either live or absent.
        unsafe { frame.Initialize(options.as_ref()) }.map_err(|_| WicError::Encode)?;
        // SAFETY: the frame is live and the size comes from the source image.
        unsafe { frame.SetSize(image.width(), image.height()) }.map_err(|_| WicError::Encode)?;
        // SAFETY: the frame is live and WIC accepts a fixed 96 DPI for screenshots.
        unsafe { frame.SetResolution(96.0, 96.0) }.map_err(|_| WicError::Encode)?;
        let mut pixel_format = GUID_WICPixelFormat32bppBGRA;
        // SAFETY: the frame is live and the pixel format pointer is writable for negotiation.
        unsafe { frame.SetPixelFormat(&mut pixel_format) }.map_err(|_| WicError::Encode)?;
        let bitmap = self.bitmap_from_image(image)?;
        let converter = {
            // SAFETY: the factory is live and returns an owned format converter interface.
            unsafe { self.factory.CreateFormatConverter() }.map_err(|_| WicError::Encode)?
        };
        // SAFETY: the bitmap and converter are live and the negotiated format came from the encoder.
        unsafe {
            converter.Initialize(
                &bitmap,
                &pixel_format,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }
        .map_err(|_| WicError::Encode)?;
        // SAFETY: the frame and converter are live and a null rectangle means the whole source.
        unsafe { frame.WriteSource(&converter, std::ptr::null()) }.map_err(|_| WicError::Encode)?;
        // SAFETY: the frame is live and receives the finished pixels once.
        unsafe { frame.Commit() }.map_err(|_| WicError::Encode)?;
        // SAFETY: the encoder is live and finalises the container once.
        unsafe { encoder.Commit() }.map_err(|_| WicError::Encode)?;
        read_stream(stream)
    }

    fn bitmap_from_image(&self, image: &Image) -> Result<IWICBitmap, WicError> {
        let length = image_bytes(image)?;
        let pixels = image.pixels().get(..length).ok_or(WicError::Dimensions)?;
        let stride = u32::try_from(image.stride()).map_err(|_| WicError::Dimensions)?;
        // SAFETY: the pixel slice is live and holds exactly stride times height bytes.
        unsafe {
            self.factory.CreateBitmapFromMemory(
                image.width(),
                image.height(),
                &GUID_WICPixelFormat32bppBGRA,
                stride,
                pixels,
            )
        }
        .map_err(|_| WicError::Encode)
    }
}

fn container_format(format: ImageFormat, webp: bool) -> Result<&'static GUID, WicError> {
    match format {
        ImageFormat::Png => Ok(&GUID_ContainerFormatPng),
        ImageFormat::Jpeg => Ok(&GUID_ContainerFormatJpeg),
        ImageFormat::WebP if webp => Ok(&GUID_ContainerFormatWebp),
        ImageFormat::WebP => Err(WicError::WebpUnavailable),
    }
}

fn image_bytes(image: &Image) -> Result<usize, WicError> {
    usize::try_from(image.height())
        .ok()
        .and_then(|height| height.checked_mul(image.stride()))
        .ok_or(WicError::Dimensions)
}

fn clamp_quality(quality: u8) -> u8 {
    quality.clamp(QUALITY_MIN, QUALITY_MAX)
}

fn thumbnail_size(width: u32, height: u32, target: u32) -> (u32, u32) {
    let target = target.min(width).max(1);
    let scaled = (u64::from(height) * u64::from(target) + u64::from(width) / 2) / u64::from(width);
    let scaled = u32::try_from(scaled).unwrap_or(1).max(1);
    (target, scaled)
}

fn scale_to_image(scaler: &IWICBitmapScaler, width: u32, height: u32) -> Result<Image, WicError> {
    let stride = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or(WicError::Dimensions)?;
    let length = stride
        .checked_mul(usize::try_from(height).map_err(|_| WicError::Dimensions)?)
        .ok_or(WicError::Dimensions)?;
    let stride_u32 = u32::try_from(stride).map_err(|_| WicError::Dimensions)?;
    let mut pixels = vec![0_u8; length];
    // SAFETY: the scaler is live and the buffer is exactly stride times height bytes.
    unsafe { scaler.CopyPixels(std::ptr::null(), stride_u32, &mut pixels) }
        .map_err(|_| WicError::Encode)?;
    Image::new(width, height, stride, pixels).map_err(|_| WicError::Dimensions)
}

fn read_stream(stream: &IStream) -> Result<Vec<u8>, WicError> {
    let mut stat = STATSTG::default();
    // SAFETY: the stream is live and the out structure is writable for the call.
    unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }.map_err(|_| WicError::Encode)?;
    let length = usize::try_from(stat.cbSize).map_err(|_| WicError::Dimensions)?;
    // SAFETY: the stream is live and seeks back to its start for reading.
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }.map_err(|_| WicError::Encode)?;
    let mut bytes = vec![0_u8; length];
    let size = u32::try_from(length).map_err(|_| WicError::Dimensions)?;
    let mut read = 0_u32;
    // SAFETY: the stream is live and the buffer holds `length` writable bytes.
    let status = unsafe {
        stream.Read(
            bytes.as_mut_ptr().cast(),
            size,
            Some(std::ptr::from_mut(&mut read)),
        )
    };
    status.ok().map_err(|_| WicError::Encode)?;
    bytes.truncate(usize::try_from(read).map_err(|_| WicError::Encode)?);
    Ok(bytes)
}

fn write_quality(bag: &IPropertyBag2, quality: u8) {
    let mut name: Vec<u16> = "ImageQuality\0".encode_utf16().collect();
    let property = PROPBAG2 {
        dwType: 0,
        vt: VT_R4,
        cfType: 0,
        dwHint: 0,
        pstrName: PWSTR(name.as_mut_ptr()),
        clsid: GUID::zeroed(),
    };
    let value = float_variant(f32::from(quality) / 100.0);
    // SAFETY: the name buffer and variant are live for the call and the bag is live.
    let _ = unsafe { bag.Write(1, &property, &value) };
}

fn float_variant(value: f32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_R4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { fltVal: value },
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image(width: u32, height: u32) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                let pixel = [
                    u8::try_from((x * 17 + y * 3) % 256).unwrap_or(0),
                    u8::try_from((x * 5 + y * 29) % 256).unwrap_or(0),
                    u8::try_from((x * 11 + y * 7) % 256).unwrap_or(0),
                    255,
                ];
                image.set_pixel(x, y, pixel).expect("pixel");
            }
        }
        image
    }

    fn solid_image(width: u32, height: u32, pixel: [u8; 4]) -> Image {
        let mut image = Image::bgra(width, height).expect("image");
        for y in 0..height {
            for x in 0..width {
                image.set_pixel(x, y, pixel).expect("pixel");
            }
        }
        image
    }

    #[test]
    fn quality_is_clamped_to_the_lossy_range() {
        assert_eq!(clamp_quality(0), 40);
        assert_eq!(clamp_quality(40), 40);
        assert_eq!(clamp_quality(90), 90);
        assert_eq!(clamp_quality(100), 100);
        assert_eq!(clamp_quality(255), 100);
    }

    #[test]
    fn thumbnails_keep_the_aspect_ratio() {
        assert_eq!(thumbnail_size(640, 480, THUMBNAIL_WIDTH), (320, 240));
        assert_eq!(thumbnail_size(1000, 3, THUMBNAIL_WIDTH), (320, 1));
        assert_eq!(thumbnail_size(100, 50, THUMBNAIL_WIDTH), (100, 50));
        assert_eq!(thumbnail_size(1920, 1080, THUMBNAIL_WIDTH), (320, 180));
    }

    #[test]
    #[ignore = "desktop"]
    fn png_round_trip_is_pixel_exact() {
        let codec = WicCodec::new().expect("codec");
        let image = test_image(7, 5);
        let bytes = codec.encode(&image, ImageFormat::Png, 90).expect("png");
        assert_eq!(&bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let decoded = codec.decode(&bytes).expect("decode");
        assert_eq!(decoded.width(), image.width());
        assert_eq!(decoded.height(), image.height());
        for y in 0..image.height() {
            for x in 0..image.width() {
                assert_eq!(decoded.pixel(x, y), image.pixel(x, y));
            }
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn jpeg_round_trip_keeps_dimensions_and_colours() {
        let codec = WicCodec::new().expect("codec");
        let image = solid_image(64, 48, [40, 120, 200, 255]);
        let bytes = codec.encode(&image, ImageFormat::Jpeg, 90).expect("jpeg");
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);
        let decoded = codec.decode(&bytes).expect("decode");
        assert_eq!(decoded.width(), image.width());
        assert_eq!(decoded.height(), image.height());
        let pixel = decoded.pixel(32, 24).expect("pixel");
        assert!((i32::from(pixel[0]) - 40).abs() <= 12);
        assert!((i32::from(pixel[1]) - 120).abs() <= 12);
        assert!((i32::from(pixel[2]) - 200).abs() <= 12);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    #[ignore = "desktop"]
    fn webp_round_trip_when_available() {
        let codec = WicCodec::new().expect("codec");
        if !codec.webp_available() {
            return;
        }
        let image = test_image(40, 30);
        let bytes = codec.encode(&image, ImageFormat::WebP, 80).expect("webp");
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        let decoded = codec.decode(&bytes).expect("decode");
        assert_eq!(decoded.width(), image.width());
        assert_eq!(decoded.height(), image.height());
    }

    #[test]
    #[ignore = "desktop"]
    fn webp_availability_is_stable() {
        let codec = WicCodec::new().expect("codec");
        let first = codec.webp_available();
        let second = codec.webp_available();
        assert_eq!(first, second);
        if !first {
            assert_eq!(
                codec.encode(&test_image(2, 2), ImageFormat::WebP, 80),
                Err(WicError::WebpUnavailable)
            );
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn truncated_bytes_report_a_decode_error() {
        let codec = WicCodec::new().expect("codec");
        assert_eq!(codec.decode(b"not an image"), Err(WicError::Decode));
        assert_eq!(codec.decode(b"\x89PNG\r\n\x1a\n"), Err(WicError::Decode));
    }

    #[test]
    #[ignore = "desktop"]
    fn thumbnails_are_320_wide_and_keep_the_aspect_ratio() {
        let codec = WicCodec::new().expect("codec");
        let image = test_image(640, 480);
        let (bytes, format) = codec
            .encode_thumbnail(&image, THUMBNAIL_WIDTH, 90)
            .expect("thumbnail");
        let expected = if codec.webp_available() {
            ImageFormat::WebP
        } else {
            ImageFormat::Jpeg
        };
        assert_eq!(format, expected);
        let decoded = codec.decode(&bytes).expect("decode");
        assert_eq!(decoded.width(), 320);
        assert_eq!(decoded.height(), 240);
    }
}
