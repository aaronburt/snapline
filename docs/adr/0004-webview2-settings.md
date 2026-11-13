# ADR 0004: WebView2 settings

> Superseded by [0007](0007-slint-only.md): there is no WebView2 any more.

Status: Accepted

## Context

Every Snapline webview is a Tauri window backed by WebView2. The window manager creates the quick panel and main window on demand and destroys them on close. After the last one closes, a 15 second linger timer runs before the app expects every WebView2 process to exit. Release builds must not expose developer tools, browser accelerator keys, page zoom or the default context menu. The extra memory that WebView2 costs must be measured rather than assumed.

## Decision

All windows use one WebView2 data directory, `%LOCALAPPDATA%\SnapLine\WebView2`, and the same additional browser arguments. Tauri's own arguments are kept because supplying the value replaces Tauri's default. The final additional arguments are:

`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-component-update`

In release builds, `ICoreWebView2Settings::put_AreDevToolsEnabled(false)`, `put_AreDefaultContextMenusEnabled(false)` and `put_IsZoomControlEnabled(false)` are set, followed by `ICoreWebView2Settings3::put_AreBrowserAcceleratorKeysEnabled(false)`.

The optional `ICoreWebView2_19::put_MemoryUsageTargetLevel(LOW)` on a minimised main window was measured and is not used, because it saved less than the required 10 MB. The code was removed.

## Gate measurement

Machine: the development PC, 2026-11-09, release build, one main window, Windows 11.

| State | App private working set | Snapline WebView2 processes | App plus WebView2 |
|-------|-------------------------|----------------------------|-------------------|
| Tray idle, no window yet | 1,605,632 bytes | 0 | 1,605,632 bytes |
| Main window open | 3,665,920 bytes | 6 | 65,814,528 bytes |
| 25 seconds after close | 3,600,384 bytes | 0 | 3,600,384 bytes |

The gate passes: zero Snapline WebView2 processes remained after the linger period plus 10 seconds, and the app's private working set was 1,994,752 bytes above the first idle figure, within the 3 MB allowance.

The initial test with the Tauri argument set alone measured 1,605,632 bytes idle, 69,156,864 bytes with the main window open and 3,674,112 bytes after close. Adding `--disable-background-networking` measured 70,365,184 bytes open, so it was rejected. Adding `--disable-component-update` measured 65,814,528 bytes open, a saving of 3,342,336 bytes, so it was kept.

With the memory target level enabled, the minimised main window measured 55,427,072 bytes against 65,228,800 bytes normal, a saving of 9,801,728 bytes. Without it, the minimised window measured 64,933,888 bytes against 65,683,456 bytes, a saving of 749,568 bytes. The target level therefore added 9,052,160 bytes of savings, below the 10 MB threshold, so it was removed.

## Consequences

The app holds no WebView2 process at idle and returns to within 2 MB of its first idle figure after every window closes. The browser accelerator and context menu settings make the release webview behave like an application rather than a browser. Disabling component updates means WebView2 components such as the PDF viewer and text-to-speech do not update through the WebView2 runtime; Snapline does not use them. The default context menu is disabled everywhere, including text inputs; the intended text-input exception needs a custom menu if it is required later.
