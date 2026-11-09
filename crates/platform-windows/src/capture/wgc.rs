use std::ffi::c_void;
use std::mem::size_of;
use std::time::Duration;

use snapline_core::ids::WindowId;
use snapline_core::image::Image;
use snapline_core::ports::CaptureError;
use windows::core::{Interface, HSTRING};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HWND, POINT, RECT, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11Texture2D, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{
    RoGetActivationFactory, RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, GetForegroundWindow, GetWindowRect, IsWindow, WindowFromPoint,
    GA_ROOT,
};

const FRAME_POLL_ATTEMPTS: usize = 200;
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(5);
const FEATURE_LEVELS: [windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL; 2] =
    [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];

pub fn capture_window(
    id: WindowId,
    keep_shadow: bool,
    include_cursor: bool,
) -> Result<Image, CaptureError> {
    let hwnd = hwnd_of(id);
    if hwnd.0.is_null() {
        return Err(CaptureError::WindowMissing);
    }
    // SAFETY: the handle was supplied by the caller and is only read by the window manager.
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err(CaptureError::WindowMissing);
    }
    let geometry = window_geometry(hwnd)?;
    let _apartment = WinRtApartment::start()?;
    if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
        return Err(CaptureError::Unavailable(
            "Windows graphics capture is not available on this PC.".to_string(),
        ));
    }
    let item = capture_item(hwnd)?;
    let image = capture_item_frame(&item, include_cursor)?;
    if keep_shadow {
        Ok(image)
    } else {
        crop_to_extended(&image, &geometry)
    }
}

pub fn foreground_window() -> Option<WindowId> {
    // SAFETY: GetForegroundWindow has no preconditions and returns a borrowed handle.
    window_id(unsafe { GetForegroundWindow() })
}

pub fn window_under_cursor() -> Option<WindowId> {
    let mut point = POINT::default();
    // SAFETY: `point` is a live out-parameter for the cursor position.
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return None;
    }
    // SAFETY: the point was just produced by GetCursorPos.
    let window = unsafe { WindowFromPoint(point) };
    if window.0.is_null() {
        return None;
    }
    // SAFETY: the window handle came from WindowFromPoint and GA_ROOT asks for its top-level ancestor.
    window_id(unsafe { GetAncestor(window, GA_ROOT) })
}

fn capture_item(hwnd: HWND) -> Result<GraphicsCaptureItem, CaptureError> {
    // SAFETY: the class name is a static Windows Runtime class that is present on supported systems.
    let interop: IGraphicsCaptureItemInterop = unsafe {
        RoGetActivationFactory(&HSTRING::from(
            "Windows.Graphics.Capture.GraphicsCaptureItem",
        ))
    }
    .map_err(|error| {
        CaptureError::Unavailable(format!("Window capture could not start: {error}"))
    })?;
    // SAFETY: the interop factory is live and the window handle was validated by IsWindow.
    unsafe { interop.CreateForWindow(hwnd) }
        .map_err(|error| CaptureError::Failed(format!("The window could not be captured: {error}")))
}

