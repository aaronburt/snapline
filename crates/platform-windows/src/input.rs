use std::cell::Cell;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};
use snapline_core::actions::ActionId;
use snapline_core::chord::{Chord, Key, Modifiers};
use snapline_core::hotkey_plan::{Blocked, Owner};
use thiserror::Error;
use windows::core::{w, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, ERROR_HOTKEY_ALREADY_REGISTERED, HINSTANCE, HWND,
    LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL,
    MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};

pub fn double_click_interval() -> std::time::Duration {
    std::time::Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }))
}
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PostQuitMessage, PostThreadMessageW, RegisterClassW, SetWindowLongPtrW,
    TranslateMessage, GWLP_USERDATA, MSG, PBT_APMRESUMEAUTOMATIC, WM_APP, WM_HOTKEY, WM_NCCREATE,
    WM_POWERBROADCAST, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

const WINDOW_CLASS: PCWSTR = w!("SnaplineInputWindow");
const WAKE_MESSAGE: u32 = WM_APP + 1;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InputError {
    #[error("The hotkey thread could not start.")]
    Start,
    #[error("The hotkey thread is not running.")]
    Stopped,
    #[error("This key cannot be used as a global hotkey.")]
    UnsupportedKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Action(ActionId),
    Resumed,
    ThemeChanged,
}

pub enum InputCmd {
    Rebind(Vec<(Owner, Chord)>),
    Pause,
    Resume,
    Shutdown,
}

struct Registered {
    id: i32,
    owner: Owner,
}

#[derive(Default)]
struct BindState {
    entries: Vec<(Owner, Chord)>,
    registered: Vec<Registered>,
    paused: bool,
}

type RebindReply = Sender<Result<Vec<Blocked>, InputError>>;

struct Context {
    events: Sender<InputEvent>,
    commands: Receiver<InputCmd>,
    pending: Arc<Mutex<Option<RebindReply>>>,
    hwnd: Cell<HWND>,
    state: Mutex<BindState>,
}

#[derive(Clone)]
pub struct InputHandle {
    thread_id: u32,
    commands: Sender<InputCmd>,
    pending: Arc<Mutex<Option<RebindReply>>>,
    gate: Arc<Mutex<()>>,
}

impl InputHandle {
    pub fn rebind(&self, bindings: Vec<(Owner, Chord)>) -> Result<Vec<Blocked>, InputError> {
        let _gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let (reply, received) = bounded(1);
        {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            *pending = Some(reply);
        }
        if let Err(error) = self.send(InputCmd::Rebind(bindings)) {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            *pending = None;
            return Err(error);
        }
        received.recv().map_err(|_| InputError::Stopped)?
    }

    pub fn pause(&self) -> Result<(), InputError> {
        self.send(InputCmd::Pause)
    }

    pub fn resume(&self) -> Result<(), InputError> {
        self.send(InputCmd::Resume)
    }

    pub fn shutdown(&self) {
        let _ = self.send(InputCmd::Shutdown);
    }

    fn send(&self, command: InputCmd) -> Result<(), InputError> {
        self.commands
            .send(command)
            .map_err(|_| InputError::Stopped)?;
        // SAFETY: the thread id came from GetCurrentThreadId on the live input thread and no pointer is passed.
        unsafe { PostThreadMessageW(self.thread_id, WAKE_MESSAGE, WPARAM(0), LPARAM(0)) }
            .map_err(|_| InputError::Stopped)
    }
}

pub fn spawn() -> Result<(InputHandle, Receiver<InputEvent>), InputError> {
    let (events, received) = unbounded();
    let handle = start(events)?;
    Ok((handle, received))
}

pub fn spawn_with<F>(mut handler: F) -> Result<InputHandle, InputError>
where
    F: FnMut(InputEvent) + Send + 'static,
{
    let (events, received) = unbounded();
    let handle = start(events)?;
    std::thread::Builder::new()
        .name("snapline-input-events".to_owned())
        .spawn(move || {
            while let Ok(event) = received.recv() {
                handler(event);
            }
        })
        .map_err(|_| InputError::Start)?;
    Ok(handle)
}

