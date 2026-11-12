use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use snapline_core::image::Image;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1Bitmap1, ID2D1Image, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
};
use windows::Win32::Graphics::DirectComposition::{IDCompositionTarget, IDCompositionVisual};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutput, IDXGISurface, IDXGISwapChain1, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, ValidateRect, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_ESCAPE,
    VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW,
    GetWindowLongPtrW, GetWindowThreadProcessId, KillTimer, LoadCursorW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos,
    ShowWindow, TranslateMessage, UnregisterClassW, GWLP_USERDATA, HTCLIENT, IDC_ARROW, IDC_HAND,
    MA_ACTIVATE, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WA_INACTIVE, WM_ACTIVATE, WM_APP,
    WM_CLOSE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::d2d::{self, Gpu};
use crate::icons::{self, Icon, IconCache};
use crate::quick_panel_logic::{
    copy_label, link_without_scheme, Content, Interaction, Item, Key, Layout, Outcome, RectF, Tile,
    BORDER, COPIED_MS, MAX_RECENT, RADIUS, WIDTH,
};
use crate::{OverlayError, Theme};

const CLASS_NAME: &str = "SnaplineQuickPanel";
static NEXT_CLASS: AtomicU32 = AtomicU32::new(1);
const SHADOW: f32 = 18.0;
const SHADOW_STEPS: i32 = 9;
const TIMER_COPIED: usize = 2;
const WM_DATA: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3;

