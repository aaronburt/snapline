use std::sync::{mpsc, Arc};

use snapline_core::geometry::{Point, Px, Rect, Size};
use snapline_core::ports::{FrozenDesktop, Monitor};
use snapline_core::ruler::RulerMeasurement;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::Graphics::Direct2D::ID2D1Image;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain1, DXGI_PRESENT};
use windows::Win32::Graphics::Gdi::{UpdateWindow, ValidateRect};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_ESCAPE,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId, LoadCursorW,
    PostQuitMessage, RegisterClassExW, SetCursor, SetForegroundWindow, SetWindowLongPtrW,
    ShowWindow, TranslateMessage, UnregisterClassW, GWLP_USERDATA, HTCLIENT, IDC_CROSS,
    MA_ACTIVATE, MSG, SW_SHOW, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_SETCURSOR,
    WM_SYSKEYDOWN, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

use crate::d2d::{self, Gpu};
use crate::tokens::Palette;
use crate::{OverlayError, Theme};

const CLASS_NAME: &str = "SnaplineRulerWindow";

pub type RulerCopy = Arc<dyn Fn(&str) + Send + Sync>;

pub struct RulerRequest {
    pub desktop: FrozenDesktop,
    pub theme: Theme,
    pub on_copy: Option<RulerCopy>,
}

