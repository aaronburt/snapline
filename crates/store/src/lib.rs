#![doc = "Settings and history persistence for Snapline. This crate owns settings.toml loading, saving, migration and crash-safe writes, plus the SQLite history database and its embedded migrations. It must not perform screen capture, networking, UI work or unsafe code."]
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod history;
pub mod io;
pub mod settings;
