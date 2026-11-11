use std::sync::Arc;

use snapline_core::geometry::{Px, Rect};
use snapline_core::image::Image;
use snapline_core::ports::{CaptureError, FrozenDesktop, FrozenMonitor};
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_INVALID_CALL,
    DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_UNSUPPORTED, DXGI_OUTDUPL_FRAME_INFO,
    DXGI_OUTPUT_DESC,
};

use super::CaptureOptions;

const ACQUIRE_TIMEOUT_MS: u32 = 500;
const FEATURE_LEVELS: [windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL; 2] =
    [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];

pub fn freeze_all(options: CaptureOptions) -> Result<FrozenDesktop, CaptureError> {
    let monitors = crate::monitors::enumerate();
    if monitors.is_empty() {
        return Err(CaptureError::Unavailable(
            "No monitors were found.".to_string(),
        ));
    }
    let captures = capture_outputs(options)?;
    if captures.is_empty() {
        return Err(CaptureError::Unavailable(
            "Desktop duplication is not available.".to_string(),
        ));
    }
    let mut frozen = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        let capture = captures
            .iter()
            .find(|capture| capture.device.eq_ignore_ascii_case(&monitor.name))
            .or_else(|| {
                captures
                    .iter()
                    .find(|capture| capture.bounds == monitor.bounds)
            });
        match capture {
            Some(capture) => frozen.push(FrozenMonitor {
                monitor,
                image: Arc::clone(&capture.image),
            }),
            None => return Err(CaptureError::DisplayChanged),
        }
    }
    Ok(FrozenDesktop { monitors: frozen })
}

pub(crate) fn copy_texture_to_image(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
) -> Result<Image, CaptureError> {
    let mut source = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: `texture` is a live texture and `source` is a live out-parameter.
    unsafe { texture.GetDesc(&mut source) };
    if source.Width == 0 || source.Height == 0 || source.SampleDesc.Count != 1 {
        return Err(CaptureError::Failed(
            "The captured surface has an unsupported size.".to_string(),
        ));
    }
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: source.Width,
        Height: source.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: source.Format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging: Option<ID3D11Texture2D> = None;
    // SAFETY: the description is valid, there is no initial data, and the out-pointer is live.
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }.map_err(
        |error| CaptureError::Failed(format!("A copy surface could not be created: {error}")),
    )?;
    let staging = staging
        .ok_or_else(|| CaptureError::Failed("A copy surface could not be created.".to_string()))?;
    // SAFETY: both textures are live, share the same format, and the destination is a staging texture.
    unsafe { context.CopyResource(&staging, texture) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: the staging texture is live and `mapped` is a live out-parameter.
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(|error| CaptureError::Failed(format!("The capture could not be read: {error}")))?;
    let guard = MapGuard {
        context: context.clone(),
        texture: staging.clone(),
    };
    if mapped.pData.is_null() {
        return Err(CaptureError::Failed(
            "The capture could not be read.".to_string(),
        ));
    }
    let width = usize::try_from(source.Width)
        .map_err(|_| CaptureError::Failed("The capture width is not supported.".to_string()))?;
    let height = usize::try_from(source.Height)
        .map_err(|_| CaptureError::Failed("The capture height is not supported.".to_string()))?;
    let row = width
        .checked_mul(4)
        .ok_or_else(|| CaptureError::Failed("The capture is too large.".to_string()))?;
    let pitch = mapped.RowPitch as usize;
    if pitch < row {
        return Err(CaptureError::Failed(
            "The capture stride is not supported.".to_string(),
        ));
    }
    let length = pitch
        .checked_mul(height)
        .ok_or_else(|| CaptureError::Failed("The capture is too large.".to_string()))?;
    // SAFETY: the mapped region is readable for `pitch * height` bytes for as long as the map guard lives.
    let bytes = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), length) };
    let mut pixels = vec![0_u8; row * height];
    for y in 0..height {
        let from = y * pitch;
        let to = y * row;
        pixels[to..to + row].copy_from_slice(&bytes[from..from + row]);
    }
    drop(guard);
    Image::new(source.Width, source.Height, row, pixels)
        .map_err(|error| CaptureError::Failed(error.to_string()))
}

fn capture_outputs(options: CaptureOptions) -> Result<Vec<OutputCapture>, CaptureError> {
    // SAFETY: no arguments are shared and the factory owns a reference for its lifetime.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|error| {
        CaptureError::Unavailable(format!("DirectX could not be started: {error}"))
    })?;
    let mut captures = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut adapter_index = 0_u32;
    loop {
        // SAFETY: the factory is live and the adapter index advances until enumeration ends.
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(_) => break,
        };
        adapter_index = adapter_index.saturating_add(1);
        let Ok(device) = create_device(&adapter) else {
            continue;
        };
        // SAFETY: the device is live and owns the returned context.
        let Ok(context) = (unsafe { device.GetImmediateContext() }) else {
            continue;
        };
        capture_adapter(
            &adapter,
            &device,
            &context,
            options,
            &mut seen,
            &mut captures,
        );
    }
    Ok(captures)
}

fn capture_adapter(
    adapter: &IDXGIAdapter1,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    options: CaptureOptions,
    seen: &mut Vec<String>,
    captures: &mut Vec<OutputCapture>,
) {
    let mut output_index = 0_u32;
    loop {
        // SAFETY: the adapter is live and the output index advances until enumeration ends.
        let output = match unsafe { adapter.EnumOutputs(output_index) } {
            Ok(output) => output,
            Err(_) => break,
        };
        output_index = output_index.saturating_add(1);
        // SAFETY: the output is live and the descriptor is a live out-parameter.
        let desc = match unsafe { output.GetDesc() } {
            Ok(desc) => desc,
            Err(_) => continue,
        };
        if !desc.AttachedToDesktop.as_bool() {
            continue;
        }
        let name = wide_string(&desc.DeviceName);
        if seen
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        match capture_output(device, context, &output, &desc, options) {
            Ok(image) => {
                seen.push(name.clone());
                captures.push(OutputCapture {
                    device: name,
                    bounds: rect_from(desc.DesktopCoordinates),
                    image: Arc::new(image),
                });
            }
            Err(error) => {
                tracing::debug!(%error, "desktop duplication output failed");
            }
        }
    }
}

