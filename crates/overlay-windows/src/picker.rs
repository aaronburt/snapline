use std::sync::mpsc;

use snapline_core::colour::{ColourFormat, Rgb};
use snapline_core::geometry::{Point, Px, Rect, Size};
use snapline_core::image::Image;
use snapline_core::ports::{FrozenDesktop, Monitor};
use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_SEMI_BOLD;
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain1, DXGI_PRESENT};
use windows::Win32::Graphics::Gdi::{UpdateWindow, ValidateRect};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VIRTUAL_KEY, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId, LoadCursorW, PostQuitMessage,
    RegisterClassExW, SetCursor, SetForegroundWindow, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, UnregisterClassW, GWLP_USERDATA, HTCLIENT, IDC_CROSS, MA_ACTIVATE, MSG,
    SW_SHOW, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_SETCURSOR, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::d2d::{self, Gpu};
use crate::magnifier;
use crate::readout;
use crate::tokens::Palette;
use crate::{MagnifierSettings, OverlayError, Theme};

const CLASS_NAME: &str = "SnaplineColourPickerWindow";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PickerError {
    #[error("There is no monitor to draw the picker on.")]
    NoMonitors,
    #[error("The picker thread could not be started.")]
    Thread,
    #[error("The picker was cancelled.")]
    Cancelled,
    #[error("The picker could not use the graphics device: {0}")]
    Device(String),
    #[error("The colour could not be copied: {0}")]
    Copy(String),
}

#[derive(Debug, Clone)]
pub struct PickerRequest {
    pub desktop: FrozenDesktop,
    pub magnifier: MagnifierSettings,
    pub format: ColourFormat,
    pub theme: Theme,
}