pub fn start(events: Sender<InputEvent>) -> Result<InputHandle, InputError> {
    let (commands, received) = unbounded();
    let pending = Arc::new(Mutex::new(None));
    let (ready, ready_rx) = bounded(1);
    let thread_pending = Arc::clone(&pending);
    std::thread::Builder::new()
        .name("snapline-input".to_owned())
        .spawn(move || run(events, received, thread_pending, ready))
        .map_err(|_| InputError::Start)?;
    let thread_id = ready_rx.recv().map_err(|_| InputError::Start)??;
    Ok(InputHandle {
        thread_id,
        commands,
        pending,
        gate: Arc::new(Mutex::new(())),
    })
}

fn run(
    events: Sender<InputEvent>,
    commands: Receiver<InputCmd>,
    pending: Arc<Mutex<Option<RebindReply>>>,
    ready: Sender<Result<u32, InputError>>,
) {
    // SAFETY: GetCurrentThreadId takes no arguments and returns this thread's identifier.
    let thread_id = unsafe { GetCurrentThreadId() };
    let context = Box::into_raw(Box::new(Context {
        events,
        commands,
        pending,
        hwnd: Cell::new(HWND::default()),
        state: Mutex::new(BindState::default()),
    }));
    // SAFETY: the context outlives the window and is only used on this thread.
    let context_ref = unsafe { &*context };
    let hwnd = match create_window(context_ref) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready.send(Err(error));
            // SAFETY: the box was created by Box::into_raw above and is reclaimed exactly once here.
            drop(unsafe { Box::from_raw(context) });
            return;
        }
    };
    context_ref.hwnd.set(hwnd);
    if ready.send(Ok(thread_id)).is_err() {
        // SAFETY: the box was created by Box::into_raw above and is reclaimed exactly once here.
        drop(unsafe { Box::from_raw(context) });
        return;
    }
    let mut message = MSG::default();
    loop {
        // SAFETY: the message pointer refers to a live MSG and no window filter is used.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 <= 0 {
            break;
        }
        if message.hwnd.0.is_null() {
            match message.message {
                WM_HOTKEY => dispatch_hotkey(context_ref, message.wParam.0 as i32),
                WAKE_MESSAGE => drain(context_ref),
                _ => {}
            }
        } else {
            // SAFETY: the message came from GetMessageW and is dispatched to its own window procedure.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    unregister_all(context_ref);
    // SAFETY: the box was created by Box::into_raw above and is reclaimed exactly once here.
    drop(unsafe { Box::from_raw(context) });
}

