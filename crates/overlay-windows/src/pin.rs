use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use snapline_core::geometry::{Point, Px, Rect, Size};
use snapline_core::image::Image;
use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext5, ID2D1Image, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR,
};
use windows::Win32::Graphics::DirectComposition::{IDCompositionTarget, IDCompositionVisual};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutput, IDXGISurface, IDXGISwapChain1, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, UpdateWindow, ValidateRect, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowThreadProcessId, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetCursor, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TrackPopupMenuEx, TranslateMessage, UnregisterClassW, CS_DBLCLKS, GWLP_USERDATA, HTCLIENT,
    IDC_ARROW, MA_ACTIVATE, MF_STRING, MSG, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SW_SHOW, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CANCELMODE, WM_CLOSE,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_PAINT, WM_RBUTTONUP,
    WM_SETCURSOR, WM_SYSKEYDOWN, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::d2d::{self, Gpu};
use crate::{OverlayError, Theme};

const CLASS_NAME: &str = "SnaplinePinWindow";
static NEXT_CLASS: AtomicU32 = AtomicU32::new(1);

pub const ZOOM_MIN: f32 = 0.25;
pub const ZOOM_MAX: f32 = 4.0;
pub const ZOOM_STEP: f32 = 0.25;
pub const OPACITY_MIN: f32 = 0.20;
pub const OPACITY_MAX: f32 = 1.0;
pub const OPACITY_STEP: f32 = 0.10;
pub const BORDER_PX: i32 = 1;
pub const SHADOW_PX: i32 = 12;
const ZOOM_DEFAULT: f32 = 1.0;
const OPACITY_DEFAULT: f32 = 1.0;
const SHADOW_STEPS: i32 = 6;
const MENU_COPY: usize = 1;
const MENU_SAVE: usize = 2;
const MENU_ZOOM_100: usize = 3;
const MENU_CLOSE: usize = 4;

