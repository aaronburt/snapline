use std::time::Instant;

use snapline_core::annotate::{crop_image, Document};
use snapline_core::geometry::{Point, Px, Rect, Size};
use snapline_core::image::Image;
use snapline_core::overlay::{
    Effect, Input, Overlay, Selection, SnapCandidate, SnapSet, State, Tool,
};
use snapline_core::ports::Monitor;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext5, ID2D1Image, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_ALIASED,
};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain1, DXGI_PRESENT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, UpdateWindow, ValidateRect, HDC, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VIRTUAL_KEY, VK_A, VK_B, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN, VK_E, VK_ESCAPE, VK_H, VK_I,
    VK_LEFT, VK_MENU, VK_N, VK_P, VK_R, VK_RETURN, VK_RIGHT, VK_S, VK_SHIFT, VK_SPACE, VK_T,
    VK_TAB, VK_UP, VK_V, VK_Y, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetForegroundWindow,
    GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId, KillTimer, LoadCursorW,
    PostQuitMessage, RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer, ShowWindow,
    TranslateMessage, UnregisterClassW, GWLP_USERDATA, HTCLIENT, IDC_ARROW, IDC_CROSS, MA_ACTIVATE,
    MSG, SW_SHOW, WM_CLOSE, WM_CTLCOLOREDIT, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_RBUTTONDOWN,
    WM_SETCURSOR, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

#[path = "../../platform-windows/src/uia.rs"]
mod uia;
#[path = "../../platform-windows/src/windows_enum.rs"]
mod windows_enum;

use crate::announce;
use crate::canvas_d2d;
use crate::d2d::Gpu;
use crate::hints;
use crate::icons::IconCache;
use crate::magnifier;
use crate::readout;
use crate::record_border::{
    is_click, resolve_record_target, RecordPickRequest, RecordTarget, RecordWindow,
};
use crate::text_input::{self, TextInput};
use crate::tokens::Palette;
use crate::toolbar::{self, ToolbarAction, ToolbarHit, ToolbarState};
use crate::tools::{self, Session};
use crate::{
    AnnotationColour, EditRequest, MagnifierSettings, OverlayAction, OverlayError, SelectRequest,
    StrokeSize, Theme,
};

pub(crate) const CLASS_NAME: &str = "SnaplineOverlayWindow";
const SNAP_TIMER_ID: usize = 1;
const PILL_TIMER_ID: usize = 2;
const SNAP_DEADLINE_MS: u32 = 30;

pub(crate) struct ScreenSource {
    pub monitor: Monitor,
    pub scale: f32,
    pub origin: Point,
    pub size: Size,
    pub image: std::sync::Arc<Image>,
}

pub(crate) struct WindowState {
    _dcomp_target: windows::Win32::Graphics::DirectComposition::IDCompositionTarget,
    _visual: windows::Win32::Graphics::DirectComposition::IDCompositionVisual,
    pub hwnd: HWND,
    pub source: ScreenSource,
    pub swap_chain: IDXGISwapChain1,
    pub target: ID2D1Bitmap1,
    pub frozen: ID2D1Bitmap1,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SnapFeed {
    windows: Vec<SnapCandidate>,
    control: Option<SnapCandidate>,
}

impl SnapFeed {
    pub(crate) fn new(set: SnapSet) -> Self {
        Self {
            windows: set.windows,
            control: set.controls.into_iter().next(),
        }
    }

    pub(crate) fn set_windows(&mut self, windows: Vec<SnapCandidate>) {
        self.windows = windows;
    }

    pub(crate) fn set_control(&mut self, rect: Option<Rect>) -> bool {
        let next = rect.map(SnapCandidate::new);
        if self.control == next {
            return false;
        }
        self.control = next;
        true
    }

    pub(crate) fn clear_control(&mut self) -> bool {
        self.set_control(None)
    }

    pub(crate) fn set(&self) -> SnapSet {
        SnapSet {
            windows: self.windows.clone(),
            controls: self.control.into_iter().collect(),
        }
    }
}

pub(crate) struct Scene {
    pub overlay: Overlay,
    pub annotate: bool,
    pub editing: bool,
    pub upload_on_finish: bool,
    pub record: bool,
    pub cursor: Option<Point>,
    pub pointer_monitor: usize,
    pub magnifier: MagnifierSettings,
    pub full_screen_crosshair: bool,
    pub theme: Theme,
    pub tool: Tool,
    pub colour: AnnotationColour,
    pub stroke: StrokeSize,
    pub pending_action: Option<OverlayAction>,
    pub document: Option<Document>,
    pub document_rect: Rect,
    pub document_scale: f32,
}

impl Scene {
    pub(crate) fn live_rect(&self) -> Option<Rect> {
        match &self.overlay.state {
            State::Dragging { from } => self
                .overlay
                .pointer
                .map(|pointer| Rect::from_points(*from, pointer)),
            _ => self.overlay.selected_rect(),
        }
    }

    pub(crate) fn shown_selection(&self) -> Option<Rect> {
        self.overlay.selected_rect()
    }
}

pub(crate) enum Outcome {
    Finish {
        action: Option<OverlayAction>,
        selection: Rect,
        monitor: Option<u32>,
    },
    Cancel,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Layout {
    pub toolbar: Option<toolbar::Toolbar>,
    pub hint: Option<hints::HintPill>,
    pub colour: Option<hints::HintPill>,
    pub keyboard: Option<hints::KeyboardPill>,
}

pub(crate) struct Context {
    pub(crate) flattened: Option<Image>,
    pub windows: Vec<WindowState>,
    pub scene: Scene,
    pub gpu: Gpu,
    pub icons: IconCache,
    pub layout: Layout,
    pub outcome: Option<Outcome>,
    pub(crate) record_target: Option<RecordTarget>,
    record_entries: Vec<windows_enum::WindowEntry>,
    record_press: Option<(Rect, RecordWindow, Point)>,
    stroke: Option<ID2D1StrokeStyle>,
    previous_pointer_monitor: usize,
    previous_selection: Option<Rect>,
    class_name: Vec<u16>,
    instance: HINSTANCE,
    sources: Vec<(ScreenSource, bool)>,
    snap: SnapFeed,
    snap_worker: Option<uia::ControlSnapWorker>,
    snap_generation: u64,
    snap_pending: Option<u64>,
    snap_timer: Option<HWND>,
    tracking_mouse: bool,
    tools: Session,
    text_edit: Option<TextInput>,
    pill: Option<tools::ColourPill>,
    pill_timer: Option<HWND>,
    toolbar_focus: Option<usize>,
    announced_selection: Option<Rect>,
}

impl Context {
    pub(crate) fn build_select(req: SelectRequest) -> Result<Self, OverlayError> {
        let SelectRequest {
            desktop,
            annotate,
            upload_on_finish,
            snap,
            magnifier,
            full_screen_crosshair,
            theme,
        } = req;
        let mut sources = Vec::new();
        for frozen in desktop.monitors {
            let monitor = frozen.monitor;
            sources.push((
                ScreenSource {
                    scale: monitor.dpi.scale(),
                    origin: monitor.bounds.origin(),
                    size: monitor.bounds.size(),
                    monitor,
                    image: frozen.image,
                },
                true,
            ));
        }
        let mut context = Self::build(
            sources,
            annotate,
            false,
            snap,
            magnifier,
            full_screen_crosshair,
            theme,
        )?;
        context.scene.upload_on_finish = upload_on_finish;
        Ok(context)
    }

    pub(crate) fn build_record(req: RecordPickRequest) -> Result<Self, OverlayError> {
        let RecordPickRequest {
            desktop,
            snap,
            magnifier,
            theme,
        } = req;
        let mut sources = Vec::new();
        for frozen in desktop.monitors {
            let monitor = frozen.monitor;
            sources.push((
                ScreenSource {
                    scale: monitor.dpi.scale(),
                    origin: monitor.bounds.origin(),
                    size: monitor.bounds.size(),
                    monitor,
                    image: frozen.image,
                },
                true,
            ));
        }
        let mut context = Self::build(sources, false, false, snap, magnifier, false, theme)?;
        context.scene.record = true;
        context.snap_worker = None;
        Ok(context)
    }

    pub(crate) fn build_edit(req: EditRequest) -> Result<Self, OverlayError> {
        let size = Size::new(
            Px(i32::try_from(req.image.width()).unwrap_or(i32::MAX)),
            Px(i32::try_from(req.image.height()).unwrap_or(i32::MAX)),
        );
        let origin = edit_origin(size);
        let monitor = Monitor {
            id: 0,
            name: String::from("Edit"),
            bounds: Rect::from_origin_size(origin, size),
            work_area: Rect::from_origin_size(origin, size),
            dpi: snapline_core::geometry::DpiScale::ONE,
            primary: true,
        };
        let sources = vec![(
            ScreenSource {
                monitor,
                scale: 1.0,
                origin,
                size,
                image: std::sync::Arc::new(req.image),
            },
            true,
        )];
        Self::build(
            sources,
            true,
            true,
            SnapSet::default(),
            req.magnifier,
            false,
            req.theme,
        )
    }

