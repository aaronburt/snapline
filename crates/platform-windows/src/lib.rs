#![doc = "Win32 and WinRT adapters for Snapline's core ports. This crate owns monitor and capture backends, the clipboard, Credential Manager, hotkey input, PrtSc takeover, autostart, toasts, sounds, WIC, shell operations, dialogs, proxy lookup, OCR, scrolling, FFmpeg process control, audio capture and known_hosts. It may use unsafe code, each FFI call wrapped with a SAFETY line."]

pub mod autostart;
pub mod clipboard;
pub mod credentials;
pub mod dialogs;
pub mod input;
pub mod notify;
pub mod prtsc;
pub mod proxy;
pub mod sound;
pub mod theme;
pub mod wic;
