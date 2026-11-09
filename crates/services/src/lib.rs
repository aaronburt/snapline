#![doc = "Frontend-neutral application layer for Snapline. This crate owns Services, the worker and uploader threads, Job and AppEvent, the action router, one module per IPC area and the DTO types shared with the webview. It must never depend on Tauri or contain unsafe code."]
#![forbid(unsafe_code)]