    fn build(
        sources: Vec<(ScreenSource, bool)>,
        annotate: bool,
        editing: bool,
        snap: SnapSet,
        magnifier: MagnifierSettings,
        full_screen_crosshair: bool,
        theme: Theme,
    ) -> Result<Self, OverlayError> {
        if sources.is_empty() {
            return Err(OverlayError::NoMonitors);
        }
        let gpu = Gpu::new()?;
        let snap = SnapFeed::new(snap);
        let snap_worker = if editing {
            None
        } else {
            Some(uia::ControlSnapWorker::spawn())
        };
        let mut overlay = Overlay::with_annotate(annotate);
        if editing {
            let first = &sources[0].0;
            overlay.state = State::Annotating {
                rect: Rect::from_origin_size(first.origin, first.size),
                tool: overlay.tool,
            };
        }
        let (overlay, _) = overlay.on(Input::SnapCandidates(snap.set()));
        let tool = overlay.tool;
        let colour = if tool == Tool::Highlighter {
            AnnotationColour::Yellow
        } else {
            AnnotationColour::Red
        };
        let scene = Scene {
            overlay,
            annotate,
            editing,
            upload_on_finish: false,
            record: false,
            cursor: None,
            pointer_monitor: 0,
            magnifier,
            full_screen_crosshair,
            theme,
            tool,
            colour,
            stroke: StrokeSize::default(),
            pending_action: None,
            document: None,
            document_rect: Rect::ZERO,
            document_scale: 1.0,
        };
        // SAFETY: returns the module handle of this process without borrowing any memory.
        let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map_err(|error| OverlayError::Device(error.to_string()))?;
        let class_name: Vec<u16> = CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let stroke = canvas_d2d::stroke_style(&gpu);
        let context = Self {
            flattened: None,
            windows: Vec::new(),
            scene,
            gpu,
            icons: IconCache::new(),
            layout: Layout::default(),
            outcome: None,
            record_target: None,
            record_entries: Vec::new(),
            record_press: None,
            stroke,
            previous_pointer_monitor: 0,
            previous_selection: None,
            class_name,
            instance: HINSTANCE(instance.0),
            sources,
            snap,
            snap_worker,
            snap_generation: 0,
            snap_pending: None,
            snap_timer: None,
            tracking_mouse: false,
            tools: Session::default(),
            text_edit: None,
            pill: None,
            pill_timer: None,
            toolbar_focus: None,
            announced_selection: None,
        };
        Ok(context)
    }

    pub(crate) fn activate(&mut self) -> Result<(), OverlayError> {
        self.register_class()?;
        self.open()?;
        self.feed_windows();
        self.sync_document();
        self.layout = make_layout(
            &self.windows,
            &self.scene,
            self.pill.as_ref(),
            &mut self.gpu,
        );
        self.redraw();
        self.show();
        Ok(())
    }

    fn feed_windows(&mut self) {
        let exclude: Vec<HWND> = self.windows.iter().map(|window| window.hwnd).collect();
        if self.scene.record {
            let entries = windows_enum::enumerate(&exclude);
            self.record_entries = entries;
            self.snap.set_windows(
                self.record_entries
                    .iter()
                    .map(|entry| SnapCandidate::new(entry.rect))
                    .collect(),
            );
            self.apply_snap();
            return;
        }
        let windows = windows_enum::snap_candidates(&exclude);
        if windows.is_empty() {
            return;
        }
        self.snap.set_windows(windows);
        self.apply_snap();
    }