fn capture_item_frame(
    item: &GraphicsCaptureItem,
    include_cursor: bool,
) -> Result<Image, CaptureError> {
    let size = item.Size().map_err(|error| {
        CaptureError::Failed(format!("The window size could not be read: {error}"))
    })?;
    if size.Width <= 0 || size.Height <= 0 {
        return Err(CaptureError::Failed(
            "The window has no drawable area.".to_string(),
        ));
    }
    let device = create_device()?;
    let dxgi_device: IDXGIDevice = device.cast().map_err(|error| {
        CaptureError::Unavailable(format!("Direct3D could not be reached: {error}"))
    })?;
    // SAFETY: the DXGI device is live and the returned WinRT device owns a reference to it.
    let inspectable =
        unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }.map_err(|error| {
            CaptureError::Unavailable(format!("Direct3D could not be shared: {error}"))
        })?;
    let winrt_device: IDirect3DDevice = inspectable.cast().map_err(|error| {
        CaptureError::Unavailable(format!("Direct3D could not be shared: {error}"))
    })?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )
    .map_err(|error| {
        CaptureError::Failed(format!(
            "A capture frame pool could not be created: {error}"
        ))
    })?;
    let session = pool.CreateCaptureSession(item).map_err(|error| {
        CaptureError::Failed(format!("A capture session could not be created: {error}"))
    })?;
    let _guard = SessionGuard {
        session: session.clone(),
        pool: pool.clone(),
    };
    if crate::theme::is_windows_11() {
        let _ = session.SetIsBorderRequired(false);
    }
    let _ = session.SetIsCursorCaptureEnabled(include_cursor);
    session.StartCapture().map_err(|error| {
        CaptureError::Failed(format!("The capture session could not start: {error}"))
    })?;
    let frame = wait_for_frame(&pool)?;
    let surface = frame.Surface().map_err(|error| {
        CaptureError::Failed(format!("The captured surface could not be read: {error}"))
    })?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(|error| {
        CaptureError::Failed(format!("The captured surface could not be read: {error}"))
    })?;
    // SAFETY: the access interface wraps a live Direct3D surface and requests its DXGI texture.
    let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(|error| {
        CaptureError::Failed(format!("The captured texture could not be read: {error}"))
    })?;
    // SAFETY: the device is live and returns its immediate context.
    let context = unsafe { device.GetImmediateContext() }.map_err(|error| {
        CaptureError::Failed(format!("The capture device could not be used: {error}"))
    })?;
    super::dxgi::copy_texture_to_image(&device, &context, &texture)
}

fn wait_for_frame(
    pool: &Direct3D11CaptureFramePool,
) -> Result<Direct3D11CaptureFrame, CaptureError> {
    for _ in 0..FRAME_POLL_ATTEMPTS {
        match pool.TryGetNextFrame() {
            Ok(frame) => return Ok(frame),
            Err(_) => std::thread::sleep(FRAME_POLL_INTERVAL),
        }
    }
    Err(CaptureError::Failed(
        "The window frame could not be captured.".to_string(),
    ))
}

fn create_device() -> Result<ID3D11Device, CaptureError> {
    if let Ok(device) = create_device_with(D3D_DRIVER_TYPE_HARDWARE) {
        return Ok(device);
    }
    create_device_with(D3D_DRIVER_TYPE_WARP)
}

fn create_device_with(
    driver: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE,
) -> Result<ID3D11Device, CaptureError> {
    let mut device: Option<ID3D11Device> = None;
    let mut level = D3D_FEATURE_LEVEL_11_0;
    let mut context = None;
    // SAFETY: no adapter is required, the feature list is static, and every out-pointer refers to a live binding.
    unsafe {
        D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            driver,
            windows::Win32::Foundation::HMODULE::default(),
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

fn window_geometry(hwnd: HWND) -> Result<WindowGeometry, CaptureError> {
    let mut window = RECT::default();
    // SAFETY: the window handle is valid and `window` is a live out-parameter.
    unsafe { GetWindowRect(hwnd, &mut window) }.map_err(|error| {
        CaptureError::Failed(format!("The window bounds could not be read: {error}"))
    })?;
    let mut extended = RECT::default();
    // SAFETY: the attribute and out-parameter are the documented types for DWMWA_EXTENDED_FRAME_BOUNDS.
    let status = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut extended as *mut RECT).cast(),
            u32::try_from(size_of::<RECT>()).unwrap_or(0),
        )
    };
    if status.is_err() {
        extended = window;
    }
    Ok(WindowGeometry { window, extended })
}

