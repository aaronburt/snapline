use windows::core::{Interface, BOOL, PCWSTR};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext5, ID2D1Factory1, ID2D1RenderTarget,
    ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1_UNIT_MODE_PIXELS,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIDevice, IDXGIFactory2, IDXGIOutput, IDXGISurface, IDXGISwapChain1,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use snapline_core::image::Image;

use crate::OverlayError;

pub(crate) const TEXT_FAMILY_SANS: &str = "Segoe UI Variable Text";
pub(crate) const TEXT_FAMILY_MONO: &str = "Cascadia Mono";
const TEXT_FAMILY_SANS_FALLBACK: &str = "Segoe UI";
const TEXT_FAMILY_MONO_FALLBACK: &str = "Consolas";
const LOCALE: &str = "en-gb";

pub(crate) struct Gpu {
    pub context: ID2D1DeviceContext5,
    pub dcomp: IDCompositionDevice,
    pub d3d: ID3D11Device,
    pub dxgi_factory: IDXGIFactory2,
    pub write: IDWriteFactory,
    pub text: TextCache,
}

pub(crate) struct SwapTarget {
    pub swap_chain: IDXGISwapChain1,
    pub target: ID2D1Bitmap1,
    pub(crate) _dcomp_target: IDCompositionTarget,
    pub(crate) _visual: IDCompositionVisual,
}

fn device_error(error: windows::core::Error) -> OverlayError {
    OverlayError::Device(error.to_string())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Gpu {
    pub(crate) fn new() -> Result<Self, OverlayError> {
        let mut d3d = None;
        let mut d3d_context = None;
        // SAFETY: the feature level slice is valid for the duration of the call and the out pointers point at live locals.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[
                    D3D_FEATURE_LEVEL_11_0,
                    D3D_FEATURE_LEVEL_10_1,
                    D3D_FEATURE_LEVEL_10_0,
                ]),
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                Some(&mut d3d_context),
            )
        }
        .map_err(device_error)?;
        let d3d = d3d.ok_or_else(|| OverlayError::Device("No Direct3D device.".to_string()))?;
        let dxgi_device: IDXGIDevice = d3d.cast().map_err(device_error)?;

        let factory: ID2D1Factory1 = {
            // SAFETY: a single threaded factory has no threading requirements and no options are needed.
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
                .map_err(device_error)?
        };
        // SAFETY: the DXGI device is live and owned by this function.
        let d2d_device = unsafe { factory.CreateDevice(&dxgi_device) }.map_err(device_error)?;
        // SAFETY: the Direct2D device is live and owned by this function.
        let context = unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }
            .map_err(device_error)?
            .cast::<ID2D1DeviceContext5>()
            .map_err(device_error)?;
        // SAFETY: scratch values for the render target configuration.
        unsafe {
            context.SetUnitMode(D2D1_UNIT_MODE_PIXELS);
            context.SetDpi(96.0, 96.0);
            context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }

        let dcomp: IDCompositionDevice = {
            // SAFETY: the DXGI device is live and owned by this function.
            unsafe { DCompositionCreateDevice(&dxgi_device) }.map_err(device_error)?
        };
        // SAFETY: DXGI factory creation has no arguments that can be invalid.
        let dxgi_factory: IDXGIFactory2 = unsafe { CreateDXGIFactory1() }.map_err(device_error)?;
        let write: IDWriteFactory = {
            // SAFETY: DirectWrite has no invalid inputs here.
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }.map_err(device_error)?
        };

        Ok(Self {
            context,
            dcomp,
            d3d,
            dxgi_factory,
            write,
            text: TextCache::new(),
        })
    }

    pub(crate) fn create_target(
        &self,
        hwnd: HWND,
        width: u32,
        height: u32,
    ) -> Result<SwapTarget, OverlayError> {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        // SAFETY: the device, description and restriction output are valid.
        let swap_chain = unsafe {
            self.dxgi_factory
                .CreateSwapChainForComposition(&self.d3d, &desc, None::<&IDXGIOutput>)
        }
        .map_err(device_error)?;
        // SAFETY: the swap chain owns at least one buffer of the requested size.
        let surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0) }.map_err(device_error)?;
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        // SAFETY: the surface is a live DXGI surface of the swap chain.
        let target = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&properties))
        }
        .map_err(device_error)?;
        let dcomp_target = {
            // SAFETY: the window handle is a live top level window owned by this thread.
            unsafe { self.dcomp.CreateTargetForHwnd(hwnd, false) }.map_err(device_error)?
        };
        // SAFETY: the composition device is live.
        let visual = unsafe { self.dcomp.CreateVisual() }.map_err(device_error)?;
        // SAFETY: the swap chain is a composition surface and the visual accepts it as content.
        unsafe { visual.SetContent(&swap_chain) }.map_err(device_error)?;
        // SAFETY: the target and visual are live and owned by this function.
        unsafe { dcomp_target.SetRoot(&visual) }.map_err(device_error)?;
        // SAFETY: commits the visual tree created above.
        unsafe { self.dcomp.Commit() }.map_err(device_error)?;
        Ok(SwapTarget {
            swap_chain,
            target,
            _dcomp_target: dcomp_target,
            _visual: visual,
        })
    }

    pub(crate) fn upload_image(&self, image: &Image) -> Result<ID2D1Bitmap1, OverlayError> {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: Default::default(),
            ..Default::default()
        };
        // SAFETY: the pixel buffer lives for the call and the size matches the stride metadata.
        unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: image.width(),
                    height: image.height(),
                },
                Some(image.pixels().as_ptr().cast()),
                image.stride() as u32,
                &properties,
            )
        }
        .map_err(device_error)
    }

    pub(crate) fn begin(&self) {
        // SAFETY: every draw call is paired with an end on the same thread.
        unsafe { self.context.BeginDraw() };
    }

    pub(crate) fn end(&self) {
        // SAFETY: pairs with begin above.
        unsafe {
            let _ = self.context.EndDraw(None, None);
        }
    }
}