    fn apply_snap(&mut self) -> bool {
        let set = self.snap.set();
        self.apply(Input::SnapCandidates(set))
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
                "The overlay window class could not be registered.".to_string(),
            ));
        }
        Ok(())
    }

    fn open(&mut self) -> Result<(), OverlayError> {
        let sources = std::mem::take(&mut self.sources);
        for (source, topmost) in sources {
            let hwnd = self.create_window(&source, topmost)?;
            let width = u32::try_from(source.size.width.get()).unwrap_or(1);
            let height = u32::try_from(source.size.height.get()).unwrap_or(1);
            let swap = match self.gpu.create_target(hwnd, width, height) {
                Ok(swap) => swap,
                Err(error) => {
                    crate::destroy_window(hwnd);
                    return Err(error);
                }
            };
            let frozen = match self.gpu.upload_image(&source.image) {
                Ok(frozen) => frozen,
                Err(error) => {
                    crate::destroy_window(hwnd);
                    return Err(error);
                }
            };
            self.windows.push(WindowState {
                hwnd,
                source,
                _dcomp_target: swap._dcomp_target,
                _visual: swap._visual,
                swap_chain: swap.swap_chain,
                target: swap.target,
                frozen,
            });
        }
        Ok(())
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

    fn create_window(
        &mut self,
        source: &ScreenSource,
        topmost: bool,
    ) -> Result<HWND, OverlayError> {
        let class = PCWSTR::from_raw(self.class_name.as_ptr());
        let style = WS_EX_TOOLWINDOW
            | WS_EX_NOREDIRECTIONBITMAP
            | if topmost {
                WS_EX_TOPMOST
            } else {
                Default::default()
            };
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
        .map_err(|error| OverlayError::Device(error.to_string()))?;
        crate::set_window_data(hwnd, self as *mut Self as isize);
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

    fn index_of(&self, hwnd: HWND) -> Option<usize> {
        self.windows.iter().position(|window| window.hwnd == hwnd)
    }

    fn sync_document(&mut self) {
        if self.scene.document.is_some() {
            return;
        }
        let Some(rect) = self.scene.overlay.selected_rect() else {
            return;
        };
        let Some(bounds) = self
            .windows
            .iter()
            .map(|window| window.source.monitor.bounds)
            .reduce(Rect::union)
        else {
            return;
        };
        if rect.intersection(bounds).is_none() {
            return;
        }
        let sources: Vec<&ScreenSource> =
            self.windows.iter().map(|window| &window.source).collect();
        let Some(image) = document_image(&sources, bounds) else {
            return;
        };
        self.scene.document_rect = bounds;
        self.scene.document_scale = document_scale_at(
            self.windows
                .iter()
                .map(|window| (window.source.monitor.bounds, window.source.scale)),
            rect.center(),
        );
        self.scene.document = Some(Document::shared(image));
    }

    fn clear_document(&mut self) {
        self.scene.document = None;
        self.scene.document_rect = Rect::ZERO;
    }

    fn annotating(&self) -> bool {
        self.scene.document.is_some()
            && matches!(
                self.scene.overlay.state,
                State::Annotating { .. } | State::Adjusting { .. }
            )
    }

    fn document_point(&self, point: Point) -> Point {
        let origin = self.scene.document_rect.origin();
        Point::new(Px(point.x.0 - origin.x.0), Px(point.y.0 - origin.y.0))
    }

    fn begin_text(&mut self, origin: Point) {
        self.commit_text();
        let monitor = self.windows.iter().find(|window| {
            window
                .source
                .monitor
                .bounds
                .contains(self.scene.document_rect.origin().offset(origin.x, origin.y))
        });
        let Some(window) = monitor else {
            return;
        };
        let area = tools::text_box(origin, self.scene.document_scale)
            .translated(self.scene.document_rect.origin());
        let area = local_rect(area, window.source.origin);
        let colour = core_colour(self.scene.colour);
        self.text_edit =
            TextInput::create(window.hwnd, area, origin, colour, self.scene.document_scale);
    }

    fn commit_text(&mut self) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        let text = edit.text();
        if let Some(document) = self.scene.document.as_mut() {
            let _ = tools::add_text(
                document,
                edit.origin(),
                text,
                core_colour(self.scene.colour),
            );
        }
    }

    fn cancel_text(&mut self) {
        self.text_edit = None;
    }

    fn copy_colour(&mut self, rgb: snapline_core::overlay::Rgb) {
        let text = tools::hex(rgb);
        let _ = tools::copy_text(&text);
        self.pill = Some(tools::ColourPill::new(rgb, Instant::now()));
        let hwnd = self
            .windows
            .get(self.scene.pointer_monitor)
            .map(|window| window.hwnd);
        if let Some(hwnd) = hwnd {
            if self.pill_timer != Some(hwnd) {
                self.clear_pill_timer();
            }
            // SAFETY: arms a one-shot dismissal timer on a live window owned by this thread.
            let timer =
                unsafe { SetTimer(Some(hwnd), PILL_TIMER_ID, tools::PILL_DURATION_MS, None) };
            self.pill_timer = if timer == 0 { None } else { Some(hwnd) };
        }
    }

    fn clear_pill_timer(&mut self) {
        let Some(hwnd) = self.pill_timer.take() else {
            return;
        };
        // SAFETY: cancels a timer on a live window owned by this thread.
        let _ = unsafe { KillTimer(Some(hwnd), PILL_TIMER_ID) };
    }

    fn flatten(&self, selection: Rect) -> Option<Image> {
        if let Some(document) = &self.scene.document {
            let region = if self.scene.editing {
                document.crop()
            } else {
                selection_in_document(
                    document.crop(),
                    self.scene.document_rect.origin(),
                    selection,
                )?
            };
            return canvas_d2d::render_document_region(
                &self.gpu,
                document,
                self.scene.document_scale,
                self.stroke.as_ref(),
                region,
            )
            .ok()
            .or_else(|| {
                if document.annotations().iter().any(|annotation| {
                    matches!(
                        annotation.shape,
                        snapline_core::annotate::Shape::Text { .. }
                    )
                }) {
                    None
                } else {
                    let image = document.render().ok()?;
                    crop_image(&image, local_rect(region, document.crop().origin())).ok()
                }
            });
        }
        let bounds = self
            .windows
            .iter()
            .map(|window| window.source.monitor.bounds)
            .reduce(Rect::union)?;
        compose_region(
            self.windows.iter().map(|window| &window.source),
            selection.intersection(bounds)?,
        )
    }

    pub(crate) fn undo(&mut self) -> bool {
        let changed = self
            .scene
            .document
            .as_mut()
            .is_some_and(|document| document.undo().unwrap_or(false));
        if changed {
            if let Some(document) = &self.scene.document {
                self.tools.sanitize(document);
            }
        }
        changed
    }

    pub(crate) fn redo(&mut self) -> bool {
        let changed = self
            .scene
            .document
            .as_mut()
            .is_some_and(|document| document.redo().unwrap_or(false));
        if changed {
            if let Some(document) = &self.scene.document {
                self.tools.sanitize(document);
            }
        }
        changed
    }

    pub(crate) fn redraw(&mut self) {
        for index in 0..self.windows.len() {
            self.redraw_window(index);
        }
        self.previous_selection = self.scene.live_rect();
    }

    fn redraw_window(&mut self, index: usize) {
        let window = &self.windows[index];
        let scene = &self.scene;
        let layout = &self.layout;
        let tools = &self.tools;
        let gpu = &mut self.gpu;
        let icons = &mut self.icons;
        let stroke = self.stroke.as_ref();
        let focus = self.toolbar_focus;
        let text_preview = self.text_edit.as_ref().map(|edit| {
            snapline_core::annotate::Shape::text(
                edit.origin(),
                edit.text(),
                core_colour(scene.colour),
            )
        });
        draw_window(
            gpu,
            icons,
            window,
            index,
            scene,
            layout,
            tools,
            stroke,
            focus,
            text_preview.as_ref(),
        );
    }

    fn redraw_active(&mut self, old_layout: &Layout) {
        let count = self.windows.len();
        let mut mask = vec![false; count];
        if let Some(cursor) = self.scene.cursor {
            if let Some(index) = self
                .windows
                .iter()
                .position(|window| window.source.monitor.bounds.contains(cursor))
            {
                mask[index] = true;
            }
        }
        let selection = self.scene.live_rect();
        if selection != self.previous_selection
            || (selection.is_none() && self.scene.full_screen_crosshair)
        {
            mask.fill(true);
        }
        if old_layout != &self.layout {
            for index in [
                old_layout.toolbar.as_ref().map(|item| item.monitor),
                old_layout.hint.as_ref().map(|item| item.monitor),
                old_layout.colour.as_ref().map(|item| item.monitor),
                old_layout.keyboard.as_ref().map(|item| item.monitor),
            ]
            .into_iter()
            .flatten()
            {
                if index < count {
                    mask[index] = true;
                }
            }
        }
        if let Some(toolbar) = &self.layout.toolbar {
            mask[toolbar.monitor] = true;
        }
        if let Some(hint) = &self.layout.hint {
            mask[hint.monitor] = true;
        }
        if let Some(colour) = &self.layout.colour {
            mask[colour.monitor] = true;
        }
        if let Some(keyboard) = &self.layout.keyboard {
            mask[keyboard.monitor] = true;
        }
        if self.scene.pointer_monitor < count {
            mask[self.scene.pointer_monitor] = true;
        }
        if self.previous_pointer_monitor < count {
            mask[self.previous_pointer_monitor] = true;
        }
        if !mask.iter().any(|value| *value) {
            mask.fill(true);
        }
        for (index, draw) in mask.into_iter().enumerate() {
            if draw {
                self.redraw_window(index);
            }
        }
        self.previous_selection = selection;
    }

    fn refresh(&mut self) {
        self.sync_document();
        if self
            .pill
            .as_ref()
            .is_some_and(|pill| !pill.visible(Instant::now()))
        {
            self.pill = None;
            self.clear_pill_timer();
        }
        let old_layout = self.layout.clone();
        self.layout = make_layout(
            &self.windows,
            &self.scene,
            self.pill.as_ref(),
            &mut self.gpu,
        );
        if self.layout.toolbar.is_none() {
            self.toolbar_focus = None;
        }
        if self.scene.shown_selection().is_none() {
            self.announced_selection = None;
        }
        self.redraw_active(&old_layout);
        self.previous_pointer_monitor = self.scene.pointer_monitor;
    }

    fn track_leave(&mut self, hwnd: HWND) {
        if self.tracking_mouse {
            return;
        }
        let mut event = TRACKMOUSEEVENT {
            cbSize: u32::try_from(std::mem::size_of::<TRACKMOUSEEVENT>()).unwrap_or(0),
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        // SAFETY: the event structure is live for the duration of the call and the window belongs to this thread.
        if unsafe { TrackMouseEvent(&mut event) }.is_ok() {
            self.tracking_mouse = true;
        }
    }

    fn query_control(&mut self, hwnd: HWND, point: Point) {
        if self.snap_worker.is_none() {
            return;
        }
        if !matches!(self.scene.overlay.state, State::Hovering { .. }) {
            return;
        }
        self.snap_generation = self.snap_generation.wrapping_add(1);
        let generation = self.snap_generation;
        let sent = self
            .snap_worker
            .as_ref()
            .is_some_and(|worker| worker.query(generation, point));
        if !sent {
            return;
        }
        self.snap_pending = Some(generation);
        if self.snap_timer != Some(hwnd) {
            self.cancel_snap_timer();
        }
        // SAFETY: arms a one-shot deadline timer on a live window owned by this thread.
        let timer = unsafe { SetTimer(Some(hwnd), SNAP_TIMER_ID, SNAP_DEADLINE_MS, None) };
        if timer == 0 {
            self.snap_pending = None;
        } else {
            self.snap_timer = Some(hwnd);
        }
    }

    fn cancel_snap_timer(&mut self) {
        let Some(hwnd) = self.snap_timer.take() else {
            return;
        };
        // SAFETY: cancels a timer on a live window owned by this thread.
        let _ = unsafe { KillTimer(Some(hwnd), SNAP_TIMER_ID) };
    }

    fn poll_control(&mut self) -> bool {
        let Some(worker) = &self.snap_worker else {
            return false;
        };
        let mut reply = None;
        while let Some(next) = worker.poll() {
            if next.generation() == self.snap_generation {
                reply = Some(next);
            }
        }
        let Some(reply) = reply else {
            return false;
        };
        let generation = self.snap_generation;
        let Some(rect) = reply.rect_for(generation) else {
            return false;
        };
        self.snap_pending = None;
        self.cancel_snap_timer();
        if !self.snap.set_control(rect) {
            return false;
        }
        self.apply_snap();
        true
    }

    fn clear_snap(&mut self) {
        self.snap_pending = None;
        self.snap_generation = self.snap_generation.wrapping_add(1);
        self.cancel_snap_timer();
        self.scene.cursor = None;
        self.scene.overlay.pointer = None;
        self.snap.clear_control();
        self.apply_snap();
    }

    fn finish(&mut self, action: Option<OverlayAction>, selection: Selection) {
        self.flattened = self.flatten(selection.rect);
        let monitor = if self.scene.editing {
            None
        } else {
            self.monitor_id(selection.rect.center())
        };
        if self.scene.record {
            let id = monitor.unwrap_or_else(|| self.pointer_monitor_id());
            self.record_target = Some(resolve_record_target(selection.rect, id, None));
        }
        self.outcome = Some(Outcome::Finish {
            action,
            selection: selection.rect,
            monitor,
        });
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
    }

    fn finish_record_window(&mut self, point: Point) -> bool {
        let Some((rect, window, start)) = self.record_press.take() else {
            return false;
        };
        if !is_click(start, point) || !rect.contains(point) {
            return false;
        }
        let monitor = self
            .monitor_id(point)
            .unwrap_or_else(|| self.pointer_monitor_id());
        self.record_target = Some(resolve_record_target(rect, monitor, Some(window)));
        self.outcome = Some(Outcome::Finish {
            action: None,
            selection: rect,
            monitor: Some(monitor),
        });
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
        true
    }

    fn record_window_at(&self, point: Point) -> Option<(Rect, RecordWindow)> {
        self.record_entries
            .iter()
            .find(|entry| entry.rect.contains(point))
            .map(|entry| {
                (
                    entry.rect,
                    RecordWindow::new(
                        entry.hwnd.0 as isize,
                        entry.title.clone().unwrap_or_default(),
                    ),
                )
            })
    }

    fn pointer_monitor_id(&self) -> u32 {
        self.windows
            .get(self.scene.pointer_monitor)
            .map_or(0, |window| window.source.monitor.id)
    }

    fn cancel(&mut self) {
        self.outcome = Some(Outcome::Cancel);
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
    }

    pub(crate) fn monitor_id(&self, point: Point) -> Option<u32> {
        self.windows
            .iter()
            .find(|window| window.source.monitor.bounds.contains(point))
            .map(|window| window.source.monitor.id)
    }

    fn monitor_index(&self, point: Point) -> usize {
        self.windows
            .iter()
            .position(|window| window.source.monitor.bounds.contains(point))
            .unwrap_or(self.scene.pointer_monitor)
    }

    fn handle_effects(&mut self, effects: Vec<Effect>) -> bool {
        let mut dirty = false;
        for effect in effects {
            match effect {
                Effect::Redraw(_) => dirty = true,
                Effect::Finish(selection) => {
                    let action = self.scene.pending_action.take();
                    self.finish(action, selection);
                }
                Effect::Cancel => self.cancel(),
                Effect::CopyColour(_) => dirty = true,
            }
        }
        dirty
    }

    fn apply(&mut self, input: Input) -> bool {
        let overlay = std::mem::take(&mut self.scene.overlay);
        let (overlay, effects) = overlay.on(input);
        self.scene.overlay = overlay;
        let dirty = self.handle_effects(effects);
        if !self.scene.editing
            && !matches!(
                self.scene.overlay.state,
                State::Annotating { .. } | State::Adjusting { .. }
            )
        {
            self.clear_document();
            self.tools.reset();
            self.text_edit = None;
        }
        dirty || self.outcome.is_some()
    }

    fn pointer(&self, index: usize, lparam: LPARAM) -> Point {
        let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
        let origin = self.windows[index].source.origin;
        Point::new(Px(x + origin.x.get()), Px(y + origin.y.get()))
    }

    fn local_point(&self, index: usize, point: Point) -> Point {
        let origin = self.windows[index].source.origin;
        Point::new(
            Px(point.x.get() - origin.x.get()),
            Px(point.y.get() - origin.y.get()),
        )
    }

    fn focus_toolbar(&mut self, hwnd: HWND, delta: i32) -> bool {
        let Some(count) = self
            .layout
            .toolbar
            .as_ref()
            .map(|toolbar| toolbar.buttons.len())
        else {
            self.toolbar_focus = None;
            return false;
        };
        if count == 0 {
            self.toolbar_focus = None;
            return false;
        }
        let next = match toolbar::next_focus(self.toolbar_focus, count, delta > 0) {
            Some(next) => next,
            None => {
                self.toolbar_focus = None;
                return false;
            }
        };
        let Some(action) = self
            .layout
            .toolbar
            .as_ref()
            .map(|toolbar| toolbar.buttons[next].action)
        else {
            return false;
        };
        let state = self.toolbar_state();
        self.toolbar_focus = Some(next);
        announce::notify(hwnd, &action.announcement(&state));
        self.refresh();
        true
    }

    fn activate_toolbar_focus(&mut self, hwnd: HWND) -> bool {
        let Some(index) = self.toolbar_focus else {
            return false;
        };
        let Some(action) = self
            .layout
            .toolbar
            .as_ref()
            .and_then(|toolbar| toolbar.buttons.get(index))
            .map(|button| button.action)
        else {
            self.toolbar_focus = None;
            return false;
        };
        announce::notify(hwnd, &format!("{} activated", action.label()));
        self.run_action(action);
        self.refresh();
        true
    }

    fn toolbar_state(&self) -> ToolbarState {
        ToolbarState {
            tool: self.scene.tool,
            colour: self.scene.colour,
            stroke: self.scene.stroke,
            can_undo: self
                .scene
                .document
                .as_ref()
                .is_some_and(|document| document.can_undo()),
            can_redo: self
                .scene
                .document
                .as_ref()
                .is_some_and(|document| document.can_redo()),
            focus: self.toolbar_focus,
        }
    }

    fn announce_selection(&mut self, hwnd: HWND) {
        let Some(selection) = self.scene.shown_selection() else {
            self.announced_selection = None;
            return;
        };
        if selection.width.get() <= 0 || selection.height.get() <= 0 {
            return;
        }
        if self.announced_selection == Some(selection) {
            return;
        }
        self.announced_selection = Some(selection);
        announce::notify(hwnd, &announce::selection_text(selection));
    }

    fn run_action(&mut self, action: ToolbarAction) {
        match action {
            ToolbarAction::Tool(tool) => self.set_tool(tool),
            ToolbarAction::Colour(colour) => self.scene.colour = colour,
            ToolbarAction::Stroke(stroke) => self.scene.stroke = stroke,
            ToolbarAction::Undo => {
                self.undo();
            }
            ToolbarAction::Redo => {
                self.redo();
            }
            ToolbarAction::Copy => {
                self.scene.pending_action = Some(OverlayAction::Copy);
                self.apply(Input::Upload);
            }
            ToolbarAction::Save => {
                self.scene.pending_action = Some(OverlayAction::Save);
                self.apply(Input::Upload);
            }
            ToolbarAction::Pin => {
                self.scene.pending_action = Some(OverlayAction::Pin);
                self.apply(Input::Upload);
            }
            ToolbarAction::Upload => {
                self.scene.pending_action = Some(OverlayAction::Upload);
                self.apply(Input::Upload);
            }
            ToolbarAction::Record => {
                self.scene.pending_action = Some(OverlayAction::Record);
                self.apply(Input::Upload);
            }
            ToolbarAction::Gif => {
                self.scene.pending_action = Some(OverlayAction::Gif);
                self.apply(Input::Upload);
            }
            ToolbarAction::Cancel => {
                self.apply(Input::Cancel);
            }
        }
    }

    fn on_toolbar(&mut self, point: Point) -> bool {
        let Some(toolbar) = &self.layout.toolbar else {
            return false;
        };
        let action = match toolbar.hit(point) {
            ToolbarHit::Outside => return false,
            ToolbarHit::Padding => return true,
            ToolbarHit::Action(action) => action,
        };
        self.toolbar_focus = None;
        self.run_action(action);
        true
    }

    fn on_key(&mut self, hwnd: HWND, key: VIRTUAL_KEY) {
        let annotating = matches!(
            &self.scene.overlay.state,
            State::Annotating { .. } | State::Adjusting { .. }
        );
        let control = key_down(VK_CONTROL);
        if control {
            match key {
                VK_Z if annotating => {
                    self.undo();
                }
                VK_Y if annotating => {
                    self.redo();
                }
                VK_C if annotating => {
                    self.scene.pending_action = Some(OverlayAction::Copy);
                    self.apply(Input::Copy);
                }
                VK_S if annotating => {
                    self.scene.pending_action = Some(OverlayAction::Save);
                    self.apply(Input::Save);
                }
                _ => {}
            }
            return;
        }
        let shift = key_down(VK_SHIFT);
        let alt = key_down(VK_MENU);
        match key {
            VK_ESCAPE => {
                if self.toolbar_focus.take().is_some() {
                    self.refresh();
                    return;
                }
                if self.text_edit.is_some() {
                    self.cancel_text();
                    return;
                }
                let cancelled = match self.scene.document.as_mut() {
                    Some(document) => self.tools.cancel(document),
                    None => false,
                };
                if cancelled {
                    return;
                }
                if self.pill.take().is_some() {
                    self.clear_pill_timer();
                    return;
                }
                // SAFETY: releases any capture held by this thread.
                unsafe {
                    let _ = ReleaseCapture();
                }
                self.apply(Input::Escape);
            }
            VK_DELETE if annotating => {
                if let Some(document) = self.scene.document.as_mut() {
                    let _ = self.tools.delete(document);
                }
            }
            VK_RETURN if annotating => {
                if self.activate_toolbar_focus(hwnd) {
                    return;
                }
                self.scene.pending_action = self.scene.editing.then_some(OverlayAction::Upload);
                self.apply(Input::Upload);
            }
            VK_SPACE if self.toolbar_focus.is_some() => {
                self.activate_toolbar_focus(hwnd);
            }
            VK_TAB => {
                if annotating && self.layout.toolbar.is_some() {
                    self.focus_toolbar(hwnd, if shift { -1 } else { 1 });
                } else {
                    self.apply(Input::Tab);
                }
            }
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN if !self.scene.editing => {
                let direction = match key {
                    VK_LEFT => snapline_core::geometry::Direction::Left,
                    VK_RIGHT => snapline_core::geometry::Direction::Right,
                    VK_UP => snapline_core::geometry::Direction::Up,
                    _ => snapline_core::geometry::Direction::Down,
                };
                self.apply(Input::Arrow {
                    direction,
                    shift,
                    alt,
                });
                self.announce_selection(hwnd);
            }
            VK_V => self.set_tool(Tool::Move),
            VK_R => self.set_tool(Tool::Rect),
            VK_E => self.set_tool(Tool::Ellipse),
            VK_A => self.set_tool(Tool::Arrow),
            VK_P => self.set_tool(Tool::Pen),
            VK_H => self.set_tool(Tool::Highlighter),
            VK_T => self.set_tool(Tool::Text),
            VK_N => self.set_tool(Tool::Step),
            VK_B => self.set_tool(Tool::Blur),
            VK_C => self.set_tool(Tool::Crop),
            VK_I => self.set_tool(Tool::Colour),
            _ => {}
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        if tool == Tool::Highlighter && self.scene.colour == AnnotationColour::Red {
            self.scene.colour = AnnotationColour::Yellow;
        }
        self.scene.tool = tool;
        self.apply(Input::SetTool(tool));
    }

    pub(crate) fn handle(
        &mut self,
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let Some(index) = self.index_of(hwnd) else {
            // SAFETY: the message is passed straight through for windows not owned by the context.
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        };
        if message != WM_DESTROY && self.poll_control() {
            self.refresh();
        }
        match message {
            text_input::CHANGE_MESSAGE => {
                self.refresh();
                return LRESULT(0);
            }
            text_input::COMMIT_MESSAGE => {
                self.commit_text();
                self.refresh();
                return LRESULT(0);
            }
            text_input::CANCEL_MESSAGE => {
                self.cancel_text();
                self.refresh();
                return LRESULT(0);
            }
            WM_CTLCOLOREDIT => {
                if let Some(edit) = &self.text_edit {
                    let hdc = HDC(wparam.0 as *mut std::ffi::c_void);
                    let brush = edit.paint(hdc);
                    return LRESULT(brush.0 as isize);
                }
            }
            WM_MOUSEACTIVATE => {
                let _ = index;
                return LRESULT(MA_ACTIVATE as isize);
            }
            WM_SETCURSOR => {
                if (lparam.0 & 0xffff) as i32 == HTCLIENT as i32 {
                    let cursor = if self.scene.shown_selection().is_some() {
                        IDC_ARROW
                    } else {
                        IDC_CROSS
                    };
                    // SAFETY: loads a system cursor and applies it to this thread.
                    unsafe {
                        if let Ok(handle) = LoadCursorW(None, cursor) {
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
            WM_TIMER if wparam.0 == SNAP_TIMER_ID => {
                self.snap_timer = None;
                // SAFETY: cancels the deadline timer on a live window owned by this thread.
                let _ = unsafe { KillTimer(Some(hwnd), SNAP_TIMER_ID) };
                if self.snap_pending.take().is_some() {
                    self.snap_generation = self.snap_generation.wrapping_add(1);
                    if self.snap.clear_control() {
                        self.apply_snap();
                    }
                    self.refresh();
                }
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == PILL_TIMER_ID => {
                self.pill = None;
                self.pill_timer = None;
                // SAFETY: cancels the one-shot pill timer on a live window owned by this thread.
                let _ = unsafe { KillTimer(Some(hwnd), PILL_TIMER_ID) };
                self.refresh();
                return LRESULT(0);
            }
            WM_MOUSELEAVE => {
                self.tracking_mouse = false;
                self.clear_snap();
                self.refresh();
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                self.track_leave(hwnd);
                let point = self.pointer(index, lparam);
                self.scene.cursor = Some(point);
                self.scene.pointer_monitor = self.monitor_index(point);
                if self.annotating() {
                    if self.tools.is_active() {
                        let local = self.document_point(point);
                        let shift = key_down(VK_SHIFT);
                        if let Some(document) = self.scene.document.as_mut() {
                            self.tools.move_to(local, document, shift);
                        }
                    } else if matches!(self.scene.overlay.state, State::Adjusting { .. }) {
                        self.apply(Input::Move(point));
                    }
                } else if !self.scene.editing {
                    self.apply(Input::Move(point));
                    self.query_control(hwnd, point);
                }
                self.refresh();
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                let point = self.pointer(index, lparam);
                self.scene.cursor = Some(point);
                self.scene.pointer_monitor = self.monitor_index(point);
                if self.scene.record {
                    self.record_press = self
                        .record_window_at(point)
                        .map(|(rect, window)| (rect, window, point));
                }
                if self
                    .text_edit
                    .as_ref()
                    .is_some_and(|edit| edit.contains(self.local_point(index, point)))
                {
                    return LRESULT(0);
                }
                if self.text_edit.is_some() {
                    self.commit_text();
                }
                if self.on_toolbar(point) {
                    self.refresh();
                    return LRESULT(0);
                }
                self.toolbar_focus = None;
                if self.annotating() {
                    let tool = self.scene.tool;
                    let on_selection = tool != Tool::Crop
                        && self
                            .scene
                            .overlay
                            .selected_rect()
                            .is_some_and(|rect| rect.grip_at(point).is_some());
                    if on_selection {
                        // SAFETY: the window handle is live and owned by this thread.
                        unsafe {
                            SetCapture(hwnd);
                        }
                        self.apply(Input::Press(point));
                    } else {
                        let result = if tool == Tool::Crop && !self.scene.editing {
                            let local = self.document_point(point);
                            if let (Some(document), Some(selection)) = (
                                self.scene.document.as_ref(),
                                self.scene.overlay.selected_rect(),
                            ) {
                                let selection =
                                    local_rect(selection, self.scene.document_rect.origin());
                                if let Some(bounds) = selection.intersection(document.crop()) {
                                    self.tools.press_crop(local, bounds);
                                }
                            }
                            tools::Press::Handled
                        } else if tool == Tool::Colour {
                            let local = self.local_point(index, point);
                            match tools::sample_image(&self.windows[index].source.image, local) {
                                Some(rgb) => tools::Press::Copied(rgb),
                                None => tools::Press::Handled,
                            }
                        } else {
                            let local = self.document_point(point);
                            let style = tools::style(self.scene.colour, self.scene.stroke);
                            match self.scene.document.as_mut() {
                                Some(document) => self.tools.press(tool, local, document, style),
                                None => tools::Press::Handled,
                            }
                        };
                        // SAFETY: the window handle is live and owned by this thread.
                        unsafe {
                            SetCapture(hwnd);
                        }
                        match result {
                            tools::Press::Text(origin) => self.begin_text(origin),
                            tools::Press::Copied(rgb) => self.copy_colour(rgb),
                            tools::Press::Handled => {}
                        }
                    }
                } else if !self.scene.editing {
                    // SAFETY: the window handle is live and owned by this thread.
                    unsafe {
                        SetCapture(hwnd);
                    }
                    self.apply(Input::Press(point));
                }
                self.refresh();
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let point = self.pointer(index, lparam);
                self.scene.cursor = Some(point);
                // SAFETY: releases a capture held by this thread.
                unsafe {
                    let _ = ReleaseCapture();
                }
                if self.scene.record && self.finish_record_window(point) {
                    self.refresh();
                    return LRESULT(0);
                }
                if self.annotating() {
                    let local = self.document_point(point);
                    let shift = key_down(VK_SHIFT);
                    if self.tools.is_active() {
                        if let Some(document) = self.scene.document.as_mut() {
                            self.tools.release(local, document, shift);
                        }
                    } else if matches!(self.scene.overlay.state, State::Adjusting { .. }) {
                        self.apply(Input::Release(point));
                    }
                } else if !self.scene.editing {
                    self.apply(Input::Release(point));
                }
                self.announce_selection(hwnd);
                self.refresh();
                return LRESULT(0);
            }
            WM_RBUTTONDOWN => {
                let point = self.pointer(index, lparam);
                self.scene.cursor = Some(point);
                // SAFETY: releases a capture held by this thread.
                unsafe {
                    let _ = ReleaseCapture();
                }
                if self.text_edit.is_some() {
                    self.commit_text();
                }
                if self.tools.is_active() {
                    if let Some(document) = self.scene.document.as_mut() {
                        let _ = self.tools.cancel(document);
                    }
                }
                self.apply(Input::RightClick);
                self.refresh();
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                let key = VIRTUAL_KEY(wparam.0 as u16);
                self.on_key(hwnd, key);
                if self.outcome.is_none() {
                    self.refresh();
                }
                return LRESULT(0);
            }
            WM_SYSKEYDOWN => {
                let key = VIRTUAL_KEY(wparam.0 as u16);
                if matches!(key, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) {
                    self.on_key(hwnd, key);
                    if self.outcome.is_none() {
                        self.refresh();
                    }
                    return LRESULT(0);
                }
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

    pub(crate) fn destroy(&mut self) {
        self.clear_pill_timer();
        self.text_edit = None;
        for window in self.windows.drain(..) {
            crate::destroy_window(window.hwnd);
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

    pub(crate) fn run_message_loop() {
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

fn edit_origin(size: Size) -> Point {
    let mut cursor = POINT::default();
    // SAFETY: the point output pointer is valid for the duration of the call.
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return Point::ORIGIN;
    }
    let mut info = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: the point came from GetCursorPos and the nearest monitor exists on a desktop.
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    // SAFETY: the info structure is live with a valid size and the handle came from MonitorFromPoint.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Point::ORIGIN;
    }
    let width = info.rcWork.right - info.rcWork.left;
    let height = info.rcWork.bottom - info.rcWork.top;
    let x = info.rcWork.left + (width - size.width.get()).max(0) / 2;
    let y = info.rcWork.top + (height - size.height.get()).max(0) / 2;
    Point::new(Px(x), Px(y))
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: reads the calling thread's key state.
    unsafe { GetKeyState(key.0 as i32) < 0 }
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

fn core_colour(colour: AnnotationColour) -> snapline_core::annotate::Colour {
    tools::colour(colour)
}

fn local_rect(area: Rect, origin: Point) -> Rect {
    area.translated(Point::new(Px(-origin.x.get()), Px(-origin.y.get())))
}

fn make_layout(
    windows: &[WindowState],
    scene: &Scene,
    pill: Option<&tools::ColourPill>,
    gpu: &mut Gpu,
) -> Layout {
    if scene.record {
        return record_mode_layout();
    }
    if windows.get(scene.pointer_monitor).is_none() {
        return Layout::default();
    }
    let monitor_index = scene
        .live_rect()
        .and_then(|selection| {
            windows
                .iter()
                .position(|candidate| candidate.source.monitor.bounds.contains(selection.center()))
        })
        .unwrap_or(scene.pointer_monitor);
    let monitor = &windows[monitor_index].source;
    let scale = monitor.scale;
    let show_toolbar = scene.annotate;
    let mut toolbar = None;
    let mut hint = None;
    if show_toolbar {
        let upload_width = readout::measure_upload_label(gpu, scale);
        toolbar = Some(toolbar::layout(
            monitor_index,
            monitor.monitor.bounds,
            scale,
            upload_width,
            scene.editing,
            scene.upload_on_finish,
        ));
        let next_step = scene
            .document
            .as_ref()
            .map_or(1, |document| document.next_step());
        let text = hints::hint_text(scene.tool, next_step);
        let width = hints::hint_width(gpu, scale, &text);
        hint = Some(hints::HintPill {
            monitor: monitor_index,
            rect: hints::hint_pill(monitor.monitor.bounds, scale, width),
            text,
        });
    }
    let width = hints::keyboard_width(gpu, scale, show_toolbar);
    let colour = pill.map(|pill| {
        let width = hints::hint_width(gpu, scale, pill.text());
        let rect = hints::hint_pill(monitor.monitor.bounds, scale, width)
            .translated(Point::new(Px(0), Px((40.0 * scale).round() as i32)));
        hints::HintPill {
            monitor: monitor_index,
            rect,
            text: pill.text().to_string(),
        }
    });
    Layout {
        toolbar,
        hint,
        colour,
        keyboard: Some(hints::KeyboardPill {
            monitor: monitor_index,
            rect: hints::keyboard_pill(monitor.monitor.bounds, scale, width),
            annotating: show_toolbar,
        }),
    }
}

fn document_scale_at(monitors: impl Iterator<Item = (Rect, f32)>, point: Point) -> f32 {
    let mut monitors = monitors;
    monitors
        .find(|(bounds, _)| bounds.contains(point))
        .map_or(1.0, |(_, scale)| scale)
}

fn selection_in_document(crop: Rect, document_origin: Point, selection: Rect) -> Option<Rect> {
    local_rect(selection, document_origin).intersection(crop)
}

fn document_image(sources: &[&ScreenSource], bounds: Rect) -> Option<std::sync::Arc<Image>> {
    if let [only] = sources {
        let exact = only.monitor.bounds == bounds
            && only.origin == bounds.origin()
            && i64::from(only.image.width()) == i64::from(bounds.width.0)
            && i64::from(only.image.height()) == i64::from(bounds.height.0);
        if exact {
            return Some(std::sync::Arc::clone(&only.image));
        }
    }
    compose_region(sources.iter().copied(), bounds).map(std::sync::Arc::new)
}

fn compose_region<'a>(
    sources: impl Iterator<Item = &'a ScreenSource>,
    bounds: Rect,
) -> Option<Image> {
    let mut image = Image::bgra(
        u32::try_from(bounds.width.0).ok()?,
        u32::try_from(bounds.height.0).ok()?,
    )
    .ok()?;
    for pixel in image.pixels_mut().chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    for source in sources {
        let Some(overlap) = bounds.intersection(source.monitor.bounds) else {
            continue;
        };
        let x = usize::try_from(overlap.x.0 - bounds.x.0).ok()? * 4;
        let source_x = usize::try_from(overlap.x.0 - source.origin.x.0).ok()? * 4;
        let width = usize::try_from(overlap.width.0).ok()? * 4;
        for row in 0..u32::try_from(overlap.height.0).ok()? {
            let from_y = u32::try_from(overlap.y.0 - source.origin.y.0).ok()? + row;
            let to_y = u32::try_from(overlap.y.0 - bounds.y.0).ok()? + row;
            let from = source.image.row(from_y)?.get(source_x..source_x + width)?;
            image
                .row_mut(to_y)?
                .get_mut(x..x + width)?
                .copy_from_slice(from);
        }
    }
    Some(image)
}

fn draw_window(
    gpu: &mut Gpu,
    icons: &mut IconCache,
    window: &WindowState,
    index: usize,
    scene: &Scene,
    layout: &Layout,
    tools: &Session,
    stroke: Option<&ID2D1StrokeStyle>,
    focus: Option<usize>,
    text_preview: Option<&snapline_core::annotate::Shape>,
) {
    let palette = scene.theme.palette();
    let origin = window.source.origin;
    let width = window.source.size.width.get() as f32;
    let height = window.source.size.height.get() as f32;
    let scale = window.source.scale;
    let context = gpu.context.clone();
    let context = &context;

    // SAFETY: the swap chain target and bitmap belong to this render pass.
    unsafe {
        context.SetTarget(&window.target);
    }
    gpu.begin();
    let black = d2d_colour([0.0, 0.0, 0.0, 1.0]);
    // SAFETY: clears the whole render target before any drawing.
    unsafe {
        let _ = context.Clear(Some(&black));
    }
    d2d_bitmap(
        context,
        &window.frozen,
        0.0,
        0.0,
        width,
        height,
        window.source.image.width() as f32,
        window.source.image.height() as f32,
    );

    let selection = scene.live_rect();

    if let Some(selection) = selection {
        let local_selection = local_rect(selection, origin);
        draw_dim(context, palette, local_selection, width, height);
        if let Some(document) = &scene.document {
            let document_origin = local_rect(scene.document_rect, origin).origin();
            let clip = d2d_rect(
                local_selection.x.get() as f32,
                local_selection.y.get() as f32,
                local_selection.width.get() as f32,
                local_selection.height.get() as f32,
            );
            // SAFETY: the clip is a plain value and is popped below.
            unsafe {
                let _ = context.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
            }
            canvas_d2d::draw_document(gpu, document, document_origin, scene.document_scale, stroke);
            if let Some(shape) = text_preview {
                let mut canvas = canvas_d2d::D2dCanvas::new(
                    context,
                    &gpu.write,
                    document_origin,
                    scene.document_scale,
                    stroke,
                );
                canvas.draw_shape(shape, document.image());
            }
            if let Some(shape) = tools.preview() {
                let mut canvas = canvas_d2d::D2dCanvas::new(
                    context,
                    &gpu.write,
                    document_origin,
                    scene.document_scale,
                    stroke,
                );
                canvas.draw_shape(&shape, document.image());
            }
            if let Some(crop) = tools.crop_preview() {
                let area = local_rect(crop.translated(scene.document_rect.origin()), origin);
                d2d_stroke(
                    context,
                    area.x.get() as f32 + 0.5,
                    area.y.get() as f32 + 0.5,
                    (area.width.get() as f32 - 1.0).max(0.0),
                    (area.height.get() as f32 - 1.0).max(0.0),
                    scale.max(1.0) * 2.0,
                    palette.accent,
                );
            }
            if let Some(id) = tools.selected() {
                if let Some(annotation) = document.annotation(id) {
                    let bounds = annotation
                        .shape
                        .bounds()
                        .translated(scene.document_rect.origin());
                    let local_bounds = local_rect(bounds, origin);
                    draw_handles(context, palette, local_bounds, scale);
                }
            }
            // SAFETY: balances the clip pushed above.
            unsafe {
                let _ = context.PopAxisAlignedClip();
            }
        }
        draw_selection(context, palette, local_selection, scale);
        if scene.shown_selection().is_some() {
            draw_handles(context, palette, local_selection, scale);
        }
        let pill_local = readout::size_pill_rect(gpu, scale, local_selection);
        if overlap(
            pill_local,
            Rect::new(Px(0), Px(0), Px(width as i32), Px(height as i32)),
        )
        .is_some()
        {
            readout::draw_size_pill(gpu, palette, scale, pill_local, selection);
        }
    } else {
        if let Some(hovered) = scene.overlay.snap.hovered {
            let local = local_rect(hovered, origin);
            if let Some(visible) = overlap(
                local,
                Rect::new(Px(0), Px(0), Px(width as i32), Px(height as i32)),
            ) {
                d2d_stroke(
                    context,
                    visible.x.get() as f32 + 0.5,
                    visible.y.get() as f32 + 0.5,
                    (visible.width.get() as f32 - 1.0).max(0.0),
                    (visible.height.get() as f32 - 1.0).max(0.0),
                    scale.max(1.0) * 2.0,
                    palette.accent,
                );
            }
        }
        if let Some(cursor) = scene.cursor {
            let cursor_local = Point::new(
                Px(cursor.x.get() - origin.x.get()),
                Px(cursor.y.get() - origin.y.get()),
            );
            if scene.full_screen_crosshair {
                if (0..width as i32).contains(&cursor_local.x.get()) {
                    d2d_fill(
                        context,
                        cursor_local.x.get() as f32,
                        0.0,
                        1.0,
                        height,
                        palette.accent,
                    );
                }
                if (0..height as i32).contains(&cursor_local.y.get()) {
                    d2d_fill(
                        context,
                        0.0,
                        cursor_local.y.get() as f32,
                        width,
                        1.0,
                        palette.accent,
                    );
                }
            }
            if scene.magnifier.enabled && !scene.editing {
                magnifier::draw(
                    gpu,
                    window,
                    palette,
                    cursor_local,
                    scene.magnifier.zoom.factor(),
                );
            }
        }
    }

    if let Some(toolbar) = &layout.toolbar {
        if toolbar.monitor == index {
            let state = ToolbarState {
                tool: scene.tool,
                colour: scene.colour,
                stroke: scene.stroke,
                can_undo: scene
                    .document
                    .as_ref()
                    .is_some_and(|document| document.can_undo()),
                can_redo: scene
                    .document
                    .as_ref()
                    .is_some_and(|document| document.can_redo()),
                focus,
            };
            toolbar::draw(gpu, icons, palette, scale, origin, toolbar, &state);
        }
    }
    if let Some(hint) = &layout.hint {
        if hint.monitor == index {
            let area = local_rect(hint.rect, origin);
            hints::draw_hint(gpu, palette, scale, area, &hint.text);
        }
    }
    if let Some(colour) = &layout.colour {
        if colour.monitor == index {
            let area = local_rect(colour.rect, origin);
            hints::draw_hint(gpu, palette, scale, area, &colour.text);
        }
    }
    if let Some(keyboard) = &layout.keyboard {
        if keyboard.monitor == index {
            let area = local_rect(keyboard.rect, origin);
            hints::draw_keyboard(gpu, palette, scale, area, keyboard.annotating);
        }
    }

    gpu.end();
    // SAFETY: the render pass is finished and the buffer can be presented.
    unsafe {
        let _ = context.SetTarget(None::<&ID2D1Image>);
    }
    // SAFETY: the swap chain belongs to the live window.
    unsafe {
        let _ = window.swap_chain.Present(0, DXGI_PRESENT(0));
    }
}

fn draw_dim(
    context: &ID2D1DeviceContext5,
    palette: &Palette,
    selection: Rect,
    width: f32,
    height: f32,
) {
    let left = selection.x.get() as f32;
    let top = selection.y.get() as f32;
    let right = selection.right().get() as f32;
    let bottom = selection.bottom().get() as f32;
    if top > 0.0 {
        d2d_fill(context, 0.0, 0.0, width, top.min(height), palette.dim);
    }
    if bottom < height {
        d2d_fill(
            context,
            0.0,
            bottom.max(0.0),
            width,
            height - bottom.max(0.0),
            palette.dim,
        );
    }
    let band_top = top.max(0.0);
    let band_height = (bottom.min(height) - band_top).max(0.0);
    if left > 0.0 {
        d2d_fill(
            context,
            0.0,
            band_top,
            left.min(width),
            band_height,
            palette.dim,
        );
    }
    if right < width {
        d2d_fill(
            context,
            right.max(0.0),
            band_top,
            width - right.max(0.0),
            band_height,
            palette.dim,
        );
    }
}

fn draw_selection(context: &ID2D1DeviceContext5, palette: &Palette, selection: Rect, scale: f32) {
    d2d_stroke(
        context,
        selection.x.get() as f32 + 0.5,
        selection.y.get() as f32 + 0.5,
        (selection.width.get() as f32 - 1.0).max(0.0),
        (selection.height.get() as f32 - 1.0).max(0.0),
        scale.max(1.0),
        palette.accent,
    );
}

fn draw_handles(context: &ID2D1DeviceContext5, palette: &Palette, selection: Rect, scale: f32) {
    let size = (9.0 * scale).max(9.0);
    let border = scale.max(1.0);
    for handle in selection.handles() {
        let left = handle.x.get() as f32 - size / 2.0;
        let top = handle.y.get() as f32 - size / 2.0;
        d2d_fill(context, left, top, size, size, palette.white);
        d2d_stroke(
            context,
            left + border / 2.0,
            top + border / 2.0,
            size - border,
            size - border,
            border,
            palette.accent,
        );
    }
}

fn d2d_colour(rgba: [f32; 4]) -> windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
    crate::d2d::colour(rgba)
}

fn d2d_rect(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
    crate::d2d::rect(x, y, width, height)
}

fn d2d_fill(
    context: &ID2D1DeviceContext5,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    rgba: [f32; 4],
) {
    crate::d2d::fill_rect(context, d2d_rect(x, y, width, height), rgba);
}

fn d2d_stroke(
    context: &ID2D1DeviceContext5,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    stroke: f32,
    rgba: [f32; 4],
) {
    crate::d2d::stroke_rect(context, d2d_rect(x, y, width, height), stroke, rgba);
}

fn d2d_bitmap(
    context: &ID2D1DeviceContext5,
    bitmap: &ID2D1Bitmap1,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    source_width: f32,
    source_height: f32,
) {
    crate::d2d::draw_bitmap(
        context,
        bitmap,
        d2d_rect(x, y, width, height),
        d2d_rect(0.0, 0.0, source_width, source_height),
        false,
    );
}

fn overlap(area: Rect, bounds: Rect) -> Option<Rect> {
    area.intersection(bounds)
}

pub(crate) fn record_mode_layout() -> Layout {
    Layout::default()
}

#[cfg(test)]
mod tests {
    use snapline_core::overlay::{Input, Overlay, SnapMode};

    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    #[test]
    fn document_scale_follows_the_monitor_under_the_selection() {
        let left = rect(0, 0, 1920, 1080);
        let right = rect(1920, 0, 2880, 1620);
        let monitors = || [(left, 1.0_f32), (right, 1.5_f32)].into_iter();
        assert_eq!(
            document_scale_at(monitors(), Point::new(Px(100), Px(100))),
            1.0
        );
        assert_eq!(
            document_scale_at(monitors(), Point::new(Px(2500), Px(300))),
            1.5
        );
        assert_eq!(
            document_scale_at(monitors(), Point::new(Px(9000), Px(9000))),
            1.0
        );
        assert_eq!(left.union(right), rect(0, 0, 4800, 1620));
    }

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    fn source(bounds: Rect, colour: u8) -> ScreenSource {
        let mut image = Image::bgra(bounds.width.0 as u32, bounds.height.0 as u32).expect("image");
        for pixel in image.pixels_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[colour, 0, 0, 255]);
        }
        ScreenSource {
            monitor: Monitor {
                id: colour as u32,
                name: String::new(),
                bounds,
                work_area: bounds,
                dpi: snapline_core::geometry::DpiScale::ONE,
                primary: false,
            },
            scale: 1.0,
            origin: bounds.origin(),
            size: bounds.size(),
            image: std::sync::Arc::new(image),
        }
    }

    #[test]
    fn selections_compose_pixels_across_monitors_and_negative_origins() {
        let sources = [source(rect(-2, 0, 2, 2), 10), source(rect(0, 0, 2, 2), 20)];
        let image = compose_region(sources.iter(), rect(-1, 0, 2, 2)).expect("image");
        assert_eq!(image.pixel(0, 0).expect("pixel"), [10, 0, 0, 255]);
        assert_eq!(image.pixel(1, 0).expect("pixel"), [20, 0, 0, 255]);
    }

    #[test]
    fn a_single_monitor_selection_shares_the_frozen_image() {
        let only = source(rect(0, 0, 4, 2), 10);
        let image = document_image(&[&only], rect(0, 0, 4, 2)).expect("image");
        assert!(std::sync::Arc::ptr_eq(&image, &only.image));
    }

    #[test]
    fn a_multi_monitor_selection_is_composed_into_a_new_image() {
        let left = source(rect(-2, 0, 2, 2), 10);
        let right = source(rect(0, 0, 2, 2), 20);
        let image = document_image(&[&left, &right], rect(-2, 0, 4, 2)).expect("image");
        assert!(!std::sync::Arc::ptr_eq(&image, &left.image));
        assert!(!std::sync::Arc::ptr_eq(&image, &right.image));
        assert_eq!((image.width(), image.height()), (4, 2));
        assert_eq!(image.pixel(0, 0).expect("pixel"), [10, 0, 0, 255]);
        assert_eq!(image.pixel(3, 0).expect("pixel"), [20, 0, 0, 255]);
    }

    #[test]
    fn finishing_uses_the_adjusted_selection() {
        let sources = [source(rect(-2, 0, 2, 2), 10), source(rect(0, 0, 2, 2), 20)];
        let image = compose_region(sources.iter(), rect(-2, 0, 4, 2)).expect("image");
        let selection = selection_in_document(rect(0, 0, 4, 2), point(-2, 0), rect(-1, 0, 3, 1))
            .expect("selection");
        let adjusted = crop_image(&image, selection).expect("crop");
        assert_eq!((adjusted.width(), adjusted.height()), (3, 1));
        assert_eq!(adjusted.pixel(0, 0).expect("pixel"), [10, 0, 0, 255]);
        assert_eq!(adjusted.pixel(2, 0).expect("pixel"), [20, 0, 0, 255]);
    }

    #[test]
    fn finishing_keeps_the_selection_after_overlay_returns_to_hovering() {
        let selection = rect(20, 30, 40, 50);
        let mut overlay = Overlay::with_annotate(true);
        overlay.state = State::Annotating {
            rect: selection,
            tool: Tool::Pen,
        };
        let (overlay, effects) = overlay.on(Input::Upload);
        assert!(overlay.selected_rect().is_none());
        let finished = effects
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Finish(selection) => Some(selection.rect),
                _ => None,
            })
            .expect("finished selection");
        assert_eq!(
            selection_in_document(rect(0, 0, 200, 200), Point::ORIGIN, finished),
            Some(selection)
        );
    }

    #[test]
    fn snap_feed_maps_windows_and_the_control() {
        let mut feed = SnapFeed::new(SnapSet::new(
            vec![rect(0, 0, 500, 500)],
            vec![rect(10, 10, 20, 20)],
        ));
        assert_eq!(
            feed.set().windows,
            vec![SnapCandidate::new(rect(0, 0, 500, 500))]
        );
        assert_eq!(
            feed.set().controls,
            vec![SnapCandidate::new(rect(10, 10, 20, 20))]
        );
        assert!(feed.set_control(Some(rect(100, 100, 50, 50))));
        let set = feed.set();
        assert_eq!(set.windows, vec![SnapCandidate::new(rect(0, 0, 500, 500))]);
        assert_eq!(
            set.controls,
            vec![SnapCandidate::new(rect(100, 100, 50, 50))]
        );
    }

    #[test]
    fn snap_feed_reports_control_changes_only_once() {
        let mut feed = SnapFeed::default();
        assert!(feed.set_control(Some(rect(1, 2, 3, 4))));
        assert!(!feed.set_control(Some(rect(1, 2, 3, 4))));
        assert!(feed.set_control(None));
        assert!(!feed.clear_control());
        assert!(feed.set().controls.is_empty());
    }

    #[test]
    fn snap_feed_replaces_the_window_list_without_touching_the_control() {
        let mut feed = SnapFeed::new(SnapSet::new(vec![rect(0, 0, 10, 10)], vec![]));
        feed.set_control(Some(rect(20, 20, 5, 5)));
        feed.set_windows(vec![
            SnapCandidate::new(rect(30, 30, 40, 40)),
            SnapCandidate::new(rect(50, 50, 60, 60)),
        ]);
        let set = feed.set();
        assert_eq!(
            set.windows,
            vec![
                SnapCandidate::new(rect(30, 30, 40, 40)),
                SnapCandidate::new(rect(50, 50, 60, 60)),
            ]
        );
        assert_eq!(set.controls, vec![SnapCandidate::new(rect(20, 20, 5, 5))]);
    }

    #[test]
    fn tab_switches_between_window_and_control_snapping() {
        let mut feed = SnapFeed::new(SnapSet::new(
            vec![rect(0, 0, 500, 500)],
            vec![rect(100, 100, 200, 200)],
        ));
        feed.set_control(Some(rect(100, 100, 200, 200)));
        let (mut overlay, _) = Overlay::new().on(Input::SnapCandidates(feed.set()));
        let (next, _) = overlay.on(Input::Move(point(150, 150)));
        overlay = next;
        assert_eq!(overlay.snap.mode, SnapMode::Window);
        assert_eq!(overlay.snap.hovered, Some(rect(0, 0, 500, 500)));
        let (next, _) = overlay.on(Input::Tab);
        overlay = next;
        assert_eq!(overlay.snap.mode, SnapMode::Control);
        assert_eq!(overlay.snap.hovered, Some(rect(100, 100, 200, 200)));
        let (next, _) = overlay.on(Input::Tab);
        overlay = next;
        assert_eq!(overlay.snap.mode, SnapMode::Window);
        assert_eq!(overlay.snap.hovered, Some(rect(0, 0, 500, 500)));
    }

    #[test]
    fn clearing_the_snap_feed_removes_the_hovered_rectangle() {
        let mut feed = SnapFeed::new(SnapSet::new(vec![rect(0, 0, 500, 500)], vec![]));
        feed.set_control(Some(rect(100, 100, 50, 50)));
        let (mut overlay, _) = Overlay::new().on(Input::SnapCandidates(feed.set()));
        let (next, _) = overlay.on(Input::Tab);
        overlay = next;
        let (next, _) = overlay.on(Input::Move(point(120, 120)));
        overlay = next;
        assert_eq!(overlay.snap.hovered, Some(rect(100, 100, 50, 50)));
        feed.clear_control();
        overlay.pointer = None;
        let (next, _) = overlay.on(Input::SnapCandidates(feed.set()));
        overlay = next;
        assert_eq!(overlay.snap.hovered, None);
    }

    #[test]
    fn local_rect_removes_the_monitor_origin() {
        assert_eq!(
            local_rect(rect(1920, 10, 100, 50), point(1920, 0)),
            rect(0, 10, 100, 50)
        );
    }

    #[test]
    fn record_mode_has_no_toolbar_hints_or_keyboard_pills() {
        let layout = record_mode_layout();
        assert!(layout.toolbar.is_none());
        assert!(layout.hint.is_none());
        assert!(layout.colour.is_none());
        assert!(layout.keyboard.is_none());
    }

    #[test]
    fn document_origin_is_the_selection_corner() {
        let Ok(image) = Image::bgra(40, 20) else {
            return;
        };
        let document = Document::new(image);
        let scene_document_rect =
            Rect::from_origin_size(point(300, 200), Size::new(Px(40), Px(20)));
        assert_eq!(scene_document_rect.origin(), point(300, 200));
        assert_eq!(document.size(), Size::new(Px(40), Px(20)));
    }
}