impl PickerRequest {
    pub fn new(desktop: FrozenDesktop) -> Self {
        Self {
            desktop,
            magnifier: MagnifierSettings::default(),
            format: ColourFormat::default(),
            theme: Theme::Dark,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerResult {
    pub colour: Rgb,
    pub format: ColourFormat,
    pub text: String,
    pub position: Point,
    pub monitor: Option<u32>,
}

pub struct PickerHostImpl;

impl Default for PickerHostImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl PickerHostImpl {
    pub const fn new() -> Self {
        Self
    }

    pub fn pick<F>(&self, request: PickerRequest, copy: F) -> Result<PickerResult, PickerError>
    where
        F: Fn(&str) -> Result<(), String> + Send + 'static,
    {
        if request.desktop.monitors.is_empty() {
            return Err(PickerError::NoMonitors);
        }
        run(request, copy)
    }
}

pub fn pick<F>(request: PickerRequest, copy: F) -> Result<PickerResult, PickerError>
where
    F: Fn(&str) -> Result<(), String> + Send + 'static,
{
    PickerHostImpl::new().pick(request, copy)
}

struct PickerSource {
    monitor: Monitor,
    scale: f32,
    origin: Point,
    size: Size,
    image: Image,
}

struct PickerWindow {
    hwnd: HWND,
    source: PickerSource,
    swap_chain: IDXGISwapChain1,
    target: ID2D1Bitmap1,
    frozen: ID2D1Bitmap1,
}

struct Picked {
    colour: Rgb,
    text: String,
    position: Point,
    monitor: Option<u32>,
}

enum PickerOutcome {
    Finish(Picked),
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SwitchLayout {
    pub area: Rect,
    pub segments: [Rect; 3],
}

pub(crate) fn switch_layout(
    readout: Rect,
    readout_above: bool,
    window: Size,
    scale: f32,
) -> SwitchLayout {
    let gap = ((4.0 * scale).round() as i32).max(1);
    let height = ((24.0 * scale).round() as i32).max(1);
    let segment = ((52.0 * scale).round() as i32).max(1);
    let width = segment * 3;
    let x = readout
        .x
        .get()
        .clamp(0, (window.width.get() - width).max(0));
    let below = readout.bottom().get() + gap;
    let above = readout.y.get() - gap - height;
    let mut y = if readout_above { above } else { below };
    if y < 0 {
        y = below;
    }
    if y + height > window.height.get() {
        y = above.max(0);
    }
    let area = Rect::new(Px(x), Px(y), Px(width), Px(height));
    let segments = [
        Rect::new(Px(x), Px(y), Px(segment), Px(height)),
        Rect::new(Px(x + segment), Px(y), Px(segment), Px(height)),
        Rect::new(Px(x + segment * 2), Px(y), Px(segment), Px(height)),
    ];
    SwitchLayout { area, segments }
}

pub(crate) fn switch_hit(layout: &SwitchLayout, point: Point) -> Option<ColourFormat> {
    ColourFormat::ALL
        .iter()
        .copied()
        .zip(layout.segments)
        .find(|(_, segment)| segment.contains(point))
        .map(|(format, _)| format)
}

pub(crate) fn colour_at(image: &Image, point: Point) -> Rgb {
    let x = point
        .x
        .get()
        .clamp(0, image.width().saturating_sub(1) as i32);
    let y = point
        .y
        .get()
        .clamp(0, image.height().saturating_sub(1) as i32);
    let pixel = image.pixel(x as u32, y as u32).unwrap_or([0, 0, 0, 255]);
    Rgb::from_bgra(pixel)
}

pub(crate) fn readout_text(colour: Rgb, format: ColourFormat, position: Point) -> (String, String) {
    (
        format.display(colour),
        format!("X {} · Y {}", position.x.get(), position.y.get()),
    )
}

struct PickerContext {
    windows: Vec<PickerWindow>,
    sources: Vec<PickerSource>,
    gpu: Gpu,
    cursor: Option<Point>,
    pointer_monitor: usize,
    previous_monitor: usize,
    format: ColourFormat,
    magnifier: MagnifierSettings,
    theme: Theme,
    outcome: Option<PickerOutcome>,
    failure: Option<PickerError>,
    class_name: Vec<u16>,
    instance: HINSTANCE,
    copy: Box<dyn Fn(&str) -> Result<(), String>>,
}

impl PickerContext {
    fn build(
        request: PickerRequest,
        copy: Box<dyn Fn(&str) -> Result<(), String>>,
    ) -> Result<Self, PickerError> {
        let mut sources = Vec::new();
        for frozen in request.desktop.monitors {
            let monitor = frozen.monitor;
            sources.push(PickerSource {
                scale: monitor.dpi.scale(),
                origin: monitor.bounds.origin(),
                size: monitor.bounds.size(),
                monitor,
                image: frozen.image,
            });
        }
        if sources.is_empty() {
            return Err(PickerError::NoMonitors);
        }
        let gpu = Gpu::new().map_err(picker_error)?;
        // SAFETY: returns the module handle of this process without borrowing any memory.
        let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map_err(|error| PickerError::Device(error.to_string()))?;
        let class_name: Vec<u16> = CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        Ok(Self {
            windows: Vec::new(),
            sources,
            gpu,
            cursor: None,
            pointer_monitor: 0,
            previous_monitor: 0,
            format: request.format,
            magnifier: request.magnifier,
            theme: request.theme,
            outcome: None,
            failure: None,
            class_name,
            instance: HINSTANCE(instance.0),
            copy,
        })
    }

    fn activate(&mut self) -> Result<(), PickerError> {
        self.register_class()?;
        self.open()?;
        self.redraw();
        self.show();
        Ok(())
    }

    fn register_class(&mut self) -> Result<(), PickerError> {
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
            return Err(PickerError::Device(
                "The picker window class could not be registered.".to_owned(),
            ));
        }
        Ok(())
    }

    fn open(&mut self) -> Result<(), PickerError> {
        let sources = std::mem::take(&mut self.sources);
        for source in sources {
            let hwnd = self.create_window(&source)?;
            let width = u32::try_from(source.size.width.get()).unwrap_or(1);
            let height = u32::try_from(source.size.height.get()).unwrap_or(1);
            let swap = match self.gpu.create_target(hwnd, width, height) {
                Ok(swap) => swap,
                Err(error) => {
                    // SAFETY: the window was created above and is owned by this thread.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    return Err(picker_error(error));
                }
            };
            let frozen = match self.gpu.upload_image(&source.image) {
                Ok(frozen) => frozen,
                Err(error) => {
                    // SAFETY: the window was created above and is owned by this thread.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    return Err(picker_error(error));
                }
            };
            self.windows.push(PickerWindow {
                hwnd,
                source,
                swap_chain: swap.swap_chain,
                target: swap.target,
                frozen,
            });
        }
        Ok(())
    }

    fn create_window(&mut self, source: &PickerSource) -> Result<HWND, PickerError> {
        let class = PCWSTR::from_raw(self.class_name.as_ptr());
        let style = WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST;
        let bounds = source.monitor.bounds;
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
        .map_err(|error| PickerError::Device(error.to_string()))?;
        let context = self as *mut Self as isize;
        // SAFETY: the context lives in a boxed value for the whole message loop.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, context);
        }
        Ok(hwnd)
    }

    fn show(&mut self) {
        for window in &self.windows {
            // SAFETY: the window handle is a live top level window owned by this thread.
            unsafe {
                let _ = ShowWindow(window.hwnd, SW_SHOW);
                let _ = UpdateWindow(window.hwnd);
            }
        }
        if let Some(window) = self.windows.first() {
            self.focus(window.hwnd);
        }
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

    fn redraw(&mut self) {
        for index in 0..self.windows.len() {
            self.redraw_window(index);
        }
    }

    fn redraw_window(&mut self, index: usize) {
        let origin = self.windows[index].source.origin;
        let local = self.cursor.map(|cursor| {
            Point::new(
                Px(cursor.x.get() - origin.x.get()),
                Px(cursor.y.get() - origin.y.get()),
            )
        });
        let inside = self
            .cursor
            .is_some_and(|cursor| self.windows[index].source.monitor.bounds.contains(cursor));
        draw_window(
            &mut self.gpu,
            &self.windows[index],
            self.theme,
            if inside { local } else { None },
            self.format,
            self.magnifier.zoom.factor(),
        );
    }

    fn index_of(&self, hwnd: HWND) -> Option<usize> {
        self.windows.iter().position(|window| window.hwnd == hwnd)
    }

    fn pointer(&self, index: usize, lparam: LPARAM) -> Point {
        let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
        let origin = self.windows[index].source.origin;
        Point::new(Px(x + origin.x.get()), Px(y + origin.y.get()))
    }

    fn monitor_index(&self, point: Point) -> usize {
        self.windows
            .iter()
            .position(|window| window.source.monitor.bounds.contains(point))
            .unwrap_or(self.pointer_monitor)
    }

    fn layout(&self, index: usize, cursor: Point) -> (magnifier::Geometry, SwitchLayout) {
        let window = &self.windows[index];
        let origin = window.source.origin;
        let local = Point::new(
            Px(cursor.x.get() - origin.x.get()),
            Px(cursor.y.get() - origin.y.get()),
        );
        let geometry = magnifier::geometry(
            local,
            window.source.size,
            Size::new(
                Px(window.source.image.width() as i32),
                Px(window.source.image.height() as i32),
            ),
            window.source.scale,
            self.magnifier.zoom.factor(),
        );
        let switch = switch_layout(
            geometry.readout,
            geometry.readout_above,
            window.source.size,
            window.source.scale,
        );
        (geometry, switch)
    }

    fn choose(&mut self, index: usize, point: Point) -> bool {
        let (_, switch) = self.layout(index, point);
        let origin = self.windows[index].source.origin;
        let local = Point::new(
            Px(point.x.get() - origin.x.get()),
            Px(point.y.get() - origin.y.get()),
        );
        let Some(format) = switch_hit(&switch, local) else {
            return false;
        };
        self.format = format;
        true
    }

    fn copy_pixel(&mut self, index: usize, point: Point) {
        let window = &self.windows[index];
        let origin = window.source.origin;
        let local = Point::new(
            Px(point.x.get() - origin.x.get()),
            Px(point.y.get() - origin.y.get()),
        );
        let colour = colour_at(&window.source.image, local);
        let text = self.format.display(colour);
        if let Err(message) = (self.copy)(&text) {
            self.failure = Some(PickerError::Copy(message));
            // SAFETY: posts a quit message to this thread's queue.
            unsafe { PostQuitMessage(0) };
            return;
        }
        let monitor = self
            .windows
            .get(index)
            .map(|window| window.source.monitor.id);
        self.outcome = Some(PickerOutcome::Finish(Picked {
            colour,
            text,
            position: point,
            monitor,
        }));
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
    }

    fn cancel(&mut self) {
        self.outcome = Some(PickerOutcome::Cancel);
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
    }

    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let Some(index) = self.index_of(hwnd) else {
            // SAFETY: the message is passed straight through for windows not owned by the context.
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        };
        match message {
            WM_MOUSEACTIVATE => {
                let _ = index;
                return LRESULT(MA_ACTIVATE as isize);
            }
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
                let point = self.pointer(index, lparam);
                self.cursor = Some(point);
                self.pointer_monitor = self.monitor_index(point);
                self.redraw_window(self.previous_monitor);
                self.redraw_window(self.pointer_monitor);
                self.previous_monitor = self.pointer_monitor;
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                let point = self.pointer(index, lparam);
                self.cursor = Some(point);
                self.pointer_monitor = index;
                if self.choose(index, point) {
                    self.redraw();
                } else {
                    self.copy_pixel(index, point);
                }
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                let key = VIRTUAL_KEY(wparam.0 as u16);
                if key == VK_ESCAPE {
                    self.cancel();
                }
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
        for window in self.windows.drain(..) {
            // SAFETY: the window handle is live and owned by this thread.
            unsafe {
                let _ = DestroyWindow(window.hwnd);
            }
        }
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

impl Drop for PickerContext {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn draw_window(
    gpu: &mut Gpu,
    window: &PickerWindow,
    theme: Theme,
    cursor: Option<Point>,
    format: ColourFormat,
    zoom: i32,
) {
    let palette = theme.palette();
    let width = window.source.size.width.get() as f32;
    let height = window.source.size.height.get() as f32;
    let context = gpu.context.clone();
    let context = &context;
    // SAFETY: the swap chain target and bitmap belong to this render pass.
    unsafe {
        context.SetTarget(&window.target);
    }
    gpu.begin();
    let black = [0.0, 0.0, 0.0, 1.0];
    // SAFETY: clears the whole render target before any drawing.
    unsafe {
        let clear = d2d::colour(black);
        let _ = context.Clear(Some(&clear));
    }
    d2d::draw_bitmap(
        context,
        &window.frozen,
        d2d::rect(0.0, 0.0, width, height),
        d2d::rect(
            0.0,
            0.0,
            window.source.image.width() as f32,
            window.source.image.height() as f32,
        ),
        false,
    );
    if let Some(cursor) = cursor {
        draw_magnifier(gpu, window, palette, cursor, format, zoom);
    }
    gpu.end();
    // SAFETY: the render pass is finished and the buffer can be presented.
    unsafe {
        let _ = context.SetTarget(None::<&windows::Win32::Graphics::Direct2D::ID2D1Image>);
    }
    // SAFETY: the swap chain belongs to the live window.
    unsafe {
        let _ = window.swap_chain.Present(1, DXGI_PRESENT(0));
    }
}

fn draw_magnifier(
    gpu: &mut Gpu,
    window: &PickerWindow,
    palette: &Palette,
    cursor: Point,
    format: ColourFormat,
    zoom: i32,
) {
    let scale = window.source.scale;
    let geometry = magnifier::geometry(
        cursor,
        window.source.size,
        Size::new(
            Px(window.source.image.width() as i32),
            Px(window.source.image.height() as i32),
        ),
        scale,
        zoom,
    );
    let context = gpu.context.clone();
    let context = &context;
    let destination = geometry.destination;
    let dx = destination.x.get() as f32;
    let dy = destination.y.get() as f32;
    let size = destination.width.get() as f32;
    d2d::draw_bitmap(
        context,
        &window.frozen,
        d2d::rect(dx, dy, size, size),
        d2d::rect(
            geometry.source.x.get() as f32,
            geometry.source.y.get() as f32,
            geometry.source.width.get() as f32,
            geometry.source.height.get() as f32,
        ),
        true,
    );
    d2d::stroke_rect(
        context,
        d2d::rect(dx + 0.5, dy + 0.5, size - 1.0, size - 1.0),
        1.0,
        palette.stroke_hi,
    );
    if geometry.zoom >= 8 {
        let step = geometry.zoom as f32;
        let mut offset = step;
        while offset < size {
            d2d::fill_rect(
                context,
                d2d::rect(dx + offset, dy, 1.0, size),
                palette.stroke_hi,
            );
            d2d::fill_rect(
                context,
                d2d::rect(dx, dy + offset, size, 1.0),
                palette.stroke_hi,
            );
            offset += step;
        }
    }
    let colour = colour_at(&window.source.image, cursor);
    let position = Point::new(
        Px(cursor.x.get() + window.source.origin.x.get()),
        Px(cursor.y.get() + window.source.origin.y.get()),
    );
    let (text, coordinates) = readout_text(colour, format, position);
    readout::draw_magnifier_readout(
        gpu,
        palette,
        scale,
        geometry.readout,
        &text,
        &coordinates,
        window.source.size.width.get() as f32,
    );
    let switch = switch_layout(
        geometry.readout,
        geometry.readout_above,
        window.source.size,
        scale,
    );
    draw_switch(gpu, palette, scale, &switch, format);
}

fn draw_switch(
    gpu: &mut Gpu,
    palette: &Palette,
    scale: f32,
    layout: &SwitchLayout,
    selected: ColourFormat,
) {
    let context = gpu.context.clone();
    let context = &context;
    let x = layout.area.x.get() as f32;
    let y = layout.area.y.get() as f32;
    let width = layout.area.width.get() as f32;
    let height = layout.area.height.get() as f32;
    d2d::fill_round(
        context,
        d2d::rect(x, y, width, height),
        4.0 * scale,
        palette.layer,
    );
    d2d::stroke_round(
        context,
        d2d::rect(x + 0.5, y + 0.5, width - 1.0, height - 1.0),
        4.0 * scale,
        1.0,
        palette.stroke,
    );
    let size = 11.0 * scale;
    for (format, segment) in ColourFormat::ALL.iter().copied().zip(layout.segments) {
        let sx = segment.x.get() as f32;
        let sy = segment.y.get() as f32;
        let sw = segment.width.get() as f32;
        let sh = segment.height.get() as f32;
        if format == selected {
            d2d::fill_round(
                context,
                d2d::rect(sx + 2.0, sy + 2.0, sw - 4.0, sh - 4.0),
                3.0 * scale,
                palette.accent_soft,
            );
        }
        let label = format.label();
        let Some(text_format) =
            gpu.text
                .format(&gpu.write, size, DWRITE_FONT_WEIGHT_SEMI_BOLD, false)
        else {
            continue;
        };
        let measured = d2d::measure(&gpu.write, &text_format, label);
        d2d::draw_text(
            context,
            &text_format,
            label,
            d2d::rect(sx + (sw - measured) / 2.0, sy, measured, sh),
            if format == selected {
                palette.accent
            } else {
                palette.text2
            },
        );
    }
}

fn picker_error(error: OverlayError) -> PickerError {
    match error {
        OverlayError::NoMonitors => PickerError::NoMonitors,
        OverlayError::Thread => PickerError::Thread,
        OverlayError::Cancelled => PickerError::Cancelled,
        OverlayError::Device(message) => PickerError::Device(message),
    }
}

fn run<F>(request: PickerRequest, copy: F) -> Result<PickerResult, PickerError>
where
    F: Fn(&str) -> Result<(), String> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name(String::from("snapline-picker"))
        .spawn(move || {
            let result = execute(request, Box::new(copy));
            let _ = sender.send(result);
        })
        .map_err(|_| PickerError::Thread)?;
    receiver.recv().map_err(|_| PickerError::Thread)?
}

fn execute(
    request: PickerRequest,
    copy: Box<dyn Fn(&str) -> Result<(), String>>,
) -> Result<PickerResult, PickerError> {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated picker thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut context = Box::new(PickerContext::build(request, copy)?);
    context.activate()?;
    PickerContext::run_message_loop();
    let outcome = context.outcome.take();
    let failure = context.failure.take();
    let format = context.format;
    context.destroy();
    drop(context);
    if let Some(error) = failure {
        return Err(error);
    }
    match outcome {
        Some(PickerOutcome::Finish(picked)) => Ok(PickerResult {
            colour: picked.colour,
            format,
            text: picked.text,
            position: picked.position,
            monitor: picked.monitor,
        }),
        Some(PickerOutcome::Cancel) | None => Err(PickerError::Cancelled),
    }
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Self {
        // SAFETY: initialises COM for the picker thread with a single threaded apartment.
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

// SAFETY: the user data slot holds a pointer to the boxed context for the message loop's lifetime.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: windows created before the context is attached use the default procedure.
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut PickerContext;
    if pointer.is_null() {
        // SAFETY: windows created before the context is attached use the default procedure.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    // SAFETY: the pointer was installed by PickerContext::create_window and remains valid.
    let context = unsafe { &mut *pointer };
    context.handle(hwnd, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: i32, height: i32) -> Size {
        Size::new(Px(width), Px(height))
    }

    #[test]
    fn switch_sits_below_the_readout_and_stays_on_screen() {
        let readout = Rect::new(Px(1500), Px(900), Px(120), Px(24));
        let layout = switch_layout(readout, false, size(1920, 1080), 1.0);
        assert_eq!(layout.area.width.get(), 156);
        assert_eq!(layout.area.height.get(), 24);
        assert!(layout.area.right().get() <= 1920);
        assert!(layout.area.y.get() >= readout.bottom().get());
        assert!(layout.area.bottom().get() <= 1080);
    }

    #[test]
    fn switch_flips_above_the_readout_when_there_is_no_room_below() {
        let readout = Rect::new(Px(100), Px(1040), Px(120), Px(24));
        let layout = switch_layout(readout, false, size(1920, 1080), 1.0);
        assert!(layout.area.bottom().get() <= 1080);
        assert!(layout.area.bottom().get() <= readout.y.get());
    }

    #[test]
    fn switch_hit_test_maps_every_segment() {
        let readout = Rect::new(Px(100), Px(100), Px(120), Px(24));
        let layout = switch_layout(readout, false, size(1920, 1080), 1.0);
        assert_eq!(
            switch_hit(&layout, layout.segments[0].center()),
            Some(ColourFormat::Hex)
        );
        assert_eq!(
            switch_hit(&layout, layout.segments[1].center()),
            Some(ColourFormat::Rgb)
        );
        assert_eq!(
            switch_hit(&layout, layout.segments[2].center()),
            Some(ColourFormat::Hsl)
        );
        assert_eq!(switch_hit(&layout, Point::new(Px(0), Px(0))), None);
    }

    #[test]
    fn picker_readout_uses_the_selected_format() {
        let colour = Rgb::new(18, 52, 86);
        let position = Point::new(Px(280), Px(160));
        assert_eq!(
            readout_text(colour, ColourFormat::Hex, position),
            ("#123456".to_owned(), "X 280 · Y 160".to_owned())
        );
        assert_eq!(
            readout_text(colour, ColourFormat::Rgb, position).0,
            "rgb(18, 52, 86)"
        );
        assert_eq!(
            readout_text(colour, ColourFormat::Hsl, position).0,
            "hsl(210, 65%, 20%)"
        );
    }

    #[test]
    fn picking_reads_bgra_and_ignores_alpha() {
        let mut image = Image::bgra(2, 1).expect("image");
        image.set_pixel(0, 0, [0x4D, 0x48, 0xE5, 0]).expect("pixel");
        image
            .set_pixel(1, 0, [0x0A, 0xD6, 0xFF, 255])
            .expect("pixel");
        assert_eq!(
            colour_at(&image, Point::new(Px(0), Px(0))),
            Rgb::new(0xE5, 0x48, 0x4D)
        );
        assert_eq!(
            colour_at(&image, Point::new(Px(1), Px(0))),
            Rgb::new(0xFF, 0xD6, 0x0A)
        );
    }

    #[test]
    fn picking_clamps_a_point_to_the_image() {
        let mut image = Image::bgra(1, 1).expect("image");
        image.set_pixel(0, 0, [3, 2, 1, 255]).expect("pixel");
        assert_eq!(
            colour_at(&image, Point::new(Px(-5), Px(99))),
            Rgb::new(1, 2, 3)
        );
    }

    #[test]
    fn switch_layout_scales_with_dpi() {
        let readout = Rect::new(Px(200), Px(200), Px(240), Px(48));
        let layout = switch_layout(readout, false, size(3840, 2160), 2.0);
        assert_eq!(layout.area.width.get(), 312);
        assert_eq!(layout.area.height.get(), 48);
        assert_eq!(layout.segments[0].width.get(), 104);
    }
}