pub type PinClipboard = Arc<dyn Fn(&Image) -> Result<(), String> + Send + Sync>;
pub type PinSave = Arc<dyn Fn(&Image) -> Result<(), String> + Send + Sync>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PinError {
    #[error("The pin thread could not be started.")]
    Thread,
    #[error("The pin window could not be created: {0}")]
    Window(String),
    #[error("The pin could not use the graphics device: {0}")]
    Device(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinPlacement {
    CaptureLocation,
    CursorMonitorCentre,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinGeometry {
    pub placement: PinPlacement,
    pub window: Rect,
    pub image: Rect,
}

#[derive(Clone)]
pub struct PinRequest {
    pub image: Image,
    pub capture_origin: Option<Point>,
    pub theme: Theme,
    pub on_copy: Option<PinClipboard>,
    pub on_save: Option<PinSave>,
}

impl PinRequest {
    pub fn new(image: Image) -> Self {
        Self {
            image,
            capture_origin: None,
            theme: Theme::Dark,
            on_copy: None,
            on_save: None,
        }
    }

    pub fn with_capture_origin(mut self, origin: Point) -> Self {
        self.capture_origin = Some(origin);
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_copy(mut self, callback: PinClipboard) -> Self {
        self.on_copy = Some(callback);
        self
    }

    pub fn with_save(mut self, callback: PinSave) -> Self {
        self.on_save = Some(callback);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMenuAction {
    Copy,
    Save,
    Zoom100,
    Close,
}

impl PinMenuAction {
    pub const ALL: [Self; 4] = [Self::Copy, Self::Save, Self::Zoom100, Self::Close];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Save => "Save as…",
            Self::Zoom100 => "Zoom to 100%",
            Self::Close => "Close",
        }
    }

    pub const fn command(self) -> usize {
        match self {
            Self::Copy => MENU_COPY,
            Self::Save => MENU_SAVE,
            Self::Zoom100 => MENU_ZOOM_100,
            Self::Close => MENU_CLOSE,
        }
    }

    pub const fn from_command(command: usize) -> Option<Self> {
        match command {
            MENU_COPY => Some(Self::Copy),
            MENU_SAVE => Some(Self::Save),
            MENU_ZOOM_100 => Some(Self::Zoom100),
            MENU_CLOSE => Some(Self::Close),
            _ => None,
        }
    }
}

pub struct PinHandle {
    window: isize,
    thread_id: u32,
    alive: Arc<Mutex<bool>>,
    geometry: PinGeometry,
    thread: Option<JoinHandle<()>>,
}

impl PinHandle {
    pub const fn geometry(&self) -> PinGeometry {
        self.geometry
    }

    pub const fn placement(&self) -> PinPlacement {
        self.geometry.placement
    }

    pub fn close(&self) {
        let Ok(mut alive) = self.alive.lock() else {
            return;
        };
        if !*alive {
            return;
        }
        // SAFETY: posts to a window created by this process on the pin thread; a stale handle only fails.
        unsafe {
            let hwnd = HWND(self.window as *mut core::ffi::c_void);
            let _ = PostMessageW(Some(hwnd), WM_CANCELMODE, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        *alive = false;
    }
}

impl Drop for PinHandle {
    fn drop(&mut self) {
        self.close();
        let Some(thread) = self.thread.take() else {
            return;
        };
        // SAFETY: reads the calling thread's identifier without borrowing any memory.
        if unsafe { GetCurrentThreadId() } == self.thread_id {
            return;
        }
        let _ = thread.join();
    }
}

pub struct PinHostImpl;

impl Default for PinHostImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl PinHostImpl {
    pub const fn new() -> Self {
        Self
    }

    pub fn open(&self, request: PinRequest) -> Result<PinHandle, PinError> {
        open(request)
    }
}

pub fn open(request: PinRequest) -> Result<PinHandle, PinError> {
    let (sender, receiver) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name(String::from("snapline-pin"))
        .spawn(move || run(request, sender))
        .map_err(|_| PinError::Thread)?;
    let ready = receiver.recv().map_err(|_| PinError::Thread)??;
    Ok(PinHandle {
        window: ready.window,
        thread_id: ready.thread_id,
        alive: ready.alive,
        geometry: ready.geometry,
        thread: Some(thread),
    })
}

struct PinReady {
    window: isize,
    thread_id: u32,
    alive: Arc<Mutex<bool>>,
    geometry: PinGeometry,
}

struct Surface {
    swap_chain: IDXGISwapChain1,
    target: ID2D1Bitmap1,
    bitmap: ID2D1Bitmap1,
    _visual: IDCompositionVisual,
    _dcomp_target: IDCompositionTarget,
}

struct Context {
    gpu: Gpu,
    hwnd: HWND,
    image: Image,
    theme: Theme,
    on_copy: Option<PinClipboard>,
    on_save: Option<PinSave>,
    surface: Option<Surface>,
    zoom: f32,
    opacity: f32,
    geometry: PinGeometry,
    drag: Option<(Point, Point)>,
    alive: Arc<Mutex<bool>>,
    class_name: Vec<u16>,
    instance: HINSTANCE,
}

impl Context {
    fn build(request: PinRequest, alive: Arc<Mutex<bool>>) -> Result<Self, PinError> {
        let PinRequest {
            image,
            capture_origin,
            theme,
            on_copy,
            on_save,
        } = request;
        let cursor = cursor_point().unwrap_or(Point::ORIGIN);
        let work_area = capture_origin
            .map(work_area_at)
            .unwrap_or_else(|| work_area_at(cursor));
        let geometry = place(capture_origin, &image, ZOOM_DEFAULT, work_area);
        let gpu = Gpu::new().map_err(pin_device)?;
        // SAFETY: returns the module handle of this process without borrowing any memory.
        let instance = unsafe { GetModuleHandleW(None) }
            .map_err(|error| PinError::Window(error.to_string()))?;
        let name = format!("{CLASS_NAME}{}", NEXT_CLASS.fetch_add(1, Ordering::Relaxed));
        let class_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        Ok(Self {
            gpu,
            hwnd: HWND::default(),
            image,
            theme,
            on_copy,
            on_save,
            surface: None,
            zoom: ZOOM_DEFAULT,
            opacity: OPACITY_DEFAULT,
            geometry,
            drag: None,
            alive,
            class_name,
            instance: HINSTANCE(instance.0),
        })
    }

    fn activate(&mut self) -> Result<(), PinError> {
        self.register_class()?;
        self.create_window()?;
        let size = self.geometry.window.size();
        let surface = create_surface(&self.gpu, self.hwnd, size, &self.image)?;
        self.surface = Some(surface);
        self.focus();
        self.redraw();
        // SAFETY: the window handle is a live top level window owned by this thread.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = UpdateWindow(self.hwnd);
        }
        Ok(())
    }

    fn ready(&self) -> PinReady {
        PinReady {
            window: self.hwnd.0 as isize,
            // SAFETY: reads the calling thread's identifier without borrowing any memory.
            thread_id: unsafe { GetCurrentThreadId() },
            alive: self.alive.clone(),
            geometry: self.geometry,
        }
    }

    fn register_class(&mut self) -> Result<(), PinError> {
        // SAFETY: loads a stock system cursor owned by the process.
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: self.instance,
            hIcon: Default::default(),
            hCursor: cursor,
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR::from_raw(self.class_name.as_ptr()),
            hIconSm: Default::default(),
        };
        // SAFETY: the class description and its string buffer outlive the call.
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0 {
            return Err(PinError::Window(
                "The pin window class could not be registered.".to_string(),
            ));
        }
        Ok(())
    }

    fn create_window(&mut self) -> Result<(), PinError> {
        let class = PCWSTR::from_raw(self.class_name.as_ptr());
        let style = WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST;
        let area = self.geometry.window;
        // SAFETY: the class name buffer, module handle and coordinates are valid.
        let hwnd = unsafe {
            CreateWindowExW(
                style,
                class,
                PCWSTR::null(),
                WS_POPUP,
                area.x.get(),
                area.y.get(),
                area.width.get(),
                area.height.get(),
                None,
                None,
                Some(self.instance),
                None,
            )
        }
        .map_err(|error| PinError::Window(error.to_string()))?;
        let context = self as *mut Self as isize;
        // SAFETY: the context lives in a boxed value for the whole message loop.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, context);
        }
        self.hwnd = hwnd;
        Ok(())
    }

    fn focus(&self) {
        // SAFETY: all handles are live; attaching input is reversed immediately.
        unsafe {
            let foreground = GetForegroundWindow();
            let thread = GetWindowThreadProcessId(foreground, None);
            let current = GetCurrentThreadId();
            let attached = thread != 0 && AttachThreadInput(current, thread, true).as_bool();
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
            if attached {
                let _ = AttachThreadInput(current, thread, false);
            }
        }
    }

    fn redraw(&mut self) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let target = surface.target.clone();
        let bitmap = surface.bitmap.clone();
        let swap_chain = surface.swap_chain.clone();
        let palette = self.theme.palette();
        let context = self.gpu.context.clone();
        let context = &context;
        let local = local_image_rect(&self.image, self.zoom);
        let left = local.x.get() as f32;
        let top = local.y.get() as f32;
        let width = local.width.get() as f32;
        let height = local.height.get() as f32;
        let opacity = self.opacity;
        // SAFETY: the swap chain target and bitmap belong to this render pass.
        unsafe {
            context.SetTarget(&target);
        }
        self.gpu.begin();
        // SAFETY: clears the whole render target to transparent before any drawing.
        unsafe {
            let clear = transparent();
            let _ = context.Clear(Some(&clear));
        }
        let shadow = SHADOW_PX as f32;
        for step in 0..SHADOW_STEPS {
            let inset = shadow * (step as f32) / (SHADOW_STEPS as f32);
            d2d::fill_round(
                context,
                d2d::rect(
                    left - shadow + inset,
                    top - shadow + inset,
                    width + (shadow - inset) * 2.0,
                    height + (shadow - inset) * 2.0,
                ),
                2.0 + inset,
                faded(palette.shadow, opacity),
            );
        }
        draw_image(
            context,
            &bitmap,
            d2d::rect(left, top, width, height),
            d2d::rect(
                0.0,
                0.0,
                self.image.width() as f32,
                self.image.height() as f32,
            ),
            opacity,
        );
        d2d::stroke_rect(
            context,
            d2d::rect(left + 0.5, top + 0.5, width - 1.0, height - 1.0),
            BORDER_PX as f32,
            faded(palette.stroke_hi, opacity),
        );
        self.gpu.end();
        // SAFETY: the render pass is finished and the buffer can be presented.
        unsafe {
            let _ = context.SetTarget(None::<&ID2D1Image>);
            let _ = swap_chain.Present(1, DXGI_PRESENT(0));
        }
    }

    fn resize_surface(&mut self, size: Size) -> Result<(), PinError> {
        let Some(surface) = self.surface.take() else {
            return Ok(());
        };
        let Surface {
            swap_chain,
            target,
            bitmap,
            _visual,
            _dcomp_target,
        } = surface;
        // SAFETY: detaches the render target before the swap chain buffers are resized.
        unsafe {
            let _ = self.gpu.context.SetTarget(None::<&ID2D1Image>);
        }
        drop(target);
        let width = u32::try_from(size.width.get()).unwrap_or(1);
        let height = u32::try_from(size.height.get()).unwrap_or(1);
        // SAFETY: no buffer references remain and both dimensions are positive.
        unsafe {
            swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                Default::default(),
            )
        }
        .map_err(device_error)?;
        let target = create_target_bitmap(&self.gpu.context, &swap_chain)?;
        self.surface = Some(Surface {
            swap_chain,
            target,
            bitmap,
            _visual,
            _dcomp_target,
        });
        Ok(())
    }

    fn set_zoom(&mut self, zoom: f32) {
        let zoom = clamp_zoom(zoom);
        if (zoom - self.zoom).abs() < f32::EPSILON {
            return;
        }
        self.zoom = zoom;
        let geometry = zoom_geometry(self.geometry, &self.image, zoom);
        self.geometry = geometry;
        let size = geometry.window.size();
        if let Err(error) = self.resize_surface(size) {
            tracing::warn!(%error, "the pin could not be resized");
        }
        // SAFETY: moves the live pin window owned by this thread.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                geometry.window.x.get(),
                geometry.window.y.get(),
                size.width.get(),
                size.height.get(),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.redraw();
    }

    fn set_opacity(&mut self, opacity: f32) {
        let opacity = clamp_opacity(opacity);
        if (opacity - self.opacity).abs() < f32::EPSILON {
            return;
        }
        self.opacity = opacity;
        self.redraw();
    }

    fn begin_drag(&mut self) {
        let Some(cursor) = cursor_point() else {
            return;
        };
        self.drag = Some((cursor, self.geometry.window.origin()));
        // SAFETY: the window handle is live and owned by this thread.
        unsafe {
            SetCapture(self.hwnd);
        }
    }

    fn drag_to(&mut self) {
        let Some((start_cursor, start_window)) = self.drag else {
            return;
        };
        let Some(cursor) = cursor_point() else {
            return;
        };
        let delta = cursor - start_cursor;
        let origin = start_window.offset(delta.x, delta.y);
        let size = self.geometry.window.size();
        let window = Rect::from_origin_size(origin, size);
        let image = self.geometry.image.translated(delta);
        self.geometry = PinGeometry {
            placement: self.geometry.placement,
            window,
            image,
        };
        // SAFETY: moves the live pin window owned by this thread.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                origin.x.get(),
                origin.y.get(),
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    fn end_drag(&mut self) {
        self.drag = None;
        // SAFETY: releases any capture held by this thread.
        unsafe {
            let _ = ReleaseCapture();
        }
    }

    fn copy(&self) {
        let Some(callback) = &self.on_copy else {
            return;
        };
        if let Err(error) = callback(&self.image) {
            tracing::warn!(%error, "the pin could not copy the image");
        }
    }

    fn save(&self) {
        let Some(callback) = &self.on_save else {
            return;
        };
        if let Err(error) = callback(&self.image) {
            tracing::warn!(%error, "the pin could not save the image");
        }
    }

    fn show_menu(&self) -> Option<PinMenuAction> {
        // SAFETY: creates a popup menu owned by this thread until DestroyMenu below.
        let menu = unsafe { CreatePopupMenu() }.ok()?;
        for action in PinMenuAction::ALL {
            let text: Vec<u16> = action
                .label()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: the menu is live and the label buffer outlives the call.
            let _ = unsafe {
                AppendMenuW(
                    menu,
                    MF_STRING,
                    action.command(),
                    PCWSTR::from_raw(text.as_ptr()),
                )
            };
        }
        let point = cursor_point().unwrap_or(Point::ORIGIN);
        // SAFETY: the window handle is live and owned by this thread.
        let _ = unsafe { SetForegroundWindow(self.hwnd) };
        // SAFETY: the menu is live and every argument is a plain value.
        let command = unsafe {
            TrackPopupMenuEx(
                menu,
                (TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY).0,
                point.x.get(),
                point.y.get(),
                self.hwnd,
                None,
            )
        };
        // SAFETY: destroys the menu created above exactly once.
        let _ = unsafe { DestroyMenu(menu) };
        PinMenuAction::from_command(command.0 as u32 as usize)
    }

    fn close(&mut self) {
        self.end_drag();
        if let Ok(mut alive) = self.alive.lock() {
            *alive = false;
        }
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
    }

    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if self.hwnd != hwnd {
            // SAFETY: the message is passed straight through for windows not owned by the context.
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        match message {
            WM_MOUSEACTIVATE => return LRESULT(MA_ACTIVATE as isize),
            WM_SETCURSOR => {
                if (lparam.0 & 0xffff) as i32 == HTCLIENT as i32 {
                    // SAFETY: loads a system cursor and applies it to this thread.
                    unsafe {
                        if let Ok(handle) = LoadCursorW(None, IDC_ARROW) {
                            SetCursor(Some(handle));
                        }
                    }
                    return LRESULT(1);
                }
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                // SAFETY: validates the paint region for the live window.
                unsafe {
                    let _ = ValidateRect(Some(hwnd), None);
                }
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                if self.drag.is_some() {
                    self.drag_to();
                }
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                self.begin_drag();
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                self.end_drag();
                return LRESULT(0);
            }
            WM_LBUTTONDBLCLK => {
                self.close();
                return LRESULT(0);
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
                if control_down() {
                    self.set_opacity(next_opacity(self.opacity, delta));
                } else {
                    self.set_zoom(next_zoom(self.zoom, delta));
                }
                return LRESULT(0);
            }
            WM_RBUTTONUP => {
                match self.show_menu() {
                    Some(PinMenuAction::Copy) => self.copy(),
                    Some(PinMenuAction::Save) => self.save(),
                    Some(PinMenuAction::Zoom100) => self.set_zoom(ZOOM_DEFAULT),
                    Some(PinMenuAction::Close) => self.close(),
                    None => {}
                }
                return LRESULT(0);
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if VIRTUAL_KEY(wparam.0 as u16) == VK_ESCAPE {
                    self.close();
                }
                return LRESULT(0);
            }
            WM_CANCELMODE => {
                self.end_drag();
                return LRESULT(0);
            }
            WM_CLOSE => {
                self.close();
                return LRESULT(0);
            }
            WM_DESTROY | WM_NCCREATE => {}
            _ => {}
        }
        // SAFETY: unhandled messages use the default window procedure.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn destroy(&mut self) {
        if !self.hwnd.is_invalid() {
            // SAFETY: the window handle is live and owned by this thread.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            self.hwnd = HWND::default();
        }
        self.surface = None;
        if self.class_name.is_empty() {
            return;
        }
        // SAFETY: unregisters the class registered for this context.
        unsafe {
            let _ = UnregisterClassW(
                PCWSTR::from_raw(self.class_name.as_ptr()),
                Some(self.instance),
            );
        }
        self.class_name.clear();
    }

    fn run_message_loop() {
        let mut message = MSG::default();
        loop {
            // SAFETY: the message buffer is valid and the window filter is empty.
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if result.0 <= 0 {
                break;
            }
            // SAFETY: standard message dispatch for windows owned by this thread.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn run(request: PinRequest, ready: mpsc::Sender<Result<PinReady, PinError>>) {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated pin thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let alive = Arc::new(Mutex::new(true));
    let result = Context::build(request, alive.clone()).and_then(|mut context| {
        context.activate()?;
        let _ = ready.send(Ok(context.ready()));
        Context::run_message_loop();
        if let Ok(mut alive) = alive.lock() {
            *alive = false;
        }
        Ok(())
    });
    if let Err(error) = result {
        if let Ok(mut alive) = alive.lock() {
            *alive = false;
        }
        let _ = ready.send(Err(error));
    }
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Self {
        // SAFETY: initialises COM for the pin thread with a single threaded apartment.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self {
            initialized: result.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn create_surface(gpu: &Gpu, hwnd: HWND, size: Size, image: &Image) -> Result<Surface, PinError> {
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: u32::try_from(size.width.get()).unwrap_or(1),
        Height: u32::try_from(size.height.get()).unwrap_or(1),
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
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    };
    // SAFETY: the device, description and restriction output are valid.
    let swap_chain = unsafe {
        gpu.dxgi_factory
            .CreateSwapChainForComposition(&gpu.d3d, &desc, None::<&IDXGIOutput>)
    }
    .map_err(device_error)?;
    let target = create_target_bitmap(&gpu.context, &swap_chain)?;
    let bitmap = gpu.upload_image(image).map_err(pin_device)?;
    let dcomp_target = {
        // SAFETY: the window handle is a live top level window owned by this thread.
        unsafe { gpu.dcomp.CreateTargetForHwnd(hwnd, true) }.map_err(device_error)?
    };
    // SAFETY: the composition device is live.
    let visual = unsafe { gpu.dcomp.CreateVisual() }.map_err(device_error)?;
    // SAFETY: the swap chain is a composition surface and the visual accepts it as content.
    unsafe { visual.SetContent(&swap_chain) }.map_err(device_error)?;
    // SAFETY: the target and visual are live and owned by this function.
    unsafe { dcomp_target.SetRoot(&visual) }.map_err(device_error)?;
    // SAFETY: commits the visual tree created above.
    unsafe {
        gpu.dcomp.Commit().map_err(device_error)?;
    }
    Ok(Surface {
        swap_chain,
        target,
        bitmap,
        _visual: visual,
        _dcomp_target: dcomp_target,
    })
}

fn create_target_bitmap(
    context: &ID2D1DeviceContext5,
    swap_chain: &IDXGISwapChain1,
) -> Result<ID2D1Bitmap1, PinError> {
    // SAFETY: the swap chain owns at least one buffer of the requested size.
    let surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0) }.map_err(device_error)?;
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..Default::default()
    };
    // SAFETY: the surface is a live DXGI surface of the swap chain.
    unsafe { context.CreateBitmapFromDxgiSurface(&surface, Some(&properties)) }
        .map_err(device_error)
}

fn transparent() -> D2D1_COLOR_F {
    d2d::colour([0.0, 0.0, 0.0, 0.0])
}

fn faded(rgba: [f32; 4], opacity: f32) -> [f32; 4] {
    [rgba[0], rgba[1], rgba[2], rgba[3] * opacity]
}

fn draw_image(
    context: &ID2D1DeviceContext5,
    bitmap: &ID2D1Bitmap1,
    destination: D2D_RECT_F,
    source: D2D_RECT_F,
    opacity: f32,
) {
    // SAFETY: the bitmap is live, the rectangles are plain values and the opacity is in range.
    unsafe {
        let _ = context.DrawBitmap(
            bitmap,
            Some(&destination),
            opacity,
            D2D1_INTERPOLATION_MODE_LINEAR,
            Some(&source),
            None,
        );
    }
}

fn pin_device(error: OverlayError) -> PinError {
    PinError::Device(error.to_string())
}

fn device_error(error: windows::core::Error) -> PinError {
    PinError::Device(error.to_string())
}

fn cursor_point() -> Option<Point> {
    let mut point = POINT::default();
    // SAFETY: the point output pointer is valid for the duration of the call.
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some(Point::new(Px(point.x), Px(point.y)))
}

fn work_area_at(cursor: Point) -> Rect {
    let mut info = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(0),
        ..Default::default()
    };
    let native = POINT {
        x: cursor.x.get(),
        y: cursor.y.get(),
    };
    // SAFETY: the point came from GetCursorPos and the nearest monitor always exists on a desktop.
    let monitor = unsafe { MonitorFromPoint(native, MONITOR_DEFAULTTONEAREST) };
    // SAFETY: the info structure is live with a valid size and the monitor handle came from MonitorFromPoint.
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Rect::new(
            Px(info.rcWork.left),
            Px(info.rcWork.top),
            Px(info.rcWork.right - info.rcWork.left),
            Px(info.rcWork.bottom - info.rcWork.top),
        );
    }
    // SAFETY: reads two plain system metrics without borrowing any memory.
    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    // SAFETY: reads two plain system metrics without borrowing any memory.
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    Rect::new(Px(0), Px(0), Px(width.max(1)), Px(height.max(1)))
}