fn crop_to_extended(image: &Image, geometry: &WindowGeometry) -> Result<Image, CaptureError> {
    let width = i32::try_from(image.width())
        .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?;
    let height = i32::try_from(image.height())
        .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?;
    let offset_x = geometry.extended.left - geometry.window.left;
    let offset_y = geometry.extended.top - geometry.window.top;
    let left = offset_x.clamp(0, width);
    let top = offset_y.clamp(0, height);
    let right = (offset_x + (geometry.extended.right - geometry.extended.left)).clamp(left, width);
    let bottom = (offset_y + (geometry.extended.bottom - geometry.extended.top)).clamp(top, height);
    let crop_width = usize::try_from(right - left)
        .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?;
    let crop_height = usize::try_from(bottom - top)
        .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?;
    if crop_width == 0 || crop_height == 0 {
        return Ok(image.clone());
    }
    if crop_width == usize::try_from(width).unwrap_or(0)
        && crop_height == usize::try_from(height).unwrap_or(0)
    {
        return Ok(image.clone());
    }
    let left = u32::try_from(left)
        .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?;
    let top = u32::try_from(top)
        .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?;
    let mut cropped = Image::bgra(
        u32::try_from(crop_width)
            .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?,
        u32::try_from(crop_height)
            .map_err(|_| CaptureError::Failed("The window capture is too large.".to_string()))?,
    )
    .map_err(|error| CaptureError::Failed(error.to_string()))?;
    for y in 0..crop_height {
        let source_row = image
            .row(top + u32::try_from(y).unwrap_or(0))
            .ok_or_else(|| CaptureError::Failed("The window capture is too small.".to_string()))?;
        let target_row = cropped
            .row_mut(u32::try_from(y).unwrap_or(0))
            .ok_or_else(|| CaptureError::Failed("The window capture is too small.".to_string()))?;
        let start = usize::try_from(left).unwrap_or(0) * 4;
        let end = start + crop_width * 4;
        target_row.copy_from_slice(&source_row[start..end]);
    }
    Ok(cropped)
}

fn hwnd_of(id: WindowId) -> HWND {
    HWND(id.get() as usize as *mut c_void)
}

fn window_id(hwnd: HWND) -> Option<WindowId> {
    if hwnd.0.is_null() {
        None
    } else {
        Some(WindowId::new(hwnd.0 as usize as u64))
    }
}

struct WindowGeometry {
    window: RECT,
    extended: RECT,
}

struct WinRtApartment {
    uninit: bool,
}

impl WinRtApartment {
    fn start() -> Result<Self, CaptureError> {
        // SAFETY: multithreaded Windows Runtime initialisation is balanced by RoUninitialize in Drop.
        let status = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        match status {
            Ok(()) => Ok(Self { uninit: true }),
            Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(Self { uninit: false }),
            Err(error) => Err(CaptureError::Unavailable(format!(
                "Windows graphics capture could not start: {error}"
            ))),
        }
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        if self.uninit {
            // SAFETY: this balances the successful RoInitialize in start on the same thread.
            unsafe { RoUninitialize() };
        }
    }
}

struct SessionGuard {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "desktop"]
    fn foreground_window_can_be_captured() {
        let Some(window) = foreground_window() else {
            panic!("there is no foreground window");
        };
        let image = capture_window(window, true, false).expect("window capture");
        assert!(image.width() > 0);
        assert!(image.height() > 0);
    }

    #[test]
    #[ignore = "desktop"]
    fn window_under_the_cursor_can_be_captured() {
        let Some(window) = window_under_cursor() else {
            panic!("there is no window under the cursor");
        };
        let image = capture_window(window, true, false).expect("window capture");
        assert!(image.width() > 0);
        assert!(image.height() > 0);
    }

    #[test]
    #[ignore = "desktop"]
    fn shadow_and_cropped_captures_have_the_expected_size() {
        let Some(window) = foreground_window() else {
            panic!("there is no foreground window");
        };
        let with_shadow = capture_window(window, true, false).expect("shadow capture");
        let cropped = capture_window(window, false, false).expect("cropped capture");
        assert!(with_shadow.width() >= cropped.width());
        assert!(with_shadow.height() >= cropped.height());
        assert!(cropped.width() > 0);
        assert!(cropped.height() > 0);
    }

    #[test]
    #[ignore = "desktop"]
    fn cursor_option_changes_the_captured_session() {
        let Some(window) = foreground_window() else {
            panic!("there is no foreground window");
        };
        let cursor = capture_window(window, true, true).expect("cursor capture");
        let plain = capture_window(window, true, false).expect("plain capture");
        assert_eq!(cursor.width(), plain.width());
        assert_eq!(cursor.height(), plain.height());
    }
}
