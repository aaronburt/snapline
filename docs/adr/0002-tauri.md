# ADR 0002: Tauri with on-demand webviews and a native overlay

Status: Accepted

## Context

The v2 design is HTML. Reproducing it in a web engine is faster and more accessible than drawing every normal window in a native toolkit. A webview per window costs tens of megabytes and starts a family of WebView2 processes. Capture and annotation must not wait for that engine.

## Decision

Normal windows are Tauri 2 windows backed by WebView2. The Rust core starts with no window and holds no webview at startup. The quick panel and main window are created only when requested and destroyed, not hidden, when they close. After the last webview closes the app starts a 15 second linger timer, then releases the WebView2 environment so its processes exit. The capture overlay, annotation toolbar, pin, ruler, colour picker and recording border are native Win32, Direct2D, DirectComposition and DirectWrite. Images never cross the IPC boundary as JSON or base64; thumbnails use a custom URI scheme.

## Consequences

Each open window costs the memory of several Edge processes. Section 6 budgets therefore replace the old 60 MB open-window figure for webviews, while idle budgets are unchanged. Opening a window cold costs a few hundred milliseconds. The app contains two UI technologies. If WebView2 processes fail to exit after the linger period, the gate in task 1.1 stops the build and Aaron chooses a fallback.