fn control_down() -> bool {
    // SAFETY: reads the calling thread's key state.
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

fn wheel_direction(delta: i32) -> i32 {
    delta.signum()
}

pub fn clamp_zoom(zoom: f32) -> f32 {
    zoom.clamp(ZOOM_MIN, ZOOM_MAX)
}

pub fn clamp_opacity(opacity: f32) -> f32 {
    opacity.clamp(OPACITY_MIN, OPACITY_MAX)
}

fn rounded(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

pub fn next_zoom(zoom: f32, delta: i32) -> f32 {
    clamp_zoom(rounded(zoom + ZOOM_STEP * wheel_direction(delta) as f32))
}

pub fn next_opacity(opacity: f32, delta: i32) -> f32 {
    clamp_opacity(rounded(
        opacity + OPACITY_STEP * wheel_direction(delta) as f32,
    ))
}

pub fn image_size(image: &Image, zoom: f32) -> Size {
    let zoom = clamp_zoom(zoom);
    Size::new(
        Px((image.width() as f32 * zoom).round().max(1.0) as i32),
        Px((image.height() as f32 * zoom).round().max(1.0) as i32),
    )
}

pub fn padded_size(image: &Image, zoom: f32) -> Size {
    let inner = image_size(image, zoom);
    let pad = (BORDER_PX + SHADOW_PX) * 2;
    Size::new(Px(inner.width.get() + pad), Px(inner.height.get() + pad))
}

pub fn local_image_rect(image: &Image, zoom: f32) -> Rect {
    let size = image_size(image, zoom);
    let pad = BORDER_PX + SHADOW_PX;
    Rect::new(Px(pad), Px(pad), size.width, size.height)
}

fn clamp_origin(origin: Point, size: Size, work_area: Rect) -> Point {
    let max_x = (work_area.right().get() - size.width.get()).max(work_area.x.get());
    let max_y = (work_area.bottom().get() - size.height.get()).max(work_area.y.get());
    Point::new(
        Px(origin.x.get().clamp(work_area.x.get(), max_x)),
        Px(origin.y.get().clamp(work_area.y.get(), max_y)),
    )
}

pub fn place(
    capture_origin: Option<Point>,
    image: &Image,
    zoom: f32,
    work_area: Rect,
) -> PinGeometry {
    let inner = image_size(image, zoom);
    let padded = padded_size(image, zoom);
    let (placement, image_origin) = match capture_origin {
        Some(origin) => (PinPlacement::CaptureLocation, origin),
        None => (
            PinPlacement::CursorMonitorCentre,
            Point::new(
                Px(work_area.center().x.get() - inner.width.get() / 2),
                Px(work_area.center().y.get() - inner.height.get() / 2),
            ),
        ),
    };
    let pad = BORDER_PX + SHADOW_PX;
    let desired = Point::new(
        Px(image_origin.x.get() - pad),
        Px(image_origin.y.get() - pad),
    );
    let origin = clamp_origin(desired, padded, work_area);
    let window = Rect::from_origin_size(origin, padded);
    let image_rect = Rect::from_origin_size(
        Point::new(Px(origin.x.get() + pad), Px(origin.y.get() + pad)),
        inner,
    );
    PinGeometry {
        placement,
        window,
        image: image_rect,
    }
}

fn zoom_geometry(current: PinGeometry, image: &Image, zoom: f32) -> PinGeometry {
    let zoom = clamp_zoom(zoom);
    let inner = image_size(image, zoom);
    let padded = padded_size(image, zoom);
    let pad = BORDER_PX + SHADOW_PX;
    let origin = Point::new(
        Px(current.image.x.get() - pad),
        Px(current.image.y.get() - pad),
    );
    let window = Rect::from_origin_size(origin, padded);
    let image_rect = Rect::from_origin_size(
        Point::new(Px(origin.x.get() + pad), Px(origin.y.get() + pad)),
        inner,
    );
    PinGeometry {
        placement: current.placement,
        window,
        image: image_rect,
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the user data slot holds a pointer to the boxed context for the message loop's lifetime.
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Context;
    if pointer.is_null() {
        // SAFETY: windows created before the context is attached use the default procedure.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    // SAFETY: the pointer was installed by Context::create_window and remains valid.
    let context = unsafe { &mut *pointer };
    context.handle(hwnd, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> Image {
        Image::bgra(width, height).expect("image")
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn close(actual: f32, expected: f32) -> bool {
        (actual - expected).abs() < 1e-6
    }

    #[test]
    fn zoom_is_clamped_between_a_quarter_and_four_times() {
        assert_eq!(clamp_zoom(0.1), ZOOM_MIN);
        assert_eq!(clamp_zoom(5.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(0.25), ZOOM_MIN);
        assert_eq!(clamp_zoom(4.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(1.5), 1.5);
        assert_eq!(ZOOM_MIN, 0.25);
        assert_eq!(ZOOM_MAX, 4.0);
    }

    #[test]
    fn the_wheel_direction_changes_zoom() {
        assert_eq!(next_zoom(1.0, 120), 1.25);
        assert_eq!(next_zoom(1.0, -120), 0.75);
        assert_eq!(next_zoom(1.0, 0), 1.0);
        assert_eq!(next_zoom(ZOOM_MAX, 120), ZOOM_MAX);
        assert_eq!(next_zoom(ZOOM_MIN, -120), ZOOM_MIN);
        assert!(next_zoom(1.0, 120) > 1.0);
        assert!(next_zoom(1.0, -120) < 1.0);
    }

    #[test]
    fn opacity_is_clamped_between_a_fifth_and_full() {
        assert_eq!(clamp_opacity(0.0), OPACITY_MIN);
        assert_eq!(clamp_opacity(2.0), OPACITY_MAX);
        assert_eq!(clamp_opacity(0.2), OPACITY_MIN);
        assert_eq!(clamp_opacity(1.0), OPACITY_MAX);
        assert!(close(clamp_opacity(0.65), 0.65));
        assert_eq!(OPACITY_MIN, 0.20);
        assert_eq!(OPACITY_MAX, 1.0);
    }

    #[test]
    fn the_wheel_direction_changes_opacity() {
        assert!(close(next_opacity(1.0, -120), 0.9));
        assert!(close(next_opacity(0.2, 120), 0.3));
        assert!(close(next_opacity(0.5, 0), 0.5));
        assert!(close(next_opacity(OPACITY_MIN, -120), OPACITY_MIN));
        assert!(close(next_opacity(OPACITY_MAX, 120), OPACITY_MAX));
        assert!(next_opacity(1.0, -120) < 1.0);
        assert!(next_opacity(0.2, 120) > 0.2);
    }

    #[test]
    fn menu_actions_map_to_unique_commands() {
        for action in PinMenuAction::ALL {
            assert_eq!(PinMenuAction::from_command(action.command()), Some(action));
        }
        for (index, action) in PinMenuAction::ALL.iter().enumerate() {
            for other in &PinMenuAction::ALL[index + 1..] {
                assert_ne!(action.command(), other.command());
            }
        }
        assert_eq!(PinMenuAction::from_command(0), None);
        assert_eq!(PinMenuAction::from_command(99), None);
        assert_eq!(PinMenuAction::Copy.label(), "Copy");
        assert_eq!(PinMenuAction::Save.label(), "Save as…");
        assert_eq!(PinMenuAction::Zoom100.label(), "Zoom to 100%");
        assert_eq!(PinMenuAction::Close.label(), "Close");
    }

    #[test]
    fn the_border_and_shadow_constants_match_the_plan() {
        assert_eq!(BORDER_PX, 1);
        assert_eq!(SHADOW_PX, 12);
        let image = image(100, 50);
        assert_eq!(padded_size(&image, 1.0), Size::new(Px(126), Px(76)));
        assert_eq!(local_image_rect(&image, 1.0), rect(13, 13, 100, 50));
        let padded = padded_size(&image, 1.0);
        assert_eq!(padded.width.get() - 100, 26);
        assert_eq!(padded.height.get() - 50, 26);
    }

    #[test]
    fn zoom_scales_the_image_and_keeps_pixel_rounding() {
        let image = image(100, 50);
        assert_eq!(image_size(&image, 1.0), Size::new(Px(100), Px(50)));
        assert_eq!(image_size(&image, 0.25), Size::new(Px(25), Px(13)));
        assert_eq!(image_size(&image, 4.0), Size::new(Px(400), Px(200)));
        assert_eq!(image_size(&image, 9.0), Size::new(Px(400), Px(200)));
    }

    #[test]
    fn a_capture_origin_places_the_image_at_its_capture_location() {
        let image = image(100, 50);
        let work_area = rect(0, 0, 1920, 1080);
        let geometry = place(Some(Point::new(Px(200), Px(150))), &image, 1.0, work_area);
        assert_eq!(geometry.placement, PinPlacement::CaptureLocation);
        assert_eq!(geometry.image, rect(200, 150, 100, 50));
        assert_eq!(geometry.window, rect(187, 137, 126, 76));
    }

    #[test]
    fn the_cursor_monitor_centre_is_used_without_a_capture_origin() {
        let image = image(100, 50);
        let work_area = rect(0, 0, 1920, 1080);
        let geometry = place(None, &image, 1.0, work_area);
        assert_eq!(geometry.placement, PinPlacement::CursorMonitorCentre);
        assert_eq!(geometry.image, rect(910, 515, 100, 50));
        assert_eq!(geometry.window, rect(897, 502, 126, 76));
    }

    #[test]
    fn placement_uses_a_secondary_monitors_work_area() {
        let image = image(100, 50);
        let work_area = rect(1920, 0, 1920, 1040);
        let geometry = place(None, &image, 1.0, work_area);
        assert_eq!(geometry.placement, PinPlacement::CursorMonitorCentre);
        assert_eq!(geometry.image, rect(2830, 495, 100, 50));
        assert_eq!(geometry.window, rect(2817, 482, 126, 76));
        assert!(work_area.contains_rect(geometry.window));
    }

    #[test]
    fn placement_is_clamped_inside_the_work_area() {
        let image = image(100, 50);
        let work_area = rect(0, 0, 1920, 1080);
        let geometry = place(Some(Point::new(Px(1900), Px(1070))), &image, 1.0, work_area);
        assert_eq!(geometry.placement, PinPlacement::CaptureLocation);
        assert_eq!(geometry.window, rect(1794, 1004, 126, 76));
        assert_eq!(geometry.image, rect(1807, 1017, 100, 50));
        assert!(work_area.contains_rect(geometry.window));
    }

    #[test]
    fn zooming_keeps_the_image_top_left_fixed() {
        let image = image(100, 50);
        let work_area = rect(0, 0, 1920, 1080);
        let geometry = place(Some(Point::new(Px(200), Px(150))), &image, 1.0, work_area);
        let zoomed = zoom_geometry(geometry, &image, 2.0);
        assert_eq!(zoomed.placement, PinPlacement::CaptureLocation);
        assert_eq!(zoomed.image, rect(200, 150, 200, 100));
        assert_eq!(zoomed.window, rect(187, 137, 226, 126));
        let clamped = zoom_geometry(zoomed, &image, 10.0);
        assert_eq!(clamped.image.width, Px(400));
        assert_eq!(clamped.image.height, Px(200));
        assert_eq!(clamped.image, rect(200, 150, 400, 200));
    }

    #[test]
    #[ignore = "desktop"]
    fn a_pin_opens_and_closes() {
        let image = image(64, 48);
        let handle = open(PinRequest::new(image).with_capture_origin(Point::new(Px(100), Px(100))))
            .expect("pin");
        assert_eq!(handle.placement(), PinPlacement::CaptureLocation);
        assert_eq!(handle.geometry().image, rect(100, 100, 64, 48));
        handle.close();
    }
}
