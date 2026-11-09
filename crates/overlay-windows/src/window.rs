use snapline_core::geometry::{Point, Px, Rect, Size};
use snapline_core::image::Image;
use snapline_core::overlay::{
    Effect, Input, Overlay, Selection, SnapCandidate, SnapSet, State, Tool,
};
use snapline_core::ports::Monitor;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;
use windows::Win32::Graphics::Gdi::{UpdateWindow, ValidateRect};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VIRTUAL_KEY, VK_A, VK_B, VK_C, VK_CONTROL, VK_DOWN, VK_E, VK_ESCAPE, VK_H, VK_I, VK_LEFT,
    VK_MENU, VK_N, VK_P, VK_R, VK_RETURN, VK_RIGHT, VK_S, VK_SHIFT, VK_T, VK_TAB, VK_UP, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, GetWindowLongPtrW, GetWindowThreadProcessId, KillTimer, LoadCursorW,
    PostQuitMessage, RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    ShowWindow, TranslateMessage, UnregisterClassW, GWLP_USERDATA, HTCLIENT, IDC_ARROW, IDC_CROSS,
    MA_ACTIVATE, MSG, SW_SHOW, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_RBUTTONDOWN,
    WM_SETCURSOR, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

#[allow(dead_code)]
#[path = "../../platform-windows/src/uia.rs"]
mod uia;
#[allow(dead_code)]
#[path = "../../platform-windows/src/windows_enum.rs"]
mod windows_enum;

use crate::d2d::Gpu;
use crate::icons::IconCache;
use crate::render::{self, Hit, Layout};
use crate::{
    AnnotationColour, EditRequest, MagnifierSettings, OverlayAction, OverlayError, SelectRequest,
    StrokeSize, Theme,
};

pub(crate) const CLASS_NAME: &str = "SnaplineOverlayWindow";
const SNAP_TIMER_ID: usize = 1;
const SNAP_DEADLINE_MS: u32 = 30;

pub(crate) struct ScreenSource {
    pub monitor: Monitor,
    pub scale: f32,
    pub origin: Point,
    pub size: Size,
    pub image: Image,
}