fn create_window(context: &Context) -> Result<HWND, InputError> {
    // SAFETY: no module name is requested, so this returns the handle of the running executable.
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|_| InputError::Start)?;
    let instance = HINSTANCE(module.0);
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: WINDOW_CLASS,
        ..Default::default()
    };
    // SAFETY: the class pointer refers to a live WNDCLASSW for the duration of the call.
    let registered = unsafe { RegisterClassW(&class) };
    if registered == 0 {
        // SAFETY: GetLastError reads the calling thread's last error with no arguments.
        let error = unsafe { GetLastError() };
        if error != ERROR_CLASS_ALREADY_EXISTS {
            return Err(InputError::Start);
        }
    }
    // SAFETY: the class name is static and the context pointer stays alive for the whole thread.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            WINDOW_CLASS,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            Some(std::ptr::from_ref(context).cast::<c_void>()),
        )
    }
    .map_err(|_| InputError::Start)?;
    // SAFETY: the window handle is live and the context outlives it.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, std::ptr::from_ref(context) as isize) };
    Ok(hwnd)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the pointer was stored by create_window and is only read while the window exists.
    let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Context;
    if context.is_null() {
        // SAFETY: the default handler accepts any window message.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    // SAFETY: context came from a live Box owned by the input thread.
    let context = unsafe { &*context };
    match message {
        WM_POWERBROADCAST if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC => {
            let _ = context.events.send(InputEvent::Resumed);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            let _ = context.events.send(InputEvent::ThemeChanged);
            LRESULT(0)
        }
        WM_NCCREATE => LRESULT(1),
        _ => {
            // SAFETY: the default handler accepts any window message.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

fn drain(context: &Context) {
    loop {
        match context.commands.try_recv() {
            Ok(InputCmd::Rebind(bindings)) => {
                let result = apply_entries(context, bindings);
                let reply = context
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            Ok(InputCmd::Pause) => {
                let mut state = state(context);
                state.paused = true;
                unregister(&mut state);
            }
            Ok(InputCmd::Resume) => {
                let mut state = state(context);
                if state.paused {
                    state.paused = false;
                    register(&mut state);
                }
            }
            Ok(InputCmd::Shutdown) => {
                shutdown(context);
                return;
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                shutdown(context);
                return;
            }
        }
    }
}

fn shutdown(context: &Context) {
    unregister_all(context);
    let hwnd = context.hwnd.get();
    if !hwnd.0.is_null() {
        // SAFETY: the handle was created by this thread and is destroyed exactly once here.
        let _ = unsafe { DestroyWindow(hwnd) };
    }
    // SAFETY: PostQuitMessage takes a plain integer exit code.
    unsafe { PostQuitMessage(0) };
}

fn apply_entries(
    context: &Context,
    bindings: Vec<(Owner, Chord)>,
) -> Result<Vec<Blocked>, InputError> {
    let mut state = state(context);
    unregister(&mut state);
    state.entries = bindings;
    if state.paused {
        return Ok(Vec::new());
    }
    let mut blocked = Vec::new();
    for (index, (owner, chord)) in state.entries.clone().into_iter().enumerate() {
        let id = i32::try_from(index + 1).map_err(|_| InputError::Start)?;
        let modifiers = hot_modifiers(&chord);
        let key = virtual_key(chord.key)?;
        // SAFETY: no window handle is passed, so Windows posts WM_HOTKEY to this thread.
        match unsafe { RegisterHotKey(None, id, modifiers, key) } {
            Ok(()) => state.registered.push(Registered { id, owner }),
            Err(error)
                if error.code() == HRESULT::from_win32(ERROR_HOTKEY_ALREADY_REGISTERED.0) =>
            {
                tracing::info!(chord = %chord, "another app already owns this hotkey");
                blocked.push(Blocked::external(chord, owner));
            }
            Err(error) => {
                tracing::warn!(
                    chord = %chord,
                    error = %error,
                    "a hotkey could not be registered"
                );
                blocked.push(Blocked::external(chord, owner));
            }
        }
    }
    Ok(blocked)
}

fn state(context: &Context) -> MutexGuard<'_, BindState> {
    context.state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn dispatch_hotkey(context: &Context, id: i32) {
    let action = {
        let state = state(context);
        state
            .registered
            .iter()
            .find(|registered| registered.id == id)
            .map(|registered| ActionId::new(registered.owner.as_str()))
    };
    if let Some(action) = action {
        let _ = context.events.send(InputEvent::Action(action));
    }
}

fn unregister_all(context: &Context) {
    let mut state = state(context);
    unregister(&mut state);
}

fn unregister(state: &mut BindState) {
    for registered in state.registered.drain(..) {
        // SAFETY: the id was registered on this thread and is released exactly once here.
        let _ = unsafe { UnregisterHotKey(None, registered.id) };
    }
}

fn register(state: &mut BindState) {
    let entries = state.entries.clone();
    state.registered.clear();
    for (index, (owner, chord)) in entries.into_iter().enumerate() {
        let Ok(id) = i32::try_from(index + 1) else {
            continue;
        };
        let Ok(key) = virtual_key(chord.key) else {
            continue;
        };
        // SAFETY: no window handle is passed, so Windows posts WM_HOTKEY to this thread.
        if unsafe { RegisterHotKey(None, id, hot_modifiers(&chord), key) }.is_ok() {
            state.registered.push(Registered { id, owner });
        }
    }
}

fn hot_modifiers(chord: &Chord) -> HOT_KEY_MODIFIERS {
    let mut modifiers = MOD_NOREPEAT;
    if chord.modifiers.contains(Modifiers::CTRL) {
        modifiers = modifiers | MOD_CONTROL;
    }
    if chord.modifiers.contains(Modifiers::ALT) {
        modifiers = modifiers | MOD_ALT;
    }
    if chord.modifiers.contains(Modifiers::SHIFT) {
        modifiers = modifiers | MOD_SHIFT;
    }
    if chord.modifiers.contains(Modifiers::WIN) {
        modifiers = modifiers | MOD_WIN;
    }
    modifiers
}

pub fn virtual_key(key: Key) -> Result<u32, InputError> {
    let name = key.to_string();
    let value = match name.as_str() {
        "PrtSc" => 0x2C,
        "Pause" => 0x13,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "Up" => 0x26,
        "Down" => 0x28,
        "Left" => 0x25,
        "Right" => 0x27,
        other => {
            let bytes = other.as_bytes();
            if bytes.len() == 1 && bytes[0].is_ascii_uppercase() {
                u32::from(bytes[0])
            } else if bytes.len() == 1 && bytes[0].is_ascii_digit() {
                0x30 + u32::from(bytes[0] - b'0')
            } else if let Some(number) = other.strip_prefix('F') {
                let number: u32 = number.parse().map_err(|_| InputError::UnsupportedKey)?;
                0x70 + number.saturating_sub(1)
            } else {
                return Err(InputError::UnsupportedKey);
            }
        }
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};

    const KEYS: &[(&str, u32)] = &[
        ("PrtSc", 0x2C),
        ("Pause", 0x13),
        ("Insert", 0x2D),
        ("Delete", 0x2E),
        ("Home", 0x24),
        ("End", 0x23),
        ("PageUp", 0x21),
        ("PageDown", 0x22),
        ("Up", 0x26),
        ("Down", 0x28),
        ("Left", 0x25),
        ("Right", 0x27),
        ("A", 0x41),
        ("Z", 0x5A),
        ("0", 0x30),
        ("9", 0x39),
        ("F1", 0x70),
        ("F12", 0x7B),
        ("F24", 0x87),
    ];

    #[test]
    fn every_key_maps_to_its_virtual_key() {
        for (name, expected) in KEYS {
            let key = Key::parse(name).expect("key");
            assert_eq!(virtual_key(key), Ok(*expected), "{name}");
        }
    }

    #[test]
    fn modifiers_add_the_windows_flags_and_no_repeat() {
        let chord = Chord::parse("Ctrl+Alt+Shift+Win+F5").expect("chord");
        let modifiers = hot_modifiers(&chord);
        assert!(modifiers.contains(MOD_CONTROL));
        assert!(modifiers.contains(MOD_ALT));
        assert!(modifiers.contains(MOD_SHIFT));
        assert!(modifiers.contains(MOD_WIN));
        assert!(modifiers.contains(MOD_NOREPEAT));
    }

    #[test]
    fn an_unbound_modifier_set_is_empty() {
        let chord = Chord::parse("PrtSc").expect("chord");
        let modifiers = hot_modifiers(&chord);
        assert!(!modifiers.contains(MOD_CONTROL));
        assert!(!modifiers.contains(MOD_ALT));
        assert!(!modifiers.contains(MOD_SHIFT));
        assert!(!modifiers.contains(MOD_WIN));
        assert!(modifiers.contains(MOD_NOREPEAT));
    }

    #[test]
    #[ignore = "desktop"]
    fn registers_and_unregisters_a_throwaway_chord() {
        let (events, _received) = unbounded();
        let input = start(events).expect("input thread");
        let chord = Chord::parse("Ctrl+Alt+Shift+F23").expect("chord");
        let blocked = input
            .rebind(vec![(Owner::new("SnapLineTest:register"), chord)])
            .expect("rebind");
        assert!(blocked.is_empty());
        input.pause().expect("pause");
        input.resume().expect("resume");
        input.shutdown();
    }

    #[test]
    #[ignore = "desktop"]
    fn detects_a_chord_registered_by_another_caller() {
        let chord = Chord::parse("Ctrl+Alt+Shift+F22").expect("chord");
        let modifiers = hot_modifiers(&chord);
        let key = virtual_key(chord.key).expect("virtual key");
        // SAFETY: the throwaway id is released before the test returns.
        unsafe { RegisterHotKey(None, 7_722, modifiers, key) }.expect("external registration");
        let (events, _received) = unbounded();
        let input = start(events).expect("input thread");
        let blocked = input
            .rebind(vec![(Owner::new("SnapLineTest:external"), chord)])
            .expect("rebind");
        assert_eq!(
            blocked,
            vec![Blocked::external(
                chord,
                Owner::new("SnapLineTest:external")
            )]
        );
        input.shutdown();
        // SAFETY: this releases the registration made above.
        let _ = unsafe { UnregisterHotKey(None, 7_722) };
    }
}