impl RulerRequest {
    pub fn new(desktop: FrozenDesktop) -> Self {
        Self {
            desktop,
            theme: Theme::Dark,
            on_copy: None,
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_copy(mut self, callback: RulerCopy) -> Self {
        self.on_copy = Some(callback);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulerMeasure {
    pub start: Point,
    pub end: Point,
    pub monitor: Option<u32>,
    pub copied: bool,
}

pub struct RulerHostImpl;

impl Default for RulerHostImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl RulerHostImpl {
    pub const fn new() -> Self {
        Self
    }

    pub fn measure(&self, request: RulerRequest) -> Result<RulerMeasure, OverlayError> {
        if request.desktop.monitors.is_empty() {
            return Err(OverlayError::NoMonitors);
        }
        run(request)
    }
}

fn run(request: RulerRequest) -> Result<RulerMeasure, OverlayError> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name(String::from("snapline-ruler"))
        .spawn(move || {
            let result = execute(request);
            let _ = sender.send(result);
        })
        .map_err(|_| OverlayError::Thread)?;
    receiver.recv().map_err(|_| OverlayError::Thread)?
}

fn execute(request: RulerRequest) -> Result<RulerMeasure, OverlayError> {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated ruler thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let cursor = cursor_position().or_else(|| {
        request
            .desktop
            .monitors
            .first()
            .map(|frozen| frozen.monitor.bounds.center())
    });
    let cursor = cursor.unwrap_or(Point::ORIGIN);
    let mut context = Box::new(Context::build(request, cursor)?);
    context.activate()?;
    Context::run_message_loop();
    let outcome = context.outcome.take();
    drop(context);
    match outcome {
        Some(Outcome::Finish(measure)) => Ok(measure),
        Some(Outcome::Cancel) | None => Err(OverlayError::Cancelled),
    }
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Self {
        // SAFETY: initialises COM for the ruler thread with a single threaded apartment.
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

fn cursor_position() -> Option<Point> {
    let mut point = POINT::default();
    // SAFETY: the point output pointer is valid for the duration of the call.
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some(Point::new(Px(point.x), Px(point.y)))
}

pub(crate) fn monitor_at(bounds: &[Rect], cursor: Point) -> Option<usize> {
    bounds
        .iter()
        .position(|rect| rect.contains(cursor))
        .or_else(|| (!bounds.is_empty()).then_some(0))
}

pub(crate) fn local_point(point: Point, origin: Point) -> Point {
    Point::new(
        Px(point.x.get() - origin.x.get()),
        Px(point.y.get() - origin.y.get()),
    )
}

pub(crate) fn readout_placement(
    cursor: Point,
    window: Size,
    width: i32,
    height: i32,
    offset: i32,
) -> Rect {
    let mut x = cursor.x.get() + offset;
    if x + width > window.width.get() {
        x = cursor.x.get() - offset - width;
    }
    x = x.clamp(0, (window.width.get() - width).max(0));
    let mut y = cursor.y.get() + offset;
    if y + height > window.height.get() {
        y = cursor.y.get() - offset - height;
    }
    y = y.clamp(0, (window.height.get() - height).max(0));
    Rect::new(Px(x), Px(y), Px(width), Px(height))
}

fn readout_strings(measurement: RulerMeasurement) -> (String, String, String) {
    (
        measurement.size_text(),
        format!("{:.1} px", measurement.diagonal()),
        format!("{:.1}°", measurement.angle_degrees()),
    )
}

fn measure_between(start: Point, end: Point, shift: bool) -> RulerMeasurement {
    RulerMeasurement::measure_with_lock(start, end, shift)
}

enum Outcome {
    Finish(RulerMeasure),
    Cancel,
}

struct Surface {
    swap_chain: IDXGISwapChain1,
    target: ID2D1Bitmap1,
    frozen: ID2D1Bitmap1,
}

struct Context {
    gpu: Gpu,
    monitor: Monitor,
    scale: f32,
    origin: Point,
    size: Size,
    image: snapline_core::image::Image,
    theme: Theme,
    on_copy: Option<RulerCopy>,
    hwnd: Option<HWND>,
    surface: Option<Surface>,
    cursor: Point,
    start: Option<Point>,
    measurement: Option<RulerMeasurement>,
    dragging: bool,
    outcome: Option<Outcome>,
    class_name: Vec<u16>,
    instance: HINSTANCE,
}

impl Context {
    fn build(request: RulerRequest, cursor: Point) -> Result<Self, OverlayError> {
        let RulerRequest {
            desktop,
            theme,
            on_copy,
        } = request;
        let bounds: Vec<Rect> = desktop
            .monitors
            .iter()
            .map(|frozen| frozen.monitor.bounds)
            .collect();
        let index = monitor_at(&bounds, cursor).ok_or(OverlayError::NoMonitors)?;
        let frozen = desktop
            .monitors
            .into_iter()
            .nth(index)
            .ok_or(OverlayError::NoMonitors)?;
        let monitor = frozen.monitor;
        let scale = monitor.dpi.scale();
        let origin = monitor.bounds.origin();
        let size = monitor.bounds.size();
        let gpu = Gpu::new()?;
        // SAFETY: returns the module handle of this process without borrowing any memory.
        let instance = unsafe { GetModuleHandleW(None) }
            .map_err(|error| OverlayError::Device(error.to_string()))?;
        let class_name: Vec<u16> = CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        Ok(Self {
            gpu,
            image: frozen.image,
            monitor,
            scale,
            origin,
            size,
            theme,
            on_copy,
            hwnd: None,
            surface: None,
            cursor,
            start: None,
            measurement: None,
            dragging: false,
            outcome: None,
            class_name,
            instance: HINSTANCE(instance.0),
        })
    }

    fn activate(&mut self) -> Result<(), OverlayError> {
        self.register_class()?;
        let hwnd = self.create_window()?;
        self.hwnd = Some(hwnd);
        let width = u32::try_from(self.size.width.get()).unwrap_or(1);
        let height = u32::try_from(self.size.height.get()).unwrap_or(1);
        let swap = self.gpu.create_target(hwnd, width, height)?;
        let frozen = self.gpu.upload_image(&self.image)?;
        self.surface = Some(Surface {
            swap_chain: swap.swap_chain,
            target: swap.target,
            frozen,
        });
        self.focus(hwnd);
        self.redraw();
        // SAFETY: the window handle is a live top level window owned by this thread.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);
        }
        Ok(())
    }

    fn register_class(&mut self) -> Result<(), OverlayError> {
        // SAFETY: loads a stock system cursor owned by the process.
        let cursor = unsafe { LoadCursorW(None, IDC_CROSS) }.unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: Default::default(),
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
            return Err(OverlayError::Device(
                "The ruler window class could not be registered.".to_string(),
            ));
        }
        Ok(())
    }

    fn create_window(&mut self) -> Result<HWND, OverlayError> {
        let class = PCWSTR::from_raw(self.class_name.as_ptr());
        let style = WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST;
        let bounds = self.monitor.bounds;
        // SAFETY: the class name buffer, module handle and coordinates are valid.
        let hwnd = unsafe {
            CreateWindowExW(
                style,
                class,
                PCWSTR::null(),
                WS_POPUP,
                bounds.x.get(),
                bounds.y.get(),
                bounds.width.get(),
                bounds.height.get(),
                None,
                None,
                Some(self.instance),
                None,
            )
        }
        .map_err(|error| OverlayError::Device(error.to_string()))?;
        let context = self as *mut Self as isize;
        // SAFETY: the context lives in a boxed value for the whole message loop.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, context);
        }
        Ok(hwnd)
    }

    fn focus(&self, hwnd: HWND) {
        // SAFETY: all handles are live; attaching input is reversed immediately.
        unsafe {
            let foreground = GetForegroundWindow();
            let thread = GetWindowThreadProcessId(foreground, None);
            let current = GetCurrentThreadId();
            let attached = thread != 0 && AttachThreadInput(current, thread, true).as_bool();
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
            if attached {
                let _ = AttachThreadInput(current, thread, false);
            }
        }
    }

    fn pointer(&self, lparam: LPARAM) -> Point {
        let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
        Point::new(Px(x + self.origin.x.get()), Px(y + self.origin.y.get()))
    }

    fn sync_measurement(&mut self, shift: bool) {
        if let Some(start) = self.start {
            self.measurement = Some(measure_between(start, self.cursor, shift));
        }
    }

    fn redraw(&mut self) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let target = surface.target.clone();
        let frozen = surface.frozen.clone();
        let swap_chain = surface.swap_chain.clone();
        let palette = self.theme.palette();
        let scale = self.scale;
        let origin = self.origin;
        let size = self.size;
        let cursor = local_point(self.cursor, origin);
        let dragging = self.dragging;
        let measurement = self.measurement;
        let context = self.gpu.context.clone();
        // SAFETY: the swap chain target and bitmap belong to this render pass.
        unsafe {
            context.SetTarget(&target);
        }
        self.gpu.begin();
        let black = [0.0, 0.0, 0.0, 1.0];
        // SAFETY: clears the whole render target before any drawing.
        unsafe {
            let clear = d2d::colour(black);
            let _ = context.Clear(Some(&clear));
        }
        d2d::draw_bitmap(
            &context,
            &frozen,
            d2d::rect(0.0, 0.0, size.width.get() as f32, size.height.get() as f32),
            d2d::rect(
                0.0,
                0.0,
                self.image.width() as f32,
                self.image.height() as f32,
            ),
            false,
        );
        d2d::fill_rect(
            &context,
            d2d::rect(cursor.x.get() as f32, 0.0, 1.0, size.height.get() as f32),
            palette.accent,
        );
        d2d::fill_rect(
            &context,
            d2d::rect(0.0, cursor.y.get() as f32, size.width.get() as f32, 1.0),
            palette.accent,
        );
        if let Some(measurement) = measurement {
            draw_measurement(&context, palette, scale, origin, measurement);
            let anchor = if dragging {
                cursor
            } else {
                local_point(measurement.end, origin)
            };
            self.draw_readout(palette, scale, anchor, measurement);
        }
        self.gpu.end();
        // SAFETY: the render pass is finished and the buffer can be presented.
        unsafe {
            let _ = context.SetTarget(None::<&ID2D1Image>);
            let _ = swap_chain.Present(1, DXGI_PRESENT(0));
        }
    }

    fn draw_readout(
        &mut self,
        palette: &Palette,
        scale: f32,
        cursor: Point,
        measurement: RulerMeasurement,
    ) {
        let (size_text, diagonal_text, angle_text) = readout_strings(measurement);
        let text_size = 12.0 * scale;
        let pad = 8.0 * scale;
        let gap = 8.0 * scale;
        let gpu = &mut self.gpu;
        let size_width = text_width(gpu, text_size, DWRITE_FONT_WEIGHT_SEMI_BOLD, &size_text);
        let diagonal_width = text_width(gpu, text_size, DWRITE_FONT_WEIGHT_NORMAL, &diagonal_text);
        let angle_width = text_width(gpu, text_size, DWRITE_FONT_WEIGHT_NORMAL, &angle_text);
        let width = (pad * 2.0 + size_width + gap + diagonal_width + gap + angle_width)
            .round()
            .max(1.0) as i32;
        let height = (24.0 * scale).round().max(1.0) as i32;
        let offset = (20.0 * scale).round().max(1.0) as i32;
        let area = readout_placement(cursor, self.size, width, height, offset);
        let x = area.x.get() as f32;
        let y = area.y.get() as f32;
        let w = area.width.get() as f32;
        let h = area.height.get() as f32;
        d2d::fill_round(
            &gpu.context,
            d2d::rect(x, y, w, h),
            4.0 * scale,
            palette.layer,
        );
        d2d::stroke_round(
            &gpu.context,
            d2d::rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0),
            4.0 * scale,
            1.0,
            palette.stroke,
        );
        if let Some(format) =
            gpu.text
                .format(&gpu.write, text_size, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)
        {
            d2d::draw_text(
                &gpu.context,
                &format,
                &size_text,
                d2d::rect(x + pad, y, size_width, h),
                palette.text,
            );
        }
        let diagonal_x = x + pad + size_width + gap;
        if let Some(format) =
            gpu.text
                .format(&gpu.write, text_size, DWRITE_FONT_WEIGHT_NORMAL, true)
        {
            d2d::draw_text(
                &gpu.context,
                &format,
                &diagonal_text,
                d2d::rect(diagonal_x, y, diagonal_width, h),
                palette.text3,
            );
            d2d::draw_text(
                &gpu.context,
                &format,
                &angle_text,
                d2d::rect(diagonal_x + diagonal_width + gap, y, angle_width, h),
                palette.text3,
            );
        }
    }