pub(crate) struct WindowState {
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
    pub cursor: Option<Point>,
    pub pointer_monitor: usize,
    pub magnifier: MagnifierSettings,
    pub full_screen_crosshair: bool,
    pub theme: Theme,
    pub tool: Tool,
    pub colour: AnnotationColour,
    pub stroke: StrokeSize,
    pub pending_action: Option<OverlayAction>,
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

pub(crate) struct Context {
    pub windows: Vec<WindowState>,
    pub scene: Scene,
    pub gpu: Gpu,
    pub icons: IconCache,
    pub layout: Layout,
    pub outcome: Option<Outcome>,
    previous_pointer_monitor: usize,
    class_name: Vec<u16>,
    instance: HINSTANCE,
    sources: Vec<(ScreenSource, bool)>,
    snap: SnapFeed,
    snap_worker: Option<uia::ControlSnapWorker>,
    snap_generation: u64,
    snap_pending: Option<u64>,
    snap_timer: Option<HWND>,
    tracking_mouse: bool,
}

impl Context {
    pub(crate) fn build_select(req: SelectRequest) -> Result<Self, OverlayError> {
        let SelectRequest {
            desktop,
            annotate,
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
        Self::build(
            sources,
            annotate,
            false,
            snap,
            magnifier,
            full_screen_crosshair,
            theme,
        )
    }

    pub(crate) fn build_edit(req: EditRequest) -> Result<Self, OverlayError> {
        let size = Size::new(
            Px(i32::try_from(req.image.width()).unwrap_or(i32::MAX)),
            Px(i32::try_from(req.image.height()).unwrap_or(i32::MAX)),
        );
        let monitor = Monitor {
            id: 0,
            name: String::from("Edit"),
            bounds: Rect::from_origin_size(Point::ORIGIN, size),
            work_area: Rect::from_origin_size(Point::ORIGIN, size),
            dpi: snapline_core::geometry::DpiScale::ONE,
            primary: true,
        };
        let sources = vec![(
            ScreenSource {
                monitor,
                scale: 1.0,
                origin: Point::ORIGIN,
                size,
                image: req.image,
            },
            false,
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
        let scene = Scene {
            overlay,
            annotate,
            editing,
            cursor: None,
            pointer_monitor: 0,
            magnifier,
            full_screen_crosshair,
            theme,
            tool,
            colour: AnnotationColour::Yellow,
            stroke: StrokeSize::default(),
            pending_action: None,
        };
        // SAFETY: returns the module handle of this process without borrowing any memory.
        let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .map_err(|error| OverlayError::Device(error.to_string()))?;
        let class_name: Vec<u16> = CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let context = Self {
            windows: Vec::new(),
            scene,
            gpu,
            icons: IconCache::new(),
            layout: Layout::default(),
            outcome: None,
            previous_pointer_monitor: 0,
            class_name,
            instance: HINSTANCE(instance.0),
            sources,
            snap,
            snap_worker,
            snap_generation: 0,
            snap_pending: None,
            snap_timer: None,
            tracking_mouse: false,
        };
        Ok(context)
    }

    pub(crate) fn activate(&mut self) -> Result<(), OverlayError> {
        self.register_class()?;
        self.open()?;
        self.feed_windows();
        self.layout = render::layout(&self.windows, &self.scene, &mut self.gpu);
        self.redraw();
        self.show();
        Ok(())
    }

    fn feed_windows(&mut self) {
        let exclude: Vec<HWND> = self.windows.iter().map(|window| window.hwnd).collect();
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
                    // SAFETY: the window was created above and is owned by this thread.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    return Err(error);
                }
            };
            let frozen = match self.gpu.upload_image(&source.image) {
                Ok(frozen) => frozen,
                Err(error) => {
                    // SAFETY: the window was created above and is owned by this thread.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    return Err(error);
                }
            };
            self.windows.push(WindowState {
                hwnd,
                source,
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

    fn index_of(&self, hwnd: HWND) -> Option<usize> {
        self.windows.iter().position(|window| window.hwnd == hwnd)
    }

    pub(crate) fn redraw(&mut self) {
        for index in 0..self.windows.len() {
            self.redraw_window(index);
        }
    }

    fn redraw_window(&mut self, index: usize) {
        let window = &self.windows[index];
        let scene = &self.scene;
        let layout = &self.layout;
        let gpu = &mut self.gpu;
        let icons = &mut self.icons;
        render::draw_window(gpu, icons, window, index, scene, layout);
    }

    fn redraw_active(&mut self) {
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
        if self.scene.live_rect().is_some() {
            mask.fill(true);
        }
        if let Some(toolbar) = &self.layout.toolbar {
            mask[toolbar.monitor] = true;
        }
        if let Some(hint) = &self.layout.hint {
            mask[hint.monitor] = true;
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
    }

    fn refresh(&mut self) {
        self.layout = render::layout(&self.windows, &self.scene, &mut self.gpu);
        self.redraw_active();
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
        let monitor = if self.scene.editing {
            None
        } else {
            self.monitor_id(selection.rect.center())
        };
        self.outcome = Some(Outcome::Finish {
            action,
            selection: selection.rect,
            monitor,
        });
        // SAFETY: posts a quit message to this thread's queue.
        unsafe { PostQuitMessage(0) };
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
        dirty || self.outcome.is_some()
    }

    fn pointer(&self, index: usize, lparam: LPARAM) -> Point {
        let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
        let origin = self.windows[index].source.origin;
        Point::new(Px(x + origin.x.get()), Px(y + origin.y.get()))
    }

    fn on_toolbar(&mut self, point: Point) -> bool {
        let Some(toolbar) = &self.layout.toolbar else {
            return false;
        };
        if !toolbar.rect.contains(point) {
            return false;
        }
        let Some(hit) = toolbar
            .buttons
            .iter()
            .find(|button| button.rect.contains(point))
            .map(|button| button.hit)
        else {
            return true;
        };
        match hit {
            Hit::Tool(tool) => {
                self.scene.tool = tool;
                self.apply(Input::SetTool(tool));
            }
            Hit::Colour(colour) => {
                self.scene.colour = colour;
            }
            Hit::Stroke(stroke) => {
                self.scene.stroke = stroke;
            }
            Hit::Undo | Hit::Redo => {}
            Hit::Copy => {
                self.scene.pending_action = Some(OverlayAction::Copy);
                self.apply(Input::Copy);
            }
            Hit::Save => {
                self.scene.pending_action = Some(OverlayAction::Save);
                self.apply(Input::Save);
            }
            Hit::Pin => {
                self.scene.pending_action = Some(OverlayAction::Pin);
                self.apply(Input::Upload);
            }
            Hit::Upload => {
                self.scene.pending_action = Some(OverlayAction::Upload);
                self.apply(Input::Upload);
            }
            Hit::Cancel => {
                self.apply(Input::Cancel);
            }
        }
        true
    }

    fn on_key(&mut self, key: VIRTUAL_KEY) {
        let annotating = matches!(&self.scene.overlay.state, State::Annotating { .. });
        let control = key_down(VK_CONTROL);
        if control {
            match key {
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
                // SAFETY: releases any capture held by this thread.
                unsafe {
                    let _ = ReleaseCapture();
                }
                self.apply(Input::Escape);
            }
            VK_RETURN if annotating => {
                self.scene.pending_action = Some(OverlayAction::Upload);
                self.apply(Input::Upload);
            }
            VK_TAB => {
                self.apply(Input::Tab);
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
                if !self.scene.editing {
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
                if self.on_toolbar(point) {
                    self.refresh();
                    return LRESULT(0);
                }
                if !self.scene.editing {
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
                if !self.scene.editing {
                    self.apply(Input::Release(point));
                }
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
                self.apply(Input::RightClick);
                self.refresh();
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                let key = VIRTUAL_KEY(wparam.0 as u16);
                self.on_key(key);
                if self.outcome.is_none() {
                    self.refresh();
                }
                return LRESULT(0);
            }
            WM_SYSKEYDOWN => {
                let key = VIRTUAL_KEY(wparam.0 as u16);
                if matches!(key, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) {
                    self.on_key(key);
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

#[cfg(test)]
mod tests {
    use snapline_core::overlay::{Input, Overlay, SnapMode};

    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
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
        let (next, _) = overlay.on(Input::Move(point(120, 120)));
        overlay = next;
        assert_eq!(overlay.snap.hovered, Some(rect(100, 100, 50, 50)));
        feed.clear_control();
        overlay.pointer = None;
        let (next, _) = overlay.on(Input::SnapCandidates(feed.set()));
        overlay = next;
        assert_eq!(overlay.snap.hovered, None);
    }
}
