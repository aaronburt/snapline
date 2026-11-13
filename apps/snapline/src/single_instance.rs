use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, SendMessageTimeoutW, HWND_MESSAGE, SMTO_ABORTIFHUNG, WM_COPYDATA,
};

use crate::main_loop::{CLASS_NAME, COPY_DATA_ARGUMENTS};

const HAND_OFF_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT_MS: u32 = 3_000;

pub struct Guard(HANDLE);

// SAFETY: a mutex handle may be held and closed from any thread.
unsafe impl Send for Guard {}
unsafe impl Sync for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateMutexW and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub enum Outcome {
    First(Guard),
    HandedOff,
}

pub fn acquire(arguments: &[String]) -> Outcome {
    // SAFETY: the name is a static wide string and no security attributes are passed.
    let mutex = unsafe { CreateMutexW(None, true, w!("Local\\Snapline.SingleInstance")) };
    let Ok(mutex) = mutex else {
        tracing::warn!("the instance lock could not be created; starting anyway");
        return Outcome::First(Guard(HANDLE::default()));
    };
    // SAFETY: GetLastError reads this thread's last error right after CreateMutexW.
    let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if !exists {
        return Outcome::First(Guard(mutex));
    }
    // SAFETY: the handle was just returned by CreateMutexW.
    let _ = unsafe { CloseHandle(mutex) };
    if hand_off(arguments) {
        Outcome::HandedOff
    } else {
        tracing::warn!("the running Snapline did not answer; starting a second one");
        Outcome::First(Guard(HANDLE::default()))
    }
}

fn hand_off(arguments: &[String]) -> bool {
    let payload: Vec<u16> = arguments
        .iter()
        .flat_map(|argument| argument.encode_utf16().chain(std::iter::once(0)))
        .collect();
    let data = COPYDATASTRUCT {
        dwData: COPY_DATA_ARGUMENTS,
        cbData: u32::try_from(payload.len() * 2).unwrap_or(0),
        lpData: payload.as_ptr().cast_mut().cast(),
    };
    let started = Instant::now();
    while started.elapsed() < HAND_OFF_TIMEOUT {
        // SAFETY: the class name is static; the search only reads window handles.
        let target = unsafe { FindWindowExW(Some(HWND_MESSAGE), None, CLASS_NAME, None) };
        if let Ok(target) = target {
            let mut result = 0usize;
            // SAFETY: `data` and `payload` outlive the call, and SendMessageTimeoutW copies them for WM_COPYDATA.
            let sent = unsafe {
                SendMessageTimeoutW(
                    target,
                    WM_COPYDATA,
                    WPARAM(0),
                    LPARAM(std::ptr::from_ref(&data) as isize),
                    SMTO_ABORTIFHUNG,
                    SEND_TIMEOUT_MS,
                    Some(&mut result),
                )
            };
            if sent.0 != 0 {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}
