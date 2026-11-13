# 0007 Slint for every window

Status: accepted.

## Context

The main window moved to Slint first. Tauri and a WebView2 renderer stayed behind for the fallback window and the web Quick Panel, so the app still carried a webview runtime, a Svelte build, an IPC layer and a second copy of every page.

## Decision

Every window is Slint. Tauri, WebView2 and the Svelte frontend are removed.

- The main window and the Quick Panel live in `crates/ui-slint` and share its tokens, widgets and services.
- The tray icon uses the `tray-icon` crate. The main thread runs a plain Win32 message loop (`main_loop.rs`) that also receives work posted from other threads and the arguments of a second launch.
- A second launch finds the first instance's hidden message window and hands over its command line with `WM_COPYDATA` (`single_instance.rs`).
- Capture overlays, the toolbar, pin, ruler and recording border stay native Direct2D.
- The installer is an NSIS script, `apps/snapline/installer/snapline.nsi`, built by `cargo xtask package`. It keeps the current-user install, the Start menu shortcut with the app user model id, and the `--uninstall-cleanup` step.
- The exe icon, manifest and version information are embedded by `build.rs` from `snapline.rc`.

## Consequences

- There is no webview process, no `ui` folder and no Node tooling.
- `cargo xtask package` needs `makensis` on the build machine.
- The Quick Panel is a frameless Slint window placed beside the tray icon. It has no drop shadow.