#[derive(Debug, Clone)]
pub struct RecentView {
    pub id: i64,
    pub name: String,
    pub time: String,
    pub duration: Option<String>,
    pub uploaded: bool,
    pub thumb: Option<Image>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UploadView {
    pub name: String,
    pub detail: String,
    pub fraction: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct PanelData {
    pub recent: Vec<RecentView>,
    pub loaded: bool,
    pub uploads: Vec<UploadView>,
    pub link: Option<String>,
}

impl PanelData {
    fn content(&self) -> Content {
        Content {
            recent: self.recent.len(),
            loaded: self.loaded,
            uploads: self.uploads.len(),
            has_link: self.link.as_deref().is_some_and(|link| !link.is_empty()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    OpenMain,
    Settings,
    ViewAll,
    Capture(&'static str),
    Recent(i64),
    CopyLink,
}

pub type ActivateFn = Box<dyn Fn(Activation) + Send>;
pub type PlaceFn = Box<dyn Fn(i32, i32) -> (i32, i32) + Send>;

pub struct PanelRequest {
    pub theme: Theme,
    pub data: PanelData,
    pub anchor: (i32, i32),
    pub place: PlaceFn,
    pub on_activate: ActivateFn,
    pub on_closed: Box<dyn FnOnce() + Send>,
}

struct Shared {
    open: AtomicBool,
    pending: Mutex<Option<PanelData>>,
}

#[derive(Clone)]
pub struct PanelHandle {
    window: isize,
    shared: Arc<Shared>,
}

impl PanelHandle {
    pub fn is_open(&self) -> bool {
        self.shared.open.load(Ordering::SeqCst)
    }

    pub fn update(&self, data: PanelData) {
        let Ok(mut pending) = self.shared.pending.lock() else {
            return;
        };
        if self.is_open() {
            *pending = Some(data);
            self.post(WM_DATA);
        }
    }

    pub fn close(&self) {
        if self.is_open() {
            self.post(WM_CLOSE);
        }
    }

    fn post(&self, message: u32) {
        let window = HWND(self.window as *mut std::ffi::c_void);
        // SAFETY: posting to a window owned by another thread is allowed and a stale handle only fails.
        unsafe {
            let _ = PostMessageW(Some(window), message, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn open(request: PanelRequest) -> Result<PanelHandle, OverlayError> {
    let shared = Arc::new(Shared {
        open: AtomicBool::new(true),
        pending: Mutex::new(None),
    });
    let (ready_sender, ready_receiver) = mpsc::channel();
    let thread_shared = shared.clone();
    let thread = std::thread::Builder::new()
        .name(String::from("snapline-quick-panel"))
        .spawn(move || run(request, thread_shared, ready_sender))
        .map_err(|_| OverlayError::Thread)?;
    match ready_receiver.recv() {
        Ok(Ok(window)) => Ok(PanelHandle { window, shared }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => Err(OverlayError::Thread),
    }
}

fn run(
    request: PanelRequest,
    shared: Arc<Shared>,
    ready: mpsc::Sender<Result<isize, OverlayError>>,
) {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated panel thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let PanelRequest {
        theme,
        data,
        anchor,
        place,
        on_activate,
        on_closed,
    } = request;
    let result = Context::build(theme, data, anchor, place, on_activate, shared.clone()).and_then(
        |context| {
            let mut context = Box::new(context);
            let result = context.activate().map(|()| {
                let _ = ready.send(Ok(context.hwnd.0 as isize));
                Context::run_message_loop();
            });
            context.destroy();
            result
        },
    );
    on_closed();
    shared.open.store(false, Ordering::SeqCst);
    if let Ok(mut pending) = shared.pending.lock() {
        pending.take();
    }
    if let Err(error) = result {
        let _ = ready.send(Err(error));
    }
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Self {
        // SAFETY: initialises COM for the panel thread with a single threaded apartment.
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

struct Surface {
    swap_chain: IDXGISwapChain1,
    target: ID2D1Bitmap1,
    _visual: IDCompositionVisual,
    _dcomp_target: IDCompositionTarget,
}

fn device_error(error: windows::core::Error) -> OverlayError {
    OverlayError::Device(error.to_string())
}

fn create_surface(gpu: &Gpu, hwnd: HWND, width: u32, height: u32) -> Result<Surface, OverlayError> {
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
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    };
    // SAFETY: the device, description and restriction output are valid.
    let swap_chain = unsafe {
        gpu.dxgi_factory
            .CreateSwapChainForComposition(&gpu.d3d, &desc, None::<&IDXGIOutput>)
    }
    .map_err(device_error)?;
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
    let target = unsafe {
        gpu.context
            .CreateBitmapFromDxgiSurface(&surface, Some(&properties))
    }
    .map_err(device_error)?;
    // SAFETY: the window handle is a live top level window owned by this thread.
    let dcomp_target =
        unsafe { gpu.dcomp.CreateTargetForHwnd(hwnd, true) }.map_err(device_error)?;
    // SAFETY: the composition device is live.
    let visual = unsafe { gpu.dcomp.CreateVisual() }.map_err(device_error)?;
    // SAFETY: the swap chain is a composition surface and the visual accepts it as content.
    unsafe { visual.SetContent(&swap_chain) }.map_err(device_error)?;
    // SAFETY: the target and visual are live and owned by this function.
    unsafe { dcomp_target.SetRoot(&visual) }.map_err(device_error)?;
    // SAFETY: commits the visual tree created above.
    unsafe { gpu.dcomp.Commit() }.map_err(device_error)?;
    Ok(Surface {
        swap_chain,
        target,
        _visual: visual,
        _dcomp_target: dcomp_target,
    })
}

struct Fonts {
    regular12: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    medium12: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    semibold14: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    regular11: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    regular10: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    mono12: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
}

fn fonts(gpu: &mut Gpu) -> Option<Fonts> {
    let write = gpu.write.clone();
    let mut make = |size: f32, weight: DWRITE_FONT_WEIGHT, mono: bool| {
        gpu.text.format(&write, size, weight, mono)
    };
    Some(Fonts {
        regular12: make(12.0, DWRITE_FONT_WEIGHT_NORMAL, false)?,
        medium12: make(12.0, DWRITE_FONT_WEIGHT_MEDIUM, false)?,
        semibold14: make(14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, false)?,
        regular11: make(11.0, DWRITE_FONT_WEIGHT_NORMAL, false)?,
        regular10: make(10.0, DWRITE_FONT_WEIGHT_NORMAL, false)?,
        mono12: make(12.0, DWRITE_FONT_WEIGHT_NORMAL, true)?,
    })
}

pub(crate) struct View<'a> {
    pub layout: &'a Layout,
    pub interaction: &'a Interaction,
    pub data: &'a PanelData,
    pub bitmaps: &'a [Option<ID2D1Bitmap1>],
    pub palette: &'static crate::tokens::Palette,
    pub copy_text: &'static str,
}

fn alpha(rgba: [f32; 4], factor: f32) -> [f32; 4] {
    [rgba[0], rgba[1], rgba[2], rgba[3] * factor]
}

fn local(area: RectF, origin: (f32, f32)) -> D2D_RECT_F {
    d2d::rect(area.x + origin.0, area.y + origin.1, area.w, area.h)
}

fn fit_text(
    gpu: &Gpu,
    format: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    text: &str,
    max_width: f32,
) -> String {
    if d2d::measure(&gpu.write, format, text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = chars.len();
    while keep > 0 {
        keep -= 1;
        let mut candidate: String = chars[..keep].iter().collect();
        candidate.push('…');
        if d2d::measure(&gpu.write, format, &candidate) <= max_width {
            return candidate;
        }
    }
    String::from("…")
}

#[allow(clippy::too_many_arguments)]
fn draw_label(
    gpu: &Gpu,
    format: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    text: &str,
    area: RectF,
    origin: (f32, f32),
    rgba: [f32; 4],
    centred: bool,
) {
    let mut rect = local(area, origin);
    if centred {
        let width = d2d::measure(&gpu.write, format, text);
        rect.left += (area.w - width) / 2.0;
    }
    rect.right = rect.left.max(rect.right) + 200.0;
    d2d::draw_text(&gpu.context, format, text, rect, rgba);
}

fn overlay_state(gpu: &Gpu, view: &View, item: Item, area: RectF, radius: f32, origin: (f32, f32)) {
    if !view.layout.is_enabled(item) {
        return;
    }
    let palette = view.palette;
    let rect = local(area, origin);
    if view.interaction.is_down(item) {
        d2d::fill_round(&gpu.context, rect, radius, palette.subtle);
        d2d::fill_round(&gpu.context, rect, radius, palette.subtle);
    } else if view.interaction.is_hot(item) {
        d2d::fill_round(&gpu.context, rect, radius, palette.subtle);
    }
    if view.interaction.show_focus_ring(item) {
        let outer = d2d::rect(
            rect.left - 1.0,
            rect.top - 1.0,
            rect.right - rect.left + 2.0,
            rect.bottom - rect.top + 2.0,
        );
        d2d::stroke_round(&gpu.context, outer, radius + 1.0, 2.0, palette.text);
    }
}

fn fill_bitmap_cover(gpu: &Gpu, bitmap: &ID2D1Bitmap1, area: D2D_RECT_F, radius: f32) {
    let Ok(base) = bitmap.cast::<ID2D1Bitmap>() else {
        return;
    };
    // SAFETY: the bitmap is live and the default brush properties are used.
    let Ok(brush) = (unsafe { gpu.context.CreateBitmapBrush(&base, None, None) }) else {
        return;
    };
    // SAFETY: the bitmap is live.
    let size = unsafe { bitmap.GetPixelSize() };
    let width = size.width.max(1) as f32;
    let height = size.height.max(1) as f32;
    let target_width = area.right - area.left;
    let target_height = area.bottom - area.top;
    let scale = (target_width / width).max(target_height / height);
    let mut transform = Default::default();
    // SAFETY: the brush is live and the transform out pointer is valid.
    unsafe { brush.GetTransform(&mut transform) };
    transform.M11 = scale;
    transform.M12 = 0.0;
    transform.M21 = 0.0;
    transform.M22 = scale;
    transform.M31 = area.left + (target_width - width * scale) / 2.0;
    transform.M32 = area.top + (target_height - height * scale) / 2.0;
    // SAFETY: the transform is a plain matrix and the brush is live.
    unsafe { brush.SetTransform(&transform) };
    let rounded = windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT {
        rect: area,
        radiusX: radius,
        radiusY: radius,
    };
    // SAFETY: the rounded rectangle and brush are live values for the call.
    unsafe { gpu.context.FillRoundedRectangle(&rounded, &brush) };
}

pub(crate) fn render(gpu: &mut Gpu, icons: &mut IconCache, view: &View, scale: f32, pad: f32) {
    let Some(fonts) = fonts(gpu) else {
        return;
    };
    let palette = view.palette;
    let context = gpu.context.clone();
    let mut previous = Default::default();
    // SAFETY: the transform out pointer is valid.
    unsafe { context.GetTransform(&mut previous) };
    let mut transform = previous;
    transform.M11 = scale;
    transform.M12 = 0.0;
    transform.M21 = 0.0;
    transform.M22 = scale;
    transform.M31 = 0.0;
    transform.M32 = 0.0;
    gpu.begin();
    // SAFETY: configures the live context for this pass and clears the target to transparent.
    unsafe {
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        context.SetTransform(&transform);
        let clear = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        context.Clear(Some(&clear));
    }
    let origin = (pad, pad);
    let layout = view.layout;
    let body = d2d::rect(origin.0, origin.1, WIDTH, layout.height);
    for step in 0..SHADOW_STEPS {
        let grow = SHADOW * (1.0 - step as f32 / SHADOW_STEPS as f32);
        d2d::fill_round(
            &context,
            d2d::rect(
                body.left - grow,
                body.top - grow + 6.0,
                WIDTH + grow * 2.0,
                layout.height + grow * 2.0,
            ),
            RADIUS + grow,
            alpha(palette.shadow, 0.075),
        );
    }
    d2d::fill_round(&context, body, RADIUS, palette.layer);
    let footer = layout.footer;
    let footer_top = footer.y - RADIUS;
    d2d::fill_round(
        &context,
        d2d::rect(
            origin.0,
            footer_top + origin.1,
            WIDTH,
            layout.height - footer_top,
        ),
        RADIUS,
        palette.mica,
    );
    d2d::fill_rect(
        &context,
        d2d::rect(
            origin.0 + BORDER,
            footer_top + origin.1,
            WIDTH - 2.0 * BORDER,
            RADIUS,
        ),
        palette.layer,
    );

    draw_header(gpu, icons, view, &fonts, origin);
    draw_tiles(gpu, icons, view, &fonts, origin);
    draw_recent(gpu, icons, view, &fonts, origin);
    draw_uploads(gpu, view, &fonts, origin);
    draw_footer(gpu, icons, view, &fonts, origin);

    d2d::stroke_round(
        &context,
        d2d::rect(
            origin.0 + 0.5,
            origin.1 + 0.5,
            WIDTH - 1.0,
            layout.height - 1.0,
        ),
        RADIUS - 0.5,
        BORDER,
        palette.stroke_hi,
    );
    unsafe {
        context.SetTransform(&previous);
    }
    gpu.end();
}

fn rule(gpu: &Gpu, view: &View, y: f32, origin: (f32, f32)) {
    d2d::fill_rect(
        &gpu.context,
        d2d::rect(origin.0 + BORDER, y + origin.1, WIDTH - 2.0 * BORDER, 1.0),
        view.palette.stroke,
    );
}

fn draw_header(gpu: &Gpu, icons: &mut IconCache, view: &View, fonts: &Fonts, origin: (f32, f32)) {
    let palette = view.palette;
    let layout = view.layout;
    let logo = layout.logo;
    d2d::fill_round(&gpu.context, local(logo, origin), 8.0, palette.accent);
    icons::draw(
        icons,
        gpu,
        Icon::PanelMarks,
        palette.on_accent,
        logo.x + logo.w / 2.0 + origin.0,
        logo.y + logo.h / 2.0 + origin.1,
        logo.w,
    );
    d2d::fill_circle(
        &gpu.context,
        logo.x + logo.w / 2.0 + origin.0,
        logo.y + logo.h / 2.0 + origin.1,
        2.4,
        palette.on_accent,
    );
    draw_label(
        gpu,
        &fonts.semibold14,
        "Snapline",
        layout.title,
        origin,
        palette.text,
        false,
    );
    for (item, icon) in [
        (Item::OpenMain, Icon::PanelOpen),
        (Item::Settings, Icon::PanelGear),
    ] {
        let Some(area) = layout.rect(item) else {
            continue;
        };
        overlay_state(gpu, view, item, area, 6.0, origin);
        icons::draw(
            icons,
            gpu,
            icon,
            palette.text2,
            area.x + area.w / 2.0 + origin.0,
            area.y + area.h / 2.0 + origin.1,
            18.0,
        );
    }
}

fn draw_tiles(gpu: &Gpu, icons: &mut IconCache, view: &View, fonts: &Fonts, origin: (f32, f32)) {
    let palette = view.palette;
    for tile in Tile::ALL {
        let item = Item::Tile(tile);
        let Some(area) = view.layout.rect(item) else {
            continue;
        };
        let accent = tile == Tile::Region;
        let (fill, edge, ink) = if accent {
            (palette.accent, palette.accent, palette.on_accent)
        } else {
            (palette.card, palette.stroke, palette.text)
        };
        let rect = local(area, origin);
        d2d::fill_round(&gpu.context, rect, 6.0, fill);
        d2d::stroke_round(
            &gpu.context,
            d2d::rect(rect.left + 0.5, rect.top + 0.5, area.w - 1.0, area.h - 1.0),
            5.5,
            1.0,
            edge,
        );
        overlay_state(gpu, view, item, area, 6.0, origin);
        let icon = match tile {
            Tile::Region => Icon::PanelRegion,
            Tile::Window => Icon::PanelWindow,
            Tile::Screen => Icon::PanelScreen,
        };
        icons::draw(
            icons,
            gpu,
            icon,
            ink,
            area.x + area.w / 2.0 + origin.0,
            area.y + 8.0 + 10.0 + origin.1,
            20.0,
        );
        draw_label(
            gpu,
            &fonts.medium12,
            tile.label(),
            RectF::new(area.x, area.y + 8.0 + 24.0, area.w, 16.0),
            origin,
            ink,
            true,
        );
    }
}

fn draw_recent(gpu: &Gpu, icons: &mut IconCache, view: &View, fonts: &Fonts, origin: (f32, f32)) {
    let palette = view.palette;
    let layout = view.layout;
    rule(gpu, view, layout.recent_area.y, origin);
    draw_label(
        gpu,
        &fonts.medium12,
        "Recent",
        layout.recent_title,
        origin,
        palette.text3,
        false,
    );
    if let Some(area) = layout.rect(Item::ViewAll) {
        overlay_state(gpu, view, Item::ViewAll, area, 4.0, origin);
        draw_label(
            gpu,
            &fonts.regular12,
            "View all",
            area,
            origin,
            palette.accent,
            true,
        );
    }
    if let Some(area) = layout.empty {
        draw_label(
            gpu,
            &fonts.regular12,
            "No captures yet",
            area,
            origin,
            palette.text3,
            false,
        );
    }
    for (thumb, meta) in layout.recent_placeholders() {
        d2d::fill_round(&gpu.context, local(thumb, origin), 4.0, palette.subtle);
        d2d::fill_round(&gpu.context, local(meta, origin), 3.0, palette.subtle);
    }
    for index in 0..view.data.recent.len().min(MAX_RECENT) {
        let (Some(thumb), Some(meta), Some(whole)) = (
            layout.thumb_rect(index),
            layout.meta_rect(index),
            layout.rect(Item::Recent(index)),
        ) else {
            continue;
        };
        let recent = &view.data.recent[index];
        let rect = local(thumb, origin);
        d2d::fill_round(&gpu.context, rect, 4.0, palette.subtle);
        if let Some(Some(bitmap)) = view.bitmaps.get(index) {
            fill_bitmap_cover(gpu, bitmap, rect, 4.0);
        }
        d2d::stroke_round(
            &gpu.context,
            d2d::rect(
                rect.left + 0.5,
                rect.top + 0.5,
                thumb.w - 1.0,
                thumb.h - 1.0,
            ),
            3.5,
            1.0,
            palette.stroke,
        );
        if let Some(duration) = &recent.duration {
            let text_width = d2d::measure(&gpu.write, &fonts.regular10, duration);
            let badge_width = text_width + 10.0;
            let badge = d2d::rect(
                rect.right - 6.0 - badge_width,
                rect.bottom - 6.0 - 15.0,
                badge_width,
                15.0,
            );
            d2d::fill_round(&gpu.context, badge, 3.0, [0.0, 0.0, 0.0, 0.72]);
            let mut label = badge;
            label.left += 5.0;
            label.right += 200.0;
            d2d::draw_text(
                &gpu.context,
                &fonts.regular10,
                duration,
                label,
                [1.0, 1.0, 1.0, 1.0],
            );
        }
        let mut text_x = meta.x;
        if recent.uploaded {
            icons::draw(
                icons,
                gpu,
                Icon::PanelLink,
                palette.ok,
                text_x + 6.0 + origin.0,
                meta.y + meta.h / 2.0 + origin.1,
                12.0,
            );
            text_x += 16.0;
        }
        draw_label(
            gpu,
            &fonts.regular11,
            &recent.time,
            RectF::new(text_x, meta.y, (meta.right() - text_x).max(0.0), meta.h),
            origin,
            palette.text3,
            false,
        );
        overlay_state(gpu, view, Item::Recent(index), whole, 4.0, origin);
    }
}

fn draw_uploads(gpu: &Gpu, view: &View, fonts: &Fonts, origin: (f32, f32)) {
    let Some(area) = view.layout.uploads_area else {
        return;
    };
    let palette = view.palette;
    rule(gpu, view, area.y, origin);
    draw_label(
        gpu,
        &fonts.medium12,
        "Uploads",
        RectF::new(area.x + 16.0, area.y + 10.0, area.w - 32.0, 16.0),
        origin,
        palette.text3,
        false,
    );
    let mut y = area.y + 10.0 + 16.0 + 8.0;
    for upload in &view.data.uploads {
        let width = area.w - 32.0;
        let name = fit_text(gpu, &fonts.regular12, &upload.name, width);
        draw_label(
            gpu,
            &fonts.regular12,
            &name,
            RectF::new(area.x + 16.0, y, width, 15.0),
            origin,
            palette.text,
            false,
        );
        draw_label(
            gpu,
            &fonts.regular10,
            &upload.detail,
            RectF::new(area.x + 16.0, y + 18.0, width, 12.0),
            origin,
            palette.text3,
            false,
        );
        let bar = d2d::rect(area.x + 16.0 + origin.0, y + 34.0 + origin.1, width, 4.0);
        d2d::fill_round(&gpu.context, bar, 2.0, palette.subtle);
        if let Some(fraction) = upload.fraction {
            let filled = (width * fraction.clamp(0.0, 1.0)).max(4.0);
            d2d::fill_round(
                &gpu.context,
                d2d::rect(bar.left, bar.top, filled, 4.0),
                2.0,
                palette.accent,
            );
        }
        y += 39.0;
    }
}

fn draw_footer(gpu: &Gpu, icons: &mut IconCache, view: &View, fonts: &Fonts, origin: (f32, f32)) {
    let palette = view.palette;
    let layout = view.layout;
    let footer = layout.footer;
    rule(gpu, view, footer.y, origin);
    let label_width = d2d::measure(&gpu.write, &fonts.regular12, "Last link");
    draw_label(
        gpu,
        &fonts.regular12,
        "Last link",
        RectF::new(footer.x + 16.0, footer.y, label_width + 4.0, footer.h),
        origin,
        palette.text3,
        false,
    );
    let copy = layout.rect(Item::Copy);
    let link_x = footer.x + 16.0 + label_width + 8.0;
    let link_right = copy.map_or(footer.right() - 8.0, |area| area.x - 8.0);
    let link = view
        .data
        .link
        .as_deref()
        .filter(|link| !link.is_empty())
        .map(link_without_scheme);
    let shown = fit_text(
        gpu,
        &fonts.mono12,
        link.unwrap_or("None yet"),
        (link_right - link_x).max(0.0),
    );
    draw_label(
        gpu,
        &fonts.mono12,
        &shown,
        RectF::new(link_x, footer.y, (link_right - link_x).max(0.0), footer.h),
        origin,
        palette.text2,
        false,
    );
    let area =
        copy.unwrap_or_else(|| RectF::new(footer.right() - 8.0 - 72.0, footer.y + 8.0, 72.0, 32.0));
    let enabled = copy.is_some();
    let dim = if enabled { 1.0 } else { 0.5 };
    let rect = local(area, origin);
    d2d::fill_round(&gpu.context, rect, 4.0, alpha(palette.card, dim));
    d2d::stroke_round(
        &gpu.context,
        d2d::rect(rect.left + 0.5, rect.top + 0.5, area.w - 1.0, area.h - 1.0),
        3.5,
        1.0,
        alpha(palette.stroke, dim),
    );
    overlay_state(gpu, view, Item::Copy, area, 4.0, origin);
    let text_width = d2d::measure(&gpu.write, &fonts.regular12, view.copy_text);
    let content = 14.0 + 6.0 + text_width;
    let start = area.x + (area.w - content) / 2.0;
    icons::draw(
        icons,
        gpu,
        Icon::Copy,
        alpha(palette.text, dim),
        start + 7.0 + origin.0,
        area.y + area.h / 2.0 + origin.1,
        14.0,
    );
    draw_label(
        gpu,
        &fonts.regular12,
        view.copy_text,
        RectF::new(start + 20.0, area.y, text_width + 4.0, area.h),
        origin,
        alpha(palette.text, dim),
        false,
    );
}

struct Context {
    gpu: Gpu,
    icons: IconCache,
    hwnd: HWND,
    theme: Theme,
    data: PanelData,
    layout: Layout,
    interaction: Interaction,
    bitmaps: Vec<Option<ID2D1Bitmap1>>,
    surface: Option<Surface>,
    scale: f32,
    pad: f32,
    body_origin: (i32, i32),
    place: PlaceFn,
    on_activate: ActivateFn,
    shared: Arc<Shared>,
    copied_at: Option<Instant>,
    activated: bool,
    tracking: bool,
    class_name: Vec<u16>,
    instance: HINSTANCE,
}

fn dpi_scale(anchor: (i32, i32)) -> f32 {
    let point = windows::Win32::Foundation::POINT {
        x: anchor.0,
        y: anchor.1,
    };
    // SAFETY: the point is a plain value and the nearest monitor is always returned.
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut x = 96_u32;
    let mut y = 96_u32;
    // SAFETY: the monitor handle came from MonitorFromPoint and both out pointers are live.
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) }.is_err() {
        return 1.0;
    }
    (x as f32 / 96.0).max(1.0)
}

fn to_px(value: f32, scale: f32) -> i32 {
    (value * scale).round() as i32
}

fn key_for(code: u16, shift: bool) -> Option<Key> {
    let key = match code {
        c if c == VK_TAB.0 => {
            if shift {
                Key::ShiftTab
            } else {
                Key::Tab
            }
        }
        c if c == VK_LEFT.0 => Key::Left,
        c if c == VK_RIGHT.0 => Key::Right,
        c if c == VK_UP.0 => Key::Up,
        c if c == VK_DOWN.0 => Key::Down,
        c if c == VK_RETURN.0 => Key::Enter,
        c if c == VK_SPACE.0 => Key::Space,
        c if c == VK_ESCAPE.0 => Key::Escape,
        _ => return None,
    };
    Some(key)
}

impl Context {
    fn build(
        theme: Theme,
        data: PanelData,
        anchor: (i32, i32),
        place: PlaceFn,
        on_activate: ActivateFn,
        shared: Arc<Shared>,
    ) -> Result<Self, OverlayError> {
        let gpu = Gpu::new()?;
        // SAFETY: returns the module handle of this process without borrowing any memory.
        let instance = unsafe { GetModuleHandleW(None) }
            .map_err(|error| OverlayError::Device(error.to_string()))?;
        let name = format!("{CLASS_NAME}{}", NEXT_CLASS.fetch_add(1, Ordering::Relaxed));
        let class_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let layout = Layout::compute(data.content());
        let scale = dpi_scale(anchor);
        Ok(Self {
            gpu,
            icons: IconCache::new(),
            hwnd: HWND::default(),
            theme,
            data,
            layout,
            interaction: Interaction::default(),
            bitmaps: Vec::new(),
            surface: None,
            scale,
            pad: SHADOW,
            body_origin: (0, 0),
            place,
            on_activate,
            shared,
            copied_at: None,
            activated: false,
            tracking: false,
            class_name,
            instance: HINSTANCE(instance.0),
        })
    }

    fn body_size(&self) -> (i32, i32) {
        (
            to_px(WIDTH, self.scale),
            to_px(self.layout.height, self.scale),
        )
    }

    fn window_rect(&self) -> (i32, i32, i32, i32) {
        let pad = to_px(self.pad, self.scale);
        let (width, height) = self.body_size();
        (
            self.body_origin.0 - pad,
            self.body_origin.1 - pad,
            width + 2 * pad,
            height + 2 * pad,
        )
    }

    fn load_bitmaps(&mut self) {
        self.bitmaps = self
            .data
            .recent
            .iter_mut()
            .take(MAX_RECENT)
            .map(|recent| {
                recent
                    .thumb
                    .take()
                    .and_then(|image| self.gpu.upload_image(&image).ok())
            })
            .collect();
    }

    fn activate(&mut self) -> Result<(), OverlayError> {
        self.register_class()?;
        self.load_bitmaps();
        let (width, height) = self.body_size();
        self.body_origin = (self.place)(width, height);
        let (x, y, window_width, window_height) = self.window_rect();
        self.create_window(x, y, window_width, window_height)?;
        let surface = create_surface(
            &self.gpu,
            self.hwnd,
            u32::try_from(window_width).unwrap_or(1),
            u32::try_from(window_height).unwrap_or(1),
        )?;
        self.surface = Some(surface);
        self.present()?;
        // SAFETY: the window handle is a live top level window owned by this thread.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
        self.focus();
        tracing::info!("native quick panel ready");
        Ok(())
    }

    fn register_class(&mut self) -> Result<(), OverlayError> {
        // SAFETY: loads a stock system cursor owned by the process.
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
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
        if unsafe { RegisterClassExW(&class) } == 0 {
            return Err(OverlayError::Device(
                "The quick panel window class could not be registered.".to_owned(),
            ));
        }
        Ok(())
    }

    fn create_window(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), OverlayError> {
        let class = PCWSTR::from_raw(self.class_name.as_ptr());
        let style = WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST;
        // SAFETY: the class name buffer, module handle and coordinates are valid.
        let hwnd = unsafe {
            CreateWindowExW(
                style,
                class,
                PCWSTR::null(),
                WS_POPUP,
                x,
                y,
                width,
                height,
                None,
                None,
                Some(self.instance),
                None,
            )
        }
        .map_err(|error| OverlayError::Device(error.to_string()))?;
        crate::set_window_data(hwnd, std::ptr::from_mut(self) as isize);
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

    fn copy_text(&self) -> &'static str {
        let elapsed = self
            .copied_at
            .map(|at| u32::try_from(at.elapsed().as_millis()).unwrap_or(u32::MAX));
        copy_label(elapsed.filter(|elapsed| *elapsed < COPIED_MS))
    }

    fn redraw(&mut self) {
        if let Err(error) = self.present() {
            tracing::warn!(%error, "could not present the native quick panel");
            self.close();
        }
    }

    fn present(&mut self) -> Result<(), OverlayError> {
        let Some(surface) = self.surface.as_ref() else {
            return Ok(());
        };
        let copy_text = self.copy_text();
        let view = View {
            layout: &self.layout,
            interaction: &self.interaction,
            data: &self.data,
            bitmaps: &self.bitmaps,
            palette: self.theme.palette(),
            copy_text,
        };
        // SAFETY: the swap chain target belongs to this render pass.
        unsafe {
            self.gpu.context.SetTarget(&surface.target);
        }
        render(&mut self.gpu, &mut self.icons, &view, self.scale, self.pad);
        // SAFETY: the render pass is finished and the buffer can be presented.
        unsafe {
            let _ = self.gpu.context.SetTarget(None::<&ID2D1Image>);
            surface.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
        }
        .map_err(device_error)
    }

    fn logical(&self, lparam: LPARAM) -> (f32, f32) {
        let x = (lparam.0 & 0xFFFF) as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
        (x / self.scale - self.pad, y / self.scale - self.pad)
    }

    fn apply(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw(),
            Outcome::Close => self.close(),
            Outcome::Activate(item) => self.activate_item(item),
        }
    }

    fn activate_item(&mut self, item: Item) {
        let activation = match item {
            Item::OpenMain => Activation::OpenMain,
            Item::Settings => Activation::Settings,
            Item::ViewAll => Activation::ViewAll,
            Item::Tile(tile) => Activation::Capture(tile.action()),
            Item::Recent(index) => match self.data.recent.get(index) {
                Some(recent) => Activation::Recent(recent.id),
                None => return,
            },
            Item::Copy => Activation::CopyLink,
        };
        (self.on_activate)(activation);
        if activation == Activation::CopyLink {
            self.copied_at = Some(Instant::now());
            // SAFETY: the one shot timer is owned by this window.
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_COPIED, COPIED_MS + 50, None);
            }
            self.redraw();
        } else {
            self.close();
        }
    }

    fn close(&mut self) {
        if self.hwnd.is_invalid() {
            return;
        }
        let hwnd = self.hwnd;
        self.hwnd = HWND::default();
        self.surface = None;
        crate::destroy_window(hwnd);
        // SAFETY: ends the message loop of this dedicated thread.
        unsafe { PostQuitMessage(0) };
    }

    fn adopt_data(&mut self) {
        let pending = self
            .shared
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.take());
        let Some(data) = pending else {
            return;
        };
        let history_loaded = !self.data.loaded && data.loaded;
        let thumbnails_loaded = data.recent.iter().any(|recent| recent.thumb.is_some());
        let previous_size = self.body_size();
        self.data = data;
        self.layout = Layout::compute(self.data.content());
        self.load_bitmaps();
        self.interaction.retain_for_layout(&self.layout);
        let size = self.body_size();
        if size != previous_size {
            self.interaction.pointer_leave();
            self.body_origin = (self.place)(size.0, size.1);
            let (x, y, width, height) = self.window_rect();
            self.surface = None;
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            match create_surface(
                &self.gpu,
                self.hwnd,
                u32::try_from(width).unwrap_or(1),
                u32::try_from(height).unwrap_or(1),
            ) {
                Ok(surface) => self.surface = Some(surface),
                Err(error) => {
                    tracing::warn!(%error, "could not resize the native quick panel");
                    self.close();
                    return;
                }
            }
        }
        self.redraw();
        if !self.hwnd.is_invalid() {
            if history_loaded {
                tracing::info!(
                    recent = self.data.recent.len(),
                    "native quick panel history ready"
                );
            }
            if thumbnails_loaded {
                tracing::info!("native quick panel thumbnails ready");
            }
        }
    }

    fn handle(&mut self, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                // SAFETY: validates the whole client area; painting happens through the swap chain.
                unsafe {
                    let _ = ValidateRect(Some(hwnd), None);
                }
                return LRESULT(0);
            }
            WM_MOUSEACTIVATE => return LRESULT(MA_ACTIVATE as isize),
            WM_ACTIVATE => {
                let state = (wparam.0 & 0xFFFF) as u32;
                if state == WA_INACTIVE {
                    if self.activated {
                        self.close();
                    }
                } else {
                    self.activated = true;
                }
                return LRESULT(0);
            }
            WM_SETCURSOR => {
                if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                    let hand = self
                        .interaction
                        .hover
                        .is_some_and(|item| self.layout.is_enabled(item));
                    let id = if hand { IDC_HAND } else { IDC_ARROW };
                    // SAFETY: loads and sets a stock system cursor.
                    unsafe {
                        if let Ok(cursor) = LoadCursorW(None, id) {
                            SetCursor(Some(cursor));
                        }
                    }
                    return LRESULT(1);
                }
            }
            WM_MOUSEMOVE => {
                if !self.tracking {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: the tracking structure is live and names this window.
                    self.tracking = unsafe { TrackMouseEvent(&mut track) }.is_ok();
                }
                let (x, y) = self.logical(lparam);
                let outcome = self.interaction.pointer_move(&self.layout, x, y);
                self.apply(outcome);
                return LRESULT(0);
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                let outcome = self.interaction.pointer_leave();
                self.apply(outcome);
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.logical(lparam);
                let outcome = self.interaction.pointer_down(&self.layout, x, y);
                self.apply(outcome);
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let (x, y) = self.logical(lparam);
                let outcome = self.interaction.pointer_up(&self.layout, x, y);
                self.apply(outcome);
                return LRESULT(0);
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                // SAFETY: reads the shift key state of the calling thread.
                let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
                if let Some(key) = key_for((wparam.0 & 0xFFFF) as u16, shift) {
                    let outcome = self.interaction.key(&self.layout, key);
                    self.apply(outcome);
                    return LRESULT(0);
                }
            }
            WM_TIMER => {
                if wparam.0 == TIMER_COPIED {
                    // SAFETY: stops the one shot timer owned by this window.
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_COPIED);
                    }
                    self.copied_at = None;
                    self.redraw();
                }
                return LRESULT(0);
            }
            WM_DATA => {
                self.adopt_data();
                return LRESULT(0);
            }
            WM_CLOSE => {
                self.close();
                return LRESULT(0);
            }
            _ => {}
        }
        // SAFETY: unhandled messages use the default window procedure.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn destroy(&mut self) {
        if !self.hwnd.is_invalid() {
            crate::destroy_window(self.hwnd);
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
    use windows::Win32::Graphics::Direct2D::{D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_MAP_OPTIONS_READ};

    fn offscreen(gpu: &Gpu, width: u32, height: u32) -> Option<ID2D1Bitmap1> {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            ..Default::default()
        };
        // SAFETY: the size and properties are plain values for a new bitmap.
        unsafe {
            gpu.context.CreateBitmap(
                windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U { width, height },
                None,
                0,
                &properties,
            )
        }
        .ok()
    }

    fn read(gpu: &Gpu, source: &ID2D1Bitmap1, width: u32, height: u32) -> Option<Vec<u8>> {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        // SAFETY: the size and properties are plain values for a new bitmap.
        let staging = unsafe {
            gpu.context.CreateBitmap(
                windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U { width, height },
                None,
                0,
                &properties,
            )
        }
        .ok()?;
        // SAFETY: both bitmaps are live and have the same size and format.
        unsafe { staging.CopyFromBitmap(None, source, None) }.ok()?;
        // SAFETY: the staging bitmap was created CPU readable and is unmapped below.
        let mapped = unsafe { staging.Map(D2D1_MAP_OPTIONS_READ) }.ok()?;
        let mut bytes = Vec::new();
        for row in 0..height as usize {
            // SAFETY: each row lies inside the mapped region of pitch bytes per row.
            let line = unsafe {
                std::slice::from_raw_parts(
                    mapped.bits.add(row * mapped.pitch as usize),
                    width as usize * 4,
                )
            };
            bytes.extend_from_slice(line);
        }
        // SAFETY: pairs with the Map call above.
        unsafe {
            let _ = staging.Unmap();
        }
        Some(bytes)
    }

    fn pixel(bytes: &[u8], width: u32, x: f32, y: f32) -> [u8; 4] {
        let index = (y as usize * width as usize + x as usize) * 4;
        [
            bytes[index],
            bytes[index + 1],
            bytes[index + 2],
            bytes[index + 3],
        ]
    }

    fn close(actual: u8, expected: f32) -> bool {
        (f32::from(actual) - expected * 255.0).abs() <= 3.0
    }

    #[test]
    fn the_panel_renders_its_regions_in_the_theme_colours() {
        let Ok(mut gpu) = Gpu::new() else {
            eprintln!("skipped: no hardware Direct3D device");
            return;
        };
        let mut icons = IconCache::new();
        let data = PanelData {
            recent: Vec::new(),
            loaded: true,
            uploads: Vec::new(),
            link: Some(String::from("https://example.com/abc")),
        };
        let layout = Layout::compute(data.content());
        let interaction = Interaction::default();
        let palette = Theme::Dark.palette();
        let view = View {
            layout: &layout,
            interaction: &interaction,
            data: &data,
            bitmaps: &[],
            palette,
            copy_text: "Copy",
        };
        let width = (WIDTH + 2.0 * SHADOW) as u32;
        let height = (layout.height + 2.0 * SHADOW).ceil() as u32;
        let Some(target) = offscreen(&gpu, width, height) else {
            return;
        };
        // SAFETY: the bitmap was created as a render target.
        unsafe { gpu.context.SetTarget(&target) };
        render(&mut gpu, &mut icons, &view, 1.0, SHADOW);
        // SAFETY: detaches the target after the pass.
        unsafe {
            let _ = gpu.context.SetTarget(None::<&ID2D1Image>);
        }
        let Some(bytes) = read(&gpu, &target, width, height) else {
            eprintln!("skipped: the render target could not be read back");
            return;
        };
        let region = layout.rect(Item::Tile(Tile::Region)).expect("region");
        let window = layout.rect(Item::Tile(Tile::Window)).expect("window");
        let at = |area: RectF, dx: f32, dy: f32| {
            pixel(&bytes, width, area.x + SHADOW + dx, area.y + SHADOW + dy)
        };
        let accent = at(region, 6.0, 6.0);
        assert!(close(accent[2], palette.accent[0]));
        assert!(close(accent[1], palette.accent[1]));
        assert!(close(accent[0], palette.accent[2]));
        let card = at(window, 6.0, 6.0);
        assert!(close(card[2], palette.card[0]));
        let body = pixel(&bytes, width, SHADOW + 170.0, SHADOW + 2.0);
        assert!(close(body[2], palette.layer[0]));
        let footer = pixel(
            &bytes,
            width,
            SHADOW + 170.0,
            SHADOW + layout.footer.y + 3.0,
        );
        assert!(close(footer[2], palette.mica[0]));
        let corner = pixel(&bytes, width, 0.0, 0.0);
        assert_eq!(corner[3], 0);
        let shadow = pixel(&bytes, width, SHADOW + 170.0, SHADOW - 4.0);
        assert!(shadow[3] > 0 && shadow[3] < 120);
    }

    #[test]
    fn keys_map_to_panel_keys() {
        assert_eq!(key_for(VK_TAB.0, false), Some(Key::Tab));
        assert_eq!(key_for(VK_TAB.0, true), Some(Key::ShiftTab));
        assert_eq!(key_for(VK_ESCAPE.0, false), Some(Key::Escape));
        assert_eq!(key_for(VK_RETURN.0, false), Some(Key::Enter));
        assert_eq!(key_for(0x41, false), None);
    }

    #[test]
    fn pixel_sizes_round_at_fractional_scales() {
        assert_eq!(to_px(344.0, 1.0), 344);
        assert_eq!(to_px(344.0, 1.25), 430);
        assert_eq!(to_px(344.0, 1.5), 516);
        assert_eq!(to_px(344.0, 2.0), 688);
    }

    #[test]
    fn panel_data_reports_its_layout_content() {
        let data = PanelData {
            recent: Vec::new(),
            loaded: true,
            uploads: Vec::new(),
            link: Some(String::new()),
        };
        assert!(!data.content().has_link);
        let with_link = PanelData {
            link: Some(String::from("https://x.test/a")),
            ..data
        };
        assert!(with_link.content().has_link);
    }
}
