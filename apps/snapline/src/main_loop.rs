use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, PoisonError};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassExW, TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COPYDATA, WM_DESTROY, WNDCLASSEXW,
};

pub const CLASS_NAME: PCWSTR = w!("SnaplineMainMessageWindow");
pub const COPY_DATA_ARGUMENTS: usize = 0x534C_4152;
const WM_TASKS: u32 = WM_APP + 7;

type Task = Box<dyn FnOnce() + Send>;
type ArgumentsHandler = Box<dyn Fn(Vec<String>) + Send + Sync>;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static TASKS: Mutex<VecDeque<Task>> = Mutex::new(VecDeque::new());
static ARGUMENTS: Mutex<Option<ArgumentsHandler>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn init(on_second_launch: impl Fn(Vec<String>) + Send + Sync + 'static) -> Result<(), String> {
    *lock(&ARGUMENTS) = Some(Box::new(on_second_launch));
    // SAFETY: no module name asks for the handle of the running executable.
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
    let class = WNDCLASSEXW {
        cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: the class description and its static class name outlive the call.
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err("The main message window class could not be registered.".to_owned());
    }
    // SAFETY: the class was just registered and every other argument is a valid default.
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            w!("Snapline"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    WINDOW.store(window.0 as isize, Ordering::SeqCst);
    Ok(())
}

fn window() -> Option<HWND> {
    let raw = WINDOW.load(Ordering::SeqCst);
    (raw != 0).then_some(HWND(raw as *mut std::ffi::c_void))
}

pub fn post(task: impl FnOnce() + Send + 'static) {
    lock(&TASKS).push_back(Box::new(task));
    let Some(window) = window() else {
        return;
    };
    // SAFETY: the handle is the live message window created by `init`; posting is allowed from any thread.
    if let Err(error) = unsafe { PostMessageW(Some(window), WM_TASKS, WPARAM(0), LPARAM(0)) } {
        tracing::error!(%error, "could not wake the main thread");
    }
}

pub fn quit() {
    post(|| {
        // SAFETY: this task runs on the thread that owns the message loop.
        unsafe { PostQuitMessage(0) };
    });
}

pub fn run() {
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is a live MSG owned by this thread.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 <= 0 {
            break;
        }
        // SAFETY: `message` was filled in by GetMessageW above.
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if let Some(window) = window() {
        // SAFETY: the window belongs to this thread and is destroyed once, here.
        let _ = unsafe { DestroyWindow(window) };
    }
}

fn drain_tasks() {
    loop {
        let task = lock(&TASKS).pop_front();
        match task {
            Some(task) => task(),
            None => break,
        }
    }
}

fn forwarded_arguments(lparam: LPARAM) -> Option<Vec<String>> {
    let data = lparam.0 as *const COPYDATASTRUCT;
    if data.is_null() {
        return None;
    }
    // SAFETY: for WM_COPYDATA the sender guarantees `lparam` points at a COPYDATASTRUCT that stays valid during the message.
    let data = unsafe { &*data };
    if data.dwData != COPY_DATA_ARGUMENTS || data.lpData.is_null() {
        return None;
    }
    let length = usize::try_from(data.cbData).ok()? / 2;
    // SAFETY: cbData bytes at lpData are readable for the message, and u16 reads are unaligned-safe through the copy below.
    let units: Vec<u16> = (0..length)
        .map(|index| unsafe { std::ptr::read_unaligned(data.lpData.cast::<u16>().add(index)) })
        .collect();
    Some(
        String::from_utf16_lossy(&units)
            .split('\0')
            .filter(|argument| !argument.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TASKS => {
            drain_tasks();
            LRESULT(0)
        }
        WM_COPYDATA => {
            if let Some(arguments) = forwarded_arguments(lparam) {
                if let Some(handler) = lock(&ARGUMENTS).as_ref() {
                    handler(arguments);
                }
                return LRESULT(1);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // SAFETY: the window procedure runs on the thread that owns the message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        // SAFETY: the arguments are exactly what the system passed to this window procedure.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