fn capture_output(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    output: &IDXGIOutput,
    desc: &DXGI_OUTPUT_DESC,
    options: CaptureOptions,
) -> Result<Image, CaptureError> {
    let output1: IDXGIOutput1 = output.cast().map_err(|error| {
        CaptureError::Unavailable(format!("The output could not be duplicated: {error}"))
    })?;
    // SAFETY: the output and device are live and belong to the same adapter.
    let duplication =
        unsafe { output1.DuplicateOutput(device) }.map_err(|error| duplication_error(&error))?;
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;
    // SAFETY: the duplication is live, the frame info is a live out-parameter, and the resource pointer is live.
    unsafe { duplication.AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut frame_info, &mut resource) }
        .map_err(|error| duplication_error(&error))?;
    let _frame = FrameGuard {
        duplication: &duplication,
    };
    let resource = resource.ok_or_else(|| {
        CaptureError::Failed("The desktop frame could not be acquired.".to_string())
    })?;
    let texture: ID3D11Texture2D = resource.cast().map_err(|error| {
        CaptureError::Failed(format!("The desktop frame could not be read: {error}"))
    })?;
    let mut image = copy_texture_to_image(device, context, &texture)?;
    if options.include_cursor {
        super::gdi::draw_cursor(
            &mut image,
            (desc.DesktopCoordinates.left, desc.DesktopCoordinates.top),
        );
    }
    Ok(image)
}

fn create_device(adapter: &IDXGIAdapter1) -> Result<ID3D11Device, CaptureError> {
    let mut device: Option<ID3D11Device> = None;
    let mut level = D3D_FEATURE_LEVEL_11_0;
    let mut context: Option<ID3D11DeviceContext> = None;
    // SAFETY: the adapter is live, the feature list is static, and every out-pointer refers to a live binding.
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&FEATURE_LEVELS),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut level),
            Some(&mut context),
        )
    }
    .map_err(|error| CaptureError::Unavailable(format!("Direct3D could not start: {error}")))?;
    device
        .ok_or_else(|| CaptureError::Unavailable("Direct3D could not create a device.".to_string()))
}

fn duplication_error(error: &windows::core::Error) -> CaptureError {
    if error.code() == DXGI_ERROR_UNSUPPORTED {
        CaptureError::Unavailable("Desktop duplication is not supported.".to_string())
    } else if error.code() == DXGI_ERROR_ACCESS_LOST
        || error.code() == DXGI_ERROR_INVALID_CALL
        || error.code() == DXGI_ERROR_SESSION_DISCONNECTED
    {
        CaptureError::DisplayChanged
    } else {
        CaptureError::Failed(format!("Desktop duplication failed: {error}"))
    }
}

fn rect_from(rect: windows::Win32::Foundation::RECT) -> Rect {
    Rect::new(
        Px(rect.left),
        Px(rect.top),
        Px(rect.right - rect.left),
        Px(rect.bottom - rect.top),
    )
}

fn wide_string(device: &[u16; 32]) -> String {
    let length = device
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(device.len());
    String::from_utf16_lossy(&device[..length])
}

struct OutputCapture {
    device: String,
    bounds: Rect,
    image: Arc<Image>,
}

struct FrameGuard<'a> {
    duplication: &'a IDXGIOutputDuplication,
}

impl Drop for FrameGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: this releases the frame acquired immediately before the guard was created.
        let _ = unsafe { self.duplication.ReleaseFrame() };
    }
}

struct MapGuard {
    context: ID3D11DeviceContext,
    texture: ID3D11Texture2D,
}

impl Drop for MapGuard {
    fn drop(&mut self) {
        // SAFETY: this unmaps the staging texture that was mapped immediately before the guard was created.
        unsafe { self.context.Unmap(&self.texture, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "desktop"]
    fn dxgi_freeze_returns_one_image_per_monitor_at_physical_size() {
        let monitors = crate::monitors::enumerate();
        match freeze_all(CaptureOptions::default()) {
            Ok(desktop) => {
                assert_eq!(desktop.monitors.len(), monitors.len());
                for frozen in &desktop.monitors {
                    assert_eq!(
                        frozen.image.width(),
                        u32::try_from(frozen.monitor.bounds.width.0).expect("width")
                    );
                    assert_eq!(
                        frozen.image.height(),
                        u32::try_from(frozen.monitor.bounds.height.0).expect("height")
                    );
                }
            }
            Err(error) => {
                eprintln!("desktop duplication is unavailable here: {error}");
            }
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn dxgi_cursor_option_keeps_the_image_size() {
        let with_cursor = freeze_all(CaptureOptions {
            include_cursor: true,
        });
        let without_cursor = freeze_all(CaptureOptions::default());
        if let (Ok(cursor), Ok(plain)) = (with_cursor, without_cursor) {
            assert_eq!(cursor.monitors.len(), plain.monitors.len());
            for (cursor, plain) in cursor.monitors.iter().zip(plain.monitors.iter()) {
                assert_eq!(cursor.image.width(), plain.image.width());
                assert_eq!(cursor.image.height(), plain.image.height());
                assert_eq!(cursor.image.stride(), plain.image.stride());
            }
        }
    }
}