    fn copy(&mut self) {
        let Some(measurement) = self.measurement else {
            return;
        };
        let text = measurement.copy_text();
        if let Some(callback) = &self.on_copy {
            callback(&text);
        }
        self.outcome = Some(Outcome::Finish(RulerMeasure {
            start: measurement.start,
            end: measurement.end,
            monitor: Some(self.monitor.id),
            copied: self.on_copy.is_some(),
        }));
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
    }

    fn cancel(&mut self) {
        self.outcome = Some(Outcome::Cancel);
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
    }

    fn on_key(&mut self, key: VIRTUAL_KEY) {
        match key {
            VK_ESCAPE => self.cancel(),
            VK_C if key_down(VK_CONTROL) => self.copy(),
            _ => {}
        }
    }

    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if self.hwnd != Some(hwnd) {
            // SAFETY: the message is passed straight through for windows not owned by the context.
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        match message {
            WM_MOUSEACTIVATE => return LRESULT(MA_ACTIVATE as isize),
            WM_SETCURSOR => {
                if (lparam.0 & 0xffff) as i32 == HTCLIENT as i32 {
                    // SAFETY: loads a system cursor and applies it to this thread.
                    unsafe {
                        if let Ok(handle) = LoadCursorW(None, IDC_CROSS) {
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
                self.cursor = self.pointer(lparam);
                if self.dragging {
                    self.sync_measurement(key_down(VK_SHIFT));
                }
                self.redraw();
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                self.cursor = self.pointer(lparam);
                self.start = Some(self.cursor);
                self.measurement = None;
                self.dragging = true;
                // SAFETY: the window handle is live and owned by this thread.
                unsafe {
                    SetCapture(hwnd);
                }
                self.redraw();
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                self.cursor = self.pointer(lparam);
                self.dragging = false;
                self.sync_measurement(key_down(VK_SHIFT));
                // SAFETY: releases a capture held by this thread.
                unsafe {
                    let _ = ReleaseCapture();
                }
                self.redraw();
                return LRESULT(0);
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                self.on_key(VIRTUAL_KEY(wparam.0 as u16));
                return LRESULT(0);
            }
            WM_CLOSE => {
                self.cancel();
                return LRESULT(0);
            }
            WM_DESTROY | WM_NCCREATE => {}
            _ => {}
        }
        // SAFETY: unhandled messages use the default window procedure.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn destroy(&mut self) {
        if let Some(hwnd) = self.hwnd.take() {
            // SAFETY: the window handle is live and owned by this thread.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
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

fn key_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: reads the calling thread's key state.
    unsafe { GetKeyState(key.0 as i32) < 0 }
}

fn text_width(gpu: &mut Gpu, size: f32, weight: DWRITE_FONT_WEIGHT, text: &str) -> f32 {
    let Some(format) = gpu.text.format(&gpu.write, size, weight, true) else {
        return 0.0;
    };
    d2d::measure(&gpu.write, &format, text)
}

fn draw_measurement(
    context: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext5,
    palette: &Palette,
    scale: f32,
    origin: Point,
    measurement: RulerMeasurement,
) {
    let area = measurement.rect();
    let left = area.x.get() - origin.x.get();
    let top = area.y.get() - origin.y.get();
    let width = area.width.get().max(0) as f32;
    let height = area.height.get().max(0) as f32;
    d2d::stroke_rect(
        context,
        d2d::rect(left as f32 + 0.5, top as f32 + 0.5, width, height),
        1.0,
        palette.accent,
    );
    for point in [measurement.start, measurement.end] {
        let local = local_point(point, origin);
        let size = (7.0 * scale).round().max(5.0);
        let x = local.x.get() as f32 - size / 2.0;
        let y = local.y.get() as f32 - size / 2.0;
        d2d::fill_rect(context, d2d::rect(x, y, size, size), palette.white);
        d2d::stroke_rect(
            context,
            d2d::rect(x + 0.5, y + 0.5, size - 1.0, size - 1.0),
            1.0,
            palette.accent,
        );
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
    use snapline_core::geometry::DpiScale;
    use snapline_core::ports::Monitor;
    use snapline_core::ruler::RulerAxis;

    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn monitor(id: u32, bounds: Rect, primary: bool) -> Monitor {
        Monitor {
            id,
            name: format!("Monitor {id}"),
            bounds,
            work_area: bounds,
            dpi: DpiScale::ONE,
            primary,
        }
    }

    #[test]
    fn the_monitor_under_the_cursor_is_chosen() {
        let bounds = vec![rect(0, 0, 1920, 1080), rect(-1920, 0, 1920, 1080)];
        assert_eq!(monitor_at(&bounds, Point::new(Px(10), Px(10))), Some(0));
        assert_eq!(monitor_at(&bounds, Point::new(Px(-100), Px(500))), Some(1));
        assert_eq!(monitor_at(&bounds, Point::new(Px(5000), Px(5000))), Some(0));
        assert_eq!(monitor_at(&[], Point::new(Px(0), Px(0))), None);
    }

    #[test]
    fn the_monitor_selection_uses_physical_bounds() {
        let monitors = vec![
            monitor(1, rect(0, 0, 2560, 1440), true),
            monitor(2, rect(2560, 0, 1920, 1080), false),
        ];
        let bounds: Vec<Rect> = monitors.iter().map(|monitor| monitor.bounds).collect();
        assert_eq!(monitor_at(&bounds, Point::new(Px(3000), Px(200))), Some(1));
    }

    #[test]
    fn window_pointer_offsets_are_local() {
        let origin = Point::new(Px(1920), Px(-120));
        assert_eq!(
            local_point(Point::new(Px(2000), Px(80)), origin),
            Point::new(Px(80), Px(200))
        );
    }

    #[test]
    fn the_readout_stays_inside_the_window() {
        let window = Size::new(Px(1920), Px(1080));
        let near = readout_placement(Point::new(Px(10), Px(10)), window, 300, 24, 20);
        assert_eq!(near, rect(30, 30, 300, 24));

        let far = readout_placement(Point::new(Px(1900), Px(1070)), window, 300, 24, 20);
        assert_eq!(far, rect(1580, 1026, 300, 24));
        assert!(far.right().get() <= 1920);
        assert!(far.bottom().get() <= 1080);

        let above_only = readout_placement(Point::new(Px(1900), Px(100)), window, 300, 24, 20);
        assert!(above_only.right().get() <= 1920);

        let oversized = readout_placement(Point::new(Px(100), Px(100)), window, 5000, 24, 20);
        assert_eq!(oversized, rect(0, 100, 5000, 24));
    }

    #[test]
    fn shift_locks_the_drag_to_one_axis() {
        let start = Point::new(Px(100), Px(100));
        let end = Point::new(Px(400), Px(180));
        let free = measure_between(start, end, false);
        assert_eq!(free.width, Px(300));
        assert_eq!(free.height, Px(80));
        let locked = measure_between(start, end, true);
        assert_eq!(locked.end, Point::new(Px(400), Px(100)));
        assert_eq!(locked.height, Px::ZERO);
        assert_eq!(locked.axis, Some(RulerAxis::Horizontal));
    }

    #[test]
    fn the_readout_lists_size_diagonal_and_angle() {
        let measurement =
            RulerMeasurement::measure(Point::new(Px(0), Px(0)), Point::new(Px(300), Px(400)));
        let (size, diagonal, angle) = readout_strings(measurement);
        assert_eq!(size, "300 × 400");
        assert_eq!(diagonal, "500.0 px");
        assert_eq!(angle, "53.1°");
    }
}