pub(crate) fn colour(rgba: [f32; 4]) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: rgba[0],
        g: rgba[1],
        b: rgba[2],
        a: rgba[3],
    }
}

pub(crate) fn rect(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

fn brush(context: &ID2D1DeviceContext5, rgba: [f32; 4]) -> Option<ID2D1SolidColorBrush> {
    let value = colour(rgba);
    // SAFETY: the brush properties pointer is valid and the colour is a plain value.
    unsafe { context.CreateSolidColorBrush(&value, None) }.ok()
}

pub(crate) fn fill_rect(context: &ID2D1DeviceContext5, area: D2D_RECT_F, rgba: [f32; 4]) {
    let Some(brush) = brush(context, rgba) else {
        return;
    };
    // SAFETY: the brush and rectangle are live values.
    unsafe {
        let _ = context.FillRectangle(&area, &brush);
    }
}

pub(crate) fn stroke_rect(
    context: &ID2D1DeviceContext5,
    area: D2D_RECT_F,
    width: f32,
    rgba: [f32; 4],
) {
    let Some(brush) = brush(context, rgba) else {
        return;
    };
    // SAFETY: the brush, rectangle and width are valid.
    unsafe {
        let _ = context.DrawRectangle(&area, &brush, width, None);
    }
}

pub(crate) fn fill_round(
    context: &ID2D1DeviceContext5,
    area: D2D_RECT_F,
    radius: f32,
    rgba: [f32; 4],
) {
    let Some(brush) = brush(context, rgba) else {
        return;
    };
    let rounded = D2D1_ROUNDED_RECT {
        rect: area,
        radiusX: radius,
        radiusY: radius,
    };
    // SAFETY: the brush and rounded rectangle are live values.
    unsafe {
        let _ = context.FillRoundedRectangle(&rounded, &brush);
    }
}

pub(crate) fn stroke_round(
    context: &ID2D1DeviceContext5,
    area: D2D_RECT_F,
    radius: f32,
    width: f32,
    rgba: [f32; 4],
) {
    let Some(brush) = brush(context, rgba) else {
        return;
    };
    let rounded = D2D1_ROUNDED_RECT {
        rect: area,
        radiusX: radius,
        radiusY: radius,
    };
    // SAFETY: the brush and rounded rectangle are live values.
    unsafe {
        let _ = context.DrawRoundedRectangle(&rounded, &brush, width, None);
    }
}

pub(crate) fn fill_circle(
    context: &ID2D1DeviceContext5,
    centre_x: f32,
    centre_y: f32,
    radius: f32,
    rgba: [f32; 4],
) {
    let half = radius * 2.0;
    fill_round(
        context,
        rect(centre_x - radius, centre_y - radius, half, half),
        radius,
        rgba,
    );
}

pub(crate) fn stroke_circle(
    context: &ID2D1DeviceContext5,
    centre_x: f32,
    centre_y: f32,
    radius: f32,
    width: f32,
    rgba: [f32; 4],
) {
    let half = radius * 2.0;
    stroke_round(
        context,
        rect(centre_x - radius, centre_y - radius, half, half),
        radius,
        width,
        rgba,
    );
}

pub(crate) fn draw_bitmap(
    context: &ID2D1DeviceContext5,
    bitmap: &ID2D1Bitmap1,
    destination: D2D_RECT_F,
    source: D2D_RECT_F,
    nearest: bool,
) {
    let interpolation = if nearest {
        D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
    } else {
        D2D1_INTERPOLATION_MODE_LINEAR
    };
    // SAFETY: the bitmap is live, the rectangles are plain values and no perspective transform is used.
    unsafe {
        let _ = context.DrawBitmap(
            bitmap,
            Some(&destination),
            1.0,
            interpolation,
            Some(&source),
            None,
        );
    }
}

fn family_available(factory: &IDWriteFactory, name: &str) -> bool {
    let mut collection: Option<IDWriteFontCollection> = None;
    // SAFETY: the out pointer is valid and the update flag is a plain value.
    if unsafe { factory.GetSystemFontCollection(&mut collection, false) }.is_err() {
        return false;
    }
    let Some(collection) = collection else {
        return false;
    };
    let family = wide(name);
    let mut index = 0u32;
    let mut exists = BOOL(0);
    // SAFETY: the UTF-16 family name, index and flag pointers are live for the call.
    if unsafe {
        collection.FindFamilyName(PCWSTR::from_raw(family.as_ptr()), &mut index, &mut exists)
    }
    .is_err()
    {
        return false;
    }
    exists.as_bool()
}

fn text_format(
    factory: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    mono: bool,
) -> Option<IDWriteTextFormat> {
    let family_name = if mono {
        if family_available(factory, TEXT_FAMILY_MONO) {
            TEXT_FAMILY_MONO
        } else {
            TEXT_FAMILY_MONO_FALLBACK
        }
    } else if family_available(factory, TEXT_FAMILY_SANS) {
        TEXT_FAMILY_SANS
    } else {
        TEXT_FAMILY_SANS_FALLBACK
    };
    let family = wide(family_name);
    let locale = wide(LOCALE);
    // SAFETY: both UTF-16 buffers outlive the call and the factory is live.
    let format = unsafe {
        factory.CreateTextFormat(
            PCWSTR::from_raw(family.as_ptr()),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            PCWSTR::from_raw(locale.as_ptr()),
        )
    }
    .ok()?;
    // SAFETY: the format is live and owned by this function.
    unsafe {
        let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    }
    Some(format)
}

pub(crate) struct TextCache {
    formats: Vec<(u32, u32, bool, IDWriteTextFormat)>,
}

impl TextCache {
    pub(crate) fn new() -> Self {
        Self {
            formats: Vec::new(),
        }
    }

    pub(crate) fn format(
        &mut self,
        factory: &IDWriteFactory,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
        mono: bool,
    ) -> Option<IDWriteTextFormat> {
        let key = (size.to_bits(), weight.0 as u32, mono);
        if let Some((_, _, _, format)) =
            self.formats
                .iter()
                .find(|(cached_size, cached_weight, cached_mono, _)| {
                    *cached_size == key.0 && *cached_weight == key.1 && *cached_mono == key.2
                })
        {
            return Some(format.clone());
        }
        let format = text_format(factory, size, weight, mono)?;
        self.formats.push((key.0, key.1, key.2, format.clone()));
        Some(format)
    }
}

pub(crate) fn measure(factory: &IDWriteFactory, format: &IDWriteTextFormat, text: &str) -> f32 {
    let buffer: Vec<u16> = text.encode_utf16().collect();
    if buffer.is_empty() {
        return 0.0;
    }
    // SAFETY: the UTF-16 buffer and format are live and the layout bounds are positive.
    let Ok(layout) = (unsafe { factory.CreateTextLayout(&buffer, format, 100_000.0, 100_000.0) })
    else {
        return 0.0;
    };
    let mut metrics = DWRITE_TEXT_METRICS::default();
    // SAFETY: the layout is live and the metrics out pointer is valid.
    if unsafe { layout.GetMetrics(&mut metrics) }.is_err() {
        return 0.0;
    }
    metrics.widthIncludingTrailingWhitespace
}

pub(crate) fn draw_text(
    context: &ID2D1DeviceContext5,
    format: &IDWriteTextFormat,
    text: &str,
    area: D2D_RECT_F,
    rgba: [f32; 4],
) {
    let buffer: Vec<u16> = text.encode_utf16().collect();
    if buffer.is_empty() {
        return;
    }
    let Some(brush) = brush(context, rgba) else {
        return;
    };
    // SAFETY: the UTF-16 buffer, text format, layout rectangle and brush are live values.
    unsafe {
        let _ = ID2D1RenderTarget::DrawText(
            context,
            &buffer,
            format,
            &area,
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}

pub(crate) fn palette_hex(rgba: [f32; 4]) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba[0]),
        channel(rgba[1]),
        channel(rgba[2])
    )
}
